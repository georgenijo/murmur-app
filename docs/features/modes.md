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
