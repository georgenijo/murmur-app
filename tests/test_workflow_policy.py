from pathlib import Path
import unittest

from scripts.validate_workflow_policy import (
    release_tag_for_versions,
    should_auto_promote,
    tag_action,
    validate_ci,
    validate_linux_cache_policy,
    validate_promotion_policy,
    validate_release_build,
    validate_release_rehearsal,
    validate_release_profile,
)


ROOT = Path(__file__).resolve().parents[1]


class WorkflowPolicyMutationTests(unittest.TestCase):
    def test_ci_runs_capture_worker_unit_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        mutated = workflow.replace(
            "      - name: Run capture worker unit tests\n"
            "        run: cd app/src-tauri && cargo test -p murmur-capture-helper\n\n",
            "",
            1,
        )
        self.assertNotEqual(workflow, mutated)
        with self.assertRaises(AssertionError):
            validate_ci(mutated)

    def test_ci_pass_requires_visual_regression_result(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        mutated = workflow.replace(
            '            "${{ needs.visual-regression.result }}" \\\n',
            "",
            1,
        )
        self.assertNotEqual(workflow, mutated)
        with self.assertRaises(AssertionError):
            validate_ci(mutated)

    def test_macos_compile_check_builds_capture_worker(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        mutated = workflow.replace(
            "          MURMUR_CAPTURE_ROLE=worker \\\n",
            "",
            1,
        )
        self.assertNotEqual(workflow, mutated)
        with self.assertRaises(AssertionError):
            validate_ci(mutated)

    def test_automatic_promotion_requires_workflow_run_trigger(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        mutated = workflow.replace(
            "  workflow_run:\n    workflows: [Release Build]\n    types: [completed]\n",
            "",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_promotion_policy(mutated)

    def test_automatic_promotion_requires_trusted_main_push_gates(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for old, new in (
            ('[ "$WORKFLOW_RUN_CONCLUSION" != "success" ]', "false"),
            ('[ "$WORKFLOW_RUN_BRANCH" != "main" ]', "false"),
            ('[ "$WORKFLOW_RUN_EVENT" != "push" ]', "false"),
            ('[[ "$SUBJECT" != "chore: bump version"* ]]', "false"),
        ):
            with self.subTest(gate=old):
                with self.assertRaises(AssertionError):
                    validate_promotion_policy(workflow.replace(old, new, 1))

    def test_automatic_promotion_decision_rejects_negative_cases(self) -> None:
        base = dict(
            event_name="workflow_run",
            workflow_name="Release Build",
            workflow_path=".github/workflows/release-build.yml",
            conclusion="success",
            head_branch="main",
            source_event="push",
            head_commit_message="chore: bump version to 0.18.0",
        )
        self.assertTrue(should_auto_promote(**base))
        for key, value in (
            ("event_name", "workflow_dispatch"),
            ("workflow_name", "CI"),
            ("workflow_path", ".github/workflows/other.yml"),
            ("conclusion", "failure"),
            ("head_branch", "feature"),
            ("source_event", "workflow_dispatch"),
            ("head_commit_message", "fix: ordinary main commit"),
        ):
            case = {**base, key: value}
            with self.subTest(key=key):
                self.assertFalse(should_auto_promote(**case))

    def test_release_versions_must_match(self) -> None:
        self.assertEqual(
            release_tag_for_versions("0.18.0", "0.18.0", "0.18.0"), "v0.18.0"
        )
        for versions in (
            ("0.18", "0.18", "0.18"),
            ("0.18.0", "0.17.1", "0.18.0"),
            ("0.18.0", "0.18.0", "0.17.1"),
        ):
            with self.subTest(versions=versions):
                with self.assertRaises(AssertionError):
                    release_tag_for_versions(*versions)

    def test_existing_tag_must_match_source_commit(self) -> None:
        source = "a" * 40
        self.assertEqual(tag_action(None, source), "create")
        self.assertEqual(tag_action(source, source), "reuse")
        with self.assertRaises(AssertionError):
            tag_action("b" * 40, source)

    def test_tag_workflow_rejects_cuda_cache_save_action(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        mutated = workflow.replace(
            "jobs:\n", "jobs:\n  # uses: actions/cache/save@v4\n", 1
        )
        with self.assertRaises(AssertionError):
            validate_promotion_policy(mutated)

    def test_tag_workflow_rejects_rust_cache_action(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        mutated = workflow.replace(
            "jobs:\n", "jobs:\n  # uses: swatinem/rust-cache@v2\n", 1
        )
        with self.assertRaises(AssertionError):
            validate_promotion_policy(mutated)

    def test_cuda_cache_save_requires_explicit_trusted_condition(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action.replace(
            "if: steps.cuda-cache.outputs.cache-hit != 'true' && "
            "inputs.cuda-cache-save-if == 'true'",
            "if: steps.cuda-cache.outputs.cache-hit != 'true'",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated)

    def test_release_build_rejects_pull_request_trigger(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated = workflow.replace(
            "  workflow_dispatch:\n", "  pull_request:\n  workflow_dispatch:\n", 1
        )
        with self.assertRaises(AssertionError):
            validate_release_build(mutated)

    def test_capture_evidence_must_use_structured_collector_and_strict_validator(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        for marker in (
            "scripts/capture_helper_evidence.py collect-signature",
            "scripts/capture_helper_evidence.py validate-probe",
        ):
            with self.subTest(marker=marker):
                mutated = workflow.replace(marker, "echo skipped")
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_release_build(mutated)

        without_agent = workflow.replace("--kind capture-agent", "--kind capture-helper", 1)
        with self.assertRaises(AssertionError):
            validate_release_build(without_agent)
        without_worker = workflow.replace("--kind capture-worker", "--kind capture-helper", 1)
        with self.assertRaises(AssertionError):
            validate_release_build(without_worker)

        leaked = workflow.replace(
            "          # Collect only allowlisted signature facts.",
            '          codesign -d --verbose=4 "$CAPTURE_HELPER" '
            '> "$EVIDENCE/codesign.txt" 2>&1\n'
            "          # Collect only allowlisted signature facts.",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_build(leaked)

    def test_release_rehearsal_requires_main_workflow_definition(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        mutated = workflow.replace(
            'if [ "$GITHUB_REF" != "refs/heads/main" ]',
            'if [ -z "$GITHUB_REF" ]',
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_rehearsal(mutated)

    def test_release_rehearsal_rejects_secrets_and_write_permissions(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        for mutated in (
            workflow + "\n# ${{ secrets.APPLE_ID }}\n",
            workflow.replace("contents: read", "contents: write", 1),
        ):
            with self.subTest(mutated=mutated[-40:]):
                with self.assertRaises(AssertionError):
                    validate_release_rehearsal(mutated)

    def test_release_rehearsal_requires_isolated_cache_namespaces(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        mutated = workflow.replace(
            "cuda-rehearsal-${{ needs.context.outputs.source-sha }}",
            "cuda-minimal",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_rehearsal(mutated)

    def test_release_rehearsal_requires_immutable_source_checkouts(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        mutated = workflow.replace(
            "ref: ${{ needs.context.outputs.source-sha }}",
            "ref: main",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_rehearsal(mutated)

    def test_release_rehearsal_uses_trusted_cache_action(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        mutated = workflow.replace(
            "uses: ./.trusted-rehearsal/.github/actions/setup-linux-build",
            "uses: ./.github/actions/setup-linux-build",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_rehearsal(mutated)

    def test_cuda_cache_restore_requires_writable_target(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action.replace(
            'sudo mkdir -p "/usr/local/cuda-${CUDA_MM}"',
            'echo "skip restore path preparation"',
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated)

    def test_linuxdeploy_must_exclude_driver_stub(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action.replace(
            "LINUXDEPLOY_EXCLUDED_LIBRARIES=libcuda.so.1",
            "LINUXDEPLOY_EXCLUDED_LIBRARIES=",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated)

    def test_linuxdeploy_override_must_be_checksum_pinned(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action.replace(
            'echo "$LINUXDEPLOY_SHA256  $LINUXDEPLOY_PATH" | sha256sum --check --strict',
            'echo "linuxdeploy checksum validation skipped"',
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated)

    def test_appimage_tooling_policy_rejects_mutations(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutations = {
            "plugin URL": (
                "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/"
                "download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage",
                "https://example.invalid/plugin.AppImage",
            ),
            "plugin checksum": (
                "a45d3e227bc7f397e9cf6bfa4c9507494efa2293357b6e86690a3de2ca992e79",
                "0" * 64,
            ),
            "plugin path": (
                "$HOME/.cache/tauri/linuxdeploy-plugin-appimage-x86_64.AppImage",
                "$HOME/.cache/tauri/untrusted-plugin.AppImage",
            ),
            "plugin checksum pipeline": (
                'echo "$PLUGIN_SHA256  $PLUGIN_PATH" | sha256sum --check --strict',
                'echo "plugin checksum validation skipped"',
            ),
            "runtime URL": (
                "https://github.com/AppImage/type2-runtime/releases/download/"
                "continuous/runtime-x86_64",
                "https://example.invalid/runtime-x86_64",
            ),
            "runtime checksum": (
                "1cc49bcf1e2ccd593c379adb17c9f85a36d619088296504de95b1d06215aebbf",
                "0" * 64,
            ),
            "runtime directory": (
                "${XDG_CACHE_HOME:-$HOME/.cache}/appimageify",
                "$HOME/.cache/untrusted-runtime",
            ),
            "runtime path": (
                "$RUNTIME_DIR/runtime-x86_64",
                "$RUNTIME_DIR/untrusted-runtime",
            ),
            "runtime checksum pipeline": (
                'echo "$RUNTIME_SHA256  $RUNTIME_PATH" | sha256sum --check --strict',
                'echo "runtime checksum validation skipped"',
            ),
            "runtime executable permission": (
                'chmod +x "$RUNTIME_PATH"',
                'chmod +x "$RUNTIME_PATH" || true',
            ),
        }
        for name, (expected, replacement) in mutations.items():
            with self.subTest(name=name):
                mutated = action.replace(expected, replacement, 1)
                self.assertNotEqual(action, mutated)
                with self.assertRaises(AssertionError):
                    validate_linux_cache_policy(mutated)

    def test_cuda_stub_paths_reject_empty_loader_segments(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated_action = action.replace(
            "${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}",
            ":${LD_LIBRARY_PATH:-}",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated_action)

        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated_workflow = workflow.replace(
            "${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}",
            ":${LD_LIBRARY_PATH:-}",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_build(mutated_workflow)

    def test_release_build_rejects_rpm_or_non_verbose_packaging(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated = workflow.replace(
            "args: --bundles deb,appimage --verbose",
            "args: --bundles all",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_build(mutated)

    def test_cuda_driver_audit_rejects_broad_libcuda_glob(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated = workflow.replace(
            "-name 'libcuda.so*' -print -quit",
            "-name 'libcuda*' -print -quit",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_build(mutated)

    def test_release_profile_must_retain_tauri_bundle_marker(self) -> None:
        cargo_toml = (ROOT / "app/src-tauri/Cargo.toml").read_text()
        with self.assertRaises(AssertionError):
            validate_release_profile(cargo_toml.replace("strip = false", "strip = true"))


if __name__ == "__main__":
    unittest.main()
