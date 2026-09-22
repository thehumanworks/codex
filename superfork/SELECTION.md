# Candidate selection and implementation contract

Final upstream scope: S1 and S2. Actual verification and promotion status are recorded in `evidence/`. The optional CLI preset is a fork-specific configuration launcher, not a third unreleased upstream implementation. Baseline: `4631a92ae60616c02d5c6c58ef3275d5c01c5ba0`, containing fork `e3bd229c57231a4cf985cfd3100ff982dcd8cb7d` and upstream `d3e584093222018fc27919403aea149ccfd3bb38`.

## Discovery coverage

The committed `evidence/upstream-branch-heads.tsv` inventories 4,671 advertised upstream heads. `evidence/open-pr-sample.json` contains 169 open PRs, fetched in two pages, with the final page reached and no API errors. This is a point-in-time inventory, **not an exhaustive review of 4,671 branch implementations or every contributor repository**. Eight full source diffs and ancestry/patch-equivalence sets were downloaded and probed in isolated worktrees; the tool-free PR was subsequently inspected directly. Contributor PR #9584 was inspected as an additional lead. Private, deleted, unadvertised, and unrelated contributor branches were not enumerated.

The exact source heads, unique commits, and initial textual conflicts are recorded in `evidence/`. All eight initial probes conflicted. A conflict is neither evidence that a feature is missing nor, by itself, a reason to reject it. Semantic review below distinguishes duplicates, regressions, and worthwhile ports. Recency and popularity did not determine scores.

## Ranking

Scores are engineering judgments, not measured productivity gains. Harness value (H) and experimental novelty (N) are each 1–5; total = 0.5H + 0.5N. Risk and confidence are separate. A high-scoring duplicate does not qualify for integration.

| Capability | H | N | Total | Integration risk | Evidence confidence | Decision |
| --- | ---: | ---: | ---: | --- | --- | --- |
| Agent/prompt hooks | 5 | 5 | 5.0 | High | Medium | Exclude this combination: broad incompatible hook-runtime and permission/config port |
| Injectable in-process thread storage | 5 | 4 | 4.5 | Medium | High on missingness; runtime verification pending | Include, first checkpoint |
| Lossless in-process event delivery and accepted-message shutdown drain | 5 | 4 | 4.5 | Medium–high | High on missingness; runtime verification pending | Include, second checkpoint |
| Detached asynchronous command hooks | 5 | 4 | 4.5 | High if old implementation restored | High | Exclude: capability already in baseline |
| External-agent memory import | 5 | 4 | 4.5 | High if old migration protocol restored | High | Exclude: capability already in baseline |
| Reconcile running-thread history on resume | 5 | 3 | 4.0 | High | Medium | Exclude: broad history/compaction reconstruction port; lower compatibility |
| Tool-free helper threads | 4 | 4 | 4.0 | Medium–high | Current-core compatibility reviewed | Exclude: broader MCP/context lifecycle port required |
| Exact tool timing metadata | 4 | 3 | 3.5 | Medium–high | Medium | Exclude: lower priority and cross-transport metadata changes |
| Preserve child environments on reload | 5 | 2 | 3.5 | Medium | High on relevant baseline behavior | Exclude: baseline already preserves explicit/inherited selections |
| Agent communication telemetry | 4 | 3 | 3.5 | High privacy regression in old branch | High | Exclude: existing communication telemetry; old branch restores plaintext logging |
| Contributor TUI SIGINT handling | 2 | 1 | 1.5 | Medium against substantially newer TUI | Low on current applicability | Exclude: narrow terminal fix, not a leading novel harness capability |

## Source locks and semantic review

### S1 — Injectable thread storage

Source branch: https://github.com/openai/codex/tree/adamvy/app-server-lossless-events

First source commit: `ec3b1ca18f25386d7224c06a74bab6eeef407c6e` — `[app-server] allow custom in-process thread stores`.

This is a substantive embedding API, independently useful without lossless events. The baseline has `start(InProcessStartArgs)` but no `InProcessStartOptions`, `start_with_options`, or injected `ThreadStore`; `MessageProcessor::new` unconditionally derives its process-scoped store from config. The source introduces explicit dependency injection with default behavior preserved.

