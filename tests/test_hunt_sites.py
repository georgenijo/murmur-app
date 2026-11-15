from pathlib import Path
import contextlib
import io
import tempfile
import unittest

from scripts.check_hunt_sites import main, scan


def write(root: Path, name: str, text: str) -> None:
    path = root / "app/src-tauri/src" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def run(root: Path) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(root)
    return code, out.getvalue(), err.getvalue()


class HuntSiteCheckTests(unittest.TestCase):
    def test_sites_with_an_issue_number_pass_and_are_listed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", 'fn f() {\n    crate::hunt!(issue = 1234, n = 1u64, "state");\n}\n')
            write(root, "sub/b.rs", "fn g() {\n    hunt!(\n        issue = 7,\n        \"x\"\n    );\n}\n")
            code, out, err = run(root)
            self.assertEqual(code, 0, err)
            self.assertIn("hunt sites: 2", out)
            self.assertIn("a.rs:2  issue #1234", out)
            self.assertIn("b.rs:2  issue #7", out)

    def test_a_site_without_an_issue_number_fails(self):
        for body in (
            'hunt!("no issue");',
            'hunt!(count = 3u64, "no issue");',
            'hunt!(issue = abc, "not a number");',
            'hunt!(issue = , "empty");',
        ):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                write(root, "a.rs", f"fn f() {{\n    {body}\n}}\n")
                code, out, err = run(root)
                self.assertEqual(code, 1)
                self.assertIn("NO ISSUE NUMBER", out)
                self.assertIn("a.rs:2", err)

    def test_comments_and_the_macro_definition_are_not_sites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "telemetry.rs",
                "/// hunt!(issue = 1, \"doc\")\n// hunt!(\"commented\")\n"
                "macro_rules! hunt {\n    () => {};\n}\n",
            )
            sites, problems = scan(root)
            self.assertEqual((sites, problems), ([], []))

    def test_emitting_on_the_hunt_target_directly_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", 'fn f() { tracing::debug!(target: "hunt", "sneaky"); }\n')
            code, _, err = run(root)
            self.assertEqual(code, 1)
            self.assertIn("bypasses hunt!", err)

    def test_the_repository_has_no_unnumbered_hunt_sites(self):
        code, _, err = run(Path(__file__).resolve().parents[1])
        self.assertEqual(code, 0, err)


if __name__ == "__main__":
    unittest.main()
