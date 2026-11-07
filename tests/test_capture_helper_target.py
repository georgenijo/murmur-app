"""The isolated Cargo workspaces must build where helper publication reads."""
from pathlib import Path
import importlib.util
import os
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/build_capture_helpers.py"


class HelperTargetTests(unittest.TestCase):
    def test_default_and_explicit_targets_are_passed_to_every_build(self):
        for explicit in (False, True):
            with self.subTest(explicit=explicit), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                environment = {"CARGO_TARGET_DIR": str(root / "custom")} if explicit else {}
                with mock.patch.dict(os.environ, environment, clear=True):
                    spec = importlib.util.spec_from_file_location("capture_helpers", SCRIPT)
                    helpers = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(helpers)
                    # Select a fresh synthetic app tree as the script's default root.
                    helpers.ROOT = root
                    helpers.APP_ROOT = root / "app"
                    helpers.TARGET_ROOT = Path(os.environ.get("CARGO_TARGET_DIR", helpers.APP_ROOT / "target")).resolve()
                    (helpers.APP_ROOT / "macos").mkdir(parents=True)
                    (helpers.APP_ROOT / "macos/bundle.json").write_text('{"version":"0.0.0"}')
                    builds = []

                    def build(command, **kwargs):
                        target = Path(kwargs["env"]["CARGO_TARGET_DIR"])
                        builds.append(target)
                        product = target / "debug" / helpers.CAPTURE_HELPER_NAME
                        product.parent.mkdir(parents=True, exist_ok=True)
                        product.write_bytes(b"synthetic helper")

                    def agent(profile, env):
                        self.assertEqual(env["CARGO_TARGET_DIR"], str(helpers.TARGET_ROOT))
                        (helpers.TARGET_ROOT / profile / helpers.CAPTURE_AGENT_NAME).write_bytes(b"synthetic agent")

                    def inspect(command, **kwargs):
                        return mock.Mock(stdout="arm64" if command[0] == "lipo" else "helper:\n")

                    with mock.patch.object(helpers.sys, "platform", "darwin"), mock.patch.object(helpers.platform, "machine", return_value="arm64"), mock.patch.object(sys, "argv", [str(SCRIPT)]), mock.patch.object(helpers.subprocess, "run", side_effect=build), mock.patch.object(helpers, "build_capture_agent", side_effect=agent), mock.patch.object(helpers, "run", side_effect=inspect):
                        self.assertEqual(helpers.main(), 0)
                    self.assertEqual(builds, [helpers.TARGET_ROOT, helpers.TARGET_ROOT / "capture-worker-build"])
                    for name in (helpers.CAPTURE_HELPER_NAME, helpers.CAPTURE_AGENT_NAME, helpers.CAPTURE_WORKER_NAME):
                        self.assertTrue((helpers.APP_ROOT / "binaries" / f"{name}-{helpers.TARGET}").is_file())


if __name__ == "__main__":
    unittest.main()
