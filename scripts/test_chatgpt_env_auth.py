#!/usr/bin/env python3
"""Hermetic black-box tests for this fork's ChatGPT environment authentication.

Run: python3 scripts/test_chatgpt_env_auth.py --codex-bin codex-rs/target/debug/codex
The service is a loopback mock; no real tokens, browser login, or paid API calls.
The PTY checks run on Linux/macOS; other checks also run on Windows.
"""

import argparse
import base64
import http.server
import json
import os
import queue
from pathlib import Path
import select
import subprocess
import tempfile
import threading
import time
import unittest

BINARY = None
PACKAGED_TUI = False
REPLY = "environment-auth-verified"


def jwt(claims):
    payload = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
    return f"eyJhbGciOiJIUzI1NiJ9.{payload}.synthetic-signature"


TOKEN = jwt(
    {
        "exp": 4102444800,
        "email": "fixture@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "account-from-token",
            "chatgpt_plan_type": "pro",
        },
    }
)


class Backend(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self, status, body, content_type="application/json"):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.server.requests.append(("GET", self.path, dict(self.headers)))
        if self.path.split("?")[0].endswith("/models"):
            self.respond(200, {"models": []})
        else:
            self.respond(404, {"error": "not found"})

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.server.requests.append(("POST", self.path, dict(self.headers)))
        if self.path.split("?")[0].endswith("/responses"):
            if self.server.reject:
                self.respond(
                    401,
                    {"error": {"message": "token rejected", "type": "invalid_token"}},
                )
                return
            events = [
                {"type": "response.created", "response": {"id": "resp-env"}},
                {
                    "type": "response.output_item.done",
                    "item": {
                        "type": "message",
                        "id": "msg-env",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": REPLY}],
                    },
                },
                {
                    "type": "response.completed",
                    "response": {
                        "id": "resp-env",
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 1,
                            "total_tokens": 2,
                        },
                    },
                },
            ]
            self.respond(
                200,
                "".join(f"data: {json.dumps(event)}\n\n" for event in events),
                "text/event-stream",
            )
        else:
            self.respond(404, {"error": "not found"})


class EnvironmentAuthTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="codex-env-auth-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "codex-home"
        self.home.mkdir()
        self.cwd = self.root / "project"
        self.cwd.mkdir()
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
        self.server.requests = []
        self.server.reject = False
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        self.env = {
            key: value
            for key, value in os.environ.items()
            if key
            in {
                "PATH",
                "SYSTEMROOT",
                "WINDIR",
                "COMSPEC",
                "PATHEXT",
                "TMP",
                "TEMP",
            }
        }
        self.env.update(
            {
                "HOME": str(self.root),
                "USERPROFILE": str(self.root),
                "CODEX_HOME": str(self.home),
                "CODEX_SQLITE_HOME": str(self.home),
                "TERM": "xterm-256color",
                "LANG": "C.UTF-8",
                "CHATGPT_AUTH_TOKEN": TOKEN,
                "CODEX_REFRESH_TOKEN_URL_OVERRIDE": f"{self.base}/oauth/token",
            }
        )
        self.configure()
        if PACKAGED_TUI:
            settings = self.home / "app-server-daemon" / "settings.json"
            settings.parent.mkdir()
            settings.write_text(json.dumps({
                "remoteControlEnabled": False,
                "shutdownGraceSeconds": 5,
                "updater": {"autoUpdateEnabled": False},
            }))
            self.addCleanup(self.stop_test_daemon)

    def stop_test_daemon(self):
        if not (self.home / "app-server-daemon" / "daemon.pid").exists():
            return
        result = subprocess.run(
            [str(BINARY), "app-server", "daemon", "stop"],
            cwd=self.cwd, env=self.env, capture_output=True, text=True, timeout=30,
        )
        output = result.stdout + result.stderr
        self.assertNotIn(TOKEN, output, "credential leaked in daemon output")
        self.assertEqual(result.returncode, 0, output)

    def configure(self, extra="", store="file"):
        (self.home / "config.toml").write_text(
            f'cli_auth_credentials_store = "{store}"\n'
            f'chatgpt_base_url = "{self.base}/backend-api"\n'
            f'openai_base_url = "{self.base}/v1"\n'
            'model = "gpt-5.6-sol"\ncheck_for_update_on_startup = false\n'
            'approval_policy = "never"\n'
            f"{extra}\n"
            f'[projects.{json.dumps(str(self.cwd))}]\ntrust_level = "trusted"\n',
            encoding="utf-8",
        )

    def run_cli(self, *args, success=True):
        result = subprocess.run(
            [str(BINARY), *args],
            cwd=self.cwd,
            env=self.env,
            input="",
            capture_output=True,
            text=True,
            timeout=45,
        )
        output = result.stdout + result.stderr
        self.assertNotIn(TOKEN, output, "credential leaked in CLI output")
        self.assertEqual(result.returncode == 0, success, output)
        return output

    def assert_no_auth_file(self):
        self.assertFalse((self.home / "auth.json").exists())
        for path in self.home.rglob("*"):
            if path.is_file():
                self.assertFalse(
                    TOKEN.encode() in path.read_bytes(),
                    f"credential persisted in {path.name}",
                )

    def assert_request_auth(self, account="account-from-token", token=TOKEN):
        requests = [
            headers
            for method, path, headers in self.server.requests
            if method == "POST" and path.endswith("/responses")
        ]
        self.assertTrue(requests, self.server.requests)
        for headers in requests:
            lowered = {key.lower(): value for key, value in headers.items()}
            self.assertEqual(lowered.get("authorization"), f"Bearer {token}")
            self.assertEqual(lowered.get("chatgpt-account-id"), account)
        self.assert_no_auth_file()

    def test_login_and_status_use_environment_without_writing_cache(self):
        for args in [("login",), ("login", "status")]:
            self.assertIn("ChatGPT (CHATGPT_AUTH_TOKEN)", self.run_cli(*args))
            self.assert_no_auth_file()

    def test_blank_token_and_account_only_do_not_login(self):
        self.env["CHATGPT_ACCOUNT_ID"] = "unused"
        for token in ["", " \t\n"]:
            self.env["CHATGPT_AUTH_TOKEN"] = token
            self.assertIn(
                "Not logged in", self.run_cli("login", "status", success=False)
            )
        del self.env["CHATGPT_AUTH_TOKEN"]
        self.assertIn("Not logged in", self.run_cli("login", "status", success=False))

    def test_cached_api_key_still_works_without_environment_token(self):
        del self.env["CHATGPT_AUTH_TOKEN"]
        (self.home / "auth.json").write_text('{"OPENAI_API_KEY":"sk-fixture"}')
        self.assertIn("Logged in using an API key", self.run_cli("login", "status"))

    def test_environment_overrides_corrupt_cache_without_reading_or_writing_it(self):
        auth = self.home / "auth.json"
        original = "{this cached file is deliberately invalid"
        auth.write_text(original)
        self.assertIn("CHATGPT_AUTH_TOKEN", self.run_cli("login", "status"))
        self.assertEqual(auth.read_text(), original)

    def test_environment_bypasses_credential_stores(self):
        for store in ["file", "ephemeral", "keyring", "auto"]:
            with self.subTest(store=store):
                self.configure(store=store)
                self.assertIn("CHATGPT_AUTH_TOKEN", self.run_cli("login", "status"))
                self.assert_no_auth_file()

    def test_missing_account_requires_only_account_override(self):
        self.env["CHATGPT_AUTH_TOKEN"] = jwt({"exp": 4102444800})
        self.assertIn(
            "set CHATGPT_ACCOUNT_ID", self.run_cli("login", "status", success=False)
        )
        self.env["CHATGPT_ACCOUNT_ID"] = "explicit-account"
        self.assertIn("CHATGPT_AUTH_TOKEN", self.run_cli("login", "status"))
        self.assert_no_auth_file()

    def test_invalid_tokens_fail_closed_without_secret_in_errors(self):
        for token, message in [
            ("private-invalid-token", "JWT"),
            (jwt({"exp": 0}), "expired"),
        ]:
            self.env["CHATGPT_AUTH_TOKEN"] = token
            for args in [
                ("login", "status"),
                ("exec", "--skip-git-repo-check", "hello"),
            ]:
                output = self.run_cli(*args, success=False)
                self.assertIn(message, output)
                self.assertNotIn(token, output)
        self.assert_no_auth_file()

    @unittest.skipUnless(os.name == "posix", "requires Unix byte environments")
    def test_invalid_unicode_credentials_report_safe_errors(self):
        for name in ["CHATGPT_AUTH_TOKEN", "CHATGPT_ACCOUNT_ID"]:
            with self.subTest(variable=name):
                self.env["CHATGPT_AUTH_TOKEN"] = TOKEN
                self.env[name] = "private-invalid-unicode-\udcff"
                output = self.run_cli("login", "status", success=False)
                self.assertIn(f"{name} must contain valid Unicode", output)
                self.assertNotIn("private-invalid-unicode", output)
                self.env.pop(name)

    def test_forced_workspace_rejects_without_deleting_cache(self):
        self.configure('forced_chatgpt_workspace_id = ["other-account"]')
        auth = self.home / "auth.json"
        original = '{"OPENAI_API_KEY":"sk-untouched"}'
        auth.write_text(original)
        output = self.run_cli("exec", "--skip-git-repo-check", "hello", success=False)
        self.assertIn("restricted to workspace", output)
        self.assertEqual(auth.read_text(), original)

    def test_forced_api_policy_does_not_use_chatgpt_token(self):
        self.configure('forced_login_method = "api"')
        self.assertIn("Not logged in", self.run_cli("login", "status", success=False))
        self.assert_no_auth_file()

    def test_logout_explains_environment_without_deleting_cache(self):
        auth = self.home / "auth.json"
        original = '{"OPENAI_API_KEY":"sk-untouched"}'
        auth.write_text(original)
        self.assertIn("unset it", self.run_cli("logout", success=False))
        self.assertEqual(auth.read_text(), original)

    def test_exec_sends_derived_auth_headers_without_cache(self):
        output = self.run_cli("exec", "--skip-git-repo-check", "hello")
        self.assertIn(REPLY, output)
        self.assert_request_auth()

    def test_exec_json_sends_explicit_account_without_cache(self):
        self.env["CHATGPT_ACCOUNT_ID"] = "selected-account"
        output = self.run_cli("exec", "--json", "--skip-git-repo-check", "hello")
        self.assertIn(REPLY, output)
        self.assert_request_auth(account="selected-account")

    def test_exec_preserves_codex_api_key_precedence(self):
        self.env["CODEX_API_KEY"] = "sk-explicit-api"
        self.assertIn(REPLY, self.run_cli("exec", "--skip-git-repo-check", "hello"))
        self.assert_request_auth(account=None, token="sk-explicit-api")

    def test_rejected_token_does_not_attempt_oauth_or_write_cache(self):
        self.server.reject = True
        output = self.run_cli("exec", "--skip-git-repo-check", "hello", success=False)
        self.assertIn("supply a fresh access token", output)
        self.assertFalse(
            any("oauth/token" in path for _, path, _ in self.server.requests)
        )
        self.assertLessEqual(
            sum(
                method == "POST" and path.endswith("/responses")
                for method, path, _ in self.server.requests
            ),
            3,
        )
        self.assert_no_auth_file()

    def test_app_server_logout_preserves_cached_credentials(self):
        auth = self.home / "auth.json"
        original = '{"OPENAI_API_KEY":"sk-untouched"}'
        auth.write_text(original)
        process = subprocess.Popen(
            [str(BINARY), "app-server"],
            cwd=self.cwd,
            env=self.env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        incoming = queue.Queue()

        def reader():
            for line in process.stdout:
                incoming.put(json.loads(line))

        thread = threading.Thread(target=reader, daemon=True)
        thread.start()

        def rpc(request_id, method, params=None):
            message = {"jsonrpc": "2.0", "id": request_id, "method": method}
            if params is not None:
                message["params"] = params
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                message = incoming.get(timeout=max(0.01, deadline - time.monotonic()))
                if message.get("id") == request_id:
                    return message
            self.fail("app-server did not respond")

        try:
            self.assertIn(
                "result",
                rpc(
                    1,
                    "initialize",
                    {"clientInfo": {"name": "env-auth-test", "version": "1"}},
                ),
            )
            # Complete the public initialize/initialized handshake before RPCs.
            process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "initialized"}) + "\n")
            process.stdin.flush()
            account = rpc(2, "account/read", {"refreshToken": False})
            self.assertIn("result", account)
            self.assertEqual(account["result"]["account"]["type"], "chatgpt")
            response = rpc(3, "account/logout")
            self.assertIn("unset it", response["error"]["message"])
            self.assertEqual(auth.read_text(), original)
            self.assertFalse(
                any("oauth/" in path for _, path, _ in self.server.requests)
            )
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)
            thread.join(timeout=5)
            process.stdout.close()

    @unittest.skipUnless(os.name == "posix", "requires a Unix PTY")
    def test_interactive_tui_sends_request_without_login_onboarding(self):
        import fcntl
        import pty
        import struct
        import termios

        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
        process = subprocess.Popen(
            [str(BINARY), *([] if PACKAGED_TUI else ["--no-daemon"]), "--no-alt-screen", "hello"],
            cwd=self.cwd,
            env=self.env,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
        )
        os.close(slave)
        output = b""
        answered = set()
        queries = {
            b"\x1b[6n": b"\x1b[1;1R",
            b"\x1b[?u": b"\x1b[?0u\x1b[?1;2c",
            b"\x1b]10;?": b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
            b"\x1b]11;?": b"\x1b]11;rgb:0000/0000/0000\x1b\\",
        }
        try:
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.1)[0]:
                    try:
                        output += os.read(master, 65536)
                    except OSError:
                        break
                    for query, response in queries.items():
                        if query in output and query not in answered:
                            os.write(master, response)
                            answered.add(query)
                    if REPLY.encode() in output:
                        break
                if process.poll() is not None:
                    break
            text = output.decode(errors="replace")
            self.assertIn(REPLY, text)
            self.assertNotIn(TOKEN, text)
            self.assertNotIn("Sign in with ChatGPT", text)
            if PACKAGED_TUI:
                self.assertNotIn("Running without the shared background server", text)
                self.assertTrue(
                    (self.home / "app-server-daemon" / "daemon.pid").is_file(),
                    "packaged TUI did not start its isolated shared daemon",
                )
            self.assert_request_auth()
        finally:
            if process.poll() is None:
                os.write(master, b"\x03")
                time.sleep(0.15)
                os.write(master, b"\x03")
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    process.wait(timeout=5)
            os.close(master)
        if PACKAGED_TUI:
            self.stop_test_daemon()
        self.assert_no_auth_file()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-bin", required=True, type=Path)
    parser.add_argument("--packaged-tui", action="store_true", help="Test shared-server TUI using a complete local package")
    args, remaining = parser.parse_known_args()
    BINARY = args.codex_bin.resolve(strict=True)
    PACKAGED_TUI = args.packaged_tui
    unittest.main(argv=[__file__, *remaining], verbosity=2)
