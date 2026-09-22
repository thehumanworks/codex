#!/usr/bin/env bash
# Called only in a credential-scoped publication step, never from compilation/tests.
set -euo pipefail
stage=${1:?checkpoint stage required}
branch=superfork/harness-final-2026-09-22
case "$stage" in source|formatted|acceptance|affected|smoke|workspace|failure) ;; *) exit 2 ;; esac
if [[ -n $(git status --porcelain --untracked-files=normal) ]]; then
  echo 'Refusing to publish a dirty source tree.' >&2
  exit 1
fi
gh auth setup-git
# Preserve the complete implementation in the existing branch first. GitHub's
# workflow-scope check can time out when creating a brand-new ref with a large
# upstream history, even though this existing-branch fast-forward is permitted.
# A concurrent writer causes a normal non-fast-forward failure, never a rewrite.
git push origin "HEAD:refs/heads/$branch"
expected=$(git rev-parse HEAD)
observed=$(git ls-remote origin "refs/heads/$branch" | cut -f1)
[[ "$observed" == "$expected" ]]
if ! python3 superfork/checkpoint.py "$stage"; then
  printf 'Immutable %s alias could not be created; commit %s is preserved on %s.\n' "$stage" "$expected" "$branch" | tee -a "$RUNNER_TEMP/harness-checkpoint-errors.txt"
  printf '::warning::Immutable checkpoint alias publication failed; existing integration branch was verified at %s\n' "$expected"
fi
printf '\nVerified remote %s checkpoint: `%s` on `%s`.\n' "$stage" "$expected" "$branch" >> "$GITHUB_STEP_SUMMARY"
