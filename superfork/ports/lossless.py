#!/usr/bin/env python3
"""Port the pinned lossless-events source to the verified storage checkpoint.

Exact anchors guard the current request-cancellation, response-envelope, and
analytics-aware shutdown contracts; unknown baselines fail instead of guessing.
"""

from pathlib import Path

root = Path.cwd()
p = root / "codex-rs/app-server/src/in_process.rs"
s = p.read_text()


def replace(old, new, count=1):
    global s
    assert s.count(old) == count, (s.count(old), old[:100])
    s = s.replace(old, new)


replace(
    "use crate::outgoing_message::OutgoingMessage;",
    "use crate::in_process_event_delivery::DeliveryPhase;\nuse crate::in_process_event_delivery::drain_writer;\nuse crate::in_process_event_delivery::drain_writer_until_task_finishes;\nuse crate::in_process_event_delivery::route_queued_message;",
)
replace(
    "use tokio::time::timeout;",
    "use tokio::time::timeout;\nuse tokio::time::timeout_at;\nuse tokio::time::Instant;\nuse tokio_util::task::AbortOnDropHandle;",
)
replace("const IN_PROCESS_CONNECTION_ID:", "pub(crate) const IN_PROCESS_CONNECTION_ID:")
replace(
    "type PendingClientRequestResponse =",
    "pub(crate) type PendingClientRequestResponse =",
)
replace(
    "fn server_notification_requires_delivery(",
    "pub(crate) fn server_notification_requires_delivery(",
)
replace(
    "    thread_store: Option<Arc<dyn ThreadStore>>,\n}",
    "    thread_store: Option<Arc<dyn ThreadStore>>,\n    event_delivery: InProcessEventDelivery,\n}",
)
replace(
    """        self.thread_store = Some(thread_store);
        self
    }
}""",
    """        self.thread_store = Some(thread_store);
        self
    }

    /// Select how the bounded event stream handles a slow consumer.
    pub fn with_event_delivery(mut self, event_delivery: InProcessEventDelivery) -> Self {
        self.event_delivery = event_delivery;
        self
    }
}

/// Backpressure policy for the in-process event stream, not network transports.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InProcessEventDelivery {
    /// Preserve the default policy: ordinary notifications may be dropped under load.
    #[default]
    BestEffort,
    /// Preserve server events in order by waiting for bounded queue capacity.
    ///
    /// The host must consume events concurrently with requests. This is not durable
    /// storage or an exactly-once processing guarantee. Forced shutdown can still
    /// discard events and is reported as an error.
    Lossless,
}""",
)
replace(
    """    Shutdown {
        done_tx: oneshot::Sender<()>,
    },
""",
    "",
)
replace(
    """    runtime_handle: tokio::task::JoinHandle<()>,
""",
    """    runtime_handle: AbortOnDropHandle<()>,
    shutdown_tx: mpsc::Sender<oneshot::Sender<IoResult<()>>>,
    shutdown_requested: AtomicBool,
""",
)
replace(
    """impl InProcessClientHandle {
""",
    """/// Token for a single bounded shutdown attempt.
///
/// Drain the corresponding handle's events before passing this token to
/// [`InProcessClientHandle::finish_shutdown`]. The deadline starts at admission.
#[derive(Debug)]
pub struct InProcessShutdown {
    done_rx: oneshot::Receiver<IoResult<()>>,
    deadline: Instant,
}

impl InProcessClientHandle {
""",
)
a = s.index("    /// Requests runtime shutdown and waits for worker termination.")
b = s.index("    pub fn sender(&self)", a)
s = (
    s[:a]
    + """    /// Begin shutdown without blocking on the saturated data or event queues.
    ///
    /// Call once, keep consuming [`next_event`](Self::next_event) until `None`,
    /// then call [`finish_shutdown`](Self::finish_shutdown). Accepted requests receive
    /// responses or explicit errors; client notifications retain their queue policy.
    /// Shutdown rejects outstanding server requests rather than hanging on approvals.
    /// A second call returns `AlreadyExists`.
    pub async fn begin_shutdown(&self) -> IoResult<InProcessShutdown> {
        if self.shutdown_requested.swap(true, Ordering::AcqRel) {
            return Err(IoError::new(ErrorKind::AlreadyExists, "shutdown already requested"));
        }
        let (done_tx, done_rx) = oneshot::channel();
        self.shutdown_tx.try_send(done_tx).map_err(|_| {
            IoError::new(ErrorKind::BrokenPipe, "in-process app-server runtime is closed")
        })?;
        Ok(InProcessShutdown {
            done_rx,
            deadline: Instant::now() + SHUTDOWN_ACK_TIMEOUT,
        })
    }

    /// Join a requested shutdown within its original deadline.
    ///
    /// Any unread events are discarded on entry. Lossless hosts should drain first.
    /// Canceling this future also cancels the runtime and its owned worker tasks.
    pub async fn finish_shutdown(self, shutdown: InProcessShutdown) -> IoResult<()> {
        let mut runtime_handle = self.runtime_handle;
        drop(self.event_rx);
        let graceful = async {
            let outcome = shutdown.done_rx.await.map_err(|error| {
                IoError::new(ErrorKind::BrokenPipe, format!("shutdown acknowledgement closed: {error}"))
            })?;
            (&mut runtime_handle).await.map_err(IoError::other)?;
            outcome
        };
        match timeout_at(shutdown.deadline, graceful).await {
            Ok(result) => result,
            Err(_) => {
                runtime_handle.abort();
                let _ = runtime_handle.await;
                Err(IoError::new(ErrorKind::TimedOut, "in-process shutdown deadline exceeded"))
            }
        }
    }

    /// Shut down while consuming and discarding final events, within one deadline.
    ///
    /// Use two-phase shutdown instead when the host must observe final events.
    pub async fn shutdown(mut self) -> IoResult<()> {
        let shutdown = self.begin_shutdown().await?;
        let _ = timeout_at(shutdown.deadline, async {
            while self.next_event().await.is_some() {}
        }).await;
        self.finish_shutdown(shutdown).await
    }

"""
    + s[b:]
)
replace(
    """            _ = &mut shutdown_rx => break,""",
    """            _ = &mut shutdown_rx => {
                outgoing_rx.close();
                while let Some(envelope) = outgoing_rx.recv().await {
                    route_outgoing_envelope(&mut outbound_connections, envelope).await;
                }
                break;
            },""",
)
replace(
    """    let (client_tx, mut client_rx) = mpsc::channel::<InProcessClientMessage>(channel_capacity);
""",
    """    let InProcessStartOptions { thread_store, event_delivery } = options;
    let (client_tx, mut client_rx) = mpsc::channel::<InProcessClientMessage>(channel_capacity);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<IoResult<()>>>(/*buffer*/ 1);
""",
)
replace(
    "                thread_store: options.thread_store,",
    "                thread_store,",
)
replace(
    """        let mut outbound_handle = tokio::spawn(run_outbound_router(
            outgoing_rx,
            outbound_connections,""",
    """        let mut outbound_handle = AbortOnDropHandle::new(tokio::spawn(run_outbound_router(
            outgoing_rx,
            outbound_connections,""",
)
replace(
    """            outbound_shutdown_rx,
        ));""",
    """            outbound_shutdown_rx,
        )));""",
)
replace(
    """        let mut processor_handle = tokio::spawn(async move {""",
    """        let mut processor_handle = AbortOnDropHandle::new(tokio::spawn(async move {""",
)
replace(
    """            processor.shutdown_threads().await;
        });""",
    """            processor.shutdown_threads().await;
        }));""",
)
replace(
    """        let mut shutdown_ack = None;

        loop {
            tokio::select! {
                message = client_rx.recv() => {""",
    """        let mut shutdown_ack = None;
        let mut shutdown_requested = false;
        let mut event_consumer_open = true;

        loop {
            if !shutdown_requested {
                match shutdown_rx.try_recv() {
                    Ok(done_tx) => {
                        shutdown_requested = true;
                        shutdown_ack = Some(done_tx);
                        client_rx.close();
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        shutdown_requested = true;
                        client_rx.close();
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }
            }
            tokio::select! {
                shutdown = shutdown_rx.recv(), if !shutdown_requested => {
                    shutdown_requested = true;
                    shutdown_ack = shutdown;
                    client_rx.close();
                }
                message = client_rx.recv() => {""",
)
replace(
    """                        Some(InProcessClientMessage::Shutdown { done_tx }) => {
                            shutdown_ack = Some(done_tx);
                            break;
                        }
""",
    "",
)
a = s.index("                    let outgoing_message = queued_message.message;")
b = s.index(
    "                }\n            }\n        }\n\n        drop(writer_rx);", a
)
s = (
    s[:a]
    + """                    if !route_queued_message(
                        queued_message,
                        &mut pending_request_responses,
                        &event_tx,
                        outgoing_message_sender.as_ref(),
                        event_delivery,
                        DeliveryPhase::Running,
                    ).await {
                        event_consumer_open = false;
                        break;
                    }
"""
    + s[b:]
)
a = s.index("        drop(writer_rx);")
b = s.index("        analytics_events_flush_client.flush().await;", a)
s = (
    s[:a]
    + """        client_rx.close();
        drop(processor_tx);
        outgoing_message_sender
            .cancel_all_requests(Some(internal_error(
                "in-process app-server runtime is shutting down",
            )))
            .await;

        let mut shutdown_result = match timeout(
            SHUTDOWN_TIMEOUT,
            drain_writer_until_task_finishes(
                &mut processor_handle,
                &mut writer_rx,
                &mut pending_request_responses,
                &event_tx,
                outgoing_message_sender.as_ref(),
                event_delivery,
            ),
        ).await {
            Ok(result) => result,
            Err(_) => Err(IoError::new(ErrorKind::TimedOut, "request processor drain timed out")),
        };
        if shutdown_result.is_err() && !processor_handle.is_finished() {
            processor_handle.abort();
            let _ = (&mut processor_handle).await;
        }

        // Explicitly close and drain the outbound queue. Detached senders must
        // neither keep shutdown alive nor allow new messages after this point.
        let _ = outbound_shutdown_tx.send(());
        let outbound_result = timeout(SHUTDOWN_TIMEOUT, async {
            drain_writer_until_task_finishes(
                &mut outbound_handle,
                &mut writer_rx,
                &mut pending_request_responses,
                &event_tx,
                outgoing_message_sender.as_ref(),
                event_delivery,
            ).await?;
            drain_writer(
                &mut writer_rx,
                &mut pending_request_responses,
                &event_tx,
                outgoing_message_sender.as_ref(),
                event_delivery,
            ).await
        }).await;
        match outbound_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                outbound_handle.abort();
                shutdown_result = shutdown_result.and(Err(error));
            }
            Err(_) => {
                outbound_handle.abort();
                shutdown_result = shutdown_result.and(Err(IoError::new(
                    ErrorKind::TimedOut, "outbound event drain timed out",
                )));
            }
        }
        if event_delivery == InProcessEventDelivery::Lossless && !event_consumer_open {
            shutdown_result = shutdown_result.and(Err(IoError::new(
                ErrorKind::BrokenPipe, "lossless event consumer closed before drain completed",
            )));
        }
        drop(writer_rx);
        drop(outgoing_message_sender);
        for (_, response_tx) in pending_request_responses {
            let _ = response_tx.send(Err(internal_error(
                "in-process app-server runtime is shutting down",
            )));
        }
        // Close the event stream before the independent analytics flush, so
        // two-phase consumers can finish draining and join within the same budget.
        drop(event_tx);

"""
    + s[b:]
)
replace(
    """            let _ = done_tx.send(());
        }
    });""",
    """            let _ = done_tx.send(shutdown_result);
        }
    });""",
)
replace(
    """        runtime_handle,
        #[cfg(test)]""",
    """        runtime_handle: AbortOnDropHandle::new(runtime_handle),
        shutdown_tx,
        shutdown_requested: AtomicBool::new(false),
        #[cfg(test)]""",
)
# Preserve the existing analytics-budget test with the out-of-band handshake.
replace(
    """        let (client_tx, mut client_rx) = mpsc::channel(/*buffer*/ 1);
        let (_event_tx, event_rx) = mpsc::channel(/*buffer*/ 1);""",
    """        let (client_tx, _client_rx) = mpsc::channel(/*buffer*/ 1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<IoResult<()>>>(/*buffer*/ 1);
        let (event_tx, event_rx) = mpsc::channel(/*buffer*/ 1);""",
)
replace(
    """            let done_tx = match client_rx.recv().await {
                Some(InProcessClientMessage::Shutdown { done_tx }) => done_tx,
                _ => panic!("expected in-process shutdown request"),
            };""",
    """            let done_tx = shutdown_rx.recv().await.expect("expected shutdown request");
            drop(event_tx);""",
)
replace(
    """            let _ = done_tx.send(());
        });""",
    """            let _ = done_tx.send(Ok(()));
        });""",
)
replace(
    """            runtime_handle,
            _test_codex_home: None,""",
    """            runtime_handle: AbortOnDropHandle::new(runtime_handle),
            shutdown_tx,
            shutdown_requested: AtomicBool::new(false),
            _test_codex_home: None,""",
)
replace(
    """//! Command submission uses `try_send` and can return `WouldBlock`, while event
//! fanout may drop notifications under saturation.""",
    """//! Command submission uses `try_send` and can return `WouldBlock`. Default event
//! fanout may drop notifications under saturation; opt-in lossless delivery waits
//! for bounded capacity, requiring the host to consume events concurrently.""",
)
s += """\n#[cfg(test)]\n#[path = "in_process_lossless_tests.rs"]\nmod lossless_tests;\n"""
p.write_text(s)
lib = root / "codex-rs/app-server/src/lib.rs"
s = lib.read_text()
assert s.count("pub mod in_process;") == 1
lib.write_text(
    s.replace(
        "pub mod in_process;", "pub mod in_process;\nmod in_process_event_delivery;"
    )
)
print("Lossless runtime port applied; behavioral verification is still required.")
