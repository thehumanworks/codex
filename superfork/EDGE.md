# Edge CLI preset

The fork's new embedding APIs are documented in [README.md](README.md). This optional launcher makes a capable combination of **existing upstream CLI features** easy to use. It is not an additional unreleased upstream implementation and does not make the CLI use the in-process event API.

## Build and start

From the repository root, build the actual fork rather than using an unrelated globally installed Codex:

```sh
cd codex-rs
cargo build --locked --release -p codex-cli
cd ..
python3 superfork/edge.py -- exec 'Inspect this project and complete the requested work'
```

For an existing development build, specify the executable explicitly:

```sh
python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex --
python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex -- resume --last
python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex -- features list
```

Use `$CARGO_TARGET_DIR/debug/codex` when Cargo's target directory is overridden, or set `CODEX_EDGE_BIN` to your fork executable. Python 3.10+ and an executable local build are required. The current verification target is Linux; other operating systems need their own native CLI checks.

## Enabled capabilities

| Feature key | Role in this preset | Important boundary |
| --- | --- | --- |
| `multi_agent_v2` | Enables the newer multi-agent coordination tool surface. | Agents still use the configured model and permissions; additional agents can consume additional model usage. |
| `agent_message_board` | Adds the shared message-board surface for supported persistent multi-agent sessions. | Experimental; not a general cross-host message bus and does not independently wake idle agents. |
| `memories` | Enables Codex's existing memory capability. | Existing eligibility, configuration and persistence rules still apply; enabling the flag is not proof that a memory has been generated. |
| `runtime_metrics` | Exposes existing runtime metrics for diagnosis. | Experimental diagnostics, not a latency or throughput guarantee. |
| `goals` | Enables goal-oriented execution support. | Does not add an independent model-based completion verifier. |
| `hooks` | Enables the existing hook system. | Runs only hooks permitted by the existing configuration and trust mechanisms. No hook commands are installed by this preset. |

These names are pinned to the source tree in the accompanying CLI verification evidence. The launcher does not bypass unsupported models, managed feature restrictions, or missing runtime prerequisites.

## Preserve user control

The launcher passes literal argument vectors directly to `os.execv`; it never runs a shell expression or prints arguments/environment variables. It leaves the caller's current directory, authentication, model choice, approval policy and sandbox settings alone. It does not rewrite `~/.codex/config.toml` or copy credentials.

Explicit exclusions retain Codex's normal disable precedence:

```sh
python3 superfork/edge.py -- --disable memories --disable agent_message_board exec 'Review this change'
```

Normal Codex remains available without the preset by running the built binary directly. Only the current process receives the preset flags.

## Verification

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s superfork -p 'test_*.py' -v
python3 superfork/verify_edge_cli.py --codex-bin ./codex-rs/target/debug/codex
```

The native control uses an isolated temporary `CODEX_HOME`, checks the real CLI's feature-resolution output both with and without explicit disables, and verifies that configuration and credentials are not written. It does not perform live inference or validate model-dependent multi-agent task quality. Read `evidence/edge-cli.json` for executed results when available; the presence of this document alone is not passing verification evidence.

## Scope and checkpoints

The feature-selection record is [SELECTION.md](SELECTION.md). Two actual upstream-derived embedding capabilities are selected: injectable thread storage and bounded lossless events with shutdown draining. The CLI preset and checkpoint infrastructure are fork-specific additions, counted separately.

Source, formatting, acceptance, affected-suite, CLI-smoke and workspace stages are committed independently. Failed or incomplete checks do not advance the default branch. Immutable checkpoint aliases are secondary recovery references; a GitHub token failure to create an alias does not erase a commit already pushed to the integration branch.
