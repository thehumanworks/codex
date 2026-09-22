# Codex Harness Superfork

Built from OpenAI Codex main on 2026-09-22, with ranked unmerged upstream feature branches evaluated for harness value and bleeding-edge novelty.

Ranking:
1. agent-communication-v2 — first-class structured inter-agent communication lifecycle/metadata; highest direct orchestration value.
2. app-server-lossless-events — lossless in-process event delivery and shutdown draining; strong durability/event-driven harness value.
3. async-command-hooks — detached asynchronous hooks; strong non-blocking extensibility value.
4. agent-hooks — agent/prompt-driven hooks; powerful programmable harness lifecycle extension.
5. external-agent-memory-import — imports external agent memory; strong portability/persistent-agent value, but broadest/riskiest patch.

The exact merge outcome is in integration-results.tsv. Candidates that conflict with current main are deliberately skipped rather than weakening current upstream behavior.
