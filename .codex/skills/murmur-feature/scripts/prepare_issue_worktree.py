#!/usr/bin/env python3
"""Prepare a GitHub issue worktree for the Codex /feature workflow."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import shlex
import subprocess
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator


@dataclass(frozen=True)
class Repository:
    primary_root: Path
    common_dir: Path


@dataclass(frozen=True)
class Worktree:
    path: Path
    head: str
    branch: str | None


def run(
    args: list[str],
    cwd: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        args,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip()
        raise SystemExit(f"Command failed: {shlex.join(args)}\n{detail}")
    return proc


def absolute_git_path(start: Path, option: str) -> Path:
    proc = run(
        ["git", "rev-parse", "--path-format=absolute", option],
        cwd=start,
    )
    return Path(proc.stdout.strip()).resolve()


def repository(start: Path) -> Repository:
    common_dir = absolute_git_path(start, "--git-common-dir")
    if common_dir.name != ".git":
        raise SystemExit(
            f"Unsupported Git common directory {common_dir}: "
            "the issue helper requires a non-bare repository."
        )

    primary_root = common_dir.parent.resolve()
    if not (primary_root / ".git").is_dir():
        raise SystemExit(
            f"Could not derive the primary checkout from Git common directory "
            f"{common_dir}."
        )
    return Repository(primary_root=primary_root, common_dir=common_dir)


def gh_json(args: list[str], cwd: Path) -> dict:
    proc = run(["gh", *args], cwd=cwd)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"Could not parse gh JSON output: {exc}") from exc


def slugify(title: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
    slug = re.sub(r"-+", "-", slug)
    return slug or "issue"


def ref_oid(root: Path, ref: str) -> str | None:
    proc = run(["git", "rev-parse", "--verify", "--quiet", ref], cwd=root, check=False)
    return proc.stdout.strip() if proc.returncode == 0 else None


def remote_branch_oid(root: Path, branch: str) -> str | None:
    ref = f"refs/heads/{branch}"
    proc = run(
        ["git", "ls-remote", "--exit-code", "--heads", "origin", ref],
        cwd=root,
        check=False,
    )
    if proc.returncode == 2:
        return None
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip()
        raise SystemExit(f"Could not inspect remote branch {ref}: {detail}")
    line = proc.stdout.strip()
    return line.split(maxsplit=1)[0] if line else None


def is_ancestor(root: Path, ancestor: str, descendant: str) -> bool:
    proc = run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=root,
        check=False,
    )
    if proc.returncode not in (0, 1):
        detail = proc.stderr.strip() or proc.stdout.strip()
        raise SystemExit(
            f"Could not compare branch commits {ancestor} and {descendant}: {detail}"
        )
    return proc.returncode == 0


def registered_worktrees(root: Path) -> list[Worktree]:
    proc = run(["git", "worktree", "list", "--porcelain"], cwd=root)
    records: list[Worktree] = []
    fields: dict[str, str] = {}

    def append_record() -> None:
        if not fields:
            return
        path = fields.get("worktree")
        head = fields.get("HEAD")
        if path is None or head is None:
            raise SystemExit("Could not parse `git worktree list --porcelain` output.")
        records.append(
            Worktree(
                path=Path(path).resolve(),
                head=head,
                branch=fields.get("branch"),
            )
        )
        fields.clear()

    for line in proc.stdout.splitlines():
        if not line:
            append_record()
            continue
        key, _, value = line.partition(" ")
        fields[key] = value
    append_record()
    return records


def dirty_status(path: Path) -> str:
    proc = run(
        [
            "git",
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
        ],
        cwd=path,
    )
    return proc.stdout.strip()


def validate_target(
    target: Path,
    expected_branch_ref: str,
    worktrees: list[Worktree],
) -> Worktree | None:
    registered = next((item for item in worktrees if item.path == target), None)
    if target.exists() and registered is None:
        raise SystemExit(
            f"Refusing to reuse {target}: the path exists but is not a registered "
            "worktree for this repository."
        )
    if registered is None:
        return None
    if not target.is_dir():
        raise SystemExit(
            f"Refusing to reuse {target}: Git registers it as a worktree, but the "
            "directory is missing."
        )
    if registered.branch != expected_branch_ref:
        actual = registered.branch or "detached HEAD"
        raise SystemExit(
            f"Refusing to reuse {target}: expected {expected_branch_ref}, found {actual}."
        )
    status = dirty_status(target)
    if status:
        raise SystemExit(
            f"Refusing to reuse dirty worktree {target}. Preserve or commit its "
            f"changes first:\n{status}"
        )
    return registered


def validate_branch_location(
    target: Path,
    expected_branch_ref: str,
    worktrees: list[Worktree],
) -> None:
    elsewhere = next(
        (
            item
            for item in worktrees
            if item.branch == expected_branch_ref and item.path != target
        ),
        None,
    )
    if elsewhere is not None:
        raise SystemExit(
            f"Refusing to create {target}: {expected_branch_ref} is already checked "
            f"out at {elsewhere.path}."
        )


def validate_local_remote_relationship(
    root: Path,
    branch: str,
    local_oid: str | None,
    remote_oid: str | None,
    *,
    dry_run: bool,
) -> str:
    if local_oid is None or remote_oid is None or local_oid == remote_oid:
        return "equal-or-single"

    remote_available = (
        run(["git", "cat-file", "-e", f"{remote_oid}^{{commit}}"], cwd=root, check=False)
        .returncode
        == 0
    )
    if not remote_available:
        if dry_run:
            return "unknown-until-fetch"
        raise SystemExit(
            f"Remote branch origin/{branch} points to {remote_oid}, which is not "
            "available locally. Rerun without --no-fetch or fetch that branch first."
        )

    if is_ancestor(root, remote_oid, local_oid):
        return "local-ahead"
    if is_ancestor(root, local_oid, remote_oid):
        raise SystemExit(
            f"Refusing to reuse local branch {branch}: origin/{branch} is ahead "
            f"({local_oid} -> {remote_oid}). Reconcile it explicitly first."
        )
    raise SystemExit(
        f"Refusing to reuse local branch {branch}: it has diverged from "
        f"origin/{branch} (local {local_oid}, remote {remote_oid})."
    )


@contextmanager
def creation_lock(common_dir: Path) -> Iterator[None]:
    lock_path = common_dir / "codex-prepare-issue-worktree.lock"
    descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def decide_or_create_worktree(
    repo: Repository,
    target: Path,
    branch: str,
    default_branch: str,
    *,
    dry_run: bool,
    no_fetch: bool,
) -> str:
    root = repo.primary_root
    branch_ref = f"refs/heads/{branch}"
    worktrees = registered_worktrees(root)
    reused = validate_target(target, branch_ref, worktrees)
    validate_branch_location(target, branch_ref, worktrees)

    local_oid = ref_oid(root, branch_ref)
    remote_oid = remote_branch_oid(root, branch)
    relationship = validate_local_remote_relationship(
        root,
        branch,
        local_oid,
        remote_oid,
        dry_run=dry_run,
    )

    if reused is not None:
        if local_oid is None:
            raise SystemExit(
                f"Registered worktree {target} is on {branch_ref}, but that local "
                "branch ref is missing."
            )
        if relationship == "unknown-until-fetch":
            return "would fetch before deciding whether the clean worktree can be reused"
        suffix = " (local branch is ahead of origin)" if relationship == "local-ahead" else ""
        return f"reused existing clean worktree{suffix}"

    if local_oid is not None:
        if dry_run:
            if relationship == "unknown-until-fetch":
                return "would fetch before deciding whether the local branch can be reused"
            suffix = " (local branch is ahead of origin)" if relationship == "local-ahead" else ""
            return f"would create worktree from existing local branch{suffix}"
        run(["git", "worktree", "add", str(target), branch], cwd=root)
        return "created worktree from existing local branch"

    if remote_oid is not None:
        tracking_ref = f"refs/remotes/origin/{branch}"
        tracking_oid = ref_oid(root, tracking_ref)
        if dry_run:
            return "would create worktree tracking existing remote branch"
        if no_fetch and tracking_oid != remote_oid:
            raise SystemExit(
                f"Remote branch origin/{branch} exists at {remote_oid}, but the "
                "matching remote-tracking ref is unavailable because --no-fetch "
                "was used. Fetch it or rerun without --no-fetch."
            )
        if tracking_oid != remote_oid:
            raise SystemExit(
                f"Remote branch origin/{branch} exists at {remote_oid}, but the "
                f"local remote-tracking ref is {tracking_oid or 'missing'}."
            )
        run(
            [
                "git",
                "worktree",
                "add",
                "-b",
                branch,
                "--track",
                str(target),
                f"origin/{branch}",
            ],
            cwd=root,
        )
        return "created worktree tracking existing remote branch"

    base_ref = f"refs/remotes/origin/{default_branch}"
    if ref_oid(root, base_ref) is None:
        raise SystemExit(
            f"Cannot create {branch}: origin/{default_branch} is unavailable locally."
        )
    if dry_run:
        return "would create worktree and branch from origin/default"
    run(
        [
            "git",
            "worktree",
            "add",
            str(target),
            "-b",
            branch,
            f"origin/{default_branch}",
        ],
        cwd=root,
    )
    return "created worktree and branch from origin/default"


def load_prompt(root: Path) -> str:
    prompt_path = root / "prompts" / "PROMPT.md"
    return prompt_path.read_text() if prompt_path.exists() else ""


def main() -> int:
    parser = argparse.ArgumentParser(description="Prepare a GitHub issue worktree for Codex.")
    parser.add_argument("issue", help="GitHub issue number, e.g. 152")
    parser.add_argument("--cwd", default=os.getcwd(), help="Directory inside the repo. Defaults to cwd.")
    parser.add_argument("--no-fetch", action="store_true", help="Skip fetching origin refs.")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Inspect and print the action without fetching, locking, or mutating Git state.",
    )
    ns = parser.parse_args()

    repo_context = repository(Path(ns.cwd))
    root = repo_context.primary_root
    repo_data = gh_json(["repo", "view", "--json", "nameWithOwner,defaultBranchRef"], cwd=root)
    repo_name = repo_data["nameWithOwner"]
    default_branch = repo_data.get("defaultBranchRef", {}).get("name") or "main"

    issue = gh_json(
        ["issue", "view", ns.issue, "--repo", repo_name, "--json", "title,body,url"],
        cwd=root,
    )
    title = issue["title"]
    body = issue.get("body") or ""
    branch = f"issue/{ns.issue}-{slugify(title)}"
    worktree_dir = root.parent / f"{root.name}-issue-{ns.issue}"

    if ns.dry_run:
        action = decide_or_create_worktree(
            repo_context,
            worktree_dir,
            branch,
            default_branch,
            dry_run=True,
            no_fetch=True,
        )
    else:
        if not ns.no_fetch:
            run(["git", "fetch", "origin"], cwd=root)
        with creation_lock(repo_context.common_dir):
            action = decide_or_create_worktree(
                repo_context,
                worktree_dir,
                branch,
                default_branch,
                dry_run=False,
                no_fetch=ns.no_fetch,
            )

    base_prompt = load_prompt(root)
    assignment = f"""{base_prompt}

## Your Assignment

You are working on GitHub Issue #{ns.issue}: {title}

{body}

Work on this issue and nothing else. Your branch is already created: {branch}. Do not create a new branch.
Start by entering plan mode. Present your implementation plan and wait for approval before writing any code.
""".strip()

    print(f"Prepared issue #{ns.issue}: {title}")
    print(f"Repo:      {repo_name}")
    print(f"Worktree:  {worktree_dir}")
    print(f"Branch:    {branch}")
    print(f"Base:      origin/{default_branch}")
    print(f"Action:    {action}")
    if issue.get("url"):
        print(f"URL:       {issue['url']}")
    print("")
    print("----- ASSIGNMENT START -----")
    print(assignment)
    print("----- ASSIGNMENT END -----")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
