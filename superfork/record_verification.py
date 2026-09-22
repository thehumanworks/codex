#!/usr/bin/env python3
"""Record successful stages only when their required logs contain executed tests."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def nextest_summary(path):
    text = re.sub(r"\x1b\[[0-9;]*m", "", path.read_text())
    lines = [line.strip() for line in text.splitlines() if "Summary [" in line]
    if len(lines) != 1:
        raise ValueError(f"Expected one executed test summary: {path}")
    counts = re.search(r"(\d+) tests run: (\d+) passed", lines[0])
    if counts is None or int(counts[1]) == 0 or counts[1] != counts[2]:
        raise ValueError(f"Incomplete or failed test stage: {path}: {lines}")
    return {"summary": lines[0], "tests_run": int(counts[1]), "retries": [line.strip() for line in text.splitlines() if "RETRY" in line or "FLAKY" in line]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("acceptance", "affected", "smoke", "workspace"))
    args = parser.parse_args()
    evidence = Path(os.environ["RUNNER_TEMP"]) / "harness-verification"
    source = (evidence / "tested-sha.txt").read_text().strip()
    tree = (evidence / "tested-code-tree.txt").read_text().strip()
    if git("rev-parse", "HEAD:codex-rs") != tree:
        raise ValueError("The runtime source changed after verification began")
    subprocess.run(["git", "diff", "--exit-code", source, "HEAD", "--", "codex-rs", "scripts"], check=True)
    result = {"stage": args.stage, "status": "stage-passed-not-default-branch-delivery", "tested_commit": source, "tested_code_tree": tree, "workflow_run": os.environ["GITHUB_RUN_ID"], "platform": "Linux x86_64"}
    if args.stage == "acceptance":
        result["acceptance"] = nextest_summary(evidence / "acceptance.log")
        result["repetitions"] = [nextest_summary(evidence / f"concurrency-{n}.log") for n in range(1, 6)]
    elif args.stage == "affected":
        result["affected_suites"] = nextest_summary(evidence / "affected.log")
    elif args.stage == "smoke":
        text = (evidence / "auth-smoke.log").read_text()
        if re.search(r"Ran [1-9][0-9]* tests? in ", text) is None or not re.search(r"^OK$", text, re.MULTILINE):
            raise ValueError("Authentication smoke suite did not finish successfully without skips")
        result["auth_smoke"] = text
        result["cli_version"] = (evidence / "cli-version.txt").read_text().strip()
    else:
        result["workspace"] = nextest_summary(evidence / "workspace.log")
        result["status"] = "verified-awaiting-default-branch-promotion"
        stages = {}
        for name in ("acceptance", "affected", "smoke"):
            previous = json.loads(Path(f"superfork/evidence/final-{name}.json").read_text())
            if previous["workflow_run"] != result["workflow_run"] or previous["tested_code_tree"] != tree:
                raise ValueError("Stage evidence belongs to a different run or runtime tree")
            stages[name] = previous
        result["stages"] = stages
        result["limitations"] = ["Only Linux x86_64 was executed; macOS/Windows are not verified.", "Injected store durability is the host implementation's responsibility.", "Lossless delivery is opt-in for the in-process API, not websocket/stdio or disk replay.", "No live paid inference or cross-host/cloud-store implementation is validated here."]
    destination = Path(f"superfork/evidence/final-{args.stage}.json")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