Value rationale: a host can own session persistence instead of binding every embedded runtime to its local Codex home. Novelty rationale: experimental storage-backend injection is a new low-level integration surface. Portability benefits are **inferred**; this change does not implement a cloud store, cross-host replication, or durable in-memory storage.

Dependencies/overlap: existing `codex-thread-store` trait and store implementations; no new dependency is needed. September upstream added a separate queue backend after this source was authored. The port must not silently couple an injected store to an unrelated ambient SQLite queue. Default startup and existing auth initialization must remain unchanged.

Acceptance: the injected store receives real typed thread requests; default startup still works; an injected store does not accidentally use the default persistent queue; clean runtime shutdown; existing auth and embedded-runtime tests remain green. Preserve authorship by merging the pinned source commit and committing an explicit semantic port.

### S2 — Lossless bounded in-process events

Same source branch; final pinned head: `16d35f17ee1b30282e8f85e149ef01536e14dcc5`.

Remaining source commits:

- `a6a349f2f095e8299aeca4bb36e91b12b880f699` — lossless event delivery.
- `68815edc1842771cba1daf69b75febe064c44d7a` — drain accepted messages during shutdown.
- `16d35f17ee1b30282e8f85e149ef01536e14dcc5` — shutdown argument annotations.

The baseline explicitly permits notification drops under saturation and has no delivery-mode option or two-phase shutdown API. Some critical notifications are already guaranteed; **those existing guarantees are not counted as a new feature**. The missing capability is opt-in losslessness for all in-process notifications, with bounded backpressure and a shutdown sequence that lets the consumer continue draining.

Value rationale: event-driven coordinators should not infer state from polling because their embedded event stream silently dropped intermediate events. Novelty rationale: low-level experimental transport/lifecycle control, not a cosmetic flag. This is in-process delivery, **not disk durability, exactly-once processing, or a websocket/stdio guarantee**.

Dependencies/risks: S1 options API; shared request/event queues; source predates boxed protocol events, typed response envelopes, request cancellation, additional required notifications, explicit outbound-router shutdown, and a 35-second analytics-aware acknowledgement budget. The port must retain all of these newer semantics. Lossless consumers must drain events concurrently; stopped consumers can backpressure the producer. Shutdown must remain bounded and must not wait on a detached sender forever.

Acceptance: saturation preserves notification order with no `Lagged` event in lossless mode; best-effort behavior remains compatible; accepted messages complete/drain during two-phase shutdown; request cancellation and overload paths retain their contracts; shutdown is bounded; combined store injection and lossless event consumption work together. Source-head CI includes successful required checks from June 2026, but those results do not validate this newer combined baseline.

### Excluded after deeper review — Tool-free helper threads

Source PR: https://github.com/openai/codex/pull/31922

Branch: `river/thread-title-no-tools`; exact head/only commit: `489d5b504a40874d5382eea85d67cbe090f0771b`. Open draft when inspected; no human approval observed.

Neither `Feature::ToolFree` nor `tool_free` exists in the combined baseline. The source adds an opt-in feature that skips MCP startup/refresh, skill/plugin enumeration, and tool registration. H=4 because it makes model-only helper work possible without irrelevant tool startup; N=4 because it introduces a distinct experimental execution mode. Claims about latency improvement are speculative until benchmarked.

The PR reports scoped tests and a manual control, but its actual diff adds **no integration tests**. Four automated review findings were read: initial context can still advertise skills/plugins; startup still warms plugin/skill state; per-thread MCP status collection can start configured servers; and outbound-request integration coverage is absent. The port must address these findings rather than treating the source as ready-made.

Acceptance required for a future port, not implemented here: model-only turns send zero model-visible tools and no installed skill/plugin guidance; normal sessions retain tools and guidance; configured MCP processes do not start on tool-free startup, refresh, or per-thread status; configuration remains opt-in/default-off and the generated schema is updated. Add actual core-suite integration tests using existing `TestCodex` fixtures. This mode is **not a security sandbox** and must not be described as suppressing every trusted hook or host-side action.

