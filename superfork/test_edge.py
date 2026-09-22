import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from edge import command, main


class EdgeLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.binary = Path(self.temp.name) / "codex with spaces"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o700)

    def test_literal_arguments_are_not_shell_expanded_or_modified(self):
        arguments = ["exec", "$(touch should-not-exist); $TOKEN", "--json"]
        result = command(self.binary, arguments)
        self.assertEqual(result[-len(arguments) :], arguments)
        self.assertEqual(result[0], str(self.binary))
        self.assertEqual(
            arguments, ["exec", "$(touch should-not-exist); $TOKEN", "--json"]
        )

    def test_exec_preserves_native_process_arguments(self):
        arguments = ["resume", "--last", "--disable", "memories"]
        with patch("edge.os.execv") as execute:
            main(["--codex-bin", str(self.binary), "--", *arguments])
        execute.assert_called_once_with(
            str(self.binary), command(self.binary, arguments)
        )

    def test_environment_and_working_directory_are_unchanged(self):
        before_cwd = Path.cwd()
        before_env = dict(os.environ)
        with patch("edge.os.execv"):
            main(["--codex-bin", str(self.binary)])
        self.assertEqual(Path.cwd(), before_cwd)
        self.assertEqual(dict(os.environ), before_env)

    def test_existing_security_and_model_overrides_are_forwarded(self):
        arguments = [
            "--sandbox",
            "read-only",
            "--ask-for-approval",
            "on-request",
            "--model",
            "example-model",
            "--disable",
            "agent_message_board",
        ]
        with patch("edge.os.execv") as execute:
            main(["--codex-bin", str(self.binary), "--", *arguments])
        self.assertEqual(execute.call_args.args[1][-len(arguments) :], arguments)

    def test_environment_can_select_the_fork_binary(self):
        with (
            patch.dict(os.environ, {"CODEX_EDGE_BIN": str(self.binary)}),
            patch("edge.os.execv") as execute,
        ):
            main(["--", "--help"])
        self.assertEqual(execute.call_args.args[0], str(self.binary))

    def test_missing_binary_fails_before_execution(self):
        with (
            patch("edge.os.execv") as execute,
            contextlib.redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as error,
        ):
            main(["--codex-bin", str(self.binary.parent / "missing")])
        self.assertEqual(error.exception.code, 2)
        execute.assert_not_called()

    def test_non_executable_file_fails_before_execution(self):
        self.binary.chmod(0o600)
        with (
            patch("edge.os.execv") as execute,
            contextlib.redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as error,
        ):
            main(["--codex-bin", str(self.binary)])
        self.assertEqual(error.exception.code, 2)
        execute.assert_not_called()

    def test_directory_cannot_be_used_as_a_binary(self):
        with (
            patch("edge.os.execv") as execute,
            contextlib.redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as error,
        ):
            main(["--codex-bin", str(self.binary.parent)])
        self.assertEqual(error.exception.code, 2)
        execute.assert_not_called()

    def test_launcher_does_not_log_user_arguments_or_environment(self):
        output, errors = io.StringIO(), io.StringIO()
        with (
            patch("edge.os.execv"),
            contextlib.redirect_stdout(output),
            contextlib.redirect_stderr(errors),
        ):
            main(
                [
                    "--codex-bin",
                    str(self.binary),
                    "--",
                    "exec",
                    "sensitive synthetic input",
                ]
            )
        self.assertEqual((output.getvalue(), errors.getvalue()), ("", ""))


if __name__ == "__main__":
    unittest.main()
