# Theme Engine Native QA

Validated on macOS 26.5 with the release build on July 28, 2026. The QA build
used a temporary bundle identifier so its local data stayed isolated from the
installed Murmur app; the shipped source and release configuration were
otherwise unchanged.

## Native matrix

| Scenario | Result |
| --- | --- |
| System, forced Light, and forced Dark modes | Passed; content and title-bar treatment changed together |
| Live macOS Light/Dark change while in System mode | Passed in the main and Log Viewer windows |
| Custom accent and maximum contrast | Passed; controls remained legible and the slider reached `+100` |
| Restart persistence and startup paint | Passed; the custom theme was present on the first captured frame after relaunch |
| Reset to Sonic | Passed; inputs returned to `#92dbfe`, `#0b0f11`, `#dbe4e9`, and contrast `0` |
| Inline invalid color | Passed; blur disclosed a linked six-digit-hex error without changing the applied color |
| Valid import | Passed; preview disclosed all accessibility adjustments before Apply |
| Malformed, unsupported-version, and 65,537-byte imports | Passed; each failed closed and preserved the active theme |
| Export | Passed; output was `0600`, authoritative-only JSON with no cache or revision |
| Clipboard safety | Passed; an explicit sentinel survived export unchanged |
| Reduced motion | Passed with the macOS setting enabled; Light/Dark changes remained functional; setting restored afterward |
| Recording overlay | Passed; the non-activating transparent overlay remained dark |
| Real recording | Passed; microphone recording stopped, transcribed locally, appeared in history, and copied to the clipboard |
| Log/event stability | Passed; live mode changes did not create an event storm |

The QA-only bundle was not granted macOS Accessibility permission, so the
global selected-text hold gesture was not exercised. The transform-review
window's deliberate always-dark boundary is covered by its existing UI tests
and the semantic-token regression gate; the native recording overlay supplies
the native transparent-glass proof.

## Review-remediation regression

After addressing the final PR review, the exact amended source was rebuilt as
an isolated ad-hoc-signed production bundle and exercised again with native
Computer Use. Appearance mode changes rendered correctly in forced Light and
Dark; the revised Sonic/Custom reset copy was present; Stop Recording retained
a strong error outline and label during a real recording; that recording
transcribed locally into history; Log Viewer stream chips remained visibly
distinct; and the file-transcription drop zone and Choose Files control
rendered correctly. Permission-reset actions were not invoked because the QA
bundle was intentionally not granted Accessibility access; their failure
presentation and promise handling are covered by focused component tests.

## Evidence

- `native-appearance-forced-light.jpg`
- `native-appearance-forced-dark.jpg`
- `native-system-follow-light.jpg`
- `native-system-sonic-dark.jpg`
- `native-custom-accent-max-contrast.jpg`
- `native-restart-persisted.jpg`
- `native-import-preview.jpg`
- `native-log-viewer-system-light.jpg`
- `native-log-viewer-system-dark.jpg`
- `native-recording-overlay.jpg`
