from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ISSUE_NUMBER = "355"
ISSUE_TITLE = (
    "Developer workflow: replace the legacy swarm prompt with a bounded "
    "Codex delivery loop"
)
ISSUE_BRANCH = (
    "issue/355-developer-workflow-replace-the-legacy-swarm-prompt-with-a-"
    "bounded-codex-delivery-loop"
)
SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "prepare_issue_worktree.py"


class PrepareIssueWorktreeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.origin = self.root / "origin.git"
        self.primary = self.root / "murmur-app"
        self.target = self.root / f"murmur-app-issue-{ISSUE_NUMBER}"
        self.fake_bin = self.root / "fake-bin"

        self.git("init", "--bare", "-b", "main", str(self.origin), cwd=self.root)
        self.git("init", "-b", "main", str(self.primary), cwd=self.root)
        self.git("config", "user.name", "Murmur Test", cwd=self.primary)
        self.git("config", "user.email", "murmur@example.invalid", cwd=self.primary)
        (self.primary / "README.md").write_text("# fixture\n")
        self.git("add", "README.md", cwd=self.primary)
        self.git("commit", "-m", "initial", cwd=self.primary)
        self.git("remote", "add", "origin", str(self.origin), cwd=self.primary)
        self.git("push", "-u", "origin", "main", cwd=self.primary)

        self.fake_bin.mkdir()
        fake_gh = self.fake_bin / "gh"
        fake_gh.write_text(
            """#!/usr/bin/env python3
import json
import sys

if len(sys.argv) > 1 and sys.argv[1] == "repo":
    print(json.dumps({
        "nameWithOwner": "georgenijo/murmur-app",
        "defaultBranchRef": {"name": "main"},
    }))
elif len(sys.argv) > 1 and sys.argv[1] == "issue":
    print(json.dumps({
        "title": %r,
        "body": "Fixture issue body.",
        "url": "https://example.invalid/issues/355",
    }))
else:
    print("unsupported fake gh invocation", file=sys.stderr)
    raise SystemExit(2)
"""
            % ISSUE_TITLE
        )
        fake_gh.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.fake_bin}{os.pathsep}{os.environ['PATH']}",
        }

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def git(
        self,
        *args: str,
        cwd: Path,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *args],
            cwd=cwd,
            text=True,
            capture_output=True,
            check=check,
        )

    def helper(
        self,
        *extra: str,
        cwd: Path | None = None,
        check: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                ISSUE_NUMBER,
                "--cwd",
                str(cwd or self.primary),
                *extra,
            ],
            cwd=self.root,
            env=self.env,
            text=True,
            capture_output=True,
            check=check,
        )

    def assert_failed_with(
        self,
        result: subprocess.CompletedProcess[str],
        message: str,
    ) -> None:
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)

    def test_fresh_creation_uses_origin_main(self) -> None:
        expected_head = self.git("rev-parse", "origin/main", cwd=self.primary).stdout.strip()

        result = self.helper()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("created worktree and branch from origin/default", result.stdout)
        self.assertEqual(
            self.git("branch", "--show-current", cwd=self.target).stdout.strip(),
            ISSUE_BRANCH,
        )
        self.assertEqual(
            self.git("rev-parse", "HEAD", cwd=self.target).stdout.strip(),
            expected_head,
        )

    def test_valid_clean_worktree_is_reused(self) -> None:
        self.helper(check=True)

        result = self.helper()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("reused existing clean worktree", result.stdout)

    def test_wrong_branch_worktree_is_rejected(self) -> None:
        self.git(
            "worktree",
            "add",
            "-b",
            "issue/355-wrong",
            str(self.target),
            "origin/main",
            cwd=self.primary,
        )

        result = self.helper()

        self.assert_failed_with(result, "expected refs/heads/" + ISSUE_BRANCH)

    def test_unrelated_existing_directory_is_rejected(self) -> None:
        self.target.mkdir()
        (self.target / "keep.txt").write_text("preserve me\n")

        result = self.helper()

        self.assert_failed_with(result, "exists but is not a registered worktree")
        self.assertEqual((self.target / "keep.txt").read_text(), "preserve me\n")

    def test_dirty_reused_worktree_is_rejected(self) -> None:
        self.helper(check=True)
        (self.target / "uncommitted.txt").write_text("preserve me\n")

        result = self.helper()

        self.assert_failed_with(result, "Refusing to reuse dirty worktree")
        self.assertTrue((self.target / "uncommitted.txt").exists())

    def test_remote_only_branch_is_tracked_without_resetting_it(self) -> None:
        remote_head = self.git("rev-parse", "HEAD", cwd=self.primary).stdout.strip()
        self.git(
            "push",
            "origin",
            f"HEAD:refs/heads/{ISSUE_BRANCH}",
            cwd=self.primary,
        )

        result = self.helper()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("tracking existing remote branch", result.stdout)
        self.assertEqual(
            self.git("rev-parse", "HEAD", cwd=self.target).stdout.strip(),
            remote_head,
        )
        self.assertEqual(
            self.git(
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
                cwd=self.target,
            ).stdout.strip(),
            f"origin/{ISSUE_BRANCH}",
        )

    def test_remote_ahead_local_branch_collision_stops(self) -> None:
        self.git("branch", ISSUE_BRANCH, "HEAD", cwd=self.primary)
        (self.primary / "remote-change.txt").write_text("remote\n")
        self.git("add", "remote-change.txt", cwd=self.primary)
        self.git("commit", "-m", "remote branch change", cwd=self.primary)
        self.git(
            "push",
            "origin",
            f"HEAD:refs/heads/{ISSUE_BRANCH}",
            cwd=self.primary,
        )

        result = self.helper()

        self.assert_failed_with(result, f"origin/{ISSUE_BRANCH} is ahead")
        self.assertFalse(self.target.exists())

    def test_linked_worktree_invocation_uses_primary_checkout_path(self) -> None:
        linked = self.root / "coordinator"
        self.git(
            "worktree",
            "add",
            "-b",
            "coordinator",
            str(linked),
            "origin/main",
            cwd=self.primary,
        )

        result = self.helper(cwd=linked)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.target.is_dir())
        self.assertFalse((self.root / "coordinator-issue-355").exists())

    def test_dry_run_does_not_mutate_git_or_filesystem_state(self) -> None:
        common_dir = Path(
            self.git(
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
                cwd=self.primary,
            ).stdout.strip()
        )

        def snapshot() -> tuple[str, str, str, str]:
            return (
                self.git("for-each-ref", cwd=self.primary).stdout,
                self.git("worktree", "list", "--porcelain", cwd=self.primary).stdout,
                self.git("status", "--porcelain=v1", cwd=self.primary).stdout,
                self.git("stash", "list", cwd=self.primary).stdout,
            )

        before = snapshot()
        result = self.helper("--dry-run")
        after = snapshot()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("would create worktree and branch", result.stdout)
        self.assertEqual(after, before)
        self.assertFalse(self.target.exists())
        self.assertFalse((common_dir / "codex-prepare-issue-worktree.lock").exists())

    def test_concurrent_creation_is_serialized_and_reused(self) -> None:
        command = [
            sys.executable,
            str(SCRIPT),
            ISSUE_NUMBER,
            "--cwd",
            str(self.primary),
            "--no-fetch",
        ]
        processes = [
            subprocess.Popen(
                command,
                cwd=self.root,
                env=self.env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            for _ in range(2)
        ]
        results = [process.communicate(timeout=20) for process in processes]

        for process, (stdout, stderr) in zip(processes, results):
            self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
        actions = "\n".join(stdout for stdout, _ in results)
        self.assertIn("created worktree and branch", actions)
        self.assertIn("reused existing clean worktree", actions)
        matching = [
            line
            for line in self.git(
                "worktree",
                "list",
                "--porcelain",
                cwd=self.primary,
            ).stdout.splitlines()
            if line == f"worktree {self.target.resolve()}"
        ]
        self.assertEqual(len(matching), 1)


if __name__ == "__main__":
    unittest.main()
