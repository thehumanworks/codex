//! In-process app-server runtime host for local embedders.
//!
//! This module runs the existing [`MessageProcessor`] and outbound routing logic
//! on Tokio tasks, but replaces socket/stdio transports with bounded in-memory
//! channels. The intent is to preserve app-server semantics while avoiding a
//! process boundary for CLI surfaces that run in the same process.
//!
//! # Lifecycle
//!
//! 1. Construct runtime state with [`InProcessStartArgs`].
//! 2. Call [`start`], which performs the `initialize` / `initialized` handshake
//!    internally and returns a ready-to-use [`InProcessClientHandle`].
//! 3. Send requests via [`InProcessClientHandle::request`], notifications via
//!    [`InProcessClientHandle::notify`], and consume events via
//!    [`InProcessClientHandle::next_event`].
//! 4. Terminate with [`InProcessClientHandle::shutdown`].
//!
//! # Transport model
//!
//! The runtime is transport-local but not protocol-free. Incoming requests are
//! typed [`ClientRequest`] values, yet responses still come back through the
//! same JSON-RPC result envelope that `MessageProcessor` uses for stdio and
//! websocket transports. This keeps in-process behavior aligned with
//! app-server rather than creating a second execution contract.
//!
//! # Backpressure
//!
//! Command submission uses `try_send` and can return `WouldBlock`. Default event
//! fanout may drop notifications under saturation; opt-in lossless delivery waits
//! for bounded capacity, requiring the host to consume events concurrently. Server requests are never
//! silently abandoned: if they cannot be queued they are failed back into
//! `MessageProcessor` with overload or internal errors so approval flows do
//! not hang indefinitely.
//!
//! # Relationship to `codex-app-server-client`
//!
//! This module provides the low-level runtime handle ([`InProcessClientHandle`]).
//! Higher-level callers (TUI, exec) should go through `codex-app-server-client`,
//! which wraps this module behind a worker task with async request/response
//! helpers, surface-specific startup policy, and bounded shutdown.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::analytics_utils::analytics_events_client_from_config;
use crate::config_manager::ConfigManager;
use crate::error_code::OVERLOADED_ERROR_CODE;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::message_processor::ConnectionSessionState;
use crate::message_processor::MessageProcessor;
use crate::message_processor::MessageProcessorArgs;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::in_process_event_delivery::DeliveryPhase;
use crate::in_process_event_delivery::drain_writer;
use crate::in_process_event_delivery::drain_writer_until_task_finishes;
use crate::in_process_event_delivery::route_queued_message;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::QueuedOutgoingMessage;
use crate::plugin_config_reload::PluginStartupConfig;
use crate::transport::CHANNEL_CAPACITY;
use crate::transport::OutboundConnectionState;
use crate::transport::route_outgoing_envelope;
use codex_analytics::AppServerRpcTransport;
use codex_app_server_protocol::AgentMessageDelivery;
use codex_app_server_protocol::ClientNotification;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::InitializeParams;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::Result;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadItem;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_config::ThreadConfigLoader;
use codex_core::check_execpolicy_for_warnings;
use codex_core::config::Config;
use codex_core::resolve_installation_id;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_login::AuthManager;
use codex_protocol::protocol::SessionSource;
pub use codex_rollout::StateDbHandle;
pub use codex_state::log_db::LogDbLayer;
use codex_thread_store::ThreadStore;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio::time::timeout_at;
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;
use toml::Value as TomlValue;
use tracing::warn;

pub(crate) const IN_PROCESS_CONNECTION_ID: ConnectionId = ConnectionId(0);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
// Covers both bounded runtime drains plus the analytics client's 25-second best-effort flush.
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(35);
/// Default bounded channel capacity for in-process runtime queues.
pub const DEFAULT_IN_PROCESS_CHANNEL_CAPACITY: usize = CHANNEL_CAPACITY;

pub(crate) type PendingClientRequestResponse = std::result::Result<Result, JSONRPCErrorError>;

