#!/usr/bin/env python3
"""Repair the reproduced accepted-RPC shutdown race on the pinned lossless port.

The processor queues initialized RPCs onto separate Tokio tasks. Dropping its
input channel before those tasks have run lets connection_closed close the RPC
gate and discard accepted work. Drain responses while that channel stays open,
then close the processor, sharing the existing five-second processor budget.
"""

from pathlib import Path

path = Path("codex-rs/app-server/src/in_process.rs")
text = path.read_text()
marker = "// Keep the RPC execution gate open until accepted responses have drained."
if marker in text:
    raise SystemExit("Shutdown drain repair is already applied.")
old = "        client_rx.close();\n        drop(processor_tx);\n        outgoing_message_sender"
assert text.count(old) == 1, "unexpected processor shutdown boundary"
text = text.replace(old, "        client_rx.close();\n        outgoing_message_sender")
start = text.index("        let mut shutdown_result = match timeout(")
end = text.index("        if shutdown_result.is_err()", start)
text = (
    text[:start]
    + """        // Keep the RPC execution gate open until accepted responses have drained.
        // process_client_request enqueues work; returning from it is not completion.
        // Both response draining and processor teardown share this existing budget.
        let mut shutdown_result = match timeout(SHUTDOWN_TIMEOUT, async {
            while !pending_request_responses.is_empty() {
                let Some(message) = writer_rx.recv().await else {
                    return Err(IoError::new(
                        ErrorKind::BrokenPipe, "response writer closed before accepted RPCs drained",
                    ));
                };
                if !route_queued_message(
                    message,
                    &mut pending_request_responses,
                    &event_tx,
                    outgoing_message_sender.as_ref(),
                    event_delivery,
                    DeliveryPhase::Draining,
                ).await {
                    return Err(IoError::new(
                        ErrorKind::BrokenPipe, "event consumer closed while draining accepted RPCs",
                    ));
                }
            }
            drop(processor_tx);
            drain_writer_until_task_finishes(
                &mut processor_handle,
                &mut writer_rx,
                &mut pending_request_responses,
                &event_tx,
                outgoing_message_sender.as_ref(),
                event_delivery,
            ).await
        }).await {
            Ok(result) => result,
            Err(_) => Err(IoError::new(ErrorKind::TimedOut, "request processor drain timed out")),
        };
"""
    + text[end:]
)
assert text.count(marker) == 1
text += '\n#[cfg(test)]\n#[path = "in_process_shutdown_drain_tests.rs"]\nmod shutdown_drain_tests;\n'
path.write_text(text)
print("Repaired accepted-RPC drain ordering; behavioral checks remain required.")
