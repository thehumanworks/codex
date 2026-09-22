# Harness superfork delivery — 2026-09-22

## Status and authority

This is an in-progress integration, not a completed or verified release. Work is confined to `thehumanworks/codex`; no upstream writes, package publication, force-pushes, or replacement of unrelated custom work are permitted. The default branch will advance only after the selected features and combined result have passed verification. A pushed branch, clean cherry-pick, or green compile-only workflow is not completion.

## Pinned starting points

- Fork default branch: `main`, `e3bd229c57231a4cf985cfd3100ff982dcd8cb7d`.
- Upstream main observed at the start of this delivery: `d3e584093222018fc27919403aea149ccfd3bb38`.
- New working branch: `superfork/harness-delivery-2026-09-22`, created directly from the fork default SHA.
- Existing branches preserved unchanged: `superfork/2026-09-22` at `e3bd229c57231a4cf985cfd3100ff982dcd8cb7d`; `superfork/bleeding-edge` at `639d2478cc2e16d6ca715952d2e726a3aecc024e`; `superfork/harness-edge-2026-09-22` at `4b1dc3e5e31e4626fda4551360da8d005fe02d92`.

The previous harness-edge workflow reported success, but its committed `integration-results.tsv` records **all five candidates skipped on conflicts**. Its check was formatting plus `cargo check`, not behavioral tests. Those outcomes are not treated as a delivered feature set or reused as passing feature-test evidence. Its old force-with-lease publication workflow will not be used.

## Execution plan

1. Fetch full ancestry for the fork's main and the pinned upstream main. Identify the common ancestor and enumerate fork-only commits and actual diffs. Merge upstream into the new branch, retaining the fork history and semantically preserving its authentication customizations. Record the resulting combined baseline SHA. Stop on unresolved conflicts rather than silently selecting either side.
2. Inventory advertised upstream branches, including branches without PRs. Inspect a bounded, explicitly reported sample of open PRs and available contributor heads. For each shortlisted source, record its exact SHA, ancestry, patch equivalence, actual diff, CI/review evidence, dependencies, and baseline absence. Do not equate ahead-of-main with missing functionality.
3. Rank harness value and experimental novelty independently, each from 1–5; weighted total = 0.5 × value + 0.5 × novelty. Report integration risk and confidence separately. Prefer event delivery, coordination, persistence/resume, memory/context, extensibility, tools, and observability. Popularity and recency are not value scores.
4. Select the strongest compatible set, aiming for 3–5 substantive capabilities without forcing a quota. Integrate incrementally, preserving source authorship and commit traceability with merges or `git cherry-pick -x`. Verify each candidate before adding another. Reject or cleanly revert unsuitable integrations and document actual reasons.
5. Run the repository's required format, scoped lint, build, and test commands. Use `just test`, not direct `cargo test`. Define observable per-feature behavior and interaction checks, especially for hook lifecycles, shared state/migrations, bounded channels, shutdown, and the preserved auth modes. Run broader checks when shared crates change. This user request authorizes the required complete verification suite. Record unrun or blocked checks as such.
6. Commit rankings, source locks, results, build/run/enable instructions, and material limitations. Verify the final integration tree and commit, then fast-forward/merge the fork's default branch without force. Re-read the remote branch to prove it contains the final commit. Use a PR if protections require one; never claim delivery while required checks or approvals are outstanding.

## Candidate leads — not yet selected

Prior work identified `agent-communication-v2`, `app-server-lossless-events`, `abhinav/async-command-hooks`, `abhinav/agent-hooks`, and external-agent memory import. These are leads only: exact current source state, missingness against the combined baseline, dependencies, and review/test evidence must be independently checked before selection. Smaller persistence, event, context, and observability branches will also be considered.

## Acceptance and promotion gate

- Fork custom commits remain ancestors of the combined baseline and final result; behavior is covered by relevant regression tests.
- The recorded upstream SHA is an ancestor of the combined baseline.
- Selected features have substantive code changes absent from that baseline, pinned source traceability, working configuration/documentation, and executed behavioral acceptance checks.
- Combined checks actually execute tests; a workflow that skips every candidate cannot pass this gate.
- No selected feature is counted as verified solely because its upstream PR has green CI.
- Default-branch promotion is blocked on unresolved regressions, missing required checks, failed permissions, or unresolved protections.
- Final report distinguishes delivered work, failed attempts, pre-existing failures, and environment limits.

## Execution environment

The connected Ares host returned `tunnel_client_not_seen` (HTTP 404). The conversation container has no Rust toolchain and cannot resolve github.com. GitHub repository writes and GitHub Actions are available, so Actions is the execution route. Builds/tests must have observable runner results before promotion; merely scheduling them is not success.

Preparation note: baseline resolution whitespace validation is scoped to the manually resolved auth file so pre-existing upstream snapshot whitespace does not masquerade as an integration regression.
