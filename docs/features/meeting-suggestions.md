# Meeting suggestions

Meeting suggestions is an optional Meetings setting that defaults off. When a
Calendar event with a supported video-call link is in progress and a meeting app
or browser is frontmost, the overlay offers **Start Notetaker** with the event
title. Accept starts the normal meeting flow. Dismiss does nothing else.

## Consent and eligibility

Enabling requires Calendar permission. The frontend requests permission only
after an explicit click; the background coordinator never opens a permission
dialog. Turning the setting off revokes pending queries, clears prompts and
event data, and releases the native Calendar observer store. No event query
starts while off. An already executing native call may finish, but its revoked
result is discarded.

Eligibility uses the native frontmost application's bundle ID and the event's
start/end times. It reads no window title, screen, browser tab, or browser URL.
Supported applications include Zoom, Teams, Slack, Webex, FaceTime, Safari,
Chrome, Chromium, Firefox, Edge, Arc, and Brave. A browser plus a current event
is a suggestion heuristic; Murmur does not determine which browser tab is open.

The native adapter recognizes parsed Zoom, Teams, Google Meet, Webex, FaceTime,
and Slack huddle links in the event URL, location, or notes. Host matching
rejects misleading suffixes and credential-bearing URLs. URL text is bounded
to 2,048 UTF-16 units; location and notes each allow at most 16 KiB. Those raw
strings never leave the native query. All-day and cancelled events are excluded.

## Scheduling and ownership

The enabled coordinator holds at most 100 upcoming video-call events within a
24-hour horizon. Titles and attendee lists use the existing Calendar naming
bounds. It schedules one deadline for the next event start, event end, or horizon
expiry. App activation, busy-state transitions, system wake, clock/time-zone
changes, and EventKit store changes can wake it earlier. App activation reuses
the snapshot. Calendar changes invalidate it and clear a displayed prompt.

A notification-only EventKit store remains alive on the main thread while the
feature is enabled. Query stores and their event objects remain scoped to each
bounded worker. Snapshot reads are separated by at least thirty seconds to
coalesce notification bursts. There is no repeating short-interval poll.

The naming picker and suggestion worker share one Calendar operation slot.
If naming owns that slot, suggestions retry after the operation releases it and
the minimum gap passes. Other query failures clear the snapshot and wait for a
new activation, activity change, Calendar notification, or horizon deadline;
the worker's own completion does not create a repeated error retry loop.

Dictation, every transform phase, active query capture or answer generation,
meeting capture/recovery/summary, file transcription, microphone previews,
benchmarks, shared backend changes, model loading/warming/unloading, speaker
analysis, and disabled-app state suppress prompts. Model lifecycle and benchmark
activity use nonblocking status reads; contention counts as busy. A
suppressed event can still prompt after Murmur becomes idle if it remains current.

Prompt publication does not open a tap, claim audio, or prepare a model. Accept
sends only the opaque prompt token to the main window. `start_meeting` acquires
the normal recording transition lock, samples the native frontmost bundle ID
again, and rechecks permission, expiry, token ownership, and every busy owner.
It then consumes the token, preserves the normal capture settings, and saves
the selected title and attendees with the Calendar source before starting capture.
An active microphone preview is refused instead of being stopped automatically.

## Deduplication and privacy

An occurrence is marked prompted when its prompt is published. Dismissal,
acceptance, switching apps, and off/on toggles cannot prompt that occurrence
again in the same app runtime. The key hashes the event identity and original
occurrence date, so later recurring occurrences remain eligible and a title
edit does not create another prompt. These hashes have seven-day expiry and a
2,048-entry cap; a full set refuses additional prompts rather than evicting
current deduplication records.

Deduplication is memory-only. Restarting Murmur clears it, and an EventKit sync
that changes an event identifier may present that event as a new occurrence.
No calendar identifier, link, notes, snapshot, or prompt token is stored in the
meeting database. Confirmed title and attendees use the same local metadata
storage as Calendar naming. Calendar and suggestion payload fields are stripped
from structured logs in all builds and streams.

## Verification

Pure coordinator tests use synthetic events and query counters to cover off
state, revoked workers, unsupported apps, occurrence deduplication, busy phases,
expired/stale acceptance, deadlines, and contention with on-demand naming.
Native verification checks signed-app permission, a current Calendar call,
frontmost-app changes, one prompt, dismiss, and Accept through the normal
meeting flow. No real calendar data is needed for automated tests.
