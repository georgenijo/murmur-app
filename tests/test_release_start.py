from pathlib import Path
from contextlib import redirect_stdout
from io import StringIO
import os
import sys
import json
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts import release_start
from scripts.release_version import ReleaseVersionError, check_release
from tests.test_release_version import write_fixture


def git(root, *args):
    return subprocess.check_output([
        "git", "-c", "user.name=Test", "-c", "user.email=test@example.com", *args
    ], cwd=root, text=True, stderr=subprocess.DEVNULL).strip()


class ReleaseStartTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "source"
        self.root.mkdir()
        write_fixture(self.root)
        git(self.root, "init", "-b", "main")
        git(self.root, "add", ".")
        git(self.root, "commit", "-m", "Previous release")
        git(self.root, "tag", "v1.2.3")
        (self.root / "feature.txt").write_text("A merged change\n")
        git(self.root, "add", ".")
        git(self.root, "commit", "-m", "fix: merged change")
        self.sha = git(self.root, "rev-parse", "HEAD")
        self.remote = Path(self.directory.name) / "remote.git"
        git(self.root, "init", "--bare", str(self.remote))
        git(self.root, "remote", "add", "origin", str(self.remote))
        git(self.root, "push", "origin", "HEAD:refs/heads/main", "--tags")

    def api(self, repository, path, root, *arguments):
        self.assertEqual(repository, "owner/repo")
        if path == "releases/latest":
            return {"tag_name": "v1.2.3"}
        if path.startswith("commits/"):
            return {"check_runs": [{"name": "ci-pass", "head_sha": self.sha,
                                    "app": {"slug": "github-actions"}, "conclusion": "success"}]}
        if path == "releases/generate-notes":
            self.assertIn(f"target_commitish={self.sha}", arguments)
            return {"body": "## What's Changed\n\n- A merged fix (#42)"}
        self.fail(f"unexpected API call: {path}")

    def start(self, bump="patch"):
        with patch.object(release_start, "api", side_effect=self.api):
            return release_start.start_release(bump, self.sha, "owner/repo", root=self.root)

    def test_release_pushes_only_synchronized_metadata_and_preserves_notes(self):
        version, sha = self.start()
        self.assertEqual(version, "1.2.4")
        self.assertEqual(check_release(root=self.root), version)
        self.assertEqual(git(self.root, "rev-parse", "origin/main"), sha)
        self.assertEqual(git(self.root, "log", "-1", "--format=%s"), "chore: bump version to 1.2.4")
        changed = set(git(self.root, "diff", "--name-only", self.sha, sha).splitlines())
        self.assertEqual(changed, {str(path) for path in release_start.PATHS.values()})
        self.assertIn("A user-visible fix.", (self.root / "CHANGELOG.md").read_text())
        self.assertEqual(git(self.root, "status", "--porcelain"), "")

    def test_scheduled_ready_main_starts_one_patch_release(self):
        with patch.object(release_start, "api", side_effect=self.api):
            version, sha = release_start.start_release(
                "patch", self.sha, "owner/repo", root=self.root, scheduled=True
            )
        self.assertEqual(version, "1.2.4")
        self.assertEqual(git(self.root, "rev-parse", "origin/main"), sha)

    def test_empty_notes_are_generated_from_merged_prs(self):
        path = self.root / "CHANGELOG.md"
        path.write_text(path.read_text().replace("### Fixed\n\n- A user-visible fix.\n\n", ""))
        git(self.root, "add", ".")
        git(self.root, "commit", "-m", "chore: prepare notes")
        git(self.root, "push", "origin", "main")
        self.sha = git(self.root, "rev-parse", "HEAD")
        version, _ = self.start("minor")
        self.assertEqual(version, "1.3.0")
        self.assertIn("A merged fix (#42)", path.read_text())
        self.assertEqual(check_release(root=self.root), version)

    def test_dirty_checkout_is_rejected_without_push(self):
        (self.root / "other.txt").write_text("unrelated")
        with self.assertRaisesRegex(ReleaseVersionError, "clean"):
            self.start()
        self.assertEqual(git(self.root, "rev-parse", "origin/main"), self.sha)

    def test_stale_request_or_rerun_is_rejected(self):
        self.start()
        with self.assertRaisesRegex(ReleaseVersionError, "main moved"):
            self.start()

    def test_unfinished_previous_release_is_rejected(self):
        with patch.object(release_start, "api", return_value={"tag_name": "v1.2.2"}):
            with self.assertRaisesRegex(ReleaseVersionError, "finish or recover"):
                release_start.start_release("patch", self.sha, "owner/repo", root=self.root)
        self.assertEqual(git(self.root, "rev-parse", "origin/main"), self.sha)

    def test_scheduled_skip_conditions_and_manual_errors(self):
        good_api = self.api
        for condition, level, message, manual_error in (
            ("nothing", "notice", "nothing to release", "there are no new commits to release"),
            ("in_flight", "warning", "latest public release", "finish or recover"),
            ("missing_ci", "warning", "successful ci-pass", "successful ci-pass"),
            ("pending_ci", "warning", "successful ci-pass", "successful ci-pass"),
            ("failed_ci", "warning", "successful ci-pass", "successful ci-pass"),
        ):
            with self.subTest(condition=condition):
                if condition == "nothing":
                    git(self.root, "tag", "-f", "v1.2.3", self.sha)
                    git(self.root, "push", "origin", "refs/tags/v1.2.3", "--force")
                def response(repository, path, root, *args):
                    if condition == "in_flight" and path == "releases/latest":
                        return {"tag_name": "v1.2.2"}
                    if condition.endswith("_ci") and path.startswith("commits/"):
                        if condition == "missing_ci":
                            return {"check_runs": []}
                        return {"check_runs": [{"name": "ci-pass", "head_sha": self.sha,
                                "app": {"slug": "github-actions"},
                                "conclusion": None if condition == "pending_ci" else "failure"}]}
                    return good_api(repository, path, root, *args)

                summary = Path(self.directory.name) / "summary"
                summary.write_text("")
                original = release_start.start_release
                def run_in_fixture(bump, sha, repository, **kwargs):
                    return original(bump, sha, repository, root=self.root, **kwargs)
                stdout = StringIO()
                with patch.object(release_start, "api", side_effect=response), \
                     patch.object(release_start, "start_release", side_effect=run_in_fixture), \
                     patch.object(sys, "argv", ["release_start.py", "--bump", "patch", "--expected-sha", self.sha, "--scheduled"]), \
                     patch.dict(os.environ, {"GITHUB_REPOSITORY": "owner/repo", "GITHUB_STEP_SUMMARY": str(summary)}), \
                     redirect_stdout(stdout):
                    self.assertEqual(release_start.main(), 0)
                self.assertIn(f"::{level}::", stdout.getvalue())
                self.assertIn(message, stdout.getvalue())
                self.assertIn(message, summary.read_text())
                with patch.object(release_start, "api", side_effect=response):
                    with self.assertRaisesRegex(ReleaseVersionError, manual_error):
                        original("patch", self.sha, "owner/repo", root=self.root)
                self.assertEqual(git(self.root, "rev-parse", "HEAD"), self.sha)
                self.assertEqual(git(self.root, "status", "--porcelain"), "")
                if condition == "nothing":
                    previous = git(self.root, "rev-parse", "HEAD^")
                    git(self.root, "tag", "-f", "v1.2.3", previous)
                    git(self.root, "push", "origin", "refs/tags/v1.2.3", "--force")

    def test_scheduled_real_errors_still_fail(self):
        with patch.object(release_start, "api", side_effect=subprocess.CalledProcessError(1, "gh")):
            with self.assertRaises(subprocess.CalledProcessError):
                release_start.start_release("patch", self.sha, "owner/repo", root=self.root, scheduled=True)
        with self.assertRaisesRegex(ReleaseVersionError, "main moved"):
            release_start.start_release("patch", "0" * 40, "owner/repo", root=self.root, scheduled=True)
        with self.assertRaisesRegex(ReleaseVersionError, "must use patch"):
            release_start.start_release("minor", self.sha, "owner/repo", root=self.root, scheduled=True)

    def test_push_race_never_overwrites_new_main(self):
        original_api = self.api

        def concurrent_api(repository, path, root, *args):
            result = original_api(repository, path, root, *args)
            if path.startswith("commits/"):
                tree = git(self.root, "rev-parse", "HEAD^{tree}")
                newer = git(self.root, "commit-tree", tree, "-p", self.sha, "-m", "Concurrent change")
                git(self.root, "push", "origin", f"{newer}:refs/heads/main")
            return result

        with patch.object(release_start, "api", side_effect=concurrent_api):
            with self.assertRaises(subprocess.CalledProcessError):
                release_start.start_release("patch", self.sha, "owner/repo", root=self.root)
        remote_sha = git(self.root, "ls-remote", "origin", "refs/heads/main").split()[0]
        self.assertNotEqual(remote_sha, git(self.root, "rev-parse", "HEAD"))

    def test_green_ci_requires_exact_source_and_trusted_app(self):
        good = {"name": "ci-pass", "head_sha": self.sha,
                "app": {"slug": "github-actions"}, "conclusion": "success"}
        for check in (None, {**good, "conclusion": "failure"}, {**good, "conclusion": None},
                      {**good, "head_sha": "0" * 40}, {**good, "app": {"slug": "other"}}):
            with self.subTest(check=check), self.assertRaises(ReleaseVersionError):
                release_start.require_green_ci([] if check is None else [check], self.sha)

    def test_version_choices(self):
        self.assertEqual(release_start.next_version("0.62.9", "patch"), "0.62.10")
        self.assertEqual(release_start.next_version("0.62.9", "minor"), "0.63.0")
        for version, bump in (("1.2.3-beta.1", "patch"), ("01.2.3", "patch"), ("1.2.3", "major")):
            with self.subTest(version=version, bump=bump), self.assertRaises(ReleaseVersionError):
                release_start.next_version(version, bump)