pub(crate) fn server_notification_requires_delivery(notification: &ServerNotification) -> bool {
    matches!(
        notification,
        ServerNotification::TurnCompleted(_)
            | ServerNotification::ThreadQueueChanged(_)
            | ServerNotification::ThreadSettingsUpdated(_)
            | ServerNotification::ThreadAttachmentUpdated(_)
            | ServerNotification::ExternalAgentConfigImportCompleted(_)
            | ServerNotification::ItemCompleted(ItemCompletedNotification {
                item: ThreadItem::AgentMessage {
                    delivery: Some(AgentMessageDelivery::Async),
                    ..
                },
                ..
            })
    )
}

/// Input needed to start an in-process app-server runtime.
///
/// These fields mirror the pieces of ambient process state that stdio and
/// websocket transports normally assemble before `MessageProcessor` starts.
#[derive(Clone)]
pub struct InProcessStartArgs {
    /// Resolved argv0 dispatch paths used by command execution internals.
    pub arg0_paths: Arg0DispatchPaths,
    /// Shared base config used to initialize core components.
    pub config: Arc<Config>,
    /// CLI config overrides that are already parsed into TOML values.
    pub cli_overrides: Vec<(String, TomlValue)>,
    /// Loader override knobs used by config API paths.
    pub loader_overrides: LoaderOverrides,
    /// Whether config API paths should reject unknown config fields.
    pub strict_config: bool,
    /// Preloaded cloud config bundle provider.
    pub cloud_config_bundle: CloudConfigBundleLoader,
    /// Loader used to fetch typed thread config sources before a thread starts.
    pub thread_config_loader: Arc<dyn ThreadConfigLoader>,
    /// Feedback sink used by app-server/core telemetry and logs.
    pub feedback: CodexFeedback,
    /// SQLite tracing layer used to flush recently emitted logs before feedback upload.
    pub log_db: Option<LogDbLayer>,
    /// Process-wide SQLite state handle shared with embedded app-server consumers.
    pub state_db: Option<StateDbHandle>,
    /// Environment manager used by core execution and filesystem operations.
    pub environment_manager: Arc<EnvironmentManager>,
    /// Startup warnings emitted after initialize succeeds.
    pub config_warnings: Vec<ConfigWarningNotification>,
    /// Session source stamped into thread/session metadata.
    pub session_source: SessionSource,
    /// Whether auth loading should honor the `CODEX_API_KEY` environment variable.
    pub enable_codex_api_key_env: bool,
    /// Initialize params used for initial handshake.
    pub initialize: InitializeParams,
    /// Capacity used for all runtime queues (clamped to at least 1).
    pub channel_capacity: usize,
}

/// Optional host overrides for the embedded runtime.
///
/// Defaults preserve config-derived persistence and the existing transport behavior.
#[derive(Clone, Default)]
pub struct InProcessStartOptions {
    thread_store: Option<Arc<dyn ThreadStore>>,
    event_delivery: InProcessEventDelivery,
}

impl InProcessStartOptions {
    /// Use a process-scoped store supplied by the embedding host.
    ///
    /// Config reloads do not replace it. The host owns this store's durability.
    /// Persistent message queues are disabled with an injected store because the
    /// default SQLite queue may belong to a different persistence backend.
    pub fn with_thread_store(mut self, thread_store: Arc<dyn ThreadStore>) -> Self {
        self.thread_store = Some(thread_store);
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
}

/// Event emitted from the app-server to the in-process client.
///
/// [`Lagged`](Self::Lagged) is a transport health marker, not an application
/// event — it signals that the consumer fell behind and some events were dropped.
#[derive(Debug, Clone)]
pub enum InProcessServerEvent {
    /// Server request that requires client response/rejection.
    ServerRequest(Box<ServerRequest>),
    /// App-server notification directed to the embedded client.
    ServerNotification(Box<ServerNotification>),
    /// Indicates one or more events were dropped due to backpressure.
    Lagged { skipped: usize },
}

/// Internal message sent from [`InProcessClientHandle`] methods to the runtime task.
///
/// Requests carry a oneshot sender for the response; notifications and server-request
/// replies are fire-and-forget from the caller's perspective (transport errors are
/// caught by `try_send` on the outer channel).
enum InProcessClientMessage {
    Request {
        request: Box<ClientRequest>,
        response_tx: oneshot::Sender<PendingClientRequestResponse>,
        cancellation: tokio_util::sync::CancellationToken,
    },
    Notification {
        notification: ClientNotification,
    },
    ServerRequestResponse {
        request_id: RequestId,
        result: Result,
    },
    ServerRequestError {
        request_id: RequestId,
        error: JSONRPCErrorError,
    },
}

enum ProcessorCommand {
    Request(Box<ClientRequest>, tokio_util::sync::CancellationToken),
    Notification(ClientNotification),
}

#[derive(Clone)]
pub struct InProcessClientSender {
    client_tx: mpsc::Sender<InProcessClientMessage>,
}

impl InProcessClientSender {
    pub async fn request(&self, request: ClientRequest) -> IoResult<PendingClientRequestResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        self.try_send_client_message(InProcessClientMessage::Request {
            request: Box::new(request),
            response_tx,
            cancellation,
        })?;
        response_rx.await.map_err(|err| {
            IoError::new(
                ErrorKind::BrokenPipe,
                format!("in-process request response channel closed: {err}"),
            )
        })
    }

