#!/usr/bin/env python3
"""Publish a clean Git checkpoint without advancing main or rewriting any ref.

Stage names describe which workflow step finished, not a blanket verification
claim. The full source commit is bound into the ref and read back after pushing.
"""

import argparse
import os
import re
import subprocess

STAGES = (
    "source",
    "formatted",
    "acceptance",
    "affected",
    "smoke",
    "workspace",
    "failure",
)


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def publish(stage, run_id, attempt, remote="origin"):
    if stage not in STAGES:
        raise ValueError("unknown checkpoint stage")
    if not re.fullmatch(r"[0-9]+", run_id) or not re.fullmatch(r"[0-9]+", attempt):
        raise ValueError("run and attempt must be decimal identifiers")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", remote):
        raise ValueError("remote must be a configured Git remote name")
    if remote not in git("remote").splitlines():
        raise ValueError("remote is not configured")
    if git("status", "--porcelain", "--untracked-files=normal"):
        raise ValueError(
            "commit or explicitly exclude all changes before checkpointing"
        )
    sha = git("rev-parse", "HEAD")
    ref = f"refs/heads/superfork/checkpoints/{run_id}-{attempt}-{stage}-{sha}"
    subprocess.run(["git", "check-ref-format", ref], check=True)
    existing = git("ls-remote", "--refs", remote, ref)
    if existing and existing.split()[0] != sha:
        raise ValueError("checkpoint ref already points to a different commit")
    if not existing:
        subprocess.run(["git", "push", remote, f"{sha}:{ref}"], check=True)
    observed = git("ls-remote", "--refs", remote, ref)
    if not observed or observed.split()[0] != sha:
        raise RuntimeError("remote checkpoint does not match the source commit")
    return {"ref": ref, "commit": sha, "stage": stage}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=STAGES)
    parser.add_argument(
        "--run-id",
        default=os.environ.get("GITHUB_RUN_ID"),
        required="GITHUB_RUN_ID" not in os.environ,
    )
    parser.add_argument("--attempt", default=os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
    args = parser.parse_args()
    result = publish(args.stage, args.run_id, args.attempt)
    print(f"Checkpoint {result['stage']}: {result['commit']} at {result['ref']}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(
                f"\nCheckpoint `{result['stage']}`: `{result['commit']}`\n\n`{result['ref']}`\n"
            )


if __name__ == "__main__":
    main()
