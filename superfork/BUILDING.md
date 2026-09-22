# Build the actual superfork

The npm, Homebrew and standalone-install commands in the upstream README install upstream releases, **not these fork changes**. Build this repository and use the resulting executable explicitly. Read the executed platform and source-tree evidence in `superfork/evidence/` before treating a branch as verified.

## Native CLI and companion Code Mode host

Use the toolchain pinned in `codex-rs/rust-toolchain.toml` and the repository's source-build prerequisites. From the repository root:

```sh
cd codex-rs
rustup show
cargo build --locked --release -p codex-cli -p codex-code-mode-host --bins
cd ..
python3 superfork/edge.py --
```

Keep `codex` and `codex-code-mode-host` together in the same build output directory. The companion host is needed when Code Mode is selected; a CLI-only build is not a complete Code Mode installation. A cold build needs dependency and V8 downloads. The verified CI setup uses `.github/actions/setup-rusty-v8` to validate its V8 archive.

For an unoptimized development build, omit `--release`, then run:

```sh
python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex -- features list
python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex -- exec 'Review this repository'
```

Substitute `$CARGO_TARGET_DIR/debug/codex` when overriding Cargo's target directory. Running the binary directly leaves the edge preset off. Existing authentication is documented in [FORK.md](../FORK.md); no new credential store or login flow is introduced.

## Test prerequisites and commands

Some app-server tests spawn binaries owned by other crates. Merely testing `codex-app-server` does not build all of those fixtures. Build them explicitly instead of skipping the dependent cases:

```sh
cd codex-rs
cargo build --locked -p codex-cli -p codex-code-mode-host -p codex-rmcp-client --bins
cd ..
just fix -p codex-app-server --locked
just fmt
just fmt-check
just test --locked -p codex-app-server --lib -E 'test(in_process)' --retries 0
just test --locked -p codex-app-server -p codex-app-server-client
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s superfork -p 'test_*.py' -v
python3 scripts/test_chatgpt_env_auth.py --codex-bin ./codex-rs/target/debug/codex
python3 superfork/verify_edge_cli.py --codex-bin ./codex-rs/target/debug/codex
```

Before the complete workspace suite, build all normal workspace executables:

```sh
cd codex-rs
cargo build --locked --workspace --bins
cd ..
just test --locked
```

The repository uses `just test`/nextest rather than direct `cargo test`. CI's recorded results distinguish real failures, retries, ignored cases and checks that did not execute. Linux is the executed platform for this integration; macOS/Windows and release-performance measurements are separate scopes unless their results are explicitly present.

See [EDGE.md](EDGE.md) for the preset and [README.md](README.md) for the new embedding APIs, bounded event consumption and shutdown contracts.
