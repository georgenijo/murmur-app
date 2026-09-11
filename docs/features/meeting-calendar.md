# Calendar naming

**Name from calendar** finds events that overlap a finished meeting and offers
a picker in the review workspace. The user selects an event, reviews its title
and attendees, and chooses Apply. One result still requires selection and
confirmation. Stopping a meeting exposes the action without querying Calendar.

## Permission

Calendar access is optional. The first explicit naming action can request it;
the setup assistant also has a skippable Calendar step. Onboarding reads only
authorization status and never fetches events. Denied access offers Calendars
settings and an app-scoped reset. Restricted or unavailable access leaves manual
naming available.

macOS 14 and later require full event authorization to read Calendar data.
Murmur declares `NSCalendarsFullAccessUsageDescription`, grants the signed host
`com.apple.security.personal-information.calendars`, and requests access through
EventKit. Capture and local-LLM helpers do not receive this entitlement.
Its adapter exposes only reads and never calls event save,
remove, or commit methods. The permission request starts on the main thread and
retains its native store until completion. Passive status reads call the native
authorization method without constructing an event store.

## Lookup and confirmation

Rust derives the lookup interval from the stored session start and end. Active
meetings cannot query Calendar. The overlap comparison excludes events ending
exactly at the meeting start or starting exactly at its end. Cancelled events
are excluded. Missing attendee names are omitted; Murmur does not query Contacts
or retain attendee email addresses. An event without a title appears as
**Untitled calendar event**.

The native query runs off the main thread and returns bounded plain values.
The picker receives a title, attendee names, start/end times, and a selection
token. The token binds the session, lookup interval, occurrence identity, and
displayed metadata. Apply repeats the same query and requires an exact token
match. A changed or removed event requires a fresh selection and leaves existing
meeting details untouched.

Only Apply writes to the existing metadata transaction. It stores the title,
attendees, and `calendar` title source, and refreshes title search. Transcript
evidence, generated summaries, and reviewed documents remain intact. A later
manual rename sets the source to `manual`.

## Bounds and privacy

- Session windows span at most seven days. A session whose recorded start and
  end share a millisecond uses a one-millisecond interval.
- A query returns at most 100 events. Titles and attendee names allow 200
  characters each, with at most 100 participants per event. Oversized data fails
  with a manual-naming fallback instead of silent truncation.
- The caller waits at most fifteen seconds for a query. Event enumeration also
  checks its deadline. A native call that outlasts that deadline retains the
  operation slot until it returns, preventing queued or concurrent queries.
- Only one permission request, reset, or event query can own the Calendar slot.
  A second operation returns a content-free busy error.
- Event objects, identifiers, and lookup results are never cached or persisted.
  The picker holds its displayed result only while open. Closing it or changing
  sessions discards that state.
- No background lookup, event annotation, recording, or model warm-up occurs.
  Calendar data is never logged or included in diagnostic output. Calendar
  commands return stable errors without native descriptions, and structured log
  sanitization strips Calendar payload fields in every build and stream.

## Verification

Rust tests cover strict overlap, bounded windows and text, permission mapping,
tokens bound to displayed data, stale application, metadata persistence, and
log redaction. Frontend tests cover explicit selection, denied access, manual
fallback, and optional onboarding. The native gate exercises signed-app Calendar
permission attribution and lookup → select → Apply through the review workspace.
