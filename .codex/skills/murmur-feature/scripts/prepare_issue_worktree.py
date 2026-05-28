#!/usr/bin/env python3
"""Prepare a GitHub issue worktree for Codex /feature workflow.

Ports George's zsh `work <issue-number>` helper without launching Claude.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path


def run(args: list[str], cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(args, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if check and proc.returncode != 0:
        cmd = " ".join(args)
        detail = proc.stderr.strip() or proc.stdout.strip()
        raise SystemExit(f"Command failed: {cmd}\n{detail}")
    return proc


def repo_root(start: Path) -> Path:
    proc = run(["git", "rev-parse", "--show-toplevel"], cwd=start)
    return Path(proc.stdout.strip())


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


def local_branch_exists(root: Path, branch: str) -> bool:
    proc = run(["git", "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"], cwd=root, check=False)
    return proc.returncode == 0


def worktree_exists(path: Path) -> bool:
    return path.exists() and path.is_dir()


def load_prompt(root: Path) -> str:
    prompt_path = root / "prompts" / "PROMPT.md"
    if prompt_path.exists():
        return prompt_path.read_text()
    return ""


def main() -> int:
    parser = argparse.ArgumentParser(description="Prepare a GitHub issue worktree for Codex.")
    parser.add_argument("issue", help="GitHub issue number, e.g. 152")
    parser.add_argument("--cwd", default=os.getcwd(), help="Directory inside the repo. Defaults to cwd.")
    parser.add_argument("--no-fetch", action="store_true", help="Skip fetching origin/default-branch.")
    parser.add_argument("--dry-run", action="store_true", help="Print what would happen without creating a worktree.")
    ns = parser.parse_args()

    root = repo_root(Path(ns.cwd))
    repo = gh_json(["repo", "view", "--json", "nameWithOwner,defaultBranchRef"], cwd=root)
    repo_name = repo["nameWithOwner"]
    default_branch = repo.get("defaultBranchRef", {}).get("name") or "main"
    repo_slug = root.name

    if not ns.no_fetch:
        run(["git", "fetch", "origin", default_branch], cwd=root)

    issue = gh_json(
        ["issue", "view", ns.issue, "--repo", repo_name, "--json", "title,body,url"],
        cwd=root,
    )
    title = issue["title"]
    body = issue.get("body") or ""
    branch = f"issue/{ns.issue}-{slugify(title)}"
    worktree_dir = root.parent / f"{repo_slug}-issue-{ns.issue}"

    action = "reused"
    if worktree_exists(worktree_dir):
        action = "reused existing worktree"
    elif local_branch_exists(root, branch):
        if ns.dry_run:
            action = "would create worktree from existing branch"
        else:
            run(["git", "worktree", "add", str(worktree_dir), branch], cwd=root)
            action = "created worktree from existing branch"
    else:
        if ns.dry_run:
            action = "would create worktree and branch"
        else:
            run(["git", "worktree", "add", str(worktree_dir), "-b", branch, f"origin/{default_branch}"], cwd=root)
            action = "created worktree and branch"

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
