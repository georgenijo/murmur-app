# Murmur Modes

## Model

A Mode is a reusable, local policy referenced by an app profile and resolved
once at recording start. It can select an existing writing style, pipeline
stage overrides, vocabulary and project-context policy, model/language policy,
and auto-paste behavior. Profile fine-tuning remains higher precedence, and an
in-flight recording never re-reads Modes or settings.

Murmur ships seven code-owned Modes with stable IDs: Everyday, Messages,
Email, Notes, Technical, Terminal, and Verbatim. User Modes are stored in the
versioned Settings document; built-ins are not duplicated into user data.

Resolution order is global settings → selected Mode → matching profile
fine-tuning → one-session overrides. A legacy profile with no `modeId` follows
the pre-Mode resolver path unchanged. An unknown Mode ID, a spoofed built-in,
or a Mode containing an invalid model/language reference fails closed: no
auto-paste, project context, technical vocabulary, or transforming stages are
enabled for that binding.

## Privacy

Modes contain policy and stable identifiers, never dictated text, selected
text, clipboard content, vocabulary terms, project index contents, or audio.
Project context still requires configured roots on the bound profile, and the
resolved snapshot retains the existing deny-by-default screen/selection rules.
Telemetry may carry only stable Mode identity or content-free outcome codes;
Mode names and user content are not logged.

## Manager and bindings

Settings shows the seven built-ins as read-only templates and stores only
custom Modes. Custom Modes can be created, duplicated, renamed, edited,
enabled or disabled, and deleted. One Mode can be bound to any number of
existing app profiles; deleting a Mode clears those references rather than
leaving an unsafe dangling binding.

Each Mode shows a compact effective-policy summary. Its before/after tester is
a pure in-window preview: sample text remains in React memory and the tester
does not invoke clipboard, paste, text injection, or target-app commands.

## Native activation

The overlay and tray display the currently resolved Mode without activating or
focusing Murmur. Clicking the Mode control cycles enabled built-ins and custom
Modes. Outside an app binding this updates the durable last-manual Mode; inside
a bound app it creates a memory-only override tied to that exact bundle ID.
Leaving the app clears the override and restores the last manual Mode.

At recording acceptance, the existing frontmost process snapshot resolves
temporary override → exact browser-site binding → app binding → last manual
Mode. The result is immutable for that recording. The runtime indicator exposes
only `temporary`, `site_binding`, `app_binding`, or `manual`; it never includes a
host or URL.

## Browser-site activation

Browser-site lookup is globally off by default. When the user enables it,
Murmur accepts rules only for an allow-listed browser bundle ID and a validated
exact host such as `github.com`. Subdomains do not inherit a parent rule. Rules
are evaluated in saved order; disabled, malformed, duplicate, unsupported, or
unavailable rules cannot activate a Mode.

The native boundary re-verifies the exact frontmost browser process, reads only
its Accessibility `AXDocument` URL, accepts only HTTP or HTTPS, and immediately
reduces the value to a lowercase host. The full URL, path, query, fragment,
window title, page text, selection, clipboard, and browsing history are never
stored, returned in runtime status, or logged. The Settings test waits up to
five seconds after the click so the user can switch from Murmur to the browser
being tested; it returns only the allowed browser bundle ID and exact host.
Turning the global switch off prevents both polling and recording-start lookup.

Site rules can bind built-in or enabled custom Modes. Deleting a custom Mode
also removes its site rules. A site change inside the same browser restores the
app binding or manual Mode on the next observation; a temporary override keeps
its existing bundle-scoped semantics and remains highest precedence.

## One recording only

Choose **Next recording: <Mode>** from the main window's ⌘K palette or the
tray's **Next recording** submenu. Both list enabled built-in and custom Modes,
with an explicit clear action while an override is pending. This selection
uses the resolver's existing highest-precedence `SessionOverrides` input and
does not change app/site bindings, the last manual Mode, or durable settings.

The expanded overlay shows **Next: <Mode>** until the selection is consumed.
Edits to a pending custom Mode apply at acceptance; disabling or deleting it
clears the pending selection. At native recording acceptance the Mode policy and a selection token are copied
into the immutable context. Focus, settings, and later Mode selections cannot
alter that recording. Cancellation (including while processing), failed starts,
and phantom captures below 0.3 seconds retain the pending selection. A real
recording's pipeline clears only its matching token before returning Idle,
including no-speech or inference failure; a newer selection remains pending.
The following recording returns to the normal bound/manual Mode.

Only native dictation claims the override. File/audio-processing paths, Voice
Query, transforms, and meetings do not consume it. All pending state disappears
when Murmur exits. Local project context remains limited to configured roots
and an available memory-only index.
