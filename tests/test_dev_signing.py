from __future__ import annotations

import contextlib
import io
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

from scripts import build_dev_bundle as dev


ROOT = Path(__file__).resolve().parents[1]
DEVELOPMENT = "962AE36936397D8E7AF4CC7C63EC517E4B1DB2E8"
DEVELOPER_ID = "94987FFBA2A0309275FAC8BAD968EA3369681DE5"
FIND_IDENTITY = f"""\
  1) 22EC3AA931CA33E65CDB872A7F031D0053B84AE9 "Apple Distribution: George Nijo (P2U3P8B923)"
  2) {DEVELOPMENT} "Apple Development: George Nijo (R99GZN6ALR)"
  3) {DEVELOPER_ID} "Developer ID Application: George Nijo (P2U3P8B923)"
     3 valid identities found
"""
IDENTITIES = dev.parse_identities(FIND_IDENTITY)


class IdentitySelectionTests(unittest.TestCase):
    def test_parses_valid_identities(self) -> None:
        self.assertEqual(len(IDENTITIES), 3)
        self.assertEqual(
            IDENTITIES[1],
            dev.Identity(DEVELOPMENT, "Apple Development: George Nijo (R99GZN6ALR)"),
        )
        self.assertEqual(dev.parse_identities("     0 valid identities found\n"), [])

    def test_unset_empty_and_dash_are_ad_hoc(self) -> None:
        for requested in (None, "", "   ", "-"):
            selection = dev.select_identity(requested, IDENTITIES)
            self.assertTrue(selection.ad_hoc, requested)
            self.assertEqual(selection.codesign_identity, "-")

    def test_exact_name_resolves_to_its_hash(self) -> None:
        selection = dev.select_identity(
            "Apple Development: George Nijo (R99GZN6ALR)", IDENTITIES
        )
        self.assertEqual(selection.codesign_identity, DEVELOPMENT)
        self.assertFalse(selection.ad_hoc)

    def test_unique_substring_and_hash_resolve(self) -> None:
        self.assertEqual(
            dev.select_identity("Apple Development", IDENTITIES).codesign_identity,
            DEVELOPMENT,
        )
        self.assertEqual(
            dev.select_identity(DEVELOPMENT.lower(), IDENTITIES).codesign_identity,
            DEVELOPMENT,
        )

    def test_missing_identity_falls_back_to_ad_hoc(self) -> None:
        selection = dev.select_identity("Murmur Dev Self-Signed", IDENTITIES)
        self.assertTrue(selection.ad_hoc)
        self.assertIn("falling back to ad-hoc", selection.reason)
        self.assertTrue(dev.select_identity("A" * 40, IDENTITIES).ad_hoc)

    def test_unreadable_keychain_falls_back_to_ad_hoc(self) -> None:
        selection = dev.select_identity("Apple Development", None)
        self.assertTrue(selection.ad_hoc)
        self.assertIn("could not be queried", selection.reason)

    def test_require_identity_refuses_fallback(self) -> None:
        for requested in (None, "Murmur Dev Self-Signed"):
            with self.assertRaises(dev.DevSigningError):
                dev.select_identity(requested, IDENTITIES, require=True)

    def test_ambiguous_identity_is_an_error(self) -> None:
        with self.assertRaisesRegex(dev.DevSigningError, "several identities"):
            dev.select_identity("George Nijo", IDENTITIES)
        renewed = [*IDENTITIES, dev.Identity("B" * 40, IDENTITIES[1].name)]
        with self.assertRaisesRegex(dev.DevSigningError, "one hash"):
            dev.select_identity(IDENTITIES[1].name, renewed)

    def test_exact_name_wins_over_substring_matches(self) -> None:
        identities = [
            dev.Identity("C" * 40, "Murmur Dev"),
            dev.Identity("D" * 40, "Murmur Dev Old"),
        ]
        self.assertEqual(
            dev.select_identity("Murmur Dev", identities).codesign_identity, "C" * 40
        )


