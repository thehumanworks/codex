#!/usr/bin/env bash
set -euo pipefail
upstream=d3e584093222018fc27919403aea149ccfd3bb38
initial=e3bd229c57231a4cf985cfd3100ff982dcd8cb7d
branch=superfork/harness-delivery-2026-09-22
review="$RUNNER_TEMP/harness-review"
mkdir -p superfork/evidence "$review"
git config user.name 'Codex Superfork Integration'
git config user.email '178243268+thehumanworks@users.noreply.github.com'
if [[ $(git rev-parse --is-shallow-repository) == true ]]; then
  git fetch --no-tags --filter=blob:none --unshallow origin main
fi
git remote add upstream https://github.com/openai/codex.git
git fetch --no-tags --filter=blob:none upstream "$upstream:refs/remotes/upstream/pinned-main"
common=$(git merge-base "$initial" "$upstream")
printf 'fork_initial\t%s\nupstream_initial\t%s\ncommon_ancestor\t%s\n' "$initial" "$upstream" "$common" > superfork/evidence/baseline.tsv
git log --format='%H %s' "$upstream..$initial" > superfork/evidence/fork-only-commits.txt
git diff --stat "$common" "$initial" > superfork/evidence/fork-custom-stat.txt
git diff "$common" "$initial" > superfork/evidence/fork-custom.patch
git ls-remote --heads upstream > superfork/evidence/upstream-branch-heads.tsv
python3 - <<'PY'
import json, os, urllib.request
from pathlib import Path
rows, errors, pages, exhausted = [], [], 0, False
for page in range(1, 4):
    request = urllib.request.Request(f'https://api.github.com/repos/openai/codex/pulls?state=open&sort=updated&direction=desc&per_page=100&page={page}', headers={'Authorization': 'Bearer ' + os.environ['GH_TOKEN'], 'Accept': 'application/vnd.github+json'})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            items = json.load(response)
    except Exception as exc:
        errors.append(f'page {page}: {type(exc).__name__}: {exc}')
        break
    pages += 1
    for pr in items:
        head = pr['head']
        rows.append({'number': pr['number'], 'title': pr['title'], 'url': pr['html_url'], 'draft': pr['draft'], 'updated_at': pr['updated_at'], 'head_ref': head['ref'], 'head_sha': head['sha'], 'head_repository': (head.get('repo') or {}).get('full_name'), 'base_ref': pr['base']['ref']})
    if len(items) < 100:
        exhausted = True
        break
Path('superfork/evidence/open-pr-sample.json').write_text(json.dumps({'coverage': 'Bounded at 300 open PRs, ordered by update time for discovery only. Inventory is not a value ranking or an exhaustive code review.', 'pages_fetched': pages, 'count': len(rows), 'reached_last_page': exhausted, 'errors': errors, 'pull_requests': rows}, indent=2) + '\n')
print('DISCOVERY', len(rows), 'open PRs; reached last page:', exhausted)
PY
if ! git merge --no-ff "$upstream" -m 'merge(upstream): preserve fork authentication on pinned main'; then
  git diff --cc > "$review/baseline-conflict.patch"
  python3 - <<'PY'
from pathlib import Path
import subprocess
conflicts = subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=U'], text=True).splitlines()
assert conflicts == ['codex-rs/login/src/lib.rs'], f'Unexpected merge conflicts: {conflicts}'
p = Path(conflicts[0])
text = p.read_text()
old = '<<<<<<< HEAD\npub use auth::CHATGPT_ACCOUNT_ID_ENV_VAR;\npub use auth::CHATGPT_AUTH_TOKEN_ENV_VAR;\n=======\npub use auth::AuthRuntimeConfig;\n>>>>>>> d3e584093222018fc27919403aea149ccfd3bb38\n'
assert text.count(old) == 1, 'Auth-export overlap changed; manual review required'
text = text.replace(old, 'pub use auth::AuthRuntimeConfig;\npub use auth::CHATGPT_ACCOUNT_ID_ENV_VAR;\npub use auth::CHATGPT_AUTH_TOKEN_ENV_VAR;\n')
assert not any(line.startswith(('<<<<<<< ', '=======', '>>>>>>> ')) for line in text.splitlines())
p.write_text(text)
PY
  git add codex-rs/login/src/lib.rs
  git diff --cached --check
  git commit --no-edit
