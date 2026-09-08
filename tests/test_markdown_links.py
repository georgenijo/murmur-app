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

    def test_duplicate_headings_skip_previously_allocated_numbered_anchors(self) -> None:
        anchors = heading_anchors("# Overview\n# Overview-1\n# Overview\n")
        self.assertEqual(anchors, {"overview", "overview-1", "overview-2"})

    def test_code_heading_anchor_uses_literal_inline_content(self) -> None:
        anchors = heading_anchors("# `__init__`\n# `[label](target)`\n")
        self.assertEqual(anchors, {"__init__", "labeltarget"})

    def test_code_blocks_do_not_create_headings_or_affect_duplicate_numbers(self) -> None:
        anchors = heading_anchors(
            "# Real\n\n```python\n# Fake\n# Real\n```\n\n## Real\n"
        )
        self.assertEqual(anchors, {"real", "real-1"})


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

    def test_code_and_duplicate_heading_anchors_validate_end_to_end(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text(
                "[init](#%5F%5Finit%5F%5F) "
                "[literal](#labeltarget) "
                "[third](#overview-2)\n\n"
                "# `__init__`\n"
                "# `[label](target)`\n"
                "# Overview\n"
                "# Overview-1\n"
                "# Overview\n"
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

    def test_links_in_code_spans_and_fenced_blocks_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text(
                "Use `[example](inline-missing.md)` as an example.\n\n"
                "```markdown\n[fenced](fenced-missing.md)\n```\n"
            )
            self.assertEqual(find_broken_links([doc], root), [])

    def test_code_block_heading_cannot_satisfy_an_anchor(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text("[wrong](#fake)\n\n```python\n# Fake\n```\n")
            broken = find_broken_links([doc], root)
            self.assertEqual(len(broken), 1)
            self.assertEqual(broken[0][2], "no heading matches #fake")

    def test_full_collapsed_and_shortcut_reference_links_are_checked(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text(
                "[full][one] [collapsed][] [shortcut]\n\n"
                "[one]: full-missing.md\n"
                "[collapsed]: collapsed-missing.md\n"
                "[shortcut]: shortcut-missing.md\n"
            )
            broken = find_broken_links([doc], root)
            self.assertEqual(
                {target for _, target, _ in broken},
                {"full-missing.md", "collapsed-missing.md", "shortcut-missing.md"},
            )

    def test_reference_links_enforce_anchors_and_repository_containment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target.md"
            target.write_text("# Real\n")
            doc = root / "AGENTS.md"
            doc.write_text(
                "[bad anchor][anchor] [outside][escape]\n\n"
                "[anchor]: target.md#missing\n"
                "[escape]: %2E%2E/outside.md\n"
            )
            broken = find_broken_links([doc], root)
            self.assertEqual(
                {(href, reason) for _, href, reason in broken},
                {
                    ("target.md#missing", "no heading matches #missing"),
                    ("%2E%2E/outside.md", "escapes repository root"),
                },
            )

    def test_valid_angle_encoded_and_titled_destinations_are_normalized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "file name.md"
            target.write_text("# Real Heading\n")
            titled_target = root / "file.md"
            titled_target.write_text("# File\n")
            doc = root / "AGENTS.md"
            doc.write_text(
                "[angle](<file name.md>)\n"
                "[encoded](file%20name.md#real%2Dheading)\n"
                "[single](file.md 'Title')\n"
                "[double](<file name.md> \"Title\")\n"
                "[parenthesized](<file name.md> (Title))\n"
            )
            self.assertEqual(find_broken_links([doc], root), [])

    def test_encoded_traversal_is_reported_as_repository_escape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            doc = root / "AGENTS.md"
            doc.write_text("[outside](%2E%2E/outside.md)\n")
            broken = find_broken_links([doc], root)
            self.assertEqual(len(broken), 1)
            self.assertEqual(broken[0][2], "escapes repository root")

    def test_symlinked_repository_root_is_canonicalized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            real_root = Path(directory) / "real"
            real_root.mkdir()
            linked_root = Path(directory) / "linked"
            linked_root.symlink_to(real_root, target_is_directory=True)
            target = real_root / "target.md"
            target.write_text("# Target\n")
            doc = linked_root / "AGENTS.md"
            doc.write_text("[target](target.md)\n")
            self.assertEqual(find_broken_links([doc], linked_root), [])


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
