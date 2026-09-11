# thehumanworks Codex: ChatGPT authentication from the environment

This is an independently maintained fork of `openai/codex`. The environment-authentication change is maintained here; it is not an upstream pull request. Based on upstream commit `7a6f469dcff1337786aae046df2d899accea59ab` (11 September 2026).

## Use

Supply a **ChatGPT OAuth access-token JWT** to the Codex process. It is not an OpenAI API key, a refresh token, an ID token, a browser cookie, or an `Authorization: Bearer ...` header. Token validity and authorization are ultimately checked by the service.

```bash
# Prefer injecting this variable with your secret manager. Do not commit tokens.
export CHATGPT_AUTH_TOKEN="<ChatGPT access token>"

# Normally omit CHATGPT_ACCOUNT_ID: Codex derives it from the access-token claims.
unset CHATGPT_ACCOUNT_ID

codex                                  # Interactive terminal UI
codex exec "Explain this repository"    # Headless execution
codex exec --json "Explain this repo"   # Headless JSON event stream
codex login status
```

When active, `codex login status` reports `Logged in using ChatGPT (CHATGPT_AUTH_TOKEN)`. A separate `codex login` step is unnecessary; running it simply checks the supplied environment credentials rather than opening a browser or writing a credential file. The status command checks local credential usability, not server-side acceptance.

`CHATGPT_ACCOUNT_ID` is required only when the token does not contain a `chatgpt_account_id` in its `https://api.openai.com/auth` claims object. It may also explicitly select the intended account/workspace:

```bash
export CHATGPT_ACCOUNT_ID="<account/workspace ID>"
codex
```

The explicit non-empty account ID overrides the embedded account claim. It must belong to an account the token is authorized to use. The service still enforces access. This is a ChatGPT account/workspace ID, not an OpenAI API organization ID.

PowerShell uses the same variables:

```powershell
$env:CHATGPT_AUTH_TOKEN = "<ChatGPT access token>"
Remove-Item Env:CHATGPT_ACCOUNT_ID -ErrorAction SilentlyContinue
codex
codex exec "Explain this repository"
```

## Behavior and precedence

- Both CLI modes use the shared `codex-login` authentication loader, including the interactive UI's local app-server. No `auth.json`, ID token, or refresh token is required. The new loader does not write file or keyring credentials; ordinary Codex configuration, session history, and logs may still be written.
- Upstream's enabled `CODEX_API_KEY` override and explicit in-memory app-server credentials retain their precedence. The ChatGPT environment token then takes precedence over `CODEX_ACCESS_TOKEN` and saved file/keyring credentials. Host-configured workload identity retains control of its authentication.
- An unset or whitespace-only token leaves existing authentication behavior unchanged. An account ID without a token has no effect. Surrounding token/account whitespace is trimmed.
- A configured malformed or expired token produces an actionable error instead of silently choosing cached credentials. Missing account metadata requests only `CHATGPT_ACCOUNT_ID`. Errors do not echo the token. Configured login-method/workspace restrictions remain enforced.
- These are externally managed access tokens. Codex has no refresh token and does not perform an OAuth refresh for them. Replace an expired or rejected token and restart the process. Environment changes in a parent shell cannot update a running child process.
- `codex logout` and local app-server logout cannot unset the launching shell's environment. They report that the variable must be unset and leave cached credentials unchanged. Run `unset CHATGPT_AUTH_TOKEN CHATGPT_ACCOUNT_ID` (or remove the PowerShell environment variables), restart Codex, and then use normal logout to remove a previously cached login.
- A remote or already-running app-server has its own environment. Supply credentials to the server process and restart it when changing tokens; setting variables only on a remote client does not reauthenticate an existing server.

The token remains a secret while in the process environment. Use a trusted secret-injection mechanism and appropriate host/process access controls. `CHATGPT_AUTH_TOKEN` is excluded from model-reachable child environments and shell-snapshot exports, even when ordinary default secret exclusions are disabled. This feature is not an isolation boundary against commands run with unrestricted access to the host.

## Build this fork

The npm `@openai/codex` package is upstream and does **not** contain this change.

```bash
git clone https://github.com/thehumanworks/codex.git codex-env-auth
cd codex-env-auth/codex-rs
cargo build --release --locked -p codex-cli --bin codex
./target/release/codex login status
```

The repository pins Rust 1.95.0. The standard upstream native build prerequisites still apply. To install the fork into Cargo's binary directory after cloning, use `cargo install --path cli --locked --force` from `codex-rs`.

## Verification

The tests use synthetic JWTs and a loopback mock service, never real credentials. They run the actual compiled CLI, including a real Unix PTY for the interactive UI and JSON-RPC for its app-server. They check outbound bearer/account headers, successful headless and interactive responses, absence of a credential file or token in the temporary Codex home, precedence, restrictions, invalid/expired inputs, and logout/401 behavior.

```bash
# From the repository root, with the pinned Rust toolchain and cargo-nextest installed:
just fmt-check
just test --locked -p codex-login -p codex-protocol -p codex-shell-command
just test --locked -p codex-cli --test login
cargo build --manifest-path codex-rs/Cargo.toml --locked -p codex-cli --bin codex
python3 scripts/test_chatgpt_env_auth.py --codex-bin codex-rs/target/debug/codex
```

The Python integration runner uses only the standard library. The PTY check runs on Linux/macOS and is explicitly skipped on Windows. See `fork/verification/REPORT.md` for the recorded run results and limitations.