    pub fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::Notification { notification })
    }

    pub fn respond_to_server_request(&self, request_id: RequestId, result: Result) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::ServerRequestResponse {
            request_id,
            result,
        })
    }

    pub fn fail_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::ServerRequestError {
            request_id,
            error,
        })
    }

    fn try_send_client_message(&self, message: InProcessClientMessage) -> IoResult<()> {
        match self.client_tx.try_send(message) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(IoError::new(
                ErrorKind::WouldBlock,
                "in-process app-server client queue is full",
            )),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server runtime is closed",
            )),
        }
    }
}

/// Handle used by an in-process client to call app-server and consume events.
///
/// This is the low-level runtime handle. Higher-level callers should usually go
/// through `codex-app-server-client`, which adds worker-task buffering,
/// request/response helpers, and surface-specific startup policy.
pub struct InProcessClientHandle {
    client: InProcessClientSender,
    event_rx: mpsc::Receiver<InProcessServerEvent>,
    runtime_handle: AbortOnDropHandle<()>,
    shutdown_tx: mpsc::Sender<oneshot::Sender<IoResult<()>>>,
    shutdown_requested: AtomicBool,
    #[cfg(test)]
    _test_codex_home: Option<tempfile::TempDir>,
}

/// Token for a single bounded shutdown attempt.
///
/// Drain the corresponding handle's events before passing this token to
/// [`InProcessClientHandle::finish_shutdown`]. The deadline starts at admission.
#[derive(Debug)]
pub struct InProcessShutdown {
    done_rx: oneshot::Receiver<IoResult<()>>,
    deadline: Instant,
}

impl InProcessClientHandle {
    /// Sends a typed client request into the in-process runtime.
    ///
    /// The returned value is a transport-level `IoResult` containing either a
    /// JSON-RPC success payload or JSON-RPC error payload. Callers must keep
    /// request IDs unique among concurrent requests; reusing an in-flight ID
    /// produces an `INVALID_REQUEST` response and can make request routing
    /// ambiguous in the caller.
    pub async fn request(&self, request: ClientRequest) -> IoResult<PendingClientRequestResponse> {
        self.client.request(request).await
    }

    /// Sends a typed client notification into the in-process runtime.
    ///
    /// Notifications do not have an application-level response. Transport
    /// errors indicate queue saturation or closed runtime.
    pub fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        self.client.notify(notification)
    }

    /// Resolves a pending [`ServerRequest`](InProcessServerEvent::ServerRequest).
    ///
    /// This should be used only with request IDs received from the current
    /// runtime event stream; sending arbitrary IDs has no effect on app-server
    /// state and can mask a stuck approval flow in the caller.
    pub fn respond_to_server_request(&self, request_id: RequestId, result: Result) -> IoResult<()> {
        self.client.respond_to_server_request(request_id, result)
    }

    /// Rejects a pending [`ServerRequest`](InProcessServerEvent::ServerRequest).
    ///
    /// Use this when the embedder cannot satisfy a server request; leaving
    /// requests unanswered can stall turn progress.
    pub fn fail_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        self.client.fail_server_request(request_id, error)
    }

    /// Receives the next server event from the in-process runtime.
    ///
    /// Returns `None` when the runtime task exits and no more events are
    /// available.
    pub async fn next_event(&mut self) -> Option<InProcessServerEvent> {
        self.event_rx.recv().await
    }

    /// Begin shutdown without blocking on the saturated data or event queues.
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

    pub fn sender(&self) -> InProcessClientSender {
        self.client.clone()
    }
}

