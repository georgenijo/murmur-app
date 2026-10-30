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
