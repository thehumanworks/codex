#!/usr/bin/env python3
"""Launch this fork with its coordination, memory and diagnostics preset.

Example: python3 superfork/edge.py --codex-bin ./codex-rs/target/debug/codex -- exec 'Review this project'
The preset changes only feature flags for this process. User-supplied --disable
flags retain Codex's normal precedence. Authentication, models, approval policy,
sandbox settings, environment and the caller's working directory are unchanged.
"""

import argparse
import os
from pathlib import Path
from typing import Sequence

FEATURES = (
    "multi_agent_v2",
    "agent_message_board",
    "memories",
    "runtime_metrics",
    "goals",
    "hooks",
)


def command(binary: Path, arguments: Sequence[str]) -> list[str]:
    """Build an argv, never a shell expression, without changing caller arguments."""
    return [
        str(binary),
        *[part for feature in FEATURES for part in ("--enable", feature)],
        *arguments,
    ]


def main(argv: Sequence[str] | None = None) -> None:
    root = Path(__file__).resolve().parent.parent
    default = os.environ.get(
        "CODEX_EDGE_BIN", str(root / "codex-rs/target/release/codex")
    )
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-bin", type=Path, default=Path(default))
    parser.add_argument(
        "codex_args", nargs=argparse.REMAINDER, help="Codex arguments, after --"
    )
    args = parser.parse_args(argv)
    try:
        binary = args.codex_bin.expanduser().resolve(strict=True)
    except OSError as error:
        parser.error(
            f"Fork executable is unavailable: {error}. Build codex-cli or set --codex-bin."
        )
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("--codex-bin must name an executable file")
    arguments = args.codex_args
    if arguments[:1] == ["--"]:
        arguments = arguments[1:]
    # exec preserves the native terminal, signal handling, exit status and env.
    # In particular, never print argv/env (they may contain user credentials).
    os.execv(str(binary), command(binary, arguments))


if __name__ == "__main__":
    main()
