"""Policy tests for scripts/c13_3_mutations.py.

Argument parsing, row selection and the row-file schema only. Nothing here runs
cargo or edits a source file: the rows themselves are re-run by hand in a scratch
worktree (docs/testing/c13-3-mutations.md).
"""

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import c13_3_mutations as mutations


def parse(*argv):
    return mutations.parse_args(list(argv))


def parse_fails(*argv):
    """The exit status and stderr of an argument list the parser refuses."""
    err = io.StringIO()
    try:
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            mutations.parse_args(list(argv))
    except SystemExit as exit_:
        return exit_.code, err.getvalue()
    raise AssertionError(f"the parser accepted {argv}")


class ArgumentParsingTests(unittest.TestCase):
    def test_a_family_selects_its_rows_on_their_own(self):
        args = parse("--family", "stop")
        self.assertEqual(args.family, ["stop"])
        self.assertFalse(args.all)

    def test_families_and_rows_repeat(self):
        args = parse("--family", "stop", "--family", "start", "--row", "M216-stop-queued-behind-start")
        self.assertEqual(args.family, ["stop", "start"])
        self.assertEqual(args.row, ["M216-stop-queued-behind-start"])

    def test_nothing_selected_is_a_usage_error(self):
        code, err = parse_fails()
        self.assertEqual(code, 2)
        self.assertIn("--family", err)

    def test_an_unknown_family_is_refused(self):
        code, _ = parse_fails("--family", "stopp")
        self.assertEqual(code, 2)

    def test_all_excludes_a_family_or_a_row(self):
        self.assertEqual(parse_fails("--all", "--family", "stop")[0], 2)
        self.assertEqual(parse_fails("--all", "--row", "x")[0], 2)

    def test_check_and_dry_run_are_separate_modes(self):
        self.assertEqual(parse_fails("--all", "--check", "--dry-run")[0], 2)

    def test_modes_and_root(self):
        args = parse("--all", "--check", "--root", "/tmp/scratch", "--keep-going")
        self.assertTrue(args.all and args.check and args.keep_going)
        self.assertEqual(args.root, Path("/tmp/scratch"))
        self.assertTrue(parse("--list").list)


class RowFileTests(unittest.TestCase):
    def test_the_checked_in_rows_are_well_formed(self):
        rows = mutations.load_rows()
        self.assertGreater(len(rows), 0)
        for row in rows:
            self.assertIn(row["family"], mutations.FAMILIES)

    def test_every_family_with_rows_selects_only_its_own(self):
        rows = mutations.load_rows()
        for family in {row["family"] for row in rows}:
            chosen = mutations.select_rows(rows, parse("--family", family))
            self.assertTrue(chosen)
            self.assertEqual({row["family"] for row in chosen}, {family})

    def test_the_stop_family_has_the_216_row(self):
        rows = mutations.load_rows()
        chosen = mutations.select_rows(rows, parse("--family", "stop"))
        self.assertIn("M216-stop-queued-behind-start", [row["name"] for row in chosen])

    def test_an_unknown_row_is_an_error(self):
        rows = mutations.load_rows()
        with self.assertRaises(mutations.SpecError):
            mutations.select_rows(rows, parse("--row", "nope"))

    def test_a_malformed_row_is_rejected(self):
        good = {
            "name": "r",
            "family": "stop",
            "bug": "#1",
            "description": "d",
            "edits": [["f.rs", "a", "b"]],
            "cmds": [["cargo", "test"]],
        }
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "r.json"
            mutations.validate_row(good, path)
            for broken in (
                {**good, "family": "nope"},
                {**good, "name": "other"},
                {**good, "edits": []},
                {**good, "edits": [["f.rs", "a", "a"]]},
                {**good, "cmds": [[]]},
                {key: value for key, value in good.items() if key != "bug"},
            ):
                with self.assertRaises(mutations.SpecError, msg=json.dumps(broken)):
                    mutations.validate_row(broken, path)


CMD = ["cargo", "test", "--lib", "--", "--test-threads=1", "bug216_stop_queued", "golden_s01"]
CORE_CMD = ["cargo", "test", "-p", "murmur-dictation-core"]
FAILING = (
    "     Running unittests src/lib.rs (target/debug/deps/ui_lib-0123456789abcdef)\n"
    "running 2 tests\n"
    "test commands::recording::bug_replay_tests::bug216_stop_queued_behind_start ... FAILED\n"
    "test commands::recording::golden_tests::golden_s01_stop_during_start ... ok\n"
    "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 2188 filtered out; finished in 0.02s\n"
    "error: test failed, to rerun pass `--lib`\n"
)
PASSING = "     Running unittests src/lib.rs (target/debug/deps/ui_lib-0123456789abcdef)\nrunning 2 tests\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 2188 filtered out; finished in 0.01s\n"


