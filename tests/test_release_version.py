from pathlib import Path
import json
import tempfile
import unittest

from scripts.release_version import (
    ReleaseVersionError,
    check_release,
    prepare_release,
)


ROOT = Path(__file__).resolve().parents[1]


def write_fixture(root: Path, *, package_version: str = "1.2.3") -> None:
    paths = ["app/macos", "app/crates/murmur-services", "app/gpui"]
    for path in paths: (root / path).mkdir(parents=True, exist_ok=True)
    (root / "app/macos/bundle.json").write_text(json.dumps({"version":"1.2.3"}))
    (root / "app/crates/murmur-services/Cargo.toml").write_text('[package]\nname="murmur-services"\nversion="1.2.3"\n')
    (root / "app/gpui/Cargo.toml").write_text(f'[package]\nname="murmur-ui"\nversion="{package_version}"\n')
    (root / "app/Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "murmur-services"\nversion = "1.2.3"\n\n[[package]]\nname = "murmur-ui"\nversion = "1.2.3"\n')
    (root / "CHANGELOG.md").write_text(
        "# Changelog\n\n"
        "## [Unreleased]\n\n"
        "### Fixed\n\n"
        "- A user-visible fix.\n\n"
        "## [1.2.3] - 2026-01-01\n\n"
        "- Previous release.\n"
    )


class ReleaseVersionTests(unittest.TestCase):
    def test_prepare_updates_all_surfaces_and_cuts_changelog(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)

            prepare_release("1.3.0", "2026-02-03", root=root)

            self.assertEqual(check_release(root=root), "1.3.0")
            changelog = (root / "CHANGELOG.md").read_text()
            self.assertIn(
                "## [Unreleased]\n\n## [1.3.0] - 2026-02-03\n\n### Fixed",
                changelog,
            )

    def test_check_rejects_each_stale_version_surface(self) -> None:
        surfaces = (
            "app/macos/bundle.json",
            "app/crates/murmur-services/Cargo.toml",
            "app/Cargo.lock",
            "app/gpui/Cargo.toml",
            "CHANGELOG.md",
        )
        for surface in surfaces:
            with (
                self.subTest(surface=surface),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                write_fixture(root)
                path = root / surface
                path.write_text(path.read_text().replace("1.2.3", "1.2.2"))
                with self.assertRaises(ReleaseVersionError):
                    check_release("1.2.3", root=root)

    def test_check_requires_unreleased_then_current_release(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            changelog = root / "CHANGELOG.md"
            text = changelog.read_text()
            changelog.write_text(
                text.replace(
                    "## [Unreleased]\n\n### Fixed",
                    "## [1.2.2] - 2025-12-31\n\n## [Unreleased]\n\n### Fixed",
                )
            )

            with self.assertRaisesRegex(
                ReleaseVersionError, r"top-level \[Unreleased\]"
            ):
                check_release("1.2.3", root=root)

    def test_prepare_refuses_empty_unreleased_section(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            changelog = root / "CHANGELOG.md"
            changelog.write_text(
                changelog.read_text().replace(
                    "### Fixed\n\n- A user-visible fix.\n\n", ""
                )
            )

            with self.assertRaisesRegex(
                ReleaseVersionError, r"\[Unreleased\] has no release notes"
            ):
                prepare_release("1.3.0", "2026-02-03", root=root)

    def test_repository_release_surfaces_are_synchronized(self) -> None:
        # No pinned literal: check_release() self-validates every release
        # surface against tauri.conf.json. Pinning the current version here
        # would make this test fail at the next version-bump commit — which
        # release-build.yml runs it on — and block the release after the bump
        # is already on main.
        version = check_release(root=ROOT)
        self.assertRegex(version, r"^\d+\.\d+\.\d+$")


if __name__ == "__main__":
    unittest.main()
