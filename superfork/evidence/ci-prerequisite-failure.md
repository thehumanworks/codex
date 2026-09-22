# Broad-suite prerequisite failure and correction

Run: https://github.com/thehumanworks/codex/actions/runs/35731608136

Runtime source tested: `b466a72ce3faa9f8bedfa0152fb42d77d5a24c25`; `codex-rs` tree `5f8fab505a1d147119496b51346a0f618c27c9d3`.

## Executed outcome

- Scoped lint, formatting and formatting check passed.
- All 17 targeted in-process acceptance tests passed, with no retries.
- All nine lossless/shutdown regression cases passed in each of five repetitions, with no retries.
- The full affected command, `just test --locked -p codex-app-server -p codex-app-server-client`, executed 1,814 tests: **1,764 passed, 50 failed; two additional tests were skipped**. This run is a failure, not a passing suite.
- CLI/auth smoke and the complete workspace command did not execute in this run because the affected-suite step failed.

## Diagnosis

All 50 final failure blocks contain an unavailable cross-crate executable:

| Missing executable | Failed cases | Evidence |
| --- | ---: | --- |
| `test_stdio_server` | 24 | The binary resolver reports that `target/debug/test_stdio_server` does not exist. |
| `codex-code-mode-host` | 22 | Five cases fail binary lookup directly; 17 further cases report host-not-found/spawn failures before their expected tool results or timing events. |
| `codex` | 4 | Executor/MCP cases fail because the native CLI executable is missing. |

These are CI setup omissions: selecting only the app-server/client test packages did not build the other crates' normal executable fixtures. The assertions and feature tests are not being weakened or skipped. The correction is to build the native CLI, Code Mode host and MCP test-server binaries before the affected suite, and all workspace binaries before the complete workspace suite.

The failed run's complete logs, source archive and disk report are retained in artifact `harness-verification-35731608136-1` (artifact ID `10697027461`). Its runtime source remains unchanged for the prerequisite-corrected rerun. Until that rerun actually passes, no passing affected-suite or workspace result is claimed.
