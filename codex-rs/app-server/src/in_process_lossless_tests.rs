use super::*;
use codex_app_server_protocol::ThreadListResponse;
use codex_thread_store::InMemoryThreadStore;
use pretty_assertions::assert_eq;

async fn warning_runtime(
    delivery: InProcessEventDelivery,
    store: Arc<InMemoryThreadStore>,
) -> InProcessClientHandle {
    let (home, mut args) = tests::build_test_start_args(SessionSource::Cli, /*channel_capacity*/ 1).await;
    args.config_warnings = ["first warning", "second warning"]
        .into_iter()
        .map(|summary| ConfigWarningNotification {
            summary: summary.to_string(),
            details: None,
            path: None,
            range: None,
        })
        .collect();
    let initialize = args.initialize.clone();
    let options = InProcessStartOptions::default()
        .with_thread_store(store)
        .with_event_delivery(delivery);
    let mut client = start_uninitialized(args, options).await.expect("runtime should start");
    client._test_codex_home = Some(home);
    client.request(ClientRequest::Initialize {
        request_id: RequestId::Integer(0),
        params: initialize,
    }).await.expect("initialize transport").expect("initialize should succeed");
    timeout(SHUTDOWN_TIMEOUT, async {
        while client.event_rx.len() != 1 {
            tokio::task::yield_now().await;
        }
    }).await.expect("event queue should saturate");
    client
}

async fn collect_shutdown_warnings(mut client: InProcessClientHandle) -> Vec<String> {
    let shutdown = timeout(SHUTDOWN_TIMEOUT, client.begin_shutdown())
        .await.expect("shutdown admission must not block on data queues")
        .expect("shutdown should be admitted");
    let mut warnings = Vec::new();
    while let Some(event) = timeout_at(shutdown.deadline, client.next_event())
        .await.expect("event stream should close within the shutdown deadline")
    {
        if let InProcessServerEvent::ServerNotification(notification) = event {
            if let ServerNotification::ConfigWarning(warning) = *notification {
                warnings.push(warning.summary);
            }
        }
    }
    client.finish_shutdown(shutdown).await.expect("shutdown should succeed");
    warnings
}

#[tokio::test]
async fn in_process_lossless_drains_accepted_store_request_and_preserves_event_order() {
    let store = Arc::new(InMemoryThreadStore::default());
    let client = warning_runtime(InProcessEventDelivery::Lossless, Arc::clone(&store)).await;
    let (response_tx, response_rx) = oneshot::channel();
    client.client.try_send_client_message(InProcessClientMessage::Request {
        request: Box::new(ClientRequest::ThreadList {
            request_id: RequestId::Integer(1),
            params: serde_json::from_value(serde_json::json!({})).expect("list params"),
        }),
        response_tx,
        cancellation: tokio_util::sync::CancellationToken::new(),
    }).expect("request should be accepted before shutdown closes admission");
    assert_eq!(
        collect_shutdown_warnings(client).await,
        vec!["first warning", "second warning"],
    );
    let response = response_rx.await.expect("accepted request must receive a result")
        .expect("accepted list request should complete successfully");
    let parsed: ThreadListResponse = serde_json::from_value(response).expect("typed list response");
    assert!(parsed.data.is_empty());
    assert_eq!(store.calls().await.list_threads, 1);
}

#[tokio::test]
async fn in_process_best_effort_retains_ordinary_notification_drop_policy() {
    let client = warning_runtime(
        InProcessEventDelivery::BestEffort,
        Arc::new(InMemoryThreadStore::default()),
    ).await;
    // A response after initialization is an ordering barrier: both warnings have
    // crossed the saturated event router before we begin consuming anything.
    client.request(ClientRequest::ConfigRequirementsRead {
        request_id: RequestId::Integer(1),
        params: None,
    }).await.expect("control request transport").expect("control request should succeed");
    assert_eq!(collect_shutdown_warnings(client).await, vec!["first warning"]);
}

#[tokio::test]
async fn in_process_lossless_rejects_duplicate_shutdown_without_unbounded_admission() {
    let mut client = warning_runtime(
        InProcessEventDelivery::Lossless,
        Arc::new(InMemoryThreadStore::default()),
    ).await;
    let shutdown = client.begin_shutdown().await.expect("first shutdown accepted");
    assert_eq!(
        client.begin_shutdown().await.expect_err("second shutdown rejected").kind(),
        ErrorKind::AlreadyExists,
    );
    while timeout_at(shutdown.deadline, client.next_event()).await.expect("bounded drain").is_some() {}
    client.finish_shutdown(shutdown).await.expect("first shutdown completes");
}

#[tokio::test]
async fn in_process_lossless_reports_a_consumer_closed_before_drain() {
    let client = warning_runtime(
        InProcessEventDelivery::Lossless,
        Arc::new(InMemoryThreadStore::default()),
    ).await;
    let shutdown = client.begin_shutdown().await.expect("shutdown accepted");
    let error = client.finish_shutdown(shutdown).await
        .expect_err("discarding a saturated lossless stream must not report graceful delivery");
    assert_eq!(error.kind(), ErrorKind::BrokenPipe);
}

#[tokio::test(start_paused = true)]
async fn in_process_finish_shutdown_enforces_one_deadline_and_aborts_a_stalled_runtime() {
    let (client_tx, _client_rx) = mpsc::channel(/*buffer*/ 1);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<IoResult<()>>>(/*buffer*/ 1);
    let (_event_tx, event_rx) = mpsc::channel(/*buffer*/ 1);
    let runtime_handle = AbortOnDropHandle::new(tokio::spawn(async move {
        let _ack = shutdown_rx.recv().await.expect("shutdown should arrive");
        std::future::pending::<()>().await;
    }));
    let abort = runtime_handle.abort_handle();
    let client = InProcessClientHandle {
        client: InProcessClientSender { client_tx },
        event_rx,
        runtime_handle,
        shutdown_tx,
        shutdown_requested: AtomicBool::new(false),
        _test_codex_home: None,
    };
    let started = Instant::now();
    let shutdown = client.begin_shutdown().await.expect("shutdown accepted");
    let error = client.finish_shutdown(shutdown).await.expect_err("stalled shutdown must time out");
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert_eq!(Instant::now() - started, SHUTDOWN_ACK_TIMEOUT);
    assert!(abort.is_finished());
}

#[tokio::test]
async fn in_process_request_cancellation_survives_the_transport_changes() {
    let (client_tx, mut client_rx) = mpsc::channel(/*buffer*/ 1);
    let sender = InProcessClientSender { client_tx };
    let request = tokio::spawn(async move {
        sender.request(ClientRequest::ConfigRequirementsRead {
            request_id: RequestId::Integer(7),
            params: None,
        }).await
    });
    let Some(InProcessClientMessage::Request { cancellation, response_tx, .. }) = client_rx.recv().await else {
        panic!("a real request should have been admitted");
    };
    assert!(!cancellation.is_cancelled());
    request.abort();
    assert!(request.await.expect_err("request future should be cancelled").is_cancelled());
    assert!(cancellation.is_cancelled());
    drop(response_tx);
}

#[tokio::test]
async fn in_process_command_overload_remains_explicit() {
    let (client_tx, _client_rx) = mpsc::channel(/*buffer*/ 1);
    let sender = InProcessClientSender { client_tx };
    sender.notify(ClientNotification::Initialized).expect("first command fits");
    let error = timeout(SHUTDOWN_TIMEOUT, sender.request(ClientRequest::ConfigRequirementsRead {
        request_id: RequestId::Integer(8),
        params: None,
    })).await.expect("overload must not block").expect_err("saturated queue rejects admission");
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
}
