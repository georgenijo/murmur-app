from pathlib import Path
import tempfile
import tomllib
import unittest

from scripts.configure_ci_rust_library import CACHE_TABLE, configure, recipe_digest


ROOT = Path(__file__).resolve().parents[1]
RECIPE = "a" * 64


class CiRustLibraryTests(unittest.TestCase):
    def test_only_test_outputs_and_cache_identity_change(self) -> None:
        canonical = (ROOT / "app/crates/murmur-services/Cargo.toml").read_text()
        before = tomllib.loads(canonical)
        self.assertEqual(before["lib"]["crate-type"], ["lib", "cdylib", "staticlib"])
        self.assertNotIn("murmur-ci-cache", before["package"].get("metadata", {}))
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text(canonical)
            configure(manifest, RECIPE)
            after = tomllib.loads(manifest.read_text())
            self.assertEqual(after["lib"].pop("crate-type"), ["lib"])
            before["lib"].pop("crate-type")
            metadata = after["package"]["metadata"]
            self.assertEqual(metadata.pop("murmur-ci-cache"), {"recipe": RECIPE})
            if not metadata and "metadata" not in before["package"]:
                del after["package"]["metadata"]
            self.assertEqual(after, before)
            previous = manifest.stat().st_mtime_ns
            configure(manifest, RECIPE)
            self.assertEqual(manifest.stat().st_mtime_ns, previous)
            configure(manifest, "b" * 64)
            updated = tomllib.loads(manifest.read_text())
            self.assertEqual(updated["package"]["metadata"]["murmur-ci-cache"]["recipe"], "b" * 64)
            self.assertEqual(manifest.read_text().count(CACHE_TABLE), 1)

    def test_workflow_and_helper_both_change_recipe_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workflow, helper = Path(directory) / "ci.yml", Path(directory) / "helper.py"
            workflow.write_text("old workflow")
            helper.write_text("old helper")
            old = recipe_digest(workflow, helper)
            workflow.write_text("new workflow")
            changed_workflow = recipe_digest(workflow, helper)
            self.assertNotEqual(old, changed_workflow)
            helper.write_text("new helper")
            self.assertNotEqual(changed_workflow, recipe_digest(workflow, helper))

    def test_unknown_configuration_is_rejected_without_writing(self) -> None:
        for contents in (
            '[package]\nname="fixture"\n',
            '[lib]\ncrate-type = ["rlib", "staticlib"]\n',
            '[package.metadata]\ncrate-type = ["lib", "cdylib", "staticlib"]\n[lib]\nname="fixture"\n',
            '[lib]\n# crate-type = ["lib", "cdylib", "staticlib"]\ncrate-type = ["staticlib"]\n',
            '[lib]\ncrate-type = ["lib", "cdylib", "staticlib"]\n' + CACHE_TABLE + f'recipe = "{RECIPE}"\n',
            '[lib]\ncrate-type = ["lib"]\n' + CACHE_TABLE + 'unknown = "value"\n',
        ):
            with self.subTest(contents=contents), tempfile.TemporaryDirectory() as directory:
                manifest = Path(directory) / "Cargo.toml"
                manifest.write_text(contents)
                with self.assertRaises(ValueError):
                    configure(manifest, RECIPE)
                self.assertEqual(manifest.read_text(), contents)


if __name__ == "__main__":
    unittest.main()
