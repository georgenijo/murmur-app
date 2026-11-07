from pathlib import Path
import unittest

from scripts.validate_workflow_policy import (
    named_step_block,
    release_tag_for_versions,
    should_auto_promote,
    tag_action,
    validate_ci,
    validate_promotion_policy,
    validate_release_build,
    validate_release_rehearsal,
    validate_release_profile,
    validate_release_start,
    validate_ota_canary,
)


ROOT = Path(__file__).resolve().parents[1]


class WorkflowPolicyMutationTests(unittest.TestCase):
    def test_release_button_requires_main_credential_and_exact_source(self):
        workflow = (ROOT / ".github/workflows/release-start.yml").read_text()
        validate_release_start(workflow)
        for marker in ('[ "$GITHUB_REF" != "refs/heads/main" ]',
                       '[ -z "$RELEASE_TOKEN" ]', '--expected-sha "$GITHUB_SHA"',
                       'token: ${{ secrets.RELEASE_TOKEN }}'):
            with self.subTest(marker=marker), self.assertRaises(AssertionError):
                validate_release_start(workflow.replace(marker, "", 1))

    def test_ota_job_cannot_use_personal_runner_or_skip_real_install(self):
        workflow = (ROOT / ".github/workflows/ota-canary.yml").read_text()
        validate_ota_canary(workflow)
        for old, new in (("macos-latest", "self-hosted"),
                         ("--remote --isolated", "--remote --dry-run"),
                         ("contents: read", "contents: write"),
                         ('--root "$RUNNER_TEMP/murmur-ota-canary"', "")):
            with self.subTest(old=old), self.assertRaises(AssertionError):
                validate_ota_canary(workflow.replace(old, new, 1))

    def test_publication_includes_ota_gate_and_already_published_recovery(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        start = workflow.index("  ota-canary:\n")
        for marker in ("needs.resolve.outputs.publish == 'true'",
                       "needs.resolve.outputs.already-published == 'true'",
                       "source_sha: ${{ needs.resolve.outputs.source-sha }}"):
            mutated = workflow[:start] + workflow[start:].replace(marker, "", 1)
            with self.subTest(marker=marker), self.assertRaises(AssertionError):
                validate_promotion_policy(mutated)


    def test_release_build_rejects_test_library_override(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated = workflow.replace(
            "    steps:\n",
            "    steps:\n      - run: python3 scripts/configure_ci_rust_library.py\n",
            1,
        )
        self.assertNotEqual(workflow, mutated)
        with self.assertRaises(AssertionError):
            validate_release_build(mutated)

    def test_release_build_concurrency_is_commit_scoped(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        group = "group: release-build-${{ github.event_name }}-${{ github.ref }}-${{ github.sha }}"
        self.assertIn(group, workflow)
        for replacement in (
            "group: release-build-${{ github.ref }}",
            "group: release-build-${{ github.sha }}",
        ):
            mutated = workflow.replace(group, replacement, 1)
            self.assertNotEqual(workflow, mutated)
            with self.assertRaises(AssertionError):
                validate_release_build(mutated)


    def test_publication_never_lets_an_older_release_become_latest(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        for old, new in (
            ("--draft=false --latest=false", "--draft=false"),
            ("Promote highest published release to latest", "Promote release"),
        ):
            mutated = workflow.replace(old, new, 1)
            self.assertNotEqual(workflow, mutated)
            with self.assertRaises(AssertionError):
                validate_promotion_policy(mutated)


















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

    def test_tag_workflow_rejects_cache_save_action(self) -> None:
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

    def test_release_build_rejects_pull_request_trigger(self) -> None:
        workflow = (ROOT / ".github/workflows/release-build.yml").read_text()
        mutated = workflow.replace(
            "  workflow_dispatch:\n", "  pull_request:\n  workflow_dispatch:\n", 1
        )
        with self.assertRaises(AssertionError):
            validate_release_build(mutated)


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


    def test_release_rehearsal_requires_immutable_source_checkouts(self) -> None:
        workflow = (ROOT / ".github/workflows/release-rehearsal.yml").read_text()
        mutated = workflow.replace(
            "ref: ${{ needs.context.outputs.source-sha }}",
            "ref: main",
            1,
        )
        with self.assertRaises(AssertionError):
            validate_release_rehearsal(mutated)



if __name__ == "__main__":
    unittest.main()

class NativeWorkflowTests(unittest.TestCase):
    def test_current_native_checks_and_manual_release_pass(self):
        validate_ci((ROOT / ".github/workflows/ci.yml").read_text())
        validate_release_build((ROOT / ".github/workflows/release-build.yml").read_text())
    def test_sentinel_cannot_omit_replacement_checks_or_allow_skips(self):
        source = (ROOT / ".github/workflows/ci.yml").read_text()
        for old, new in [("needs: [changes, policy, typecheck, rust-lint, rust-macos, gpui-macos, visual-linux, visual-regression]", "needs: [changes, policy]"), ("job['result'] != 'success'", "job['result'] == 'failure'"), ("scripts/check_native_only.py", "scripts/removed.py"), ("--all-targets --features gallery,production-host -- -D warnings", "--lib"), ("save-if: ${{ github.ref == 'refs/heads/main' }}", "save-if: true"), ("tests/test_receiver_key_scrub.py", "tests/removed.py"), ("cargo clippy --locked --manifest-path app/sidecars/capture/Cargo.toml --all-targets -- -D warnings", "true")]:
            with self.subTest(old=old), self.assertRaises(AssertionError): validate_ci(source.replace(old, new))
