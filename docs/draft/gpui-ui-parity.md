# GPUI UI parity spec (council round 1)

Goal: the GPUI app must look like today's Tauri app. The React UI in `app/src/` is the baseline;
when this spec and the React source disagree, the React source wins. Round 1 scores: Claude Opus 5.5
3/10, GPT-6 Astra 4/10, Muse Wonder 4/10. All three agree the features are present but the shared
controls, icons, tokens and navigation make it look like a different app. Target: every screen 8+/10.

Paths: `G/` = `app/gpui/src/`, `R/` = `app/src/`. Line numbers are from 47abc7c6 and may drift.

## Cross-cutting (do first; they move every screen)

X1. Button variants (`G/button.rs:137-190`). Every intent, including Ghost, gets an opaque 1px border
today, and `toggled()` (`:80`) changes nothing visible.
- Ghost: no border, transparent, ~5% hover tint.
- Secondary: hairline border (outline at 22-30% alpha), raised tint, small shadow, per
  `.ui-dashboard-action[data-action=secondary]` (`R/styles.css:~615`).
- Primary: accent fill/gradient with inset highlight (`R/styles.css:~611`).
- `toggled(true)` paints the selected/raised background.
- Add leading icon, icon-only square, pill (fully rounded) and nav-row variants.
- Buttons hug content; stop stacking full-width buttons in screens.
- Keep accessible names/states and the 32/28/24px heights.

X2. Icons. The vendored gpui-component ships Lucide (`G/overlay/mod.rs:~549` already uses
`gpui_component::Icon`). Add a small icon helper; use the React mappings: house, calendar,
message-square, bar-chart (sidebar, `R/components/home/HomeSidebar.tsx`); mic, square, spinner
(talk tile, `R/components/home/HomeRecordingBar.tsx`); settings, panel-left, search, list-filter,
ellipsis, copy, chevron-right, download, refresh-cw; settings nav icons (`R/components/settings/SettingsPanel.tsx:~239-254`).
Brand: 4-bar logo tile + "Murmur" wordmark. Remove letter captions (`G/home/mod.rs:~348` H/N/I,
`G/settings/mod.rs:~1222` G/R/C...) and the `☰` text.

X3. Tokens (`G/tokens.rs`). Borders are opaque `0xabb3b9`; React uses hairlines at 22-30% alpha
(`R/styles.css:~866,1072,1149 --ui-hairline`). Add `hairline`, `hairline_strong`, `tint_raised`,
`surface_container_low`, card shadow (`--ui-shadow-1`). Cards must be lighter than the page in light
mode (`--ui-tint-raised` = surface-container-lowest, `R/styles.css:~154`); GPUI's `p.raised` is darker
(history cards `G/history/mod.rs:~743`). Keep Increase Contrast / Reduce Transparency handling and the
Sonic accent values (already correct).

X4. Type scale. Add 11px micro, 12px caption, label weight, monospace tabular time
(`--home-font-mono`, `R/styles.css:~1238,1307`) and large tabular stat numbers. Today almost everything
is 14px body.

X5. Chrome and navigation.
- One navigation owner. The production host (`G/host/services/root.rs:~532-620`) wraps every screen
  in its own 176px, 9-item sidebar ("Meetings", "Search commands ⌘K", "Updates"), on top of Home's
  sidebar; `G/remaining.rs:~235-262` adds a QA tab strip. Use the React structure: Home sidebar with
  Home / Notetaker / (Assistant if the React app shows it) / Insights; gear opens Settings with its own
  nav; Models, Knowledge, Modes, Customize, Appearance live inside Settings. Keep every destination
  reachable (command palette ⌘K, gear, Settings nav). QA tab strips are harness chrome only.
- Title bar: match the Tauri window (check `app/src-tauri/tauri.conf.json` titleBarStyle). Expected:
  no visible title band, transparent title bar with traffic lights inset over the sidebar
  (`G/host/shell.rs:~107,134`). Keep native traffic lights.
- Home header (`G/home/mod.rs:~298-327`): replace "Command palette"/"Settings" text buttons with a
  panel-left icon at the top of the sidebar and a gear icon top-right. ⌘K still opens the palette.
- Home sidebar (`G/home/mod.rs:~330-370`): pale flat/gradient background (`R/styles.css:~964-975`),
  left-aligned icon+label rows, 3px accent bar + tinted icon on the active row (`R/styles.css:~650-662`),
  logo tile + wordmark, privacy note as a small card with a green dot (`R/styles.css:~1072`).
  Collapsed: 48px icon rail with tooltips and the privacy dot. Expanded width 256-384px.
