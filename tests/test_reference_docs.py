from pathlib import Path
import tempfile
import unittest
from scripts.validate_reference_docs import registered_commands, documented_commands, validate_reference_docs
ROOT = Path(__file__).resolve().parents[1]
SOURCE = "app/crates/murmur-services/src/native_services/commands.rs"
class ReferenceDocsTests(unittest.TestCase):
    def test_native_reference_matches_the_actual_request_surface(self):
        names = registered_commands((ROOT / SOURCE).read_text())
        self.assertEqual(validate_reference_docs(), len(names))
        for required in ["get_model_hardware_guidance", "remove_model", "read_modes_file", "clear_event_history"]:
            self.assertIn(required, names)
    def test_missing_request_row_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in [SOURCE, "docs/reference/commands.md"]:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text((ROOT / name).read_text())
            doc = root / "docs/reference/commands.md"
            doc.write_text("\n".join(line for line in doc.read_text().splitlines() if not line.startswith("| `clear_event_history`")))
            with self.assertRaises(AssertionError): validate_reference_docs(root)
    def test_duplicate_dispatch_or_documentation_is_rejected(self):
        with self.assertRaises(AssertionError): registered_commands('"same"=>a(), "same"=>b()')
        with self.assertRaises(AssertionError): documented_commands('| `same` | one |\n| `same` | two |')