class VerdictTests(unittest.TestCase):
    """What a command's output says about a mutation. Fixed output only; no cargo."""

    def test_a_named_failing_test_is_caught(self):
        self.assertEqual(mutations.classify_command(CMD, 101, FAILING), "CAUGHT")

    def test_passing_tests_are_a_survivor(self):
        self.assertEqual(mutations.classify_command(CMD, 0, PASSING), "PASSED")

    def test_a_cargo_infrastructure_failure_is_invalid_not_caught(self):
        for output in (
            "error: failed to download `serde v1.0.0`\n\nCaused by:\n  network failure\n",
            "    Updating crates.io index\nerror: failed to get `tauri` as a dependency\n",
            "error: failed to get `tauri` as a dependency of package `ui`\n",
            "",
            "error: linker `cc` not found\n",
        ):
            self.assertEqual(mutations.classify_command(CMD, 101, output), "INVALID", output)

    def test_a_compile_error_is_invalid(self):
        output = "error[E0425]: cannot find value `x`\nerror: could not compile `ui` (lib test)\n"
        self.assertEqual(mutations.classify_command(CMD, 101, output), "INVALID")

    def test_zero_tests_run_is_invalid_whatever_the_exit_status(self):
        none = "     Running unittests src/lib.rs (x)\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2190 filtered out\n"
        self.assertEqual(mutations.classify_command(CMD, 0, none), "INVALID")
        self.assertEqual(mutations.classify_command(CMD, 101, none), "INVALID")

    def test_a_failure_the_row_does_not_name_is_invalid(self):
        other = FAILING.replace("bug216_stop_queued_behind_start", "unrelated_thing")
        self.assertEqual(mutations.classify_command(CMD, 101, other), "INVALID")

    def test_a_crash_with_no_failed_test_is_invalid(self):
        crashed = PASSING + "error: test failed, to rerun pass `--lib`\n\nCaused by:\n  process didn't exit successfully (signal: 9, SIGKILL)\n"
        self.assertEqual(mutations.classify_command(CMD, 101, crashed), "INVALID")

    def test_an_abnormal_exit_after_a_matching_failure_is_invalid(self):
        """Sol's case 1: the first binary passes, the second prints a matching FAILED line
        and then exits abnormally before its summary. The first binary's summary must not
        stand in for the second's."""
        for ending in (
            "Caused by:\n  process didn't exit successfully: `/x/ui_lib-1` (signal: 11, SIGSEGV: invalid memory reference)\n",
            "Caused by:\n  process didn't exit successfully: `/x/ui_lib-1` (exit status: 3)\n",
            "",
        ):
            output = (
                PASSING
                + "     Running tests/c13_3_bug_replays.rs (target/debug/deps/c13_3_bug_replays-abc)\n"
                + "running 2 tests\n"
                + "test bug216_stop_queued_behind_start ... FAILED\n"
                + "error: test failed, to rerun pass `--test c13_3_bug_replays`\n\n"
                + ending
            )
            self.assertEqual(mutations.classify_command(CMD, 101, output), "INVALID", ending)
            self.assertEqual(mutations.classify_command(CMD, 3, output), "INVALID", ending)

    def test_an_abnormal_exit_after_a_completed_failing_summary_is_invalid(self):
        output = FAILING + "\nCaused by:\n  process didn't exit successfully: `/x/ui_lib-1` (exit status: 3)\n"
        self.assertEqual(mutations.classify_command(CMD, 3, output), "INVALID")
        killed = FAILING + "\nCaused by:\n  process didn't exit successfully: `/x/ui_lib-1` (signal: 9, SIGKILL: kill)\n"
        self.assertEqual(mutations.classify_command(CMD, 101, killed), "INVALID")

    def test_libtests_own_failure_status_is_not_abnormal(self):
        # cargo prints no `Caused by` for an ordinary libtest failure (exit status 101).
        self.assertEqual(mutations.classify_command(CMD, 101, FAILING), "CAUGHT")

    def test_a_failure_in_a_later_completed_binary_counts_when_the_earlier_one_passed(self):
        output = (
            PASSING
            + "     Running tests/c13_3_bug_replays.rs (target/debug/deps/c13_3_bug_replays-abc)\n"
            + "running 1 test\n"
            + "test bug216_stop_queued_behind_start ... FAILED\n"
            + "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n"
            + "error: test failed, to rerun pass `--test c13_3_bug_replays`\n"
        )
        self.assertEqual(mutations.classify_command(CMD, 101, output), "CAUGHT")

    def test_a_nonzero_exit_that_is_not_libtests_is_invalid_even_with_a_failure(self):
        self.assertEqual(mutations.classify_command(CMD, 1, FAILING), "INVALID")
        self.assertEqual(mutations.classify_command(CMD, 0, FAILING), "INVALID")

    def test_a_command_without_filters_accepts_any_failed_test(self):
        self.assertEqual(mutations.command_filters(CORE_CMD), [])
        self.assertEqual(mutations.command_filters(CMD), ["bug216_stop_queued", "golden_s01"])
        self.assertEqual(mutations.classify_command(CORE_CMD, 101, FAILING), "CAUGHT")

    def test_a_row_is_caught_only_if_no_command_is_invalid(self):
        self.assertEqual(mutations.row_verdict(["CAUGHT", "PASSED"]), "CAUGHT")
        self.assertEqual(mutations.row_verdict(["PASSED", "PASSED"]), "SURVIVED")
        self.assertEqual(mutations.row_verdict(["CAUGHT", "INVALID"]), "INVALID")
        self.assertEqual(mutations.row_verdict(["INVALID", "CAUGHT"]), "INVALID")


