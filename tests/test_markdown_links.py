from pathlib import Path
import tempfile
import unittest

from scripts.validate_markdown_links import (
    find_broken_links,
    heading_anchors,
    is_external,
    maintained_docs,
    slugify,
    validate_markdown_links,
)


ROOT = Path(__file__).resolve().parents[1]


class RepositoryMarkdownLinksTests(unittest.TestCase):
    def test_repository_markdown_links_are_valid(self) -> None:
        self.assertGreater(validate_markdown_links(), 0)


class SlugifyTests(unittest.TestCase):
    def test_strips_markdown_emphasis_markers(self) -> None:
        self.assertEqual(slugify("**Bold** and _italic_"), "bold-and-italic")

    def test_preserves_underscores_inside_inline_code(self) -> None:
        self.assertEqual(
            slugify("Server-armed hang diagnostics (`hang_diagnostics.rs`)"),
            "server-armed-hang-diagnostics-hang_diagnosticsrs",
        )

    def test_duplicate_headings_get_numbered_anchors(self) -> None:
        anchors = heading_anchors("# Overview\n\n## Overview\n")
        self.assertEqual(anchors, {"overview", "overview-1"})


class IsExternalTests(unittest.TestCase):
    def test_http_and_mailto_are_external(self) -> None:
        self.assertTrue(is_external("https://example.com/docs"))
        self.assertTrue(is_external("mailto:someone@example.com"))

    def test_relative_paths_are_not_external(self) -> None:
        self.assertFalse(is_external("docs/ARCHITECTURE.md"))
        self.assertFalse(is_external("../features/vad.md#speech-filtering"))


class FindBrokenLinksTests(unittest.TestCase):
    def test_missing_target_file_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text("See [missing](docs/does-not-exist.md) for details.\n")
            broken = find_broken_links([doc], root)
            self.assertEqual(len(broken), 1)
            path, target, reason = broken[0]
            self.assertEqual(path, doc)
            self.assertEqual(target, "docs/does-not-exist.md")
            self.assertEqual(reason, "no such file or directory")

    def test_missing_heading_anchor_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target_doc = root / "docs" / "example.md"
            target_doc.parent.mkdir(parents=True)
            target_doc.write_text("# Real Heading\n")
            doc = root / "AGENTS.md"
            doc.write_text("See [broken](docs/example.md#not-a-real-heading).\n")
            broken = find_broken_links([doc], root)
            self.assertEqual(len(broken), 1)
            self.assertIn("no heading matches #not-a-real-heading", broken[0][2])

    def test_valid_local_and_anchor_links_are_not_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target_doc = root / "docs" / "example.md"
            target_doc.parent.mkdir(parents=True)
            target_doc.write_text("# Real Heading\n")
            doc = root / "AGENTS.md"
            doc.write_text(
                "See [file](docs/example.md) and its "
                "[section](docs/example.md#real-heading).\n"
            )
            self.assertEqual(find_broken_links([doc], root), [])

    def test_external_and_bare_scheme_links_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text(
                "[web](https://example.com/missing) and "
                "[mail](mailto:nobody@example.com).\n"
            )
            self.assertEqual(find_broken_links([doc], root), [])

    def test_link_escaping_repository_root_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text("[outside](../../etc/passwd)\n")
            broken = find_broken_links([doc], root)
            self.assertEqual(len(broken), 1)
            self.assertEqual(broken[0][2], "escapes repository root")

    def test_image_links_are_not_treated_as_markdown_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text("![missing image](does-not-exist.png)\n")
            self.assertEqual(find_broken_links([doc], root), [])


class MaintainedDocsTests(unittest.TestCase):
    def test_archive_directories_and_files_are_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "AGENTS.md").write_text("# Agents\n")
            docs = root / "docs"
            (docs / "archive").mkdir(parents=True)
            (docs / "archive" / "old.md").write_text("[broken](nope.md)\n")
            (docs / "research-structured-events.md").write_text("[broken](nope.md)\n")
            (docs / "kept.md").write_text("# Kept\n")

            paths = maintained_docs(root)
            names = {path.relative_to(root) for path in paths}
            self.assertIn(Path("AGENTS.md"), names)
            self.assertIn(Path("docs/kept.md"), names)
            self.assertNotIn(Path("docs/archive/old.md"), names)
            self.assertNotIn(Path("docs/research-structured-events.md"), names)

    def test_validate_markdown_links_reports_broken_link_with_file_and_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "AGENTS.md").write_text("[missing](docs/nope.md)\n")
            (root / "docs").mkdir()

            with self.assertRaisesRegex(
                AssertionError, r"AGENTS\.md: \[docs/nope\.md\] — no such file or directory"
            ):
                validate_markdown_links(root)


if __name__ == "__main__":
    unittest.main()
