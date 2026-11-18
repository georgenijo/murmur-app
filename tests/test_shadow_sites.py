from pathlib import Path
import contextlib
import io
import tempfile
import unittest

from scripts.check_shadow_sites import main, scan


def write(root: Path, name: str, text: str) -> None:
    path = root / "app/src-tauri/src" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def run(root: Path) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(root)
    return code, out.getvalue(), err.getvalue()


class ShadowSiteCheckTests(unittest.TestCase):
    def test_sites_with_an_issue_number_pass_and_are_listed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", "fn f() {\n    crate::shadow_site!(issue = 1154);\n}\n")
            write(root, "sub/b.rs", "fn g() {\n    shadow_site!(\n        issue = 7,\n    );\n}\n")
            code, out, err = run(root)
            self.assertEqual(code, 0, err)
            self.assertIn("shadow sites: 2", out)
            self.assertIn("a.rs:2  issue #1154", out)
            self.assertIn("b.rs:2  issue #7", out)

    def test_no_sites_is_not_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", "fn f() {}\n")
            code, out, err = run(root)
            self.assertEqual(code, 0, err)
            self.assertIn("shadow sites: 0", out)

    def test_a_site_without_an_issue_number_fails(self):
        for body in (
            "shadow_site!();",
            "shadow_site!(count = 3);",
            "shadow_site!(issue = abc);",
            "shadow_site!(issue = );",
        ):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                write(root, "a.rs", f"fn f() {{\n    {body}\n}}\n")
                code, out, err = run(root)
                self.assertEqual(code, 1)
                self.assertIn("NO ISSUE NUMBER", out)
                self.assertIn("a.rs:2", err)

    def test_a_site_with_other_arguments_fails(self):
        for body in (
            'shadow_site!(issue = 1, "label");',
            "shadow_site!(issue = 1, rec = 2);",
            "shadow_site!(issue = 1, issue = 2);",
        ):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                write(root, "a.rs", f"fn f() {{\n    {body}\n}}\n")
                code, _, err = run(root)
                self.assertEqual(code, 1)
                self.assertIn("takes only `issue = <number>`", err)
                self.assertIn("a.rs:2", err)

    def test_brace_and_bracket_invocations_are_sites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", "fn f() {\n    shadow_site!{ issue = 5 };\n    shadow_site![issue = 6];\n}\n")
            write(root, "b.rs", "fn g() {\n    shadow_site! { };\n    shadow_site![ ];\n}\n")
            sites, _ = scan(root)
            self.assertEqual(
                [(site.path.split("/")[-1], site.issue) for site in sites],
                [("a.rs", 5), ("a.rs", 6), ("b.rs", None), ("b.rs", None)],
            )
            self.assertEqual(run(root)[0], 1)

    def test_a_trailing_comma_is_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "a.rs", "fn f() { shadow_site!(issue = 9,); }\n")
            code, _, err = run(root)
            self.assertEqual(code, 0, err)

    def test_comments_are_not_sites_including_nested_block_comments(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "a.rs",
                "/* example:\n   shadow_site!();\n*/\n"
                "/* outer /* inner */ shadow_site!(); */\n"
                "// shadow_site!()\n"
                "fn f() { shadow_site!(issue = 3); }\n",
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
                'fn f() {\n    let url = "https://example.com"; shadow_site!();\n}\n',
            )
            sites, _ = scan(root)
            self.assertEqual([(site.line, site.issue) for site in sites], [(2, None)])

    def test_the_macro_definition_and_test_modules_are_not_sites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "telemetry.rs",
                "#[macro_export]\nmacro_rules! shadow_site {\n    (issue = $i:literal) => {};\n}\n"
                "fn real() { shadow_site!(); }\n"
                "#[cfg(test)]\nmod tests {\n    fn t() { shadow_site!(); }\n}\n",
            )
            code, out, _ = run(root)
            self.assertEqual(code, 1)
            self.assertIn("shadow sites: 1", out)
            self.assertIn("telemetry.rs:5  NO ISSUE NUMBER", out)

    def test_production_code_after_a_test_module_is_still_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(
                root,
                "a.rs",
                '#[cfg(test)]\nmod tests {\n    fn t() { let s = "}"; shadow_site!(); }\n}\n'
                "fn later() {\n    shadow_site!();\n}\n",
            )
            sites, _ = scan(root)
            self.assertEqual([(site.line, site.issue) for site in sites], [(6, None)])

    def test_the_repository_has_no_malformed_shadow_sites(self):
        code, _, err = run(Path(__file__).resolve().parents[1])
        self.assertEqual(code, 0, err)


if __name__ == "__main__":
    unittest.main()
