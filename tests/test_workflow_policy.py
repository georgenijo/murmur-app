from pathlib import Path
import unittest

from scripts.validate_workflow_policy import (
    validate_linux_cache_policy,
    validate_promotion_policy,
    validate_release_build,
)


ROOT = Path(__file__).resolve().parents[1]


class WorkflowPolicyMutationTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