- Splitter (`G/home/mod.rs:~410`): 1px hairline with a wider invisible hit area.

## Per screen

Home talk card (`G/home/mod.rs:~580-648`, ref `HomeRecordingBar.tsx`, `R/styles.css:~1145-1222`):
whole card is one clickable row; 32px primary tile with mic (idle) / red stop square (recording) /
spinner (processing); never show a "Start" button while processing; hint with key-cap chip
("or hold ⇧ Left Shift in any app"); waveform inline after "Listening · 0:12"; error-tinted border while
recording; footer strip with hairline: "This month **N** words · **M** dictations" caption, quiet
"View insights ›" link right.

Discovery checklist (`G/home/mod.rs:~700`, ref `R/components/home/DiscoveryChecklist.tsx`): match
the progress strip, completion circles, quiet row actions; capture it enabled in QA.

History list (`G/history/mod.rs:~649-770`, toolbar `~1040-1160`, ref `R/components/history/HistoryPanel.tsx:~307,443-481`,
`R/styles.css:~1281-1307`): one toolbar row = "Your dictations" heading, search field with icon, `/`
key chip and inline × clear, "Filter ⌄", ⋯ overflow (holds "More history actions"). No standing
Previous/Next buttons (scroll-driven paging). Day group labels (Today, Yesterday, date); one card per
day with hairline dividers; 72px monospace local time column ("3:26 PM"), source badge chip for
imported files; remove per-row "Show more / actions" button; Copy + ⋯ on hover/selection; "Copied"
feedback tints the row in place.

History detail (`G/history/mod.rs:~803-940`): quiet "‹ Back" link, Copy as icon button, Markdown /
Pin / Delete in ⋯ (Delete destructive text, not a filled red box). Keep the actions.

Insights (`G/insights/mod.rs:~60-180`, ref `R/components/home/InsightsView.tsx`, `R/styles.css:~671`):
"‹ Back to Home" link beside title/subtitle; 4-column tile grid (muted 12px label, ~24-26px semibold
tabular value, caption); heatmap and 7-day bar chart cards visible in the first viewport at 880x720.

Meetings (`G/meetings/view.rs`, ref `R/components/history/MeetingsPanel.tsx`, `MeetingReviewWorkspace.tsx`):
header with back link, "Notetaker" title + subtitle; green-dot status line with mic/system-audio
caption; "Start meeting" primary on the right; Me/Them explanation to a tooltip; list = search with
icon + ⋯ (holds "Delete all…"), date groups, meeting cards (title, preview, time, duration, accent
bar when selected); detail = title with "Name meeting" pill, "Name from calendar…" + trash icon,
tinted notice, Format select (default per React) + Copy + Export on one row; transcript grid of
monospace timestamp, coloured speaker (Me/Them), text; play via clickable timestamp or hover icon,
no full-width per-segment Play buttons.

Settings shell (`G/settings/mod.rs:~976-1247`, ref `SettingsPanel.tsx:~948`, `R/styles/settings.css`):
240px nav without border box, "Murmur Settings" wordmark, icons, "Tools" group (Performance Lab,
Diagnostics) at the bottom, "Done" pill top-right, fixed heading with separator.

Settings General (`G/settings/panes.rs:~38,202`, ref `SettingsPanel.tsx:~424`): one grouped card with
dividers in React order: Launch at login, Keyboard shortcuts (chevron row), Run setup assistant (inset
button + caption), Recording overlay select, Live words toggle, "Advanced ⌄" disclosure row, Check for
updates (button with refresh icon), centred version caption. Remove the standalone "Show feature
checklist" and "Update details…" buttons unless React has them.

Customize (`G/customize/mod.rs:~489-560`, settings QA hub `G/settings/panes.rs`, ref
`R/components/settings/CustomizationHub.tsx`): cards with title, description, chevron. Match the React
destination list exactly (verify whether "Selected-text rewrite" is in it).

Appearance (`G/themes/mod.rs:~471-605`, ref `R/components/settings/AppearanceSettings.tsx:~282-330`,
`ThemeLibrary.tsx:~82,250`): System/Light/Dark segmented control with ✓ on selected; Themes header with
"+ Create theme" and "Import theme"; grid of theme cards (~200x100) with light+dark swatch orbs, name,
icon actions (duplicate/export/delete), primary ring on the selected card; no "Use X light / Use theme
/ Duplicate / Export / Remove" text rows. Collection cards and variant buttons must keep their
accessible labels (the Playwright test selects `button[aria-label*="dark variant"]`).