/// Starts an in-process app-server runtime and performs initialize handshake.
///
/// This function sends `initialize` followed by `initialized` before returning
/// the handle, so callers receive a ready-to-use runtime. If initialize fails,
/// the runtime is shut down and an `InvalidData` error is returned.
pub async fn start(args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {
    start_with_options(args, InProcessStartOptions::default()).await
}

/// Starts an embedded runtime with explicit host overrides and the normal handshake.
///
/// See [`InProcessStartOptions`] for persistence ownership and queue limitations.
pub async fn start_with_options(
    mut args: InProcessStartArgs,
    options: InProcessStartOptions,
) -> IoResult<InProcessClientHandle> {
    if let Ok(Some(err)) = check_execpolicy_for_warnings(&args.config.config_layer_stack).await {
        let (path, range) = crate::exec_policy_warning_location(&err);
        args.config_warnings.push(ConfigWarningNotification {
            summary: "Error parsing rules; custom rules not applied.".to_string(),
            details: Some(err.to_string()),
            path,
            range,
        });
    }
    let initialize = args.initialize.clone();
    let client = start_uninitialized(args, options).await?;

    let initialize_response = client
        .request(ClientRequest::Initialize {
            request_id: RequestId::Integer(0),
            params: initialize,
        })
        .await?;
    if let Err(error) = initialize_response {
        let _ = client.shutdown().await;
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!("in-process initialize failed: {}", error.message),
        ));
    }
    client.notify(ClientNotification::Initialized)?;

    Ok(client)
}

async fn run_outbound_router(
    mut outgoing_rx: mpsc::Receiver<OutgoingEnvelope>,
    mut outbound_connections: HashMap<ConnectionId, OutboundConnectionState>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                outgoing_rx.close();
                while let Some(envelope) = outgoing_rx.recv().await {
                    route_outgoing_envelope(&mut outbound_connections, envelope).await;
                }
                break;
            },
            envelope = outgoing_rx.recv() => {
                let Some(envelope) = envelope else {
                    break;
                };
                route_outgoing_envelope(&mut outbound_connections, envelope).await;
            }
        }
    }
}