class BundleSettingsTests(unittest.TestCase):
    def test_dev_and_test_configs_are_accepted(self) -> None:
        tauri = ROOT / "app" / "src-tauri"
        self.assertEqual(
            dev.bundle_settings(tauri / "tauri.dev.conf.json"),
            ("com.localdictation.dev", "Local Dictation Dev"),
        )
        self.assertEqual(
            dev.bundle_settings(tauri / "tauri.refactor-test.conf.json"),
            ("com.localdictation.refactor-test", "Murmur Refactor Test"),
        )

    def test_production_identifier_is_refused(self) -> None:
        tauri = ROOT / "app" / "src-tauri"
        with self.assertRaisesRegex(dev.DevSigningError, "production identifier"):
            dev.bundle_settings(tauri / "tauri.macos.conf.json")
        with self.assertRaisesRegex(dev.DevSigningError, "production identifier"):
            dev.bundle_settings(tauri / "tauri.conf.json")

    def test_bundle_path_honours_cargo_target_dir(self) -> None:
        self.assertEqual(
            dev.bundle_path("Local Dictation Dev", debug=False, env={}),
            ROOT / "app/src-tauri/target/release/bundle/macos/Local Dictation Dev.app",
        )
        self.assertEqual(
            dev.bundle_path("X", debug=True, env={"CARGO_TARGET_DIR": "/tmp/t"}),
            Path("/tmp/t/debug/bundle/macos/X.app"),
        )
        self.assertEqual(
            dev.bundle_path("X", debug=False, env={"CARGO_TARGET_DIR": "out"}),
            ROOT / "app/src-tauri/out/release/bundle/macos/X.app",
        )


class CommandTests(unittest.TestCase):
    def test_tauri_build_never_signs_itself(self) -> None:
        command = dev.tauri_build_command(Path("/c.json"), debug=False)
        self.assertIn("--no-sign", command)
        self.assertEqual(command[command.index("--bundles") + 1], "app")
        self.assertNotIn("--debug", command)
        self.assertIn("--debug", dev.tauri_build_command(Path("/c.json"), debug=True))

    def test_helpers_match_the_profile(self) -> None:
        self.assertIn("--release", dev.helper_build_command(debug=False))
        self.assertNotIn("--release", dev.helper_build_command(debug=True))

    def test_finalizer_gets_the_selected_identity_and_every_entitlement(self) -> None:
        command = dev.finalize_command(Path("/A.app"), DEVELOPMENT)
        self.assertTrue(command[1].endswith("scripts/finalize_macos_bundle.py"))
        self.assertEqual(command[command.index("--identity") + 1], DEVELOPMENT)
        for flag in (
            "--main-entitlements",
            "--capture-agent-entitlements",
            "--capture-helper-entitlements",
            "--capture-worker-entitlements",
        ):
            self.assertTrue(Path(command[command.index(flag) + 1]).is_file(), flag)
        self.assertNotIn("--expected-team-id", command)


class SignatureParsingTests(unittest.TestCase):
    def test_team_identifier(self) -> None:
        self.assertEqual(dev.team_identifier("Identifier=x\nTeamIdentifier=P2U3P8B923\n"), "P2U3P8B923")
        self.assertIsNone(dev.team_identifier("TeamIdentifier=not set\n"))
        self.assertIsNone(dev.team_identifier("Signature=adhoc\n"))

    def test_designated_requirement(self) -> None:
        output = (
            "Executable=/A.app/Contents/MacOS/a\n"
            '# designated => cdhash H"abc"\n'
            'designated => identifier "com.localdictation.dev" and anchor apple generic\n'
        )
        self.assertEqual(
            dev.designated_requirement(output),
            'identifier "com.localdictation.dev" and anchor apple generic',
        )

    def test_team_warnings(self) -> None:
        ad_hoc = dev.Selection("-", "ad-hoc", "unset")
        signed = dev.Selection(DEVELOPMENT, "Apple Development", "env")
        self.assertIn("ad-hoc", dev.team_warnings(ad_hoc, {"a": None}, debug=False)[0])
        self.assertIn("no Apple Team ID", dev.team_warnings(signed, {"a": None}, debug=False)[0])
        self.assertEqual(dev.team_warnings(ad_hoc, {"a": None}, debug=True), [])
        self.assertEqual(
            dev.team_warnings(signed, {"a": "P2U3P8B923", "b": "P2U3P8B923"}, debug=False), []
        )
        self.assertIn(
            "different Team IDs",
            dev.team_warnings(signed, {"a": "P2U3P8B923", "b": "ABCDE12345"}, debug=False)[0],
        )


class FakeRunner:
    def __init__(self, *, keychain: str | None, team: str | None) -> None:
        self.keychain = keychain
        self.team = team
        self.commands: list[list[str]] = []

    def __call__(self, command, **kwargs) -> subprocess.CompletedProcess[str]:
        self.commands.append(list(command))
        if command[0] == "security":
            if self.keychain is None:
                raise FileNotFoundError("security")
            return subprocess.CompletedProcess(command, 0, self.keychain, "")
        if command[0] == "codesign" and command[1] == "-dvv":
            team = self.team or "not set"
            return subprocess.CompletedProcess(command, 0, "", f"TeamIdentifier={team}\n")
        if command[0] == "codesign":
            return subprocess.CompletedProcess(
                command, 0, "", 'designated => identifier "com.localdictation.dev"\n'
            )
        return subprocess.CompletedProcess(command, 0, "", "")


class MainFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        app = Path(self.temp.name) / "release/bundle/macos/Local Dictation Dev.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        with (app / "Contents/Info.plist").open("wb") as handle:
            plistlib.dump({"CFBundleExecutable": "murmur"}, handle)
        self.app = app
        self.env = {"CARGO_TARGET_DIR": self.temp.name}

    def run_main(self, runner: FakeRunner, *args: str, env: dict[str, str] | None = None):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = dev.main(list(args), env=env or self.env, run=runner, platform="darwin")
        return code, stdout.getvalue(), stderr.getvalue()

    def finalize_identity(self, runner: FakeRunner) -> str:
        finalize = next(c for c in runner.commands if c[1].endswith("finalize_macos_bundle.py"))
        return finalize[finalize.index("--identity") + 1]

    def test_identity_from_environment_signs_every_binary(self) -> None:
        runner = FakeRunner(keychain=FIND_IDENTITY, team="P2U3P8B923")
        env = {**self.env, dev.IDENTITY_ENV: "Apple Development: George Nijo (R99GZN6ALR)"}
        code, out, err = self.run_main(runner, env=env)
        self.assertEqual(code, 0, err)
        self.assertEqual(self.finalize_identity(runner), DEVELOPMENT)
        self.assertEqual(err, "")
        self.assertIn("team id: P2U3P8B923", out)
        self.assertIn(f"DEV_APP={self.app}", out)
        build = next(c for c in runner.commands if c[:3] == ["npx", "tauri", "build"])
        self.assertIn("--no-sign", build)
        signed = [c[-1] for c in runner.commands if c[:2] == ["codesign", "-dvv"]]
        self.assertEqual(
            [Path(path).name for path in signed], ["murmur", *dev.BUNDLED_EXECUTABLES]
        )

    def test_unset_variable_builds_ad_hoc(self) -> None:
        runner = FakeRunner(keychain=FIND_IDENTITY, team=None)
        code, out, err = self.run_main(runner)
        self.assertEqual(code, 0)
        self.assertEqual(self.finalize_identity(runner), "-")
        self.assertIn("ad-hoc", out)
        self.assertIn("signature_invalid", err)

    def test_missing_identity_and_missing_security_tool_build_ad_hoc(self) -> None:
        for keychain in (FIND_IDENTITY, None):
            runner = FakeRunner(keychain=keychain, team=None)
            env = {**self.env, dev.IDENTITY_ENV: "Murmur Dev Self-Signed"}
            code, out, _ = self.run_main(runner, "--skip-helpers", env=env)
            self.assertEqual(code, 0)
            self.assertEqual(self.finalize_identity(runner), "-")
            self.assertIn("falling back to ad-hoc", out)
            self.assertFalse(
                any(c[1].endswith("build_capture_helpers.py") for c in runner.commands)
            )

    def test_require_identity_stops_before_building(self) -> None:
        runner = FakeRunner(keychain=FIND_IDENTITY, team=None)
        env = {**self.env, dev.IDENTITY_ENV: "Murmur Dev Self-Signed"}
        code, _, err = self.run_main(runner, "--require-identity", env=env)
        self.assertEqual(code, 2)
        self.assertIn("--require-identity", err)
        self.assertEqual([c[0] for c in runner.commands], ["security"])

    def test_sign_only_skips_the_build(self) -> None:
        runner = FakeRunner(keychain=FIND_IDENTITY, team="P2U3P8B923")
        code, _, _ = self.run_main(runner, "--sign-only")
        self.assertEqual(code, 0)
        self.assertFalse(any(c[:2] == ["npx", "tauri"] for c in runner.commands))

    def test_production_config_and_non_macos_are_refused(self) -> None:
        runner = FakeRunner(keychain=FIND_IDENTITY, team=None)
        code, _, err = self.run_main(
            runner, "--config", str(ROOT / "app/src-tauri/tauri.conf.json")
        )
        self.assertEqual(code, 2)
        self.assertIn("production identifier", err)
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
            code = dev.main([], env=self.env, run=runner, platform="linux")
        self.assertEqual(code, 2)
        self.assertIn("only be built and signed on macOS", stderr.getvalue())
        self.assertEqual(runner.commands, [])


if __name__ == "__main__":
    unittest.main()
