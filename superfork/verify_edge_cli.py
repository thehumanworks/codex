#!/usr/bin/env python3
"""Exercise real CLI feature resolution without inference or persistent config edits."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from edge import FEATURES


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-bin", required=True, type=Path)
    binary = parser.parse_args().codex_bin.resolve(strict=True)
    launcher = Path(__file__).with_name("edge.py").resolve()
    with tempfile.TemporaryDirectory(prefix="codex-edge-test-") as temporary:
        home = Path(temporary)
        env = dict(os.environ)
        env["CODEX_HOME"] = temporary
        for key in ("OPENAI_API_KEY", "CHATGPT_AUTH_TOKEN", "CHATGPT_ACCOUNT_ID", "CODEX_ACCESS_TOKEN", "CODEX_EDGE_BIN"):
            env.pop(key, None)
        config = home / "config.toml"
        original = 'approval_policy = "on-request"\nsandbox_mode = "read-only"\n'
        config.write_text(original)
        for disabled in ((), ("memories", "agent_message_board")):
            args = [sys.executable, str(launcher), "--codex-bin", str(binary), "--"]
            for feature in disabled:
                args.extend(("--disable", feature))
            args.extend(("features", "list"))
            result = subprocess.run(args, cwd=home, env=env, capture_output=True, text=True, check=True, timeout=45)
            states = {}
            for line in result.stdout.splitlines():
                parts = line.split()
                if len(parts) >= 3 and parts[-1] in ("true", "false"):
                    states[parts[0]] = parts[-1] == "true"
            expected = {feature: feature not in disabled for feature in FEATURES}
            actual = {feature: states.get(feature) for feature in FEATURES}
            if actual != expected:
                raise AssertionError(f"Unexpected feature resolution: {actual}; expected {expected}")
            if config.read_text() != original or (home / "auth.json").exists():
                raise AssertionError("Launcher modified persistent configuration or credentials")
            print(f"PASS: real CLI feature resolution; disabled={list(disabled)}; persistent config unchanged")
    print("PASS: 2 native CLI controls, with explicit user-disable precedence")


if __name__ == "__main__":
    main()