## Excluded source locks and reasons

- Agent/prompt hooks: https://github.com/openai/codex/tree/abhinav/agent-hooks at `bc4b55c09c0c4beccc1cac4825f21856272edac6`. Six commits and 50 changed files introduce prompt/agent runtimes, client schemas, permissions/configuration paths, and TUI surfaces. Current async/MCP hook dispatch has diverged. A coherent port needs a separate review of nested-agent permissions, feature inheritance, timeouts, and model-context budgets; combining that rewrite with the selected runtime/state work is materially higher risk. High potential value does not override compatibility.
- Async hooks: https://github.com/openai/codex/tree/abhinav/async-command-hooks at `218e45801afa0ab3a4c8bc6334d6bbf2d1e33925`. Baseline already dispatches configured async command hooks and tracks completions/shutdown in `hooks/src/registry.rs` and `hooks/src/engine/`. The old `async_output` design is not evidence of a missing async capability.
- Memory import: https://github.com/openai/codex/tree/dev/charlesgong/import-external-agent-memory at `923df3e976a315019f4e2530070b8fd102d38e30`. Baseline already contains external memory discovery/import and newer migration/event/TUI paths; reapplying this older protocol is not a new memory feature.
- Resume reconciliation: https://github.com/openai/codex/pull/30866 at `5e37df08c5611afbefdff9440c2f9d859653f25e`. Actual patch spans thread history, rollout reconstruction, compaction, and storage. Initial probe conflicts across 15 paths. Not claimed fully equivalent or absent solely by ancestry: excluded on compatibility/risk, not as proven merged.
- Exact tool timing: https://github.com/openai/codex/pull/29065 at `870f7f94dd4f3887a58f21f880757d1988a5de5f`. Draft, three commits. Adds per-call timing carried through HTTP/websocket Responses metadata; includes an unrelated alpha workspace-version change. Useful observability, but lower novelty and broader transport adaptation than the selected set. No timing improvement is claimed without measurement.
- Child environments: https://github.com/openai/codex/pull/31116 at `ab86f230223ccdd189f17fd68c61addfbce6ce87`. Current `resume_thread_with_history_with_source` retains explicit `environment_selections`, falling back to inherited snapshots rather than unconditionally replacing them with manager defaults. The source's old entry point is not the current architecture.
- Communication telemetry: https://github.com/openai/codex/tree/codex/agent-communication-v2 at `a0ce4abc4069f91e3296ada835cbaf95a6ff0b75`. Baseline already has first-class internal communication logging. Critically, its current content helper records encrypted content or `[plaintext]`, whereas this old source exposes raw plaintext communication content to trace/log/feedback paths. Do not regress that privacy behavior for an apparent ahead-of-main feature.
- Contributor SIGINT lead: https://github.com/openai/codex/pull/9584 from `zerone0x/codex`, head `ae2f0e4cb498fff8a20f5917e4f666e4a473cee0`. Narrow older TUI fix; source reports old unit/build checks but no manual WSL validation. Not selected over substantive storage/execution capabilities; no claim of exhaustive contributor review.

## Incremental integration and verification order

1. Merge S1 with a reviewed port preserving current auth/config initialization and queue-store coherence. Run its behavioral tests plus embedded-runtime regressions before S2.
2. Merge the remaining S2 source history with current cancellation/notification/shutdown semantics preserved. Test saturation, ordering, accepted-message drain, timeouts, and interaction with S1 before combined verification.
3. Exclude tool-free helpers from this combination. Current MCP runtime projections, extension-context contributors and tool-registry planning differ materially from the source. Closing the reviewed gaps requires a separate broader lifecycle port, not a partial flag merely to meet a quota.
4. Run required scoped formatting/lint/build/test checks, the complete suite because core changes, CLI smoke checks, and the existing fork's synthetic environment-auth acceptance runner. Record exact commands, exit statuses, test counts, and any skipped/blocked platform checks.
5. Commit evidence and build/run/enable instructions. Only then promote the verified integration to `main` without force and re-read the remote SHA. If checks fail, preserve the pushed work and report the exact gate; do not count a clean merge or scheduled workflow as success.