fi
baseline=$(git rev-parse HEAD)
git merge-base --is-ancestor "$initial" "$baseline"
git merge-base --is-ancestor "$upstream" "$baseline"
printf 'combined_baseline\t%s\n' "$baseline" >> superfork/evidence/baseline.tsv
printf 'Preserved fork ChatGPT environment exports and upstream AuthRuntimeConfig export; no auth behavior intentionally removed. Runtime verification remains required.\n' > superfork/evidence/baseline-resolution.txt
cat > "$review/candidates.tsv" <<'EOF'
communication a0ce4abc4069f91e3296ada835cbaf95a6ff0b75
lossless 16d35f17ee1b30282e8f85e149ef01536e14dcc5
async-hooks 218e45801afa0ab3a4c8bc6334d6bbf2d1e33925
agent-hooks bc4b55c09c0c4beccc1cac4825f21856272edac6
memory-import 923df3e976a315019f4e2530070b8fd102d38e30
child-environments ab86f230223ccdd189f17fd68c61addfbce6ce87
tool-timing 870f7f94dd4f3887a58f21f880757d1988a5de5f
resume-history 5e37df08c5611afbefdff9440c2f9d859653f25e
EOF
: > superfork/evidence/candidate-probes.tsv
while read -r name source; do
  git fetch --no-tags --filter=blob:none upstream "$source"
  git diff "$upstream...$source" > "$review/$name.patch"
  git diff --stat "$upstream...$source" > "$review/$name.stat"
  git log --format='%H %s' "$upstream..$source" > "$review/$name.commits"
  git cherry "$baseline" "$source" > "superfork/evidence/cherry-$name.txt"
  mapfile -t commits < <(git rev-list --reverse --no-merges "$upstream..$source")
  count=${#commits[@]}
  if (( count == 0 )); then
    printf '%s\t%s\talready-ancestor\t0\n' "$name" "$source" >> superfork/evidence/candidate-probes.tsv
    continue
  fi
  if (( count > 30 )); then
    printf '%s\t%s\tlarge-stack-not-probed\t%s\n' "$name" "$source" "$count" >> superfork/evidence/candidate-probes.tsv
    continue
  fi
  probe="$review/probe-$name"
  git worktree add --detach "$probe" "$baseline"
  if git -C "$probe" cherry-pick -x "${commits[@]}" > "$review/$name.probe-log" 2>&1; then
    printf '%s\t%s\tclean-unverified\t%s\n' "$name" "$source" "$count" >> superfork/evidence/candidate-probes.tsv
    git -C "$probe" diff "$baseline" HEAD > "$review/$name.applied.patch"
  else
    printf '%s\t%s\tconflict\t%s\n' "$name" "$source" "$count" >> superfork/evidence/candidate-probes.tsv
    git -C "$probe" diff --cc > "$review/$name.conflicts.patch"
    git -C "$probe" diff --name-only --diff-filter=U > "$review/$name.conflicts.txt"
    while read -r path; do
      mkdir -p "$review/conflicts/$name/$(dirname "$path")"
      cp "$probe/$path" "$review/conflicts/$name/$path"
    done < "$review/$name.conflicts.txt"
    git -C "$probe" cherry-pick --abort
  fi
  git worktree remove "$probe"
done < "$review/candidates.tsv"
cat superfork/evidence/baseline.tsv superfork/evidence/candidate-probes.tsv
git archive --format=tar.gz --prefix=codex-baseline/ "$baseline" > "$review/baseline.tar.gz"
cp -R superfork/evidence "$review/evidence"
git add superfork/evidence
git commit -m 'docs(superfork): record preserved baseline and candidate compatibility probes'
git push origin "HEAD:refs/heads/$branch"
