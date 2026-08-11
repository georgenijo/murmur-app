from pathlib import Path
import tempfile
import unittest

from scripts.validate_reference_docs import (
    documented_commands,
    registered_commands,
    validate_reference_docs,
)


ROOT = Path(__file__).resolve().parents[1]
REFERENCE_FILES = (
    "app/src-tauri/src/lib.rs",
    "docs/reference/commands.md",
    "CLAUDE.md",
    "AGENTS.md",
    "docs/ARCHITECTURE.md",
)


def copy_reference_fixture(root: Path) -> None:
    for relative in REFERENCE_FILES:
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text((ROOT / relative).read_text())


class ReferenceDocsTests(unittest.TestCase):
    def test_repository_reference_docs_match_registered_commands(self) -> None:
        self.assertEqual(validate_reference_docs(), 121)

    def test_missing_command_row_fails_even_when_prose_count_is_current(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            copy_reference_fixture(root)
            commands_doc = root / "docs/reference/commands.md"
            commands_doc.write_text(
                commands_doc.read_text().replace(
                    "| `set_overlay_vertical_offset`",
                    "| `not_a_registered_command`",
                    1,
                )
            )
            with self.assertRaisesRegex(
                AssertionError, r"missing=.*set_overlay_vertical_offset.*extra="
            ):
                validate_reference_docs(root)

    def test_stale_human_facing_count_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            copy_reference_fixture(root)
            architecture = root / "docs/ARCHITECTURE.md"
            architecture.write_text(
                architecture.read_text().replace(
                    "121 registered commands", "118 registered commands", 1
                )
            )

            with self.assertRaisesRegex(AssertionError, r"stale command count"):
                validate_reference_docs(root)

    def test_duplicate_handler_or_documentation_entries_fail(self) -> None:
        lib_rs = (ROOT / "app/src-tauri/src/lib.rs").read_text()
        duplicated_handler = lib_rs.replace(
            "            commands::recording::init_dictation,\n",
            "            commands::recording::init_dictation,\n"
            "            commands::recording::init_dictation,\n",
            1,
        )
        with self.assertRaisesRegex(AssertionError, "duplicate command names"):
            registered_commands(duplicated_handler)

        commands_doc = (ROOT / "docs/reference/commands.md").read_text()
        duplicated_doc = commands_doc.replace(
            "| `init_dictation`",
            "| `init_dictation` | — | duplicate | duplicate |\n| `init_dictation`",
            1,
        )
        with self.assertRaisesRegex(AssertionError, "duplicate command rows"):
            documented_commands(duplicated_doc)


if __name__ == "__main__":
    unittest.main()
