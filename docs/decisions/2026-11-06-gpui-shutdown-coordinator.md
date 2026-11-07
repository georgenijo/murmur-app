# Decision: coordinated bounded shutdown before native termination

Status: accepted design, implementation in draft GPUI stack layer 07 (#992),
part of #1009. The native Tauri quit matrix and independent review are gates
before this draft can become mergeable.

Use a Tauri-free reducer, one asynchronous adapter, and owned licensed tao
0.35.3 source. Keep the existing delegate; add typed termination admission and
a common-mode CF terminate relay. Do not swizzle or replace the delegate.
Main-thread dispatch must continue while NSTerminateLater holds termination.

Close recording/helper admission permanently at the first request. Bound capture,
helpers and writers by absolute +5/+7/+10-second deadlines. Preserve original
recording generation/configuration, exact helper ownership, durable meeting rows
and spools, and pending writer publications. Never claim stopped helpers or
durable data from a timed-out in-memory waiter. Suppress restart unless all
required receipts prove a drain. Quit/system termination wins before terminal
commit; only the shutdown adapter can request_restart.

Approve private completion-pending.json under the existing retention contract:
no transcript for retention-off work, bounded content, replay once, Clear/Delete
and caps covering pending copies, no content in diagnostics, and no eviction of
uncommitted work. Use immutable publication envelopes with recovery before later
mutations. Do not add persistent dictation audio. Report unfinished utterances
at forced timeout. Forced/uncontrolled exits depend on previously durable data;
no universal lossless or crash-cleanup guarantee is made.

Keep the process IPC compatibility plugin until native updater ownership (#990).
Catchable termination signals use a self-pipe; RunEvent::Exit records only the
terminal receipt. See [implementation and native matrix](../draft/gpui-shutdown-coordinator.md).
