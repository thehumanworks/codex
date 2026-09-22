#!/usr/bin/env python3
"""Record successful, source-bound final checks without mutating branch refs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

from record_verification import nextest_summary


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def unittest_summary(path, expected):
    text = path.read_text()
    matches = re.findall(r"^Ran (\d+) tests in .+$", text, re.MULTILINE)
    if matches != [str(expected)] or not re.search(r"^OK$", text, re.MULTILINE):
        raise ValueError(f"Missing complete, unskipped unittest success: {path}")
    return {"tests_run": expected, "log": text}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "stage", choices=("source-auth", "packaged-auth", "workspace", "combined")
    )
    args = parser.parse_args()
    evidence = Path(os.environ["RUNNER_TEMP"]) / "combined-evidence"
    tested = (evidence / "tested-sha.txt").read_text().strip()
    code_tree = git("rev-parse", "HEAD:codex-rs")
    if code_tree != "5f8fab505a1d147119496b51346a0f618c27c9d3":
        raise ValueError("Rust source no longer matches the accepted runtime")
    subprocess.run(
        [
            "git",
            "diff",
            "--exit-code",
            tested,
            "HEAD",
            "--",
            "codex-rs",
            "scripts",
            "superfork/edge.py",
            "superfork/test_edge.py",
            "superfork/verify_edge_cli.py",
            "superfork/complete_verification.py",
        ],
        check=True,
    )
    result = {
        "status": "stage-passed-not-default-branch-delivery",
        "stage": args.stage,
        "tested_commit": tested,
        "runtime_code_tree": code_tree,
        "workflow_run": os.environ["GITHUB_RUN_ID"],
        "platform": "Linux x86_64",
    }
    if args.stage in ("source-auth", "packaged-auth"):
        result["authentication"] = unittest_summary(evidence / f"{args.stage}.log", 17)
        result["auth_test_sha256"] = hashlib.sha256(
            Path("scripts/test_chatgpt_env_auth.py").read_bytes()
        ).hexdigest()
        if args.stage == "packaged-auth":
            output = (evidence / "edge-native.log").read_text()
            if (
                "PASS: 2 native CLI controls, with explicit user-disable precedence"
                not in output
            ):
                raise ValueError("Packaged native edge controls did not pass")
            result["edge_native_controls"] = output
            result["package_manifest"] = json.loads(
                (evidence / "package-manifest.json").read_text()
            )
            result["package_binaries"] = (
                evidence / "package-binaries.sha256"
            ).read_text()
    elif args.stage == "workspace":
        result["workspace"] = nextest_summary(evidence / "workspace.log")
        result["commands"] = [
            "cargo build --locked --workspace --bins",
            "just test --locked",
        ]
    else:
        stages = {}
        for name in ("source-auth", "packaged-auth", "workspace"):
            data = json.loads(
                Path(f"superfork/evidence/completion-{name}.json").read_text()
            )
            if any(
                data[key] != result[key]
                for key in ("tested_commit", "runtime_code_tree", "workflow_run")
            ):
                raise ValueError(f"Mismatched final-stage evidence: {name}")
            stages[name] = data
        earlier = {}
        for name in ("final-acceptance", "final-affected"):
            data = json.loads(Path(f"superfork/evidence/{name}.json").read_text())
            if (
                data["tested_code_tree"] != code_tree
                or data["status"] != "stage-passed-not-default-branch-delivery"
            ):
                raise ValueError(f"Mismatched earlier runtime evidence: {name}")
            earlier[name] = data
        edge = json.loads(Path("superfork/evidence/edge-cli.json").read_text())
        if edge["runtime_code_tree"] != code_tree:
            raise ValueError("Edge control uses a different runtime")
        for name, expected in edge["source_sha256"].items():
            if (
                hashlib.sha256(Path("superfork", name).read_bytes()).hexdigest()
                != expected
            ):
                raise ValueError(f"Edge source changed: {name}")
        result.update(
            {
                "status": "verified-awaiting-default-branch-promotion",
                "stages": stages,
                "earlier_exact_runtime_checks": earlier,
                "edge_verification_run": edge["workflow_run"],
                "helper_tests": unittest_summary(evidence / "helpers.log", 24),
                "toolchain": (evidence / "toolchain.txt").read_text(),
                "limitations": [
                    "Linux x86_64 only; macOS and Windows were not executed.",
                    "In-process lossless events are not disk replay or websocket/stdio delivery guarantees.",
                    "Injected persistence durability belongs to the host store implementation.",
                    "The edge preset activates existing features and selects embedded TUI mode under current CLI compatibility rules.",
                    "Packaged plain TUI daemon authentication is tested with isolated synthetic credentials, not live inference or per-client credential isolation.",
                    "No performance, cross-host replication, model-quality or complete Bazel matrix claim is made.",
                ],
            }
        )
    destination = Path(f"superfork/evidence/completion-{args.stage}.json")
    destination.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Recorded {args.stage}: {result['status']}, runtime {code_tree}")


if __name__ == "__main__":
    main()
