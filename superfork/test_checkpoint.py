"""Exercise real local Git remotes; no API token or network required."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from checkpoint import git, publish


class CheckpointTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.remote = root / "remote.git"
        self.work = root / "work"
        subprocess.run(["git", "init", "--bare", "--quiet", str(self.remote)], check=True)
        subprocess.run(["git", "init", "--quiet", "--initial-branch=main", str(self.work)], check=True)
        previous = Path.cwd()
        os.chdir(self.work)
        self.addCleanup(os.chdir, previous)
        git("config", "user.name", "Checkpoint Test")
        git("config", "user.email", "checkpoint@example.invalid")
        git("remote", "add", "origin", str(self.remote))
        self.file = self.work / "code.txt"
        self.file.write_text("initial\n")
        git("add", "code.txt")
        git("commit", "-qm", "initial")
        self.initial = git("rev-parse", "HEAD")
        subprocess.run(["git", "push", "--quiet", "origin", "HEAD:main"], check=True)
        git("checkout", "-qb", "superfork/test")
        self.file.write_text("feature\n")
        git("commit", "-qam", "feature")
        self.feature = git("rev-parse", "HEAD")

    def test_publishes_exact_commit_and_does_not_advance_main(self):
        result = publish("source", "12", "1")
        self.assertEqual(result["commit"], self.feature)
        self.assertEqual(git("ls-remote", "origin", "refs/heads/main").split()[0], self.initial)
        self.assertEqual(git("ls-remote", "origin", result["ref"]).split()[0], self.feature)
        self.assertEqual(git("rev-parse", "HEAD"), self.feature)

    def test_repeated_checkpoint_is_idempotent(self):
        self.assertEqual(publish("acceptance", "12", "1"), publish("acceptance", "12", "1"))

    def test_each_attempt_and_stage_has_a_distinct_ref(self):
        refs = {publish(stage, "12", attempt)["ref"] for stage, attempt in [("source", "1"), ("source", "2"), ("formatted", "1")]}
        self.assertEqual(len(refs), 3)

    def test_changed_tracked_file_is_rejected(self):
        self.file.write_text("not committed\n")
        with self.assertRaises(ValueError):
            publish("source", "12", "1")

    def test_untracked_source_is_rejected(self):
        Path("missing.rs").write_text("fn missing() {}\n")
        with self.assertRaises(ValueError):
            publish("source", "12", "1")

    def test_staged_source_is_rejected(self):
        self.file.write_text("staged\n")
        git("add", "code.txt")
        with self.assertRaises(ValueError):
            publish("source", "12", "1")

    def test_invalid_identifiers_and_remote_are_rejected(self):
        for stage, run, attempt, remote in [("main", "12", "1", "origin"), ("source", "../main", "1", "origin"), ("source", "12", "-1", "origin"), ("source", "12", "1", "--force"), ("source", "12", "1", "unknown")]:
            with self.subTest(stage=stage, run=run, attempt=attempt, remote=remote):
                with self.assertRaises(ValueError):
                    publish(stage, run, attempt, remote)

    def test_existing_conflicting_ref_is_never_overwritten(self):
        ref = f"refs/heads/superfork/checkpoints/12-1-source-{self.feature}"
        subprocess.run(["git", "push", "--quiet", "origin", f"{self.initial}:{ref}"], check=True)
        with self.assertRaises(ValueError):
            publish("source", "12", "1")
        self.assertEqual(git("ls-remote", "origin", ref).split()[0], self.initial)

    def test_remote_rejection_is_not_reported_as_success(self):
        hook = self.remote / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o700)
        with self.assertRaises(subprocess.CalledProcessError):
            publish("source", "12", "1")

    def test_mismatching_readback_is_not_reported_as_success(self):
        real = git
        queries = 0
        def changed_readback(*args):
            nonlocal queries
            if args[0] == "ls-remote":
                queries += 1
                if queries == 2:
                    return ""
            return real(*args)
        with patch("checkpoint.git", side_effect=changed_readback):
            with self.assertRaises(RuntimeError):
                publish("source", "12", "1")


if __name__ == "__main__":
    unittest.main()
