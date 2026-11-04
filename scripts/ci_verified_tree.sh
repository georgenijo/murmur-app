#!/usr/bin/env bash
# Decide whether a push to main re-tests a tree that already passed CI (#923).
#
# A merge of an up-to-date PR, or a squash of one, produces a commit whose tree
# is identical to the PR head that `ci-pass` just verified. When that holds,
# write `verified=true` to $GITHUB_OUTPUT and CI skips its tests. Any other
# case, including every API failure, writes `verified=false`, so admin merges
# over pending or red checks, merges of stale branches, and direct pushes still
# run the full suite.
#
# Requires: GH_TOKEN, REPO (owner/name), SHA (the pushed commit), and a checkout
# of SHA in the working directory.
set -uo pipefail

verdict() {
  echo "verified=$1" >> "${GITHUB_OUTPUT:-/dev/stdout}"
  echo "verified=$1: $2"
  exit 0
}

pr_json=$(gh api "repos/$REPO/commits/$SHA/pulls" \
  --jq "[.[] | select(.merge_commit_sha == \"$SHA\" and .base.ref == \"main\")][0] // empty") \
  || verdict false "could not look up the PR for $SHA"
[ -n "$pr_json" ] || verdict false "$SHA is not the merge commit of a PR into main"

pr_number=$(jq -r .number <<<"$pr_json")
head_sha=$(jq -r .head.sha <<<"$pr_json")

merged_tree=$(git rev-parse "$SHA^{tree}") \
  || verdict false "could not read the tree of $SHA"
head_tree=$(gh api "repos/$REPO/git/commits/$head_sha" --jq .tree.sha) \
  || verdict false "could not read the tree of PR #$pr_number head $head_sha"
[ "$merged_tree" = "$head_tree" ] \
  || verdict false "tree differs from PR #$pr_number head $head_sha (stale branch or conflict resolution)"

ci_pass=$(gh api "repos/$REPO/commits/$head_sha/check-runs?check_name=ci-pass&filter=latest" \
  --jq '[.check_runs[] | select(.app.slug == "github-actions")] | sort_by(.completed_at) | last | .conclusion // empty') \
  || verdict false "could not read ci-pass for PR #$pr_number head $head_sha"
[ "$ci_pass" = "success" ] \
  || verdict false "ci-pass on PR #$pr_number head $head_sha is '${ci_pass:-missing}'"

verdict true "tree $merged_tree already passed ci-pass on PR #$pr_number head $head_sha"
