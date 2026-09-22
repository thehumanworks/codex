//! Bounded in-process event routing and shutdown drains.
//!
//! Adapted from upstream's lossless-events branch while retaining current typed
//! envelopes, connection-scoped errors, and delivery acknowledgements.

use crate::error_code::OVERLOADED_ERROR_CODE;
use crate::error_code::internal_error;
use crate::in_process::IN_PROCESS_CONNECTION_ID;
use crate::in_process::InProcessEventDelivery;
use crate::in_process::InProcessServerEvent;
use crate::in_process::PendingClientRequestResponse;
use crate::in_process::server_notification_requires_delivery;
use crate::outgoing_message::OutgoingMessage;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::QueuedOutgoingMessage;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use std::collections::HashMap;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_util::task::AbortOnDropHandle;
use tracing::warn;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum DeliveryPhase {
    Running,
    Draining,
}

pub(crate) async fn route_queued_message(
    queued_message: QueuedOutgoingMessage,
    pending_request_responses: &mut HashMap<RequestId, oneshot::Sender<PendingClientRequestResponse>>,
    event_tx: &mpsc::Sender<InProcessServerEvent>,
    outgoing: &OutgoingMessageSender,
    event_delivery: InProcessEventDelivery,
    phase: DeliveryPhase,
) -> bool {
    match queued_message.message {
        OutgoingMessage::Response(response) => {
            if let Some(response_tx) = pending_request_responses.remove(&response.id) {
                let result = serde_json::to_value(response.result).map_err(|error| {
                    internal_error(format!("failed to serialize response: {error}"))
                });
                let _ = response_tx.send(result);
            } else {
                warn!(request_id = ?response.id, "dropping unmatched in-process response");
            }
        }
        OutgoingMessage::Error(error) => {
            if let Some(response_tx) = pending_request_responses.remove(&error.id) {
                let _ = response_tx.send(Err(error.error));
            } else {
                warn!(request_id = ?error.id, "dropping unmatched in-process error response");
            }
        }
        OutgoingMessage::Request(request) => {
            if phase == DeliveryPhase::Draining {
                // New approvals cannot be answered after client admission closes.
                // Reject them explicitly rather than stalling processor cleanup.
                outgoing.notify_client_error(
                    IN_PROCESS_CONNECTION_ID,
                    request.id().clone(),
                    internal_error("in-process app-server runtime is shutting down"),
                ).await;
            } else if event_delivery == InProcessEventDelivery::Lossless {
                if let Err(error) = event_tx
                    .send(InProcessServerEvent::ServerRequest(Box::new(request)))
                    .await
                {
                    let InProcessServerEvent::ServerRequest(request) = error.0 else {
                        unreachable!("only a server request was sent");
                    };
                    outgoing.notify_client_error(
                        IN_PROCESS_CONNECTION_ID,
                        request.id().clone(),
                        internal_error("in-process server request consumer is closed"),
                    ).await;
                    return false;
                }
            } else if let Err(error) = event_tx
                .try_send(InProcessServerEvent::ServerRequest(Box::new(request)))
            {
                let (error, event, consumer_open) = match error {
                    mpsc::error::TrySendError::Full(event) => (
                        JSONRPCErrorError {
                            code: OVERLOADED_ERROR_CODE,
                            message: "in-process server request queue is full".to_string(),
                            data: None,
                        },
                        event,
                        true,
                    ),
                    mpsc::error::TrySendError::Closed(event) => (
                        internal_error("in-process server request consumer is closed"),
                        event,
                        false,
                    ),
                };
                let InProcessServerEvent::ServerRequest(request) = event else {
                    unreachable!("only a server request was sent");
                };
                outgoing.notify_client_error(
                    IN_PROCESS_CONNECTION_ID,
                    request.id().clone(),
                    error,
                ).await;
                if !consumer_open {
                    return false;
                }
            }
        }
        OutgoingMessage::AppServerNotification(envelope) => {
            let notification = envelope.notification;
            if event_delivery == InProcessEventDelivery::Lossless
                || server_notification_requires_delivery(&notification)
            {
                if event_tx
                    .send(InProcessServerEvent::ServerNotification(Box::new(notification)))
                    .await
                    .is_err()
                {
                    return false;
                }
            } else if let Err(error) = event_tx
                .try_send(InProcessServerEvent::ServerNotification(Box::new(notification)))
            {
                match error {
                    mpsc::error::TrySendError::Full(_) => {
                        warn!("dropping in-process server notification (queue full)");
                        // Do not acknowledge an ordinary notification we dropped.
                        return true;
                    }
                    mpsc::error::TrySendError::Closed(_) => return false,
                }
            }
        }
    }
    if let Some(write_complete_tx) = queued_message.write_complete_tx {
        let _ = write_complete_tx.send(());
    }
    true
}

pub(crate) async fn drain_writer_until_task_finishes(
    task: &mut AbortOnDropHandle<()>,
    writer_rx: &mut mpsc::Receiver<QueuedOutgoingMessage>,
    pending: &mut HashMap<RequestId, oneshot::Sender<PendingClientRequestResponse>>,
    event_tx: &mpsc::Sender<InProcessServerEvent>,
    outgoing: &OutgoingMessageSender,
    event_delivery: InProcessEventDelivery,
) -> IoResult<()> {
    loop {
        tokio::select! {
            biased;
            result = &mut *task => return result.map_err(IoError::other),
            message = writer_rx.recv() => {
                let Some(message) = message else {
                    return task.await.map_err(IoError::other);
                };
                if !route_queued_message(
                    message, pending, event_tx, outgoing, event_delivery, DeliveryPhase::Draining,
                ).await {
                    return Err(IoError::new(ErrorKind::BrokenPipe, "event consumer closed during drain"));
                }
            }
        }
    }
}

pub(crate) async fn drain_writer(
    writer_rx: &mut mpsc::Receiver<QueuedOutgoingMessage>,
    pending: &mut HashMap<RequestId, oneshot::Sender<PendingClientRequestResponse>>,
    event_tx: &mpsc::Sender<InProcessServerEvent>,
    outgoing: &OutgoingMessageSender,
    event_delivery: InProcessEventDelivery,
) -> IoResult<()> {
    while let Some(message) = writer_rx.recv().await {
        if !route_queued_message(
            message, pending, event_tx, outgoing, event_delivery, DeliveryPhase::Draining,
        ).await {
            return Err(IoError::new(ErrorKind::BrokenPipe, "event consumer closed during drain"));
        }
    }
    Ok(())
}
