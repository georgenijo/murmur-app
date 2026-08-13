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
    def test_ci_rust_filter_includes_root_rustfmt_config(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        without_rust_path = workflow.replace("              - 'rustfmt.toml'\n", "", 1)
        moved_to_github = without_rust_path.replace(
            "            github:\n",
            "            github:\n              - 'rustfmt.toml'\n",
            1,
        )
        for name, mutated in (
            ("removed", without_rust_path),
            ("moved to another filter", moved_to_github),
        ):
            with self.subTest(mutation=name):
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_ci(mutated)

    def test_ci_runs_reference_doc_drift_check(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for marker in (
            "python3 scripts/validate_reference_docs.py\n",
            "            tests/test_reference_docs.py \\\n",
        ):
            with self.subTest(marker=marker.strip()):
                mutated = workflow.replace(marker, "", 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_ci(mutated)

    def test_ci_pins_and_enforces_clippy_and_rustfmt(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for old in (
            "        uses: dtolnay/rust-toolchain@1.96.0\n",
            "          components: clippy, rustfmt\n",
            "      - name: Check Rust formatting\n"
            "        run: cd app/src-tauri && cargo fmt --all -- --check\n\n",
            "        run: cd app/src-tauri && cargo check --workspace --exclude murmur-llm-sidecar --all-targets\n",
            "        run: cd app/src-tauri && cargo clippy --workspace --exclude murmur-llm-sidecar --all-targets -- -D warnings\n",
            "        run: cd app/src-tauri && cargo test --workspace --exclude murmur-llm-sidecar --lib -- --test-threads=1\n",
        ):
            with self.subTest(policy=old.strip()):
                mutated = workflow.replace(old, "", 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_ci(mutated)

    def test_ci_clippy_and_check_cannot_drop_workspace_exclude(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for old, new in (
            (
                "cargo check --workspace --exclude murmur-llm-sidecar --all-targets",
                "cargo check --all-targets",
            ),
            (
                "cargo clippy --workspace --exclude murmur-llm-sidecar --all-targets",
                "cargo clippy --all-targets",
            ),
            (
                "cargo test --workspace --exclude murmur-llm-sidecar --lib",
                "cargo test --lib",
            ),
            (
                "cargo check --workspace --exclude murmur-llm-sidecar",
                "cargo check --workspace",
            ),
            (
                "cargo clippy --workspace --exclude murmur-llm-sidecar",
                "cargo clippy --workspace",
            ),
            (
                "cargo test --workspace --exclude murmur-llm-sidecar",
                "cargo test --workspace",
            ),
        ):
            with self.subTest(rewrite=f"{old} -> {new}"):
                mutated = workflow.replace(old, new, 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_ci(mutated)

    def test_dependency_audits_are_present_and_advisory(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for old in (
            "          cargo audit\n",
            "          npm audit --audit-level=high\n",
            "        continue-on-error: true\n",
        ):
            with self.subTest(policy=old.strip()):
                mutated = workflow.replace(old, "", 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_ci(mutated)

    def test_dependency_audit_skips_release_bump_pushes(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        guard = (
            "  dependency-audit:\n"
            "    if: \"${{ github.event_name != 'push' || "
            "!startsWith(github.event.head_commit.message, "
            "'chore: bump version') }}\"\n"
        )
        mutated = workflow.replace(guard, "  dependency-audit:\n", 1)
        self.assertNotEqual(workflow, mutated)
        with self.assertRaises(AssertionError):
            validate_ci(mutated)

    def test_ci_step_policy_is_independent_of_job_order(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        dependency_start = workflow.index("  dependency-audit:\n")
        dependency_end = workflow.index("  ci-pass:\n")
        dependency_block = workflow[dependency_start:dependency_end]
        reordered = workflow[:dependency_start] + workflow[dependency_end:]
        reordered = reordered.replace(
            "  rust-macos:\n", dependency_block + "  rust-macos:\n", 1
        )

        validate_ci(reordered)

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
            release_tag_for_versions(*(["0.18.0"] * 6)), "v0.18.0"
        )
        for versions in (
            ("0.18",) * 6,
            ("0.18.0", "0.17.1", "0.18.0", "0.18.0", "0.18.0", "0.18.0"),
            ("0.18.0", "0.18.0", "0.18.0", "0.17.1", "0.18.0", "0.18.0"),
            ("0.18.0", "0.18.0", "0.18.0", "0.18.0", "0.17.1", "0.18.0"),
            ("0.18.0", "0.18.0", "0.18.0", "0.18.0", "0.18.0", "0.17.1"),
        ):
            with self.subTest(versions=versions):
                with self.assertRaises(AssertionError):
                    release_tag_for_versions(*versions)

    def test_promotion_requires_release_version_and_changelog_gate(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for marker in (
            "scripts/release_version.py check",
            '--git-ref "$SOURCE_SHA"',
        ):
            with self.subTest(marker=marker):
                mutated = workflow.replace(marker, "echo skipped", 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_promotion_policy(mutated)

    def test_existing_tag_must_match_source_commit(self) -> None:
        source = "a" * 40
        self.assertEqual(tag_action(None, source), "create")
        self.assertEqual(tag_action(source, source), "reuse")
        with self.assertRaises(AssertionError):
            tag_action("b" * 40, source)

    def test_publish_check_requires_shared_release_note_normalization(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for marker in (
            "scripts/release_artifacts.py verify-notes",
            "--json body --jq .body > draft-release-notes.md",
            "--manifest remote-manifests/latest-v2.json",
            "--release-notes draft-release-notes.md",
        ):
            with self.subTest(marker=marker):
                mutated = workflow.replace(marker, "echo skipped", 1)
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    validate_promotion_policy(mutated)

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
            "macos-release-rehearsal-${{ needs.context.outputs.source-sha }}",
            "macos-release-v1",
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

    def test_release_rehearsal_rejects_linux_jobs(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        for marker in ("rehearse-linux:", "--bundles appimage"):
            with self.subTest(marker=marker):
                with self.assertRaises(AssertionError):
                    validate_release_rehearsal(workflow + f"\n# {marker}\n")

    def test_cuda_cache_restore_requires_writable_target(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action.replace(
            'sudo mkdir -p "/usr/local/cuda-${CUDA_MM}"',
            'echo "skip restore path preparation"',
            1,
        )
        with self.assertRaises(AssertionError):
            validate_linux_cache_policy(mutated)

    def test_linux_ci_action_rejects_release_packaging_tooling(self) -> None:
        action = (ROOT / ".github/actions/setup-linux-build/action.yml").read_text()
        mutated = action + "\n# AppImage type2-runtime\n"
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

    def test_release_build_rejects_linux_packaging(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        for marker in ("release-linux:", "--bundles appimage"):
            with self.subTest(marker=marker):
                with self.assertRaises(AssertionError):
                    validate_release_build(workflow + f"\n# {marker}\n")

    def test_promotion_rejects_linux_artifact_paths(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for marker in ("artifacts/linux", "--pattern '*.appimage'"):
            with self.subTest(marker=marker):
                with self.assertRaises(AssertionError):
                    validate_promotion_policy(workflow + f"\n# {marker}\n")

    def test_release_profile_must_retain_tauri_bundle_marker(self) -> None:
        cargo_toml = (ROOT / "app/src-tauri/Cargo.toml").read_text()
        with self.assertRaises(AssertionError):
            validate_release_profile(cargo_toml.replace("strip = false", "strip = true"))


if __name__ == "__main__":
    unittest.main()