async fn start_uninitialized(
    args: InProcessStartArgs,
    options: InProcessStartOptions,
) -> IoResult<InProcessClientHandle> {
    args.config.auth_config().validate()?;
    let channel_capacity = args.channel_capacity.max(1);
    let installation_id = resolve_installation_id(&args.config.codex_home).await?;
    let auth_manager =
        AuthManager::shared_from_config(args.config.as_ref(), args.enable_codex_api_key_env)
            .await
            .map_err(IoError::other)?;
    let InProcessStartOptions { thread_store, event_delivery } = options;
    let (client_tx, mut client_rx) = mpsc::channel::<InProcessClientMessage>(channel_capacity);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<IoResult<()>>>(/*buffer*/ 1);
    let (event_tx, event_rx) = mpsc::channel::<InProcessServerEvent>(channel_capacity);

    let runtime_handle = tokio::spawn(async move {
        let (outgoing_tx, outgoing_rx) = mpsc::channel::<OutgoingEnvelope>(channel_capacity);
        let analytics_events_client =
            analytics_events_client_from_config(Arc::clone(&auth_manager), args.config.as_ref());
        let analytics_events_flush_client = analytics_events_client.clone();
        let outgoing_message_sender = Arc::new(OutgoingMessageSender::new(
            outgoing_tx,
            analytics_events_client.clone(),
        ));

        let (writer_tx, mut writer_rx) = mpsc::channel::<QueuedOutgoingMessage>(channel_capacity);
        let outbound_initialized = Arc::new(AtomicBool::new(false));
        let outbound_experimental_api_enabled = Arc::new(AtomicBool::new(false));
        let outbound_opted_out_notification_methods = Arc::new(RwLock::new(HashSet::new()));

        let mut outbound_connections = HashMap::<ConnectionId, OutboundConnectionState>::new();
        outbound_connections.insert(
            IN_PROCESS_CONNECTION_ID,
            OutboundConnectionState::new(
                writer_tx,
                Arc::clone(&outbound_initialized),
                Arc::clone(&outbound_experimental_api_enabled),
                Arc::clone(&outbound_opted_out_notification_methods),
                /*disconnect_sender*/ None,
            ),
        );
        let (outbound_shutdown_tx, outbound_shutdown_rx) = oneshot::channel();
        let mut outbound_handle = AbortOnDropHandle::new(tokio::spawn(run_outbound_router(
            outgoing_rx,
            outbound_connections,
            outbound_shutdown_rx,
        )));

        let processor_outgoing = Arc::clone(&outgoing_message_sender);
        let config_manager = ConfigManager::new(
            args.config.codex_home.to_path_buf(),
            args.cli_overrides,
            args.loader_overrides,
            args.strict_config,
            args.cloud_config_bundle,
            args.arg0_paths.clone(),
            args.thread_config_loader,
        );
        let (processor_tx, mut processor_rx) = mpsc::channel::<ProcessorCommand>(channel_capacity);
        let mut processor_handle = AbortOnDropHandle::new(tokio::spawn(async move {
            let processor = Arc::new(MessageProcessor::new(MessageProcessorArgs {
                outgoing: Arc::clone(&processor_outgoing),
                analytics_events_client,
                arg0_paths: args.arg0_paths,
                config: args.config,
                config_manager,
                environment_manager: args.environment_manager,
                feedback: args.feedback,
                log_db: args.log_db,
                state_db: args.state_db,
                thread_store,
                config_warnings: args.config_warnings,
                session_source: args.session_source,
                user_verification: Arc::new(crate::user_verification::Service::new(Arc::clone(
                    &auth_manager,
                ))),
                auth_manager,
                installation_id,
                code_mode_session_provider: None,
                rpc_transport: AppServerRpcTransport::InProcess,
                remote_control_handle: None,
                plugin_startup_tasks: Some(PluginStartupConfig::Current),
            }));
            let mut thread_created_rx = processor.thread_created_receiver();
            let session = Arc::new(ConnectionSessionState::new(
                crate::transport::ConnectionOrigin::InProcess,
            ));
            let mut listen_for_threads = true;

            loop {
                tokio::select! {
                    command = processor_rx.recv() => {
                        match command {
                            Some(ProcessorCommand::Request(request, cancellation)) => {
                                let was_initialized = session.initialized();
                                processor
                                    .process_client_request(
                                        IN_PROCESS_CONNECTION_ID,
                                        *request,
                                        Arc::clone(&session),
                                        &outbound_initialized,
                                        cancellation,
                                    )
                                    .await;
                                let opted_out_notification_methods_snapshot =
                                    session.opted_out_notification_methods();
                                let experimental_api_enabled =
                                    session.experimental_api_enabled();
                                let is_initialized = session.initialized();
                                if let Ok(mut opted_out_notification_methods) =
                                    outbound_opted_out_notification_methods.write()
                                {
                                    *opted_out_notification_methods =
                                        opted_out_notification_methods_snapshot;
                                } else {
                                    warn!("failed to update outbound opted-out notifications");
                                }
                                outbound_experimental_api_enabled.store(
                                    experimental_api_enabled,
                                    Ordering::Release,
                                );
                                if !was_initialized && is_initialized {
                                    processor.send_initialize_notifications().await;
                                }
                            }
                            Some(ProcessorCommand::Notification(notification)) => {
                                processor.process_client_notification(notification).await;
                            }
                            None => {
                                break;
                            }
                        }
                    }
                    created = thread_created_rx.recv(), if listen_for_threads => {
                        match created {
                            Ok(thread_id) => {
                                let connection_ids = if session.initialized() {
                                    vec![IN_PROCESS_CONNECTION_ID]
                                } else {
                                    Vec::<ConnectionId>::new()
                                };
                                processor
                                    .try_attach_thread_listener(thread_id, connection_ids)
                                    .await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                warn!("thread_created receiver lagged; skipping resync");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                listen_for_threads = false;
                            }
                        }
                    }
                }
            }

            processor.clear_runtime_references();
            processor.cancel_active_login().await;
            processor
                .connection_closed(IN_PROCESS_CONNECTION_ID, &session)
                .await;
            processor.clear_all_thread_listeners().await;
            processor.drain_background_tasks().await;
            processor.shutdown_threads().await;
        }));
        let mut pending_request_responses =
            HashMap::<RequestId, oneshot::Sender<PendingClientRequestResponse>>::new();
        let mut shutdown_ack = None;
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
                message = client_rx.recv() => {
                    match message {
                        Some(InProcessClientMessage::Request { request, response_tx, cancellation }) => {
                            let request = *request;
                            let request_id = request.id().clone();
                            match pending_request_responses.entry(request_id.clone()) {
                                Entry::Vacant(entry) => {
                                    entry.insert(response_tx);
                                }
                                Entry::Occupied(_) => {
                                    let _ = response_tx.send(Err(invalid_request(format!(
                                        "duplicate request id: {request_id:?}"
                                    ))));
                                    continue;
                                }
                            }

                            match processor_tx.try_send(ProcessorCommand::Request(Box::new(request), cancellation)) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    if let Some(response_tx) =
                                        pending_request_responses.remove(&request_id)
                                    {
                                        let _ = response_tx.send(Err(JSONRPCErrorError {
                                            code: OVERLOADED_ERROR_CODE,
                                            message: "in-process app-server request queue is full"
                                                .to_string(),
                                            data: None,
                                        }));
                                    }
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    if let Some(response_tx) =
                                        pending_request_responses.remove(&request_id)
                                    {
                                        let _ = response_tx.send(Err(internal_error(
                                            "in-process app-server request processor is closed",
                                        )));
                                    }
                                    break;
                                }
                            }
                        }
                        Some(InProcessClientMessage::Notification { notification }) => {
                            match processor_tx.try_send(ProcessorCommand::Notification(notification)) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    warn!("dropping in-process client notification (queue full)");
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    break;
                                }
                            }
                        }
                        Some(InProcessClientMessage::ServerRequestResponse { request_id, result }) => {
                            outgoing_message_sender
                                .notify_client_response(IN_PROCESS_CONNECTION_ID, request_id, result)
                                .await;
                        }
                        Some(InProcessClientMessage::ServerRequestError { request_id, error }) => {
                            outgoing_message_sender
                                .notify_client_error(IN_PROCESS_CONNECTION_ID, request_id, error)
                                .await;
                        }
                        None => {
                            break;
                        }
                    }
                }
                queued_message = writer_rx.recv() => {
                    let Some(queued_message) = queued_message else {
                        break;
                    };
                    if !route_queued_message(
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
                }
            }
        }

        client_rx.close();
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

        analytics_events_flush_client.flush().await;

        if let Some(done_tx) = shutdown_ack {
            let _ = done_tx.send(shutdown_result);
        }
    });

    Ok(InProcessClientHandle {
        client: InProcessClientSender { client_tx },
        event_rx,
        runtime_handle: AbortOnDropHandle::new(runtime_handle),
        shutdown_tx,
        shutdown_requested: AtomicBool::new(false),
        #[cfg(test)]
        _test_codex_home: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::ClientInfo;
    use codex_app_server_protocol::ConfigRequirementsReadResponse;
    use codex_app_server_protocol::ExternalAgentConfigImportCompletedNotification;
    use codex_app_server_protocol::SessionSource as ApiSessionSource;
    use codex_app_server_protocol::ThreadAttachmentOperation;
    use codex_app_server_protocol::ThreadAttachmentUpdatedNotification;
    use codex_app_server_protocol::ThreadQueueChangedNotification;
    use codex_app_server_protocol::ThreadStartParams;
    use codex_app_server_protocol::ThreadStartResponse;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnCompletedNotification;
    use codex_app_server_protocol::TurnItemsView;
    use codex_app_server_protocol::TurnStatus;
    use codex_core::config::ConfigBuilder;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::TempDir;

    async fn build_test_config(codex_home: &Path) -> Config {
        match ConfigBuilder::default()
            .codex_home(codex_home.to_path_buf())
            .build()
            .await
        {
            Ok(config) => config,
            Err(_) => Config::load_default_with_cli_overrides_for_codex_home(
                codex_home.to_path_buf(),
                Vec::new(),
            )
            .await
            .expect("default config should load"),
        }
    }

    pub(super) async fn build_test_start_args(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> (TempDir, InProcessStartArgs) {
        let codex_home = TempDir::new().expect("temp dir");
        let config = Arc::new(build_test_config(codex_home.path()).await);
        let state_db = codex_rollout::state_db::try_init(config.as_ref())
            .await
            .expect("state db should initialize for in-process test");
        let args = InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config,
            cli_overrides: Vec::new(),
            loader_overrides: LoaderOverrides::default(),
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
            thread_config_loader: Arc::new(codex_config::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: Some(state_db),
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source,
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-in-process-test".to_string(),
                    title: None,
                    version: "0.0.0".to_string(),
                },
                capabilities: None,
            },
            channel_capacity,
        };
        (codex_home, args)
    }

    async fn start_test_client_with_capacity(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> InProcessClientHandle {
        let (codex_home, args) = build_test_start_args(session_source, channel_capacity).await;
        let mut client = start(args).await.expect("in-process runtime should start");
        client._test_codex_home = Some(codex_home);
        client
    }

    async fn start_test_client(session_source: SessionSource) -> InProcessClientHandle {
        start_test_client_with_capacity(session_source, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await
    }

    #[tokio::test]
    async fn in_process_start_initializes_and_handles_typed_v2_request() {
        let client = start_test_client(SessionSource::Cli).await;
        let response = client
            .request(ClientRequest::ConfigRequirementsRead {
                request_id: RequestId::Integer(1),
                params: None,
            })
            .await
            .expect("request transport should work")
            .expect("request should succeed");
        assert!(response.is_object());

        let _parsed: ConfigRequirementsReadResponse =
            serde_json::from_value(response).expect("response should match v2 schema");
        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
    }

    #[tokio::test]
    async fn in_process_start_uses_requested_session_source_for_thread_start() {
        for (requested_source, expected_source) in [
            (SessionSource::Cli, ApiSessionSource::Cli),
            (SessionSource::Exec, ApiSessionSource::Exec),
        ] {
            let client = start_test_client(requested_source).await;
            let response = client
                .request(ClientRequest::ThreadStart {
                    request_id: RequestId::Integer(2),
                    params: ThreadStartParams {
                        ephemeral: Some(true),
                        ..ThreadStartParams::default()
                    },
                })
                .await
                .expect("request transport should work")
                .expect("thread/start should succeed");
            let parsed: ThreadStartResponse =
                serde_json::from_value(response).expect("thread/start response should parse");
            assert_eq!(parsed.thread.source, expected_source);
            client
                .shutdown()
                .await
                .expect("in-process runtime should shutdown cleanly");
        }
    }

    #[tokio::test]
    async fn in_process_start_clamps_zero_channel_capacity() {
        let client =
            start_test_client_with_capacity(SessionSource::Cli, /*channel_capacity*/ 0).await;
        let response = loop {
            match client
                .request(ClientRequest::ConfigRequirementsRead {
                    request_id: RequestId::Integer(4),
                    params: None,
                })
                .await
            {
                Ok(response) => break response.expect("request should succeed"),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::task::yield_now().await;
                }
                Err(err) => panic!("request transport should work: {err}"),
            }
        };
        let _parsed: ConfigRequirementsReadResponse =
            serde_json::from_value(response).expect("response should match v2 schema");
        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
    }

    #[tokio::test(start_paused = true)]
    async fn in_process_outbound_router_shutdown_does_not_wait_for_retained_sender() {
        let (outgoing_tx, outgoing_rx) = mpsc::channel(/*buffer*/ 1);
        let retained_outgoing_tx = outgoing_tx.clone();
        drop(outgoing_tx);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut outbound_handle = tokio::spawn(run_outbound_router(
            outgoing_rx,
            HashMap::new(),
            shutdown_rx,
        ));

        assert!(!retained_outgoing_tx.is_closed());
        shutdown_tx
            .send(())
            .expect("outbound router should accept explicit shutdown");
        timeout(SHUTDOWN_TIMEOUT, &mut outbound_handle)
            .await
            .expect("outbound router should not wait for its retained sender")
            .expect("outbound router should complete successfully");
        assert!(retained_outgoing_tx.is_closed());
    }

    #[tokio::test(start_paused = true)]
    async fn in_process_shutdown_waits_for_analytics_flush_budget() {
        let (client_tx, _client_rx) = mpsc::channel(/*buffer*/ 1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<IoResult<()>>>(/*buffer*/ 1);
        let (event_tx, event_rx) = mpsc::channel(/*buffer*/ 1);
        let completed = Arc::new(AtomicBool::new(false));
        let runtime_completed = Arc::clone(&completed);
        let runtime_handle = tokio::spawn(async move {
            let done_tx = shutdown_rx.recv().await.expect("expected shutdown request");
            drop(event_tx);
            tokio::time::sleep(SHUTDOWN_TIMEOUT + SHUTDOWN_TIMEOUT + Duration::from_secs(24)).await;
            runtime_completed.store(true, Ordering::Release);
            let _ = done_tx.send(Ok(()));
        });
        let client = InProcessClientHandle {
            client: InProcessClientSender { client_tx },
            event_rx,
            runtime_handle: AbortOnDropHandle::new(runtime_handle),
            shutdown_tx,
            shutdown_requested: AtomicBool::new(false),
            _test_codex_home: None,
        };

        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
        assert!(completed.load(Ordering::Acquire));
    }

    #[test]
    fn guaranteed_delivery_helpers_cover_required_server_notifications() {
        assert!(server_notification_requires_delivery(
            &ServerNotification::TurnCompleted(TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    items_view: TurnItemsView::NotLoaded,
                    status: TurnStatus::Completed,
                    error: None,
                    started_at: None,
                    completed_at: Some(0),
                    duration_ms: None,
                },
            })
        ));
        assert!(server_notification_requires_delivery(
            &ServerNotification::ThreadQueueChanged(ThreadQueueChangedNotification {
                thread_id: "thread-1".to_string(),
            })
        ));
        assert!(server_notification_requires_delivery(
            &ServerNotification::ExternalAgentConfigImportCompleted(
                ExternalAgentConfigImportCompletedNotification {
                    import_id: "import".to_string(),
                    item_type_results: Vec::new(),
                },
            )
        ));
        assert!(server_notification_requires_delivery(
            &ServerNotification::ThreadAttachmentUpdated(ThreadAttachmentUpdatedNotification {
                thread_id: "thread-1".to_string(),
                attachment_type: "pull_request".to_string(),
                identity_key: r#"["github.com","openai","codex",123]"#.to_string(),
                attachment_id: "attachment-1".to_string(),
                operation: ThreadAttachmentOperation::Deleted,
            })
        ));
        assert!(server_notification_requires_delivery(
            &ServerNotification::ItemCompleted(ItemCompletedNotification {
                item: ThreadItem::AgentMessage {
                    id: "item-1".to_string(),
                    text: "Still working".to_string(),
                    phase: None,
                    memory_citation: None,
                    delivery: Some(AgentMessageDelivery::Async),
                    questions: None,
                },
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                completed_at_ms: 0,
            })
        ));
    }
}

#[cfg(test)]
#[path = "in_process_stores_tests.rs"]
mod stores_tests;

#[cfg(test)]
#[path = "in_process_lossless_tests.rs"]
mod lossless_tests;
