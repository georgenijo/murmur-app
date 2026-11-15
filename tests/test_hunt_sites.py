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
            write(root, "a.rs", 'fn f() {\n    crate::hunt!(issue = 1234, "state");\n}\n')
            write(root, "sub/b.rs", "fn g() {\n    hunt!(\n        issue = 7,\n        \"x\",\n    );\n}\n")
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

    def test_a_site_with_fields_fails(self):
        for body in (
            'hunt!(issue = 1, queue_len = 3u64, "x");',
            'hunt!(issue = 1, message = transcript, "x");',
            'hunt!(issue = 1, "x", armed = true);',
            "hunt!(issue = 1, text);",
        ):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                write(root, "a.rs", f"fn f() {{\n    {body}\n}}\n")
                code, _, err = run(root)
                self.assertEqual(code, 1)
                self.assertIn("accepts no fields", err)
                self.assertIn("a.rs:2", err)

    def test_brace_and_bracket_invocations_are_sites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", 'fn f() {\n    hunt!{ issue = 5, "brace" };\n    hunt![issue = 6, "bracket"];\n}\n')
            write(root, "b.rs", 'fn g() {\n    hunt! { "no issue" };\n    hunt![ "no issue" ];\n}\n')
            sites, _ = scan(root)
            self.assertEqual(
                [(site.path.split("/")[-1], site.issue) for site in sites],
                [("a.rs", 5), ("a.rs", 6), ("b.rs", None), ("b.rs", None)],
            )
            self.assertEqual(run(root)[0], 1)

    def test_comments_are_not_sites_including_nested_block_comments(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "a.rs",
                '/* example:\n   hunt!("no issue");\n*/\n'
                '/* outer /* inner */ hunt!("still commented"); */\n'
                '// hunt!("line comment")\n'
                'fn f() { hunt!(issue = 3, "real"); }\n',
            )
            sites, problems = scan(root)
            self.assertEqual([(site.line, site.issue) for site in sites], [(6, 3)])
            self.assertEqual(problems, [])

    def test_slashes_inside_a_string_are_not_a_comment(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "a.rs",
                'fn f() {\n    let url = "https://example.com"; hunt!("no issue");\n}\n',
            )
            sites, _ = scan(root)
            self.assertEqual([(site.line, site.issue) for site in sites], [(2, None)])

    def test_telemetry_rs_is_scanned_except_its_macro_and_test_module(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "telemetry.rs",
                "#[macro_export]\nmacro_rules! hunt {\n    (issue = $i:literal, $m:literal) => {\n"
                '        ::tracing::debug!(target: "hunt", issue = $i as u64, "{}", $m)\n    };\n}\n'
                'fn real() { hunt!("forgot the issue"); }\n'
                '#[cfg(test)]\nmod tests {\n    fn t() { hunt!("fixture"); tracing::debug!(target: "hunt", "x"); }\n}\n',
            )
            code, out, err = run(root)
            self.assertEqual(code, 1)
            self.assertIn("hunt sites: 1", out)
            self.assertIn("telemetry.rs:7  NO ISSUE NUMBER", out)
            self.assertNotIn("bypasses", err)

    def test_production_code_after_a_test_module_is_still_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "a.rs",
                '#[cfg(test)]\nmod tests {\n    fn t() { let s = "}"; hunt!("fixture"); }\n}\n'
                'fn later() {\n    hunt!("production");\n}\n',
            )
            sites, _ = scan(root)
            self.assertEqual([(site.line, site.issue) for site in sites], [(6, None)])

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

    def test_the_repository_has_no_malformed_hunt_sites(self):
        code, _, err = run(Path(__file__).resolve().parents[1])
        self.assertEqual(code, 0, err)


if __name__ == "__main__":
    unittest.main()