Site modes (`G/modes/mod.rs:~567-586`, ref `R/components/settings/ModesManager.tsx:~225`): "Browser
sites" toggle row, not a button; add-rule row = browser select + host field + Add; each rule = toggle,
browser select showing the browser's display name (not the quoted bundle id), host field, red-text
"Remove…".

Onboarding (`G/onboarding/mod.rs:~618-1070`, ref `R/components/onboarding/OnboardingFlow.tsx:~518`):
centred ~520px column; dot stepper with "Step n of 8"; 64px tinted icon tile; centred title/subtitle;
permission row with icon and status chip ("Not set"/"Denied"); footer: quiet Back left, quiet Skip +
primary Continue right; "Reset permission…" as a quiet link; keep "Open System Settings…" guidance in
the denied state.

Update dialog (`G/host/services/root.rs:~663-845`, ref `R/components/UpdateModal.tsx:~180,292-334`):
centred modal with darker/blurred scrim, download icon tile header, "Update available", version
caption, × close, hairline, "What's Changed" bullets in a scrollable body, footer with primary pill
"Download update" and secondary pill "Later" side by side.

Pill (`G/overlay/mod.rs:~425-625`, ref `R/components/overlay/OverlayPill.tsx:~63`): keep 120x34 and the
dark contrast; no "⌄" disclosure in the plain recording state (keep quick settings reachable another
way the React app offers, or on hover); red vector dot + compact waveform alignment as React; verify
the window corners are transparent (capture showed black square corners).

## Keep from GPUI

Native window and traffic lights, focus rings, accessible names/states on every control, keyboard
splitter, Increase Contrast / Reduce Transparency, Sonic accent values, subtle accent bar on pinned
transcripts, explicit "Denied" state with "Open System Settings…", explicit history detail actions
(restyled), 32/28/24px control heights.

## Round 2 findings (council at 77635e35)

Scores: Claude Opus 5.5 7/10, GPT-6 Astra 7/10, Muse Wonder 1.0 7/10. Round-1 identity issues are
resolved. Target unchanged: 8+ overall, no screen below 7. Worst screens: site modes (4/5/5),
meeting review (5/6/6), meeting captions (6/6/6), Insights (6/7/7).

Settled by the council (do not chase):
- Pill corners are transparent (alpha 0); the black corners were a sheet-compositing artifact.
- "Assistant" (sidebar) and "Selected-text rewrite" (Customize) appear only in stale Darwin goldens;
  current React (`R/components/home/HomeSidebar.tsx`, `R/components/settings/CustomizationHub.tsx`)
  has neither. Do not add them.
- "Reset to Sonic" exists in React (`R/components/settings/AppearanceSettings.tsx:423`); keep it.

### Comparison baseline (round 3)