class RunRowTests(unittest.TestCase):
    """`run_row` with a simulated cargo: the edit is applied, judged and always undone."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        crate = self.root / "app/src-tauri/src"
        crate.mkdir(parents=True)
        self.source = crate / "a.rs"
        self.source.write_text("fn a() { let keep = 1; }\n")
        self.row = {
            "name": "r",
            "family": "stop",
            "bug": "#1",
            "description": "d",
            "edits": [["src/a.rs", "keep = 1", "keep = 2"]],
            "cmds": [CMD, CORE_CMD],
        }

    def run_with(self, results):
        calls = []

        def fake_run(cmd, **kwargs):
            calls.append((cmd, self.source.read_text()))
            code, output = results[len(calls) - 1]
            return mutations.subprocess.CompletedProcess(cmd, code, output, "")

        with mock.patch.object(mutations.subprocess, "run", side_effect=fake_run), mock.patch.object(
            mutations, "git_dirty", return_value=False
        ), contextlib.redirect_stdout(io.StringIO()):
            verdict = mutations.run_row(self.row, self.root, False, False)
        return verdict, calls

    def test_a_cargo_infrastructure_failure_is_reported_invalid(self):
        down = "error: failed to download `serde v1.0.0`\n"
        verdict, calls = self.run_with([(101, down), (101, down)])
        self.assertEqual(verdict, "INVALID")
        self.assertEqual(len(calls), 2, "every command of the row runs")
        self.assertEqual(self.source.read_text(), "fn a() { let keep = 1; }\n", "the edit is undone")

    def test_the_edit_is_applied_while_cargo_runs(self):
        verdict, calls = self.run_with([(101, FAILING), (0, PASSING)])
        self.assertEqual(verdict, "CAUGHT")
        self.assertTrue(all("keep = 2" in text for _, text in calls))
        self.assertEqual(self.source.read_text(), "fn a() { let keep = 1; }\n")

    def test_an_infrastructure_failure_in_a_later_command_still_invalidates_the_row(self):
        verdict, _ = self.run_with([(101, FAILING), (101, "error: failed to download `x`\n")])
        self.assertEqual(verdict, "INVALID")

    def test_a_missing_cargo_is_invalid_and_the_remaining_commands_still_run(self):
        """Sol's case 3: `subprocess.run` raising FileNotFoundError must neither abort
        the runner nor skip the row's other commands, and the edit is still undone."""
        calls = []

        def fake_run(cmd, **kwargs):
            calls.append((cmd, self.source.read_text()))
            if len(calls) == 1:
                raise FileNotFoundError(2, "No such file or directory: 'cargo'")
            return mutations.subprocess.CompletedProcess(cmd, 101, FAILING, None)

        with mock.patch.object(mutations.subprocess, "run", side_effect=fake_run), mock.patch.object(
            mutations, "git_dirty", return_value=False
        ), contextlib.redirect_stdout(io.StringIO()):
            verdict = mutations.run_row(self.row, self.root, False, False)
        self.assertEqual(verdict, "INVALID")
        self.assertEqual(len(calls), 2, "the second command still ran")
        self.assertEqual(self.source.read_text(), "fn a() { let keep = 1; }\n")

    def test_main_exits_nonzero_for_an_invalid_row(self):
        with mock.patch.object(mutations, "run_row", return_value="INVALID"), contextlib.redirect_stdout(
            io.StringIO()
        ):
            self.assertEqual(mutations.main(["--row", "M216-stop-queued-behind-start"]), 1)


class NoProcessTests(unittest.TestCase):
    def test_listing_and_checking_never_run_a_process(self):
        with mock.patch.object(mutations.subprocess, "run", side_effect=AssertionError("ran a process")):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(mutations.main(["--list"]), 0)
                self.assertEqual(mutations.main(["--family", "stop", "--list"]), 0)
                self.assertEqual(mutations.main(["--family", "cancel"]), 0)


if __name__ == "__main__":
    unittest.main()
