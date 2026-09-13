# Assistant conversations

Assistant replaces the Queries sidebar label. Conversations are an explicitly
connected, opt-in surface; Previous queries keeps the existing bounded Voice
Query history without migrating, deleting, or silently changing its retention.

The workspace supports typed messages, microphone capture, streaming answers,
Stop/Escape, local conversation deletion, and reopening retained conversations.
It reuses the native QueryCoordinator and capture/ASR owner. Typed asks do not
open the quick-query popover, capture app context, copy the clipboard, or paste.
Leaving the workspace or closing the main window cancels its owned active query;
it never starts background tasks. Completed conversations remain available.

## Dictate, review, then send

The Assistant microphone dictates a **local composer draft**, not a request to
Pi. Live Core ML Parakeet transcription appears directly in the text box; it does not
open the floating transcript or quick-query popover. Other existing ASR models
provide the final transcript if live partials are unavailable.
Live previews use the existing bounded 20-second trailing decode window for
long speech; Finish replaces that provisional preview with the full transcript.

Finish speaking stops recording and finalizes local ASR. The composer becomes
editable, and only an explicit Enter or Send starts the Pi bridge. Dictation
appends to any text already in the composer; Stop or Escape cancels the speech
and restores that typed prefix. Drafts are ephemeral and are not added to chat
history, Pi sessions, logs, or the clipboard. Leaving the page cancels capture.

This uses main-only `start_assistant_dictation` / `finish_assistant_dictation`
commands and pass-scoped `assistant-draft-state` / `assistant-draft-partial`
events. A draft pass is refused at provider-child spawn and legacy finish
boundaries. The separate quick Voice Query shortcut retains its original
record-and-submit behavior.

## Setup and data boundaries

Configure a Custom executable that supports the Pi Personal Assistant V1 bridge
in Voice Query settings. Connect Pi Assistant explains persistence before consent:
the Mac stores a private conversation mirror and the configured Pi host stores
native model sessions. The model provider may receive question/tool content.
Connecting grants no new tools; existing Pi policy remains authoritative.

Consent is bound to the canonical executable and fixed arguments. Each saved
conversation is also bound to that connection. Reconfiguring the provider cannot
silently send an old conversation to another bridge. Restore the original
connection or create a new conversation. Disconnect revokes the Assistant
connection without deleting data or changing the legacy Voice Query settings.

The Mac store is Rust-owned `assistant/conversations-v1.json` beneath its app-data
directory, with mode 0600 in a mode-0700 directory. It is bounded to 100
conversations, 200 messages per conversation and 16 MiB total. It is not browser
localStorage or diagnostic telemetry. On restart unfinished messages become
interrupted and are never automatically resent. Delete from this Mac does not
erase Pi's private remote session; the confirmation says so explicitly.

Only main-window commands can read/mutate the store. Content updates are targeted
to that window. No conversation text belongs in diagnostic logs. Rendered
Markdown is sanitized; remote images and clickable model-generated links are
disabled in the workspace.

## Bridge and context continuity

The Custom executable still receives one literal final argument, never a shell
command. Assistant messages use `MURMUR_ASSISTANT_V1\n` followed by JSON with
opaque UUID `conversationId`, `requestId`, and `message`. Completed popover
promotion additionally seeds exactly one user/assistant pair. Native bounds are
32 KiB per new message and 64 KiB for the complete encoded frame.

Pi Personal PR #7 supplies this opt-in protocol. It resumes a native Pi JSONL
session across processes while reapplying current tool/provider restrictions.
Only settled successful turns advance its committed context. Kernel locking
prevents simultaneous writers and completed request IDs replay without executing
again. A retried attempt that invalidates already-streamed text fails rather than
committing concatenated partial replies. Raw one-shot questions remain backward
compatible. Deploy the reviewed bridge before enabling this workspace.

## Quick query handoff

A completed Custom-provider popover offers Open in Assistant. Save and open
explicitly retains the displayed exchange and opens a conversation in the main
window; the next message seeds that exchange into a new Pi conversation once.
It does not recover arbitrary older popover sessions or unseen tool transcripts.
Subsequent workspace follow-ups use the same conversation ID. The source must
match the explicitly connected Pi bridge; generic providers are not silently
treated as Pi.

## Verification and release

Frontend tests cover explicit setup, composer behavior, safe Markdown, deletion
wording, microphone phase ordering, listener failure, and cancellation during
pending IPC. Native tests cover private store/restart recovery, bounds, exact
pass ownership, bridge binding and reconnect races. Pi tests cover actual native
session continuation against an offline model fixture, idempotency and cleanup.

Native UI acceptance additionally checks typed follow-up, reopen/restart, voice,
Stop/Escape and popover handoff in the built app. Release and deployment receipts
must distinguish these observations from unit tests and CI. This change requires
a Murmur application release and a Pi bridge update, but no new AgentOS or CPA
deployment. Household actions remain out of scope.
