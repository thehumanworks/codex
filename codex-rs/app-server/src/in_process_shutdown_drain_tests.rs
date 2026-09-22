use super::*;
use codex_app_server_protocol::ThreadListResponse;
use codex_thread_store::InMemoryThreadStore;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_process_shutdown_drains_multiple_accepted_rpcs_in_both_delivery_modes() {
    for delivery in [InProcessEventDelivery::BestEffort, InProcessEventDelivery::Lossless] {
        let (_home, args) = tests::build_test_start_args(SessionSource::Cli, /*channel_capacity*/ 4).await;
        let store = Arc::new(InMemoryThreadStore::default());
        let client = start_with_options(
            args,
            InProcessStartOptions::default()
                .with_thread_store(Arc::clone(&store) as Arc<dyn ThreadStore>)
                .with_event_delivery(delivery),
        ).await.expect("runtime starts");
        let mut responses = Vec::new();
        // Queue requests synchronously, then close admission before waiting on any
        // result. The runtime must keep its RPC gate open through response draining.
        for id in 1..=3 {
            let (response_tx, response_rx) = oneshot::channel();
            client.client.try_send_client_message(InProcessClientMessage::Request {
                request: Box::new(ClientRequest::ThreadList {
                    request_id: RequestId::Integer(id),
                    params: serde_json::from_value(serde_json::json!({})).expect("list params"),
                }),
                response_tx,
                cancellation: tokio_util::sync::CancellationToken::new(),
            }).expect("request accepted before shutdown");
            responses.push(response_rx);
        }
        client.shutdown().await.expect("accepted work drains gracefully");
        for response in responses {
            let value = response.await.expect("response sender survives").expect("list RPC completes");
            let result: ThreadListResponse = serde_json::from_value(value).expect("typed list response");
            assert!(result.data.is_empty());
        }
        assert_eq!(store.calls().await.list_threads, 3);
    }
}

#[tokio::test]
async fn in_process_shutdown_rejects_late_requests_on_retained_senders() {
    let (_home, args) = tests::build_test_start_args(
        SessionSource::Cli, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    ).await;
    let mut client = start_with_options(
        args, InProcessStartOptions::default().with_event_delivery(InProcessEventDelivery::Lossless),
    ).await.expect("runtime starts");
    let sender = client.sender();
    let shutdown = client.begin_shutdown().await.expect("shutdown admitted");
    while timeout_at(shutdown.deadline, client.next_event())
        .await.expect("event drain remains bounded").is_some() {}
    let error = sender.request(ClientRequest::ConfigRequirementsRead {
        request_id: RequestId::Integer(99),
        params: None,
    }).await.expect_err("retained sender cannot reopen admission");
    assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    client.finish_shutdown(shutdown).await.expect("retained sender does not stall shutdown");
}
