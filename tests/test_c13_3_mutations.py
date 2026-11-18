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


class NoProcessTests(unittest.TestCase):
    def test_listing_and_checking_never_run_a_process(self):
        with mock.patch.object(mutations.subprocess, "run", side_effect=AssertionError("ran a process")):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(mutations.main(["--list"]), 0)
                self.assertEqual(mutations.main(["--family", "stop", "--list"]), 0)
                self.assertEqual(mutations.main(["--family", "cancel"]), 0)


if __name__ == "__main__":
    unittest.main()
