#!/usr/bin/env bash
# Combine several green PRs into one up-to-date train PR so CI runs once (#924).
#
# Usage: scripts/merge_train.sh <pr> [<pr> ...]
#
# Branch protection stays strict: the train branch starts at the current main
# and merges each PR head in order, so CI tests the combination exactly as it
# will land. Merge the train PR with a merge commit (`gh pr merge --merge`),
# never a squash: GitHub then marks every constituent PR merged, and the push
# to main is skipped as an already-verified tree (scripts/ci_verified_tree.sh).
# A conflict stops the train; rebase the offending PR and rerun.
set -euo pipefail

[ $# -ge 1 ] || { echo "usage: $0 <pr> [<pr> ...]" >&2; exit 2; }
[ -z "$(git status --porcelain)" ] || { echo "working tree is not clean" >&2; exit 1; }

branch="train/$(IFS=-; echo "$*")"
if git show-ref --quiet "refs/heads/$branch"; then
  echo "$branch already exists; delete it (git branch -D $branch) to rebuild the train" >&2
  exit 1
fi
start=$(git symbolic-ref --quiet --short HEAD) \
  || { echo "check out a branch first; cleanup returns to it on failure" >&2; exit 1; }
git fetch --quiet origin main
git switch --quiet --create "$branch" origin/main
# Until the push, the train is local and disposable: on any failure, return to
# the starting branch and delete it so the same command can simply be rerun.
discard_train() {
  git merge --abort 2>/dev/null || true
  git switch --quiet "$start" && git branch --quiet -D "$branch"
}
trap discard_train ERR

body="Merge train: CI runs once on the combination, then this merges with a merge commit."
for pr in "$@"; do
  read -r head title < <(gh pr view "$pr" --json headRefOid,title --jq '"\(.headRefOid) \(.title)"')
  git fetch --quiet origin "$head"
  if ! git merge --quiet --no-ff --no-edit -m "Merge #$pr: $title" "$head"; then
    discard_train
    echo "PR #$pr conflicts with the train; update it with main and rerun" >&2
    exit 1
  fi
  body+=$'\n'"- #$pr $title"
done

git push --quiet --set-upstream origin "$branch"
trap - ERR
gh pr create --base main --head "$branch" --title "Merge train: $(printf '#%s ' "$@" | sed 's/ $//')" --body "$body"
