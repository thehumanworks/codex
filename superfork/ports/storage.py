#!/usr/bin/env python3
"""Apply the reviewed storage port to the preserved baseline, once.

The integration workflow merges the pinned upstream source for authorship, restores
its four outdated paths from the pre-merge tree, then runs these checked edits.
Unexpected source changes fail closed rather than selecting ours/theirs globally.
"""
from pathlib import Path

root = Path.cwd()


def edit(path, old, new, count=1):
    target = root / path
    text = target.read_text()
    assert text.count(old) == count, (path, text.count(old), old[:100])
    target.write_text(text.replace(old, new))


path = "codex-rs/app-server/src/in_process.rs"
edit(path, "use codex_protocol::protocol::SessionSource;", "use codex_protocol::protocol::SessionSource;\nuse codex_thread_store::ThreadStore;")
edit(path, "/// Event emitted from the app-server to the in-process client.", """/// Optional host overrides for the embedded runtime.
///
/// Defaults preserve config-derived persistence and the existing transport behavior.
#[derive(Clone, Default)]
pub struct InProcessStartOptions {
    thread_store: Option<Arc<dyn ThreadStore>>,
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
}

/// Event emitted from the app-server to the in-process client.""")
edit(path, "pub async fn start(mut args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {", """pub async fn start(args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {
    start_with_options(args, InProcessStartOptions::default()).await
}

/// Starts an embedded runtime with explicit host overrides and the normal handshake.
///
/// See [`InProcessStartOptions`] for persistence ownership and queue limitations.
pub async fn start_with_options(
    mut args: InProcessStartArgs,
    options: InProcessStartOptions,
) -> IoResult<InProcessClientHandle> {""")
edit(path, "let client = start_uninitialized(args).await?;", "let client = start_uninitialized(args, options).await?;")
edit(path, "async fn start_uninitialized(args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {", """async fn start_uninitialized(
    args: InProcessStartArgs,
    options: InProcessStartOptions,
) -> IoResult<InProcessClientHandle> {""")
edit(path, "                state_db: args.state_db,", "                state_db: args.state_db,\n                thread_store: options.thread_store,")
edit(path, """    async fn start_test_client_with_capacity(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> InProcessClientHandle {
        let codex_home""", """    pub(super) async fn build_test_start_args(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> (TempDir, InProcessStartArgs) {
        let codex_home""")
edit(path, '        let mut client = start(args).await.expect("in-process runtime should start");', """        (codex_home, args)
    }

    async fn start_test_client_with_capacity(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> InProcessClientHandle {
        let (codex_home, args) = build_test_start_args(session_source, channel_capacity).await;
        let mut client = start(args).await.expect("in-process runtime should start");""")
with (root / path).open("a") as output:
    output.write('\n#[cfg(test)]\n#[path = "in_process_stores_tests.rs"]\nmod stores_tests;\n')

path = "codex-rs/app-server/src/message_processor.rs"
edit(path, "    pub(crate) state_db: Option<StateDbHandle>,", "    pub(crate) state_db: Option<StateDbHandle>,\n    pub(crate) thread_store: Option<Arc<dyn ThreadStore>>,")
edit(path, "            state_db,\n            config_warnings,", "            state_db,\n            thread_store,\n            config_warnings,")
edit(path, """        let thread_store = codex_core::thread_store_from_config(config.as_ref(), state_db.clone());
        // Queue persistence requires SQLite, so in-memory thread stores and
        // app servers without a state database do not have a queue backend.
        let queue_store: Option<Arc<dyn QueueStore>> = match &config.experimental_thread_store {
            ThreadStoreConfig::Local => state_db.as_ref().map(|state_db| {
                Arc::new(LocalQueueStore::new(Arc::clone(state_db))) as Arc<dyn QueueStore>
            }),
            ThreadStoreConfig::InMemory { .. } => None,
        };""", """        // An injected thread store must not accidentally use an unrelated local
        // SQLite queue. Queue persistence is available only for config-derived
        // local stores with a state database; custom stores opt out explicitly.
        let queue_store: Option<Arc<dyn QueueStore>> = if thread_store.is_some() {
            None
        } else {
            match &config.experimental_thread_store {
                ThreadStoreConfig::Local => state_db.as_ref().map(|state_db| {
                    Arc::new(LocalQueueStore::new(Arc::clone(state_db))) as Arc<dyn QueueStore>
                }),
                ThreadStoreConfig::InMemory { .. } => None,
            }
        };
        let thread_store = thread_store.unwrap_or_else(|| {
            codex_core::thread_store_from_config(config.as_ref(), state_db.clone())
        });""")

# Preserve default storage in every current constructor, including newer tests.
for target in (root / "codex-rs/app-server/src").rglob("*.rs"):
    if target.name in {"message_processor.rs", "in_process.rs"}:
        continue
    text = target.read_text()
    if "MessageProcessorArgs {" not in text:
        continue
    result, in_args, added = [], False, 0
    for line in text.splitlines(keepends=True):
        if "MessageProcessorArgs {" in line:
            in_args = True
        result.append(line)
        if in_args and ("state_db:" in line or line.strip() == "state_db,"):
            result.append(line[:len(line) - len(line.lstrip())] + "thread_store: None,\n")
            in_args, added = False, added + 1
    assert added, target
    target.write_text("".join(result))
