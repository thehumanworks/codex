# ChatGPT environment authentication: verification record

Completed: **2026-09-11T14:42:44+00:00**. Implementation commit: `fab2e63e55d1a833dd3560b9435545daf002fe74`. Upstream baseline: `7a6f469dcff1337786aae046df2d899accea59ab`.

The implementation was built and exercised in a Linux x86-64 Modal sandbox using Rust 1.95.0. The source and complete raw verification logs are retained in the accompanying persistent Modal workspace snapshot. Recorded runner output is in [results.txt](results.txt).

## Results

| Check | Result |
| --- | --- |
| `cargo build --locked -p codex-cli --bin codex` | Passed; actual CLI binary built |
| `just test --locked -p codex-login -p codex-protocol -p codex-shell-command` | **755 passed**, zero runner-reported skips |
| `just test --locked -p codex-cli --test login` | **7 passed**, zero runner-reported skips |
| `python3 scripts/test_chatgpt_env_auth.py --codex-bin codex-rs/target/debug/codex` | **17 passed**, including a real interactive PTY and app-server JSON-RPC |
| `cargo clippy --locked --tests -p codex-login -p codex-cli -p codex-protocol -p codex-shell-command` | Passed; no warnings in the final check |
| `just fmt-check` | Passed |
| `uv run --frozen --project scripts ruff check scripts/test_chatgpt_env_auth.py` | Passed |

The optional `just fix` pass proposed an unrelated `clippy::question_mark` cleanup in `codex-rs/shell-command/src/bash.rs`. That automatic edit was inspected and reverted to keep this fork's change scoped; the final non-mutating Clippy check passed. Final Rust sources exactly match the tested implementation commit.

## Acceptance evidence

| Invariant | Evidence |
| --- | --- |
| Interactive and headless modes accept the environment token without `auth.json` | Real CLI subprocesses complete loopback SSE requests; PTY test submits a prompt through the interactive app |
| Account ID is derived unless explicitly needed or overridden | Parser tests and HTTP assertions cover embedded, absent, blank and explicit account IDs |
| Correct HTTP authentication is sent | Mock service inspects `Authorization: Bearer ...` and `ChatGPT-Account-Id` on actual inference requests |
| No credential store dependency or token persistence | File, ephemeral, keyring and auto modes tested; temporary Codex homes scanned for the synthetic token, including after TUI exit |
| Cached credentials and existing login behavior are preserved | Corrupt cache override, ordinary cached API login, `CODEX_API_KEY` precedence, CLI/app-server logout and existing upstream login regressions |
| Invalid credentials and restrictions fail safely | Malformed/expired/non-Unicode inputs, unsafe header values and forced workspace/login-policy tests; credential values absent from new diagnostic errors |
| Environment-managed credentials are not refreshed or revoked as cached OAuth credentials | Mock 401 test verifies an actionable replacement error and no OAuth refresh request; logout leaves existing credentials unchanged |
| Token is not inherited by model-reachable children or replayed through snapshots | Shared environment-policy, real child-process scrubber and snapshot-rendering regression tests |
| Changes stay downstream | Implementation and verification commits pushed only to `thehumanworks/codex`; no upstream PR created |

The persistence scan initially found that the upstream non-brokered snapshot renderer serialized the token's exported value. The shipped fix adds the token to the shared non-inheritable set and excludes host-only exports from that renderer. The PTY persistence scan subsequently passed.

## Test boundaries

The backend is a loopback mock and the JWTs are deliberately synthetic. These tests prove the real CLI's credential loading, request headers, interactive/headless behavior, and persistence properties; they do not demonstrate live acceptance by ChatGPT. A live credential lookup was attempted without displaying its value, but the configured Doppler source could not authenticate. No live ChatGPT request was claimed or used as evidence.

The executable tested is a development build. macOS/Windows builds and the complete upstream workspace test suite were not run. The changed authentication parser is platform-independent Rust; the Python PTY and byte-environment tests are explicitly Unix-only. Runtime-specific branches in existing upstream tests retain their original platform checks.

## Raw-log integrity

These are SHA-256 digests of the full successful-run logs retained with the Modal snapshot, not only the summaries above.

| Log | SHA-256 |
| --- | --- |
| `auth-and-shell-tests.log` | `a0389454e1fe7ffb333b10c354599b6366ffe04ab0da92009f6e09226b3155f4` |
| `cli-login-tests.log` | `e8f502ceda807bc15bb8f9934668bfaa2fd54fb791716b012f7094ffb0bbb69d` |
| `black-box-tests.log` | `0bb4eca8649d7e40cde77fc026ae75d98a71df02d17f2cd8c93573604aab2efb` |
| `build-final.log` | `9ea2d767fdfd67eea28cd73f87425dc82c44eae893757feb83839d615c0618cd` |
| `clippy-final.log` | `f13901892809f898ce9306d73b5076f805c648a9bbb715730f0a3a9fc7f7d279` |
| `fmt-check.log` | `02e690d37e535e3e7e48497212051a4bb86f8d6bafeb7cf5412107d3948ca909` |
| `python-lint.log` | `82b3e6a6c090a57601d22943bd23fca9218d1031dbe5a7b754092f9a156b4f18` |