- OLD: render current React (this branch's `app/src`, same as `main`) on macOS WebKit for all 19 keys,
  instead of the stale Darwin goldens. Do not commit regenerated snapshots.
- Put OLD and NEW in the same state: discovery checklist visible at 2/5 on Home, recording and
  processing (`G/bin/home-qa.rs:43,109` currently dismisses it); same selected theme and scheme on
  Appearance; captions view with SubRip selected and its caption note; meeting review opened at the
  top (title visible); onboarding microphone in the same permission state on both sides.

### Cross-cutting

R1. Chevrons are text characters (`G/history/mod.rs:~1380` "Filter  ⌄", `G/home/mod.rs:~864`
"View insights  ›"). Use label + Lucide chevron icon (12px, muted). Grep for ⌄ › ‹ in captions.
R2. Off toggles are almost invisible (`G/toggle.rs:~98` uses `p.raised`). React: grey track
(surface-container-highest) with white knob and hairline.
R3. Search fields: Settings (`G/settings/mod.rs:~1280`) and Meetings (`G/meetings/view.rs:~882`) lack
the magnifier and Meetings is square. Share History's search style (icon, rounded, 32px).
R4. Weights and sizes: titles, row titles and stat numbers are semibold where React mostly uses medium
(talk-card title, "Your dictations", Insights numbers). Keep semibold only where React says
`font-semibold` (e.g. hub card titles, `CustomizationHub.tsx:64`). Talk-card subtitle 13px/20px.
R5. Navigation: inactive labels and 14px icons use on-surface-variant; active Home row tints its icon
with the accent; active Settings row is a raised white row in light mode (`R/styles/settings.css:~421,427`;
`G/choice.rs:~433`, `G/settings/mod.rs:~1209,1286`). Match each destination's Lucide icon to React
(Customize, Delivery, AI differ today, `G/icons.rs:~147`).
R6. Transcript previews are cut mid-word (`G/history/mod.rs:~823` `chars().take`). Clamp to 2 lines
with an ellipsis and cap the text column at a readable width as React does.
R7. Row rhythm and card recipes: Home/History rows are ~48px one-line vs React's ~60px two-line.
Port per-component padding/leading/border/surface from React (`R/styles.css:~124,1281-1307`,
`R/styles/settings.css:~446-461`) rather than one card recipe everywhere; settings rows 12px/16px.
The fixed 76px description-button height inflates Customize and Keyboard shortcuts.

### Per screen

R8. Site modes (`G/modes/mod.rs:~440-800`, ref `R/components/settings/ModesManager.tsx:~205-263`).
Rebuild into React's grouped structure:
- Selected mode card: mode row with "Built-in template · …" caption and Duplicate right; Customize row
  with the read-only note.
- "Where it applies" section heading.
- One divided card: Bound applications ("Add an app" right-aligned), Browser sites toggle, "Add a site
  rule" with its caption, browser select + host field (placeholder `github.com`, capped width) + Add,
  rule rows (toggle, browser, host, bordered red-text "Remove…"), Current site with "Test current
  site" on the right.
- Move Create/Import/Export into the mode-list area like React. Fix the clipped mode-chip row; open at top.
R9. Meeting review/captions (`G/meetings/view.rs`, `G/meetings/model.rs:~89`, ref
`R/components/history/MeetingReviewWorkspace.tsx:~220`, `R/styles.css:~1365-1429`):
- Detail opens scrolled to top.
- Playback on one compact row (Play, time, channel, slider); remove the "0 ms" label.
- Fold the lone download icon into the Format/Copy/Export row.
- "Copy transcript" / "Copy captions" full labels; SubRip default and caption note in captions view.
- Hairline above "Transcript", muted smaller "N segments"; 28px pill actions; rounded "Speakers"/"Rename"
  in secondary style; speaker column wide enough that "(Them)" doesn't wrap.
R10. Meeting list rows (`G/meetings/view.rs:~42-80`): replace the description-button string with
React's row: title + right-aligned time, one-line preview with ellipsis, right-aligned duration;
selected = raised card + border + accent edge; unselected unfilled; no 112px reservation and no empty
per-item date headings.
R11. Insights (`G/data_components.rs:~900-967`, `G/insights/mod.rs`, ref `R/components/ui/DayChart.tsx:~103,256`,
`R/components/home/InsightsView.tsx`, `R/styles.css:~692,706`): heatmap month labels, weekday labels,
date-range caption, stronger low-value tint; weekday letters under bars, narrow rounded bars; both
charts in one outer container as React; React copy ("Focus a day for exact values."); values ~24px
medium and long values ("Everyday") must fit.
R12. Appearance (`G/themes/mod.rs:~526-620`, ref `ThemeLibrary.tsx:~82,173,497,516`): layered radial
orbs (~36px) with a small sun/moon badge on the lower-right edge, not a glyph inside; selected card is
ring only, no tinted fill; muted 12-14px action icons (delete not red); "Create theme"/"Import theme"
labels; remove header icon buttons React lacks; segmented control without the grey container if
React has none.
R13. Customize: heading "Customize Murmur" (`SettingsPanel.tsx:~906`) with "Customize" kept as nav
label; React's top gap (`mt-6`) and card height (measure React, ~68-76px).
R14. Settings General: "Keyboard shortcuts" row uses the same padding/weight as other rows; "Run setup
assistant…" gets a visible hairline border.
R15. Update dialog (`G/update_dialog.rs`, ref `R/components/UpdateModal.tsx:~139,161,190,313`): blur
or ~0.45 scrim; 16px header padding, 20px body; "What's Changed" 16px; tighter disc bullets;
36-40px footer pills with React's primary gradient.
R16. Pill (`G/overlay/mod.rs:~554,635-690`, ref `R/components/overlay/OverlayPill.tsx:~63`): measure
React for the dot (round, not faceted), bar count/width/gap and the symmetric envelope (~24 bars of
~1.5px); match the light fill tint and hairline.
R17. Onboarding: column ~20px lower; permission row height per React. Collapsed Home rail: privacy dot
keeps its capsule.
