# Embedded edge mode and the shared app-server daemon

The edge launcher passes feature flags to the existing CLI. The current TUI compatibility policy does not allow its six feature overrides through the shared daemon, so it selects **embedded mode**. This is intentional upstream behavior, not a new daemon capability or a disabled security check. The preset still activates the requested coordination, memory and metrics features for its process.

The new lossless event and injected-store APIs are also explicit in-process embedding APIs. They do not change stdio/websocket delivery or make every CLI session use a custom persistence backend.

## Source build versus complete local package

A raw Cargo-built **plain** interactive CLI now needs `--no-daemon` when no complete local package is available:

```sh
/path/to/codex-rs/target/debug/codex --no-daemon
```

The edge preset already requires embedded mode under the current compatibility policy. Running the plain CLI without `--no-daemon` is a different control and requires a complete package for first-time daemon setup.

Use the repository's existing package builder to assemble that layout locally. It builds/copies the CLI, Code Mode host, Linux bubblewrap, ripgrep, the patched shell and a package manifest; this does **not** publish a package or install it globally:

```sh
# From the repository root, on a supported Unix host:
target="$(cd codex-rs && rustc -vV | sed -n 's/^host: //p')"
just assemble-codex-package --target "$target" --cargo-profile release \
  --package-dir "$PWD/codex-rs/target/local-package"
```

For CI and fully locked reproducibility, first run `cargo build --locked --workspace --bins` in `codex-rs`, then pass the resulting binaries through the package builder's `--entrypoint-bin`, `--code-mode-host-bin` and platform-specific `--bwrap-bin` arguments. The verification workflow records the exact source and executable hashes. Upstream-provided auxiliary assets use the existing package-builder verification mechanism.

## Keep daemon identity explicit

An existing daemon and its managed package can be reused even when another CLI checkout is invoked. Therefore, launching a newly built fork does not by itself prove that a previously installed background server also runs that fork.

Use an explicitly chosen, isolated `CODEX_HOME` when validating a fork's packaged daemon rather than overwriting an existing installation. The daemon inherits its environment when it starts; changing authentication variables in a later terminal does not update an already-running process or provide per-client credential isolation. This fork does not add such isolation.

The test suite creates a fresh home, disables remote control and automatic daemon updating there, proves that the packaged TUI starts its own daemon without embedded fallback, checks synthetic request authentication, and stops that daemon afterward. No real credentials or paid inference are required. These are test-only settings, not changes to a user's normal configuration.

## Reproduce both authentication modes

After building the source binaries and assembling the package:

```sh
# Raw source binary: embedded TUI; all other existing auth controls remain.
python3 scripts/test_chatgpt_env_auth.py \
  --codex-bin ./codex-rs/target/debug/codex

# Complete package: plain TUI using an isolated shared daemon.
python3 scripts/test_chatgpt_env_auth.py \
  --codex-bin ./codex-rs/target/local-package/bin/codex --packaged-tui
```

The app-server smoke client completes both `initialize` and `initialized` before account operations. Read the final verification evidence before claiming either control has passed. The earlier failure at that handshake and the unpackaged TUI launch is retained as a failed attempt; it is not relabelled a passing auth test.

For ordinary edge operation and event-driven coordination, see [EDGE.md](EDGE.md) and [OPERATING.md](OPERATING.md). For the host embedding contracts, see [README.md](README.md).
