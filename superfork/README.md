# Personalised harness superfork

For the optional coordination/memory/metrics CLI preset, see [EDGE.md](EDGE.md). It activates existing upstream features without changing your authentication, model or permission settings. The two new embedding APIs remain separate opt-in host APIs.

This fork preserves thehumanworks' ChatGPT environment-authentication changes and
adds two opt-in embedding capabilities: host-owned thread persistence and bounded,
lossless in-process app-server events with a two-phase shutdown lifecycle.

The implementation is an adaptation of four pinned upstream commits, not an
unreviewed branch-tip merge. See [SELECTION.md](SELECTION.md) for rankings,
missingness, source authorship, exclusions, and discovery limits. The original
fork and upstream starting SHAs are recorded in [evidence/baseline.tsv](evidence/baseline.tsv).
Executed checks and limitations are in `evidence/`; successful checkpoint evidence
is not, by itself, proof of final default-branch promotion.

## Build and run

Use the repository's pinned Rust toolchain (`codex-rs/rust-toolchain.toml`) and the
existing [source-build prerequisites](../docs/install.md). On Ubuntu the CI runner
also installs `pkg-config libcap-dev libasound2-dev bubblewrap clang lld`.

```sh
git clone https://github.com/thehumanworks/codex.git
cd codex/codex-rs
rustup show
cargo build --locked -p codex-cli
cargo run --locked --bin codex -- --help
cargo run --locked --bin codex
```

For an optimized local executable, use `cargo build --locked --release -p codex-cli`.
Do not assume a development build's timing represents release performance. No
packages are published by this integration. The CI workflow uses the repository's
checksum-verified V8 setup action; default Cargo builds otherwise follow upstream's
V8 download mechanism and need network access on a cold cache.

Existing authentication remains documented in [FORK.md](../FORK.md). This change
neither introduces another credential store nor prints credentials in verification
logs. The synthetic loopback/PTY acceptance runner is
`scripts/test_chatgpt_env_auth.py`.

## Enable the new embedding APIs

These options belong to `codex_app_server::in_process`. They are **not** new CLI
flags, and they do not silently change the TUI, `codex exec`, websocket, or stdio
transport policy. Existing callers of `start(args)` retain the default behavior.

For a host that already constructs `InProcessStartArgs`:

```rust,ignore
use codex_app_server::in_process::{
    InProcessEventDelivery, InProcessStartOptions, start_with_options,
};
use codex_thread_store::InMemoryThreadStore;
use std::sync::Arc;

let store = Arc::new(InMemoryThreadStore::default());
let options = InProcessStartOptions::default()
    .with_thread_store(store.clone())
    .with_event_delivery(InProcessEventDelivery::Lossless);
let mut client = start_with_options(args, options).await?;
```

An injected `Arc<dyn ThreadStore>` is process-scoped and survives runtime restarts
when the host retains the same store. Its durability is the implementation's
responsibility. `InMemoryThreadStore` is **not durable across process death**.
This fork does not implement cloud synchronization, a new database, or cross-host
replication. The tests exercise actual typed thread requests and runtime restart,
not just constructor fields.

An injected store deliberately does **not** inherit the configuration's unrelated
SQLite user-message queue. Queue operations report that the queue is unavailable.
Configuration-derived local stores keep their existing queue behavior. A future
custom queue API would require an explicit, coherent backend contract.

## Event consumption and shutdown

Lossless delivery uses bounded channels and backpressure, not an unbounded buffer.
Keep consuming `next_event()` concurrently with request work. Awaiting a request
while refusing to consume a saturated event stream can stall the producer by design.
Cloned senders allow independent request producers while one owner consumes events.
Request admission still reports overload explicitly; lossless events do not turn
command queues into unlimited storage.

When final events matter, use the two-phase lifecycle:

```rust,ignore
let shutdown = client.begin_shutdown().await?;
while let Some(event) = client.next_event().await {
    // Process final notifications; do not start new requests during shutdown.
    consume(event);
}
client.finish_shutdown(shutdown).await?;
```

`begin_shutdown` uses a separate bounded control channel and rejects duplicate
attempts. Already accepted requests receive responses or explicit errors. Client
notification admission retains its existing best-effort queue policy. Outstanding
server approval requests are failed rather than waiting indefinitely for responses
after command admission closes.

`finish_shutdown` joins against the original 35-second deadline, including the
existing analytics budget. The processor and outbound drains each have a five-second
budget. A stopped event consumer or a drain timeout is reported as an error, not as
successful lossless delivery. Canceling the join future aborts its owned runtime
worker handles. This is not a claim that every detached upstream task or external
process is forcibly terminated.

`client.shutdown().await` is the convenience alternative: it drains and discards
final events before joining. Do not use it when your application must observe those
events. Enqueue acknowledgement is not a durable commit or proof that a host has
processed an event. There is no disk-backed replay or exactly-once guarantee.

## Reproduce verification

Install `just`, `cargo-nextest`, DotSlash, and `uv` as required by the repository.
From the repository root, run formatting/linting before tests:

```sh
just fix -p codex-app-server --locked
just fmt
just fmt-check
just test --locked -p codex-app-server --lib -E 'test(in_process)'
just test --locked -p codex-app-server -p codex-app-server-client -p codex-login
just test --locked
cd codex-rs
cargo build --locked -p codex-cli
cd ..
python3 scripts/test_chatgpt_env_auth.py --codex-bin codex-rs/target/debug/codex
```

Set the final binary path to `$CARGO_TARGET_DIR/debug/codex` when overriding Cargo's
target directory. Linux CI is the executed platform; macOS, Windows, and release
performance are separate verification scopes unless explicitly recorded otherwise.
No live ChatGPT inference or paid API call is needed by the custom auth smoke suite.
