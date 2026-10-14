# Meeting Auto-Export

Meeting auto-export hands each completed meeting to a local folder as Markdown,
so a separate local process (for example a folder watcher) can pick it up. It
is off by default. Enable **Settings → Meetings → Auto-Export Completed
Meetings** and optionally choose a folder; the default is `~/Meetings/inbox`.

## Behavior

- The setting is resolved once when a meeting starts. Changing it affects the
  next meeting.
- Only sessions that finish with status Complete are exported. Failed and
  interrupted sessions are skipped.
- When remote speaker labels are enabled, export waits for that pass to end
  (success, failure, cancellation, or timeout) so speaker labels are included.
  Otherwise it runs immediately after capture finishes.
- Export runs on its own thread and never blocks or fails capture. Logs contain
  only stable event and error codes (`meeting.auto_export_written`,
  `meeting.auto_export_skipped`, `meeting.auto_export_failed`), never paths,
  titles, session IDs, or transcript text.
- Each session exports once. A content-free marker named by session ID is kept
  under the meeting store's `auto-export/` directory, so moving the file out of
  the folder does not cause a second write.
- The file is written to a hidden temporary sibling and renamed into place, so a
  watcher never sees a partial `.md` file.
- Nothing is sent over the network.

## File format

The file name is `YYYY-MM-DD HHmm <title>.md`, using the local start time and a
sanitized title (`Untitled` when none is known). A name already used by an
unrelated file gets a ` (2)`, ` (3)`, … suffix.

The body is the existing Markdown review export, prefixed with YAML front
matter:

```yaml
---
murmur_session_id: "3f8a2b1c-0d4e-4f6a-9b7c-1e2d3c4b5a69"
started_at: "2026-09-30T14:05:00-04:00"
ended_at: "2026-09-30T14:35:34-04:00"
duration_seconds: 1834
title: "Weekly sync"
title_source: calendar
attendees:
  - "Alex"
  - "Taylor"
calendar_event_title: "Weekly sync"
calendar_event_attendees:
  - "Alex"
  - "Taylor"
---
```

`title_source` is `manual`, `calendar`, `generated`, or `null`.

## Calendar fallback

If the session has no title and Calendar access is already granted, Murmur looks
up events overlapping the meeting window with the existing local EventKit query.
The event with the largest overlap fills `calendar_event_title` and
`calendar_event_attendees` and names the file. It never prompts for access and
does not change the stored session title or attendees.
