#!/usr/bin/env python3
"""Adapt existing auth controls to the current handshake and source layout.

No auth assertions are removed. The same suite exercises an unpackaged,
embedded TUI or an explicitly supplied complete package with a shared daemon.
"""

from pathlib import Path

path = Path("scripts/test_chatgpt_env_auth.py")
text = path.read_text()
if "PACKAGED_TUI = False" in text:
    raise SystemExit("Current auth smoke adaptation is already applied.")
replacements = [
    ("BINARY = None\n", "BINARY = None\nPACKAGED_TUI = False\n"),
    (
        "        self.configure()\n\n    def configure(",
        """        self.configure()
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

    def configure(""",
    ),
    (
        '            account = rpc(2, "account/read", {"refreshToken": False})\n',
        """            # Complete the public initialize/initialized handshake before RPCs.
            process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "initialized"}) + "\\n")
            process.stdin.flush()
            account = rpc(2, "account/read", {"refreshToken": False})
            self.assertIn("result", account)
""",
    ),
    (
        '            [str(BINARY), "--no-alt-screen", "hello"],\n',
        """            [str(BINARY), *([] if PACKAGED_TUI else ["--no-daemon"]), "--no-alt-screen", "hello"],
""",
    ),
    (
        '            self.assertNotIn("Sign in with ChatGPT", text)\n',
        """            self.assertNotIn("Sign in with ChatGPT", text)
            if PACKAGED_TUI:
                self.assertNotIn("Running without the shared background server", text)
                self.assertTrue(
                    (self.home / "app-server-daemon" / "daemon.pid").is_file(),
                    "packaged TUI did not start its isolated shared daemon",
                )
""",
    ),
    (
        "            os.close(master)\n        self.assert_no_auth_file()\n",
        "            os.close(master)\n        if PACKAGED_TUI:\n            self.stop_test_daemon()\n        self.assert_no_auth_file()\n",
    ),
    (
        '    parser.add_argument("--codex-bin", required=True, type=Path)\n',
        """    parser.add_argument("--codex-bin", required=True, type=Path)
    parser.add_argument("--packaged-tui", action="store_true", help="Test shared-server TUI using a complete local package")
""",
    ),
    (
        "    BINARY = args.codex_bin.resolve(strict=True)\n",
        "    BINARY = args.codex_bin.resolve(strict=True)\n    PACKAGED_TUI = args.packaged_tui\n",
    ),
]
for old, new in replacements:
    if text.count(old) != 1:
        raise SystemExit(f"Auth smoke source drifted at {old[:70]!r}")
    text = text.replace(old, new)
compile(text, str(path), "exec")
path.write_text(text)
print(
    "Adapted handshake and explicit TUI modes; all 17 authentication controls remain."
)
