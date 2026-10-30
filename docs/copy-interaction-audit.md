# Copy and interaction audit — issue #850

The [UI design-system copy contract](features/ui-design-system.md#copy-and-interaction)
adapts Hex `a07f994`, `docs/design-system.md:24–131`. This change finishes the
partial issue/850 work without changing typography tokens, native lifecycle, or
the theme-library implementation.

## Violations found and fixed

- [x] Mixed title case in Settings navigation, search results, section headings,
  recording controls, file actions, updates, setup, discovery, and diagnostics.
  Labels and their callers/tests now use sentence case. macOS menu titles,
  physical key names, branded feature/model/provider names, Calendar,
  Accessibility, System Settings, and Open VSX retain their proper names.
- [x] Three-dot progress strings and missing ellipses on further-choice actions.
  Model installation, microphone startup, file exchange, updates, and setup use
  `…`; open/import/export/editor and destructive choices use it as appropriate.
  References to an action inside prose retain normal sentence punctuation.
- [x] Closing periods on one-phrase row help. Settings/search/Customize rows now
  use phrases. Multi-sentence permission, retention, scope, fallback, and privacy
  disclosures keep their grammar and information.
- [x] Introductory prose repeated by the page title and controls. Removed page
  subtitles and redundant introductions from Customize, Modes, Knowledge,
  saved transforms, command editors, theme creation, and What's New. Kept help
  that explains a control's consequence or a permission/privacy choice.
- [x] Competing primary buttons in command preview/save, transform practice and
  model setup, app overrides, and the corpus recorder. Ancillary actions are
  secondary; the corpus recorder has one retake control. Active meeting/corpus
  capture and cancellation use secondary styling rather than danger styling.
- [x] Extra Knowledge header and saved-transform row actions. Knowledge file
  actions and delete-all live in an overflow menu; saved transforms keep common
  actions visible and put delete last in overflow. Menus use Base UI menu items
  for arrow-key navigation, Escape dismissal, and trigger focus restoration.
- [x] One-click permanent removal of optional models, aliases, site rules, app
  overrides/project roots, saved reports, and diagnostics events. These now
  require a second activation and show an ellipsis. Existing mode/transform/
  transcription-model and typed bulk-delete confirmation contracts remain.
  Destructive controls sit last within their action groups. Unsaved draft
  editing and a disposable in-memory index remain directly editable.
- [x] Return activated a focused standing action or armed deletion. Main,
  Settings/editor, Home, and dialog boundaries now dispatch Return only to an
  explicitly marked enabled recoverable default. Destructive controls and
  destructive transcript menu items suppress Return. Space still activates the
  focused button. Disabled defaults and repeated/composing Return do not save;
  nested boundaries cannot save twice. Textareas, native selects, editable
  content, links, and menu selection retain their normal editing/navigation.
- [x] Stale tests/callers after copy edits. Updated accessible names, navigation
  paths, file actions, confirmation tests, and the corpus single-retake test.
- [x] Attribution. Extended the existing Hex MIT notice instead of duplicating
  it. The grouped integration must retain the Rune MIT notice owned by #847 and
  the T3 Code notice owned by #854; this child does not add duplicate notices.

## Coverage and evidence

The `settings-copy` visual fixture renders real Settings panes and editors with
synthetic local data. It sends editor requests after mount, then waits for the
actual editor heading; an overview screenshot cannot satisfy editor coverage.
Both light and dark cover General, Recording, Delivery, Meetings, Text,
AI overview, speech recognition, rewriting, Voice Query, Modes, Appearance,
Performance Lab, Diagnostics, shortcuts, and every text editor. Disclosures are
also captured expanded, scrolled to the final controls. Additional snapshots
show a recoverable command default, Knowledge overflow, typed bulk deletion,
and an armed alias deletion.

Existing Home, discovery, meeting, first-run, updater, and theme goldens cover
changed copy in those surfaces. The exact new/updated paths are in
[the golden manifest](copy-interaction-goldens.txt). Darwin snapshots are
Darwin evidence; Linux screenshot proof is reserved for the lead's local Docker
run on the mini. No MacBook, native app, or GUI session was operated here.

The alias action column is 120px so an armed confirmation and switch cannot
overlap; the browser test checks its computed size and control bounds.

Focused tests cover default/disabled/default-less Return, nested boundaries,
Space activation, repeated/composing keys, two-step removal, blur/Escape
rearming, typed DELETE, overflow interaction, and destructive menu handling.
The existing theme contrast matrix covers Sonic and imported low-contrast,
flat-dark, and high-saturation themes; semantic color tokens are unchanged.

Local logs and the isolated port-14850 Playwright configuration are retained in
`~/scratch/e846/850-evidence/`. The configuration uses `reuseExistingServer:
false`; it never connects to a worker's port 1420. The lead owns grouped CI,
independent Sol review, Linux proof, and native verification.

## Accepted review corrections C1–C5

- [x] C1: Shift, Command, Control, and Option with Return cannot natively
  activate a focused destructive button. Ordinary field modifiers, multiline
  Return, IME composition, and Space retain their editing/activation behavior.
  The browser regression arms an actual alias deletion, tries each modifier,
  and then deletes with Space.
- [x] C2: Transform capture deletion, private dictation capture deletion, and
  Performance data clearing use the existing Knowledge confirmation markup,
  extracted into a shared controlled dialog. Irreversible confirmations have
  no Return default; Cancel/Escape do nothing destructive and Space confirms.
  Capture ID and displayed pass/recording stay pinned while background lists
  change. Capture consent and upload consent keep their existing behavior.
- [x] C3: Appearance validates pending hex-color fields before Done. Invalid
  Return keeps the editor open with its error, and Done or Return elsewhere
  focuses the invalid field. A valid field Return still commits and closes.
- [x] C4: Command editors and Knowledge confirmations follow the established
  modal focus pattern through a shared hook: initial focus inside, Tab and
  Shift-Tab wrap, Escape cancels, and closing restores the opener. Knowledge
  overflow confirmations restore the persistent overflow trigger when their
  menu item has unmounted. Browser focus assertions need no test focus setup.
- [x] C5: Disabled setup Continue uses secondary styling; enabled Continue
  remains the recoverable Return default. Real-component browser assertions
  check class emphasis and computed backgrounds for microphone, Accessibility,
  System Audio, and Calendar across unknown, granted, denied, loading,
  restricted, and unsupported scenarios, in light and dark. Pending permission
  requests also preserve the single-primary rule.

Verification for this correction: TypeScript passed; all 181 Vitest files /
1,866 tests passed. All 78 affected browser tests passed, including the 23
new regressions and the existing copy, theme-contrast, and shell coverage. The
dedicated port-14850 configuration keeps
`reuseExistingServer: false`. Browser evidence and logs are under
`~/scratch/e846/850-evidence/c1-c5-*`; the lead retains final Sol review,
integration, and native verification. This correction does not change fonts,
App.tsx, styles.css, SettingsPanel, or #848's shared settings controls.

The prior 54-new / 18-updated golden manifest remains unchanged. This correction
adds only these 16 Darwin goldens under
`app/visual-tests/copy-review-850.spec.ts-snapshots/` (each row is both files):

| Light | Dark |
| --- | --- |
| `light-850-transform-confirm-darwin.png` | `dark-850-transform-confirm-darwin.png` |
| `light-850-dictation-confirm-darwin.png` | `dark-850-dictation-confirm-darwin.png` |
| `light-850-runs-confirm-darwin.png` | `dark-850-runs-confirm-darwin.png` |
| `light-850-invalid-accent-darwin.png` | `dark-850-invalid-accent-darwin.png` |
| `light-850-microphone-permission-darwin.png` | `dark-850-microphone-permission-darwin.png` |
| `light-850-accessibility-permission-darwin.png` | `dark-850-accessibility-permission-darwin.png` |
| `light-850-system-permission-darwin.png` | `dark-850-system-permission-darwin.png` |
| `light-850-calendar-permission-darwin.png` | `dark-850-calendar-permission-darwin.png` |

Native delta to verify after integration, using disposable local data:

1. Aliases: arm Delete…, then plain/Shift/Command/Control/Option Return must
   retain the alias; Space confirms. Multiline command fields still accept
   Return and Shift-Return as newlines, and IME confirmation remains intact.
2. Diagnostics → Transforms: delete from both the capture list and Review.
   Diagnostics → Private dictation capture: Review → Delete local copy….
   Diagnostics → Runs: Clear performance data…. Each dialog starts inside,
   wraps Tab, ignores every Return variant, cancels with Escape/Cancel, and
   confirms only with an explicit click or Space. Let a capture list refresh
   while open and verify the displayed pass/recording remains the target.
3. Appearance → Create theme: enter `not-a-color` in Accent; Return and Done
   must retain the editor and error. Enter a valid hex and Return; reopen to
   confirm the committed value. An invalid Background must also block Return
   from another field.
4. Voice commands → New command… and Knowledge → overflow → Delete all…:
   verify initial focus, both Tab wrap directions, Escape cancellation, and
   opener restoration without first clicking inside the dialog. Recoverable
   Save command / Import still use Return; typed DELETE still needs Space.
5. General → Run setup assistant…: check all four permission steps before,
   during, and after a grant or denial. At most one button is primary, disabled
   Continue is secondary, Return cannot skip an unmet step, and enabled
   Continue remains its default. Run these native checks in light and dark.

## Exact native flows for the lead

Use the integrated build with isolated test data, in light and dark:

1. Home: check discovery copy and start/stop capture; focus Start and press
   Return (no capture), then Space (starts). Focus a transcript and Space to
   copy. Open its overflow, arm Delete…, focus Confirm delete…, press Return
   (keeps transcript), then Space (deletes the synthetic test transcript).
2. Settings: visit every sidebar pane and AI drill-in; expand Advanced in
   General, Recording, Delivery, Text, and the AI pages. Check short row help,
   retained multi-sentence privacy guidance, titles, action emphasis, and no
   clipping at 880×720 and 720×560. Check `⌘,` and shortcut navigation still work.
3. Customize → Text & vocabulary → Aliases: arm Delete… on a synthetic alias;
   Return must keep it, Escape/blur must disarm, and two Space activations must
   remove it. Add/import/export a synthetic alias and verify the import preview
   only applies after its default confirmation. Repeat Mode import/export.
4. Customize → Voice commands: New command…; enter a synthetic phrase and
   replacement. Focus Cancel and press Return: the enabled Save command default
   must save once. In a multiline snippet, Return adds a newline. Space on Cancel
   cancels. Check the preview action is secondary. Check default validation and
   busy/disabled handling. Repeat Knowledge and saved-transform recoverable Save.
5. Knowledge: use arrow keys in More knowledge actions; open Delete all… and
   type DELETE. Return must never delete, including with Delete everything…
   focused; Space confirms. Cancel must write nothing. Repeat Delete all shown…
   under a filter and verify the captured-scope count remains truthful.
6. Modes: duplicate a built-in, arm its Delete…, Return must keep it, then
   complete the two-step deletion. Add a synthetic site rule and verify its
   two-step removal. Delivery → Advanced: verify app-override and project-root
   removals require two activations, while normal switches still work with Space.
7. AI → Speech-to-text: unused-model Remove… needs a second activation; selected
   and active models remain protected. AI → Selected-text rewrite, Recording →
   Smart Auto-Stop, and Meetings → remote speaker model: verify remove/confirm
   copy, blur disarming, Return protection, busy states, and removal/re-download
   only against disposable test models. No transcription or native lifecycle
   behavior was changed by this child.
8. General → Run setup assistant…: verify sentence case, one default Continue
   per step, disabled Continue while unmet/loading, Space on Back/Skip, and
   Return never selecting Skip as a default. Check microphone, Accessibility,
   system audio, Calendar, model, shortcut, and completion steps.
9. Appearance: inspect the custom editor's phrase help and Done default;
   text-field validation must still work. Inspect Community themes consent and
   loading/error copy. Check About and What's New dismissal defaults; update
   download/restart/quit remain deliberate actions with no Return default.
10. Diagnostics: Clear events… requires a second activation. Performance Lab
    and microphone startup saved-report Clear reports… require confirmation.
    Verify recording/cancellation stays secondary and does not acquire delete
    styling. Native startup/capture checks and Retina captures are run serially
    by the lead after integration.

### C6 focus fallback after capture removal

The Transform and Dictation capture lists expose a programmatically focusable
heading as the confirmation dialog's restore fallback. A surviving opener
still receives focus; if refresh or deletion removes it, focus returns to the
resulting list heading. Browser tests cover cancellation after row replacement
and confirmation from Review, in both color schemes.

For direct list deletion, remove the confirmed capture from local list state
before closing its dialog. This prevents focus returning to an opener that a
later asynchronous refresh will detach. The delayed-refresh regression fails
in both schemes without that ordering and passes with it. Cancellation keeps
its existing opener-restoration behavior.
