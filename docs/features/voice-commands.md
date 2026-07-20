# Voice Commands 2.0

Voice Commands are local, deterministic text transformations for live dictation. Built-in commands such as `new line` still run first. User commands are stored in the Rust-owned personal knowledge repository and captured in the immutable recording-start context.

## Command types

- **Text replacement** performs the existing literal, case-insensitive, word-boundary phrase replacement. Migrated legacy `{ phrase, replacement }` pairs become enabled global text replacements without changing their text or ordering. Variable-looking text remains literal.
- **Snippet** inserts a multiline template. Snippets support only `{{date}}`, `{{time}}`, and `{{clipboard}}`.

Date renders as local `YYYY-MM-DD`, time as local 24-hour `HH:mm`, and one timestamp is shared by every date/time variable in one expansion. Unknown or malformed variables are rejected when the command is saved.

## Clipboard permission boundary

`{{clipboard}}` requires an explicit permission on that command. Murmur does not read the clipboard when a command is listed, saved, or captured into a recording context. It reads clipboard text only after the permitted command phrase actually matches. Preview has a separate, off-by-default checkbox before it can perform the read.

Clipboard input remains separate from clipboard-first delivery: final text is still copied once after all transforms, and auto-paste behavior is unchanged. Selected text, surrounding screen text, shell execution, Shortcuts, and computer-control actions are not Voice Command capabilities.

If clipboard text is unavailable or expansion would exceed 65,536 characters, that command occurrence remains unchanged. Logs contain only privacy-safe command type and permission/outcome flags, never phrases, templates, clipboard text, or expanded output.

## Scope and conflicts

Commands are global or scoped to one configured app bundle identifier. The existing frontmost-app resolver samples the app once at recording start; focus or command changes during recording apply only to the next recording.

- Same-phrase commands in one scope are rejected case-insensitively after whitespace normalization.
- Built-in phrases are reserved.
- The same phrase may exist in different app scopes.
- An exact app command overrides its global counterpart in that app; the global command remains active elsewhere.
- Disabled commands never enter the recording snapshot.

Vocabulary aliases and Voice Command phrases remain one conflict domain. Saving either side validates against the other atomically.

## Preview and delivery

Settings can create, test, preview, edit, enable, disable, and delete commands. Preview invokes the real Rust matcher but never writes to the clipboard or triggers paste. Live command expansion remains in the existing ordered pipeline:

```text
cleanup → built-in/user Voice Commands → Smart Correction → Smart Formatting → IDE context → CLI formatting → final delivery
```

Imported-file transcription remains verbatim and does not execute Voice Commands.
