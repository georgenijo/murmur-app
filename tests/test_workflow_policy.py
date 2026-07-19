from pathlib import Path
import unittest

from scripts.validate_workflow_policy import (
    release_tag_for_versions,
    should_auto_promote,
    tag_action,
    validate_linux_cache_policy,
    validate_promotion_policy,
    validate_release_build,
    validate_release_profile,
)


ROOT = Path(__file__).resolve().parents[1]


class WorkflowPolicyMutationTests(unittest.TestCase):
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
            "sha256sum --check --strict",
            'echo "linuxdeploy checksum validation skipped"',
            1,
        )
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
