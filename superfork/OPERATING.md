# Operating the edge harness

Build the fork and companion Code Mode host as described in [BUILDING.md](BUILDING.md). The [edge preset](EDGE.md) activates existing upstream coordination, message-board, memory, goals, hooks and metrics features. These operating patterns are not additional upstream ports or claims about model-dependent task quality.

## Interactive goals

Start an interactive session from your project directory, using an absolute path to the launcher when the fork checkout is elsewhere:

```sh
python3 /path/to/codex/superfork/edge.py --
```

Inside the TUI, the current goal surface is:

```text
/goal Implement the scoped change, run its acceptance checks, and report remaining failures.
/goal pause
/goal resume
/goal edit
/goal clear
```

Use `pause` to stop goal continuation while preserving the goal, `resume` to continue it, and `clear` to remove it. This is the existing Codex goal mechanism, not a separately implemented independent verifier agent. The parser contract is defined in `codex-rs/tui/src/goal_display.rs` and exercised by the goal tests under `codex-rs/tui/src/chatwidget/tests/`.

To reopen a saved session with the same preset:

```sh
python3 /path/to/codex/superfork/edge.py -- resume --last
```

The normal model, authentication, approval and sandbox configuration still apply. The launcher does not install hook commands or rewrite persistent settings.

## Delegate without creating idle-message deadlocks

A useful task instruction is:

> Act as coordinator. Split genuinely independent work between implementation and review agents. Give each task an observable acceptance criterion and avoid overlapping file ownership. Use the shared message board for decisions and evidence. Use direct follow-up tasks to assign work to idle agents, and event-driven waits rather than repeated status polling. Integrate only verified changes and report blockers explicitly.

This is an example prompt, not a guarantee that every model or managed configuration will expose or use every tool. The actual registered multi-agent-v2 tools distinguish three operations:

| Operation | Current source behavior | Consequence |
| --- | --- | --- |
| `send_message` | Uses `MessageDeliveryMode::QueueOnly`. | Send information to another agent; do not rely on this alone to start an idle agent's next turn. |
| `followup_task` | Uses `MessageDeliveryMode::TriggerTurn`. | Use an explicit follow-up task when assigning new work that must start another turn. |
| `wait_agent` | Subscribes to input-queue activity with a bounded timeout. | Wait for activity instead of repeatedly listing or polling agent state. |

These implementations are in `codex-rs/core/src/tools/handlers/multi_agents_v2/`. Their timeout bounds, session policies and available tool surfaces remain governed by the current runtime.

The message board supports channels, threaded posts, bounded reads/search, and subscriptions. Its tool contracts explicitly say that notices reach currently running turns and **never start idle agents**; they are not a later-delivery notification backlog. Posts and explicit follow-up tasks serve different purposes. Do not arrange a cycle where every worker is idle and waiting for a board notice to start it. See `codex-rs/ext/agent-message-board/src/tools/spec.rs` for the exact contract.

## Embed Codex in an external coordinator

Use the two new host APIs in [README.md](README.md) when building your own coordinator:

- Supply a process-scoped `ThreadStore` to own thread persistence. Choose and validate a durable implementation when survival across host-process death is required; an in-memory store is not that implementation.
- Select `InProcessEventDelivery::Lossless`, continuously consume events, and use the two-phase shutdown API to observe final notifications while accepted requests drain.

Keep one event consumer running concurrently with request producers. A bounded lossless queue intentionally applies backpressure; waiting for a request while refusing to drain a saturated event queue is not a valid consumer design. Treat timeout, closed-consumer and overload errors as explicit failures, not evidence that all work was persisted.

The CLI preset does not silently switch TUI, stdio or websocket transports onto these embedding APIs. There is no new cross-host replication service, durable event replay, or exactly-once delivery claim.

## Recover from a checkpoint

Source, formatting, targeted acceptance, affected-suite and runnable-CLI checkpoints are pushed independently. Look at the checkpoint's committed evidence before using it as a release: `source` means saved source, not verified behavior. The final default branch must contain the promoted integration commit before it counts as delivery.

`superfork/evidence/` records source-tree identities, executed test summaries, retries, skips, failed attempts and native CLI controls. A successful launcher test is not a substitute for runtime acceptance or broader regression tests.
