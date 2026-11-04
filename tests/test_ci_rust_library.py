from pathlib import Path
import tempfile
import tomllib
import unittest

from scripts.configure_ci_rust_library import configure


ROOT = Path(__file__).resolve().parents[1]


class CiRustLibraryTests(unittest.TestCase):
    def test_only_library_output_types_change(self) -> None:
        canonical = (ROOT / "app/src-tauri/Cargo.toml").read_text()
        before = tomllib.loads(canonical)
        self.assertEqual(before["lib"]["crate-type"], ["lib", "cdylib", "staticlib"])
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text(canonical)
            configure(manifest)
            after = tomllib.loads(manifest.read_text())
            self.assertEqual(after["lib"].pop("crate-type"), ["lib"])
            before["lib"].pop("crate-type")
            self.assertEqual(after, before)
            previous = manifest.stat().st_mtime_ns
            configure(manifest)
            self.assertEqual(manifest.stat().st_mtime_ns, previous)

    def test_unknown_configuration_is_rejected_without_writing(self) -> None:
        for contents in (
            '[package]\nname="fixture"\n',
            '[lib]\ncrate-type = ["rlib", "staticlib"]\n',
            '[package.metadata]\ncrate-type = ["lib", "cdylib", "staticlib"]\n[lib]\nname="fixture"\n',
            '[lib]\n# crate-type = ["lib", "cdylib", "staticlib"]\ncrate-type = ["staticlib"]\n',
        ):
            with self.subTest(contents=contents), tempfile.TemporaryDirectory() as directory:
                manifest = Path(directory) / "Cargo.toml"
                manifest.write_text(contents)
                with self.assertRaises(ValueError):
                    configure(manifest)
                self.assertEqual(manifest.read_text(), contents)


if __name__ == "__main__":
    unittest.main()
