use super::*;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::ThreadListParams;
use codex_app_server_protocol::ThreadListResponse;
use codex_app_server_protocol::ThreadQueueListParams;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_thread_store::InMemoryThreadStore;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn in_process_injected_store_receives_thread_list_requests() {
    let (_home, args) =
        tests::build_test_start_args(SessionSource::Cli, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await;
    let store = Arc::new(InMemoryThreadStore::default());
    let client = start_with_options(
        args,
        InProcessStartOptions::default()
            .with_thread_store(Arc::clone(&store) as Arc<dyn ThreadStore>),
    )
    .await
    .expect("injected runtime should start");
    let params: ThreadListParams = serde_json::from_value(serde_json::json!({}))
        .expect("default list params should deserialize");
    let result = client
        .request(ClientRequest::ThreadList {
            request_id: RequestId::Integer(1),
            params,
        })
        .await
        .expect("list transport should work")
        .expect("list should succeed");
    let result: ThreadListResponse = serde_json::from_value(result).expect("typed list response");
    assert!(result.data.is_empty());
    assert_eq!(store.calls().await.list_threads, 1);
    client.shutdown().await.expect("shutdown should succeed");
}

#[tokio::test]
async fn in_process_injected_store_survives_runtime_restart_without_ambient_queue() {
    let store = Arc::new(InMemoryThreadStore::default());
    let (_first_home, mut first_args) =
        tests::build_test_start_args(SessionSource::Cli, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await;
    first_args.initialize.capabilities = Some(InitializeCapabilities {
        experimental_api: true,
        ..InitializeCapabilities::default()
    });
    let client = start_with_options(
        first_args,
        InProcessStartOptions::default()
            .with_thread_store(Arc::clone(&store) as Arc<dyn ThreadStore>),
    )
    .await
    .expect("first runtime should start");
    let created = client
        .request(ClientRequest::ThreadStart {
            request_id: RequestId::Integer(1),
            params: ThreadStartParams::default(),
        })
        .await
        .expect("start transport should work")
        .expect("thread should start");
    let created: ThreadStartResponse =
        serde_json::from_value(created).expect("typed start response");
    let thread_id = created.thread.id;
    let queue_error = client
        .request(ClientRequest::ThreadQueueList {
            request_id: RequestId::Integer(2),
            params: ThreadQueueListParams {
                thread_id: thread_id.clone(),
                cursor: None,
                limit: None,
            },
        })
        .await
        .expect("queue transport should work")
        .expect_err("injected store must not attach the ambient SQLite queue");
    assert_eq!(
        queue_error,
        invalid_request("user message queue is unavailable")
    );
    client
        .shutdown()
        .await
        .expect("first runtime should shut down");

    let (_second_home, second_args) =
        tests::build_test_start_args(SessionSource::Cli, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await;
    let client = start_with_options(
        second_args,
        InProcessStartOptions::default()
            .with_thread_store(Arc::clone(&store) as Arc<dyn ThreadStore>),
    )
    .await
    .expect("second runtime should start with the host-owned store");
    let result = client
        .request(ClientRequest::ThreadList {
            request_id: RequestId::Integer(3),
            params: serde_json::from_value(serde_json::json!({})).expect("list params"),
        })
        .await
        .expect("list transport should work")
        .expect("list should succeed");
    let result: ThreadListResponse = serde_json::from_value(result).expect("typed list response");
    assert!(result.data.iter().any(|thread| thread.id == thread_id));
    client
        .shutdown()
        .await
        .expect("second runtime should shut down");
}
