# Settings progressive disclosure (#849)

Behavior reference: Hex `a07f994`, `src/rune_ui/controls.rs`
(`CardRow::Folding`, `folds`), `general.rs`, `labs.rs`, and `sound_sets.rs`.
The MIT attribution is in `THIRD_PARTY_NOTICES.md`. T3 Code `cba30591f9`
remains the visual authority; Rune `100b7d93` remains the optional theme source.
This wave starts at merged main `95904d4130cb706f6851f2c954b1fc5afb13f97d`.

## Behavior and state

Dependent controls remain mounted in `SettingsBranch`, preserving controlled
saved values and local drafts. Their separator is inside the clipped content,
so it collapses with them. Entry takes 300 ms with a critically damped easing;
exit takes 450 ms. The fading content approaches 0.4 opacity as the clip closes.
Reduce Motion makes both layout and visibility changes immediate. Once entry
settles, the clip releases overflow so existing select menus remain usable.
Closing immediately applies `inert` and `aria-hidden`, disables pointer input,
and restores contained focus to the owner. The clip remains during exit.

Rare Advanced children use that same transition with `unmountOnClose`:
initially absent, mounted for entry, inert throughout exit, then genuinely
unmounted. Rapid reversal cancels disposal. Backend values are never reset by
visibility changes. The UI-only localStorage document
`murmur.settings-disclosure.v1` is versioned, limited to 2 KiB on read, and
projects only boolean values for the finite pane-key allowlist. Unknown keys,
malformed documents and future versions fall back safely. No settings values,
paths, model/provider credentials, or permission state enter this document.
Storage failure retains session state. Per-pane Advanced state survives a
normal app relaunch independently of backend preferences.

The title, descriptions, card/row geometry, narrow container query, reset
handlers and disabled contract remain #848's: 240/56 navigation, 12/16 row
insets, 14/20 titles, 12/18 descriptions, 32px column gap, 512px stacking
boundary with 12px gap, and 8px controls / 12px cards. Descriptions wrap.
Existing Select/microphone values retain end ellipsis. The project-scan,
IDE-root, shared-output and Performance Lab paths use `MiddleTruncate`.
Editable executable/environment path fields remain editable inputs.

## Complete pane audit

These are all 22 navigation flows exercised by `settings-geometry.spec.ts`.
Repeated destinations are called out because their caller and focus return
matter. “Unchanged” means no applicable dependency or rare settings group,
not an omitted audit. Hubs, reference lists and dedicated editor workflows do
not get empty Advanced buttons or invented settings.

| Flow | Before | After / ownership |
|---|---|---|
| General | Advanced `<details>` kept calibration/reset mounted | Muted Advanced text button; calibration and reset genuinely unmount after close; General remembers its state |
| General → Keyboard shortcuts | Read-only shortcut index | Unchanged reference list; links still reach explicit owner pages and Back restores the shortcut row |
| Recording | Standby rows and timing feedback appeared abruptly; sound children used the old branch; all major sections exposed | Standby window/pre-roll fold while standby is on; timing feedback folds only for double-tap/both; rare standby, sound cues/volume/preview/meeting cues, and Smart Auto-Stop live under Recording Advanced; model installation remains available there regardless of feature enablement |
| Customize | Five destination rows | Existing hub retained; Voice commands now opens a sheet over this hub, and closing restores that row; other destination Back routes retained |
| Delivery | Auto-paste/file children used old branches; app editor lived inside Advanced | Paste delay/Smart destination fold with auto-paste; shared folder folds while either file output is on; paused auto-paste retains disabled notice and saved preference; Advanced lazily reveals the app-editor launch; editor opens over Delivery |
| Meetings | Calendar details always shown; echo/retention always shown | Calendar details/access action fold only while suggestions are on; echo and age/session retention move into remembered Advanced; remote-speaker install/manage remains visible while off; explicitly unsupported speaker capability is omitted |
| Text & vocabulary | Cleanup and correction branches kept drafts; Advanced `<details>` kept rare children mounted | Cleanup filler/capitalization fold together; Advanced genuinely unmounts developer terms, corrections, structured writing and spoken formatting; project scan folds while developer terms are on; fuzzy/homophone controls fold while preferred spellings are on |
| AI & models | Three install/configure destinations | Hub retained so installable features stay discoverable; enabled Query/rewrite also gain explicit sidebar destinations |
| Modes | Site-rule fields conditionally mounted while host lookup enabled | Site fields now clip/fade within their existing card and retain drafts/saved rules while off; permission-sensitive lookup remains an explicit switch; bindings, import/export and preview remain the dedicated editor workflow |
| Appearance | Library with separate theme/community dialogs | Existing sheet pattern retained; theme editor now uses shared dialog focus containment, Escape and caller restoration; no empty Advanced group around the theme library |
| Performance Lab | Dedicated test/report workflow; output path wrapped | Existing workflow retained; shared export path middle-truncates (manual export also uses it, so it is not dependent on auto-save) |
| Diagnostics | Dedicated Events/Runs/Transforms/Reports workspace | Unchanged; content/privacy disclosures remain owned by that workspace, with no invented settings |
| AI & models → Speech-to-text | Model/language/storage/idle timeout all exposed | Idle timeout moves to remembered Advanced; model selection, language's temporary disabled contract and install/remove storage remain discoverable |
| AI & models → Voice Query | Shortcut picker remained visible disabled while off; timeout always shown | Shortcut row folds only while Query is enabled; timeout moves to remembered Advanced; executable, arguments, environment repair, privacy/context and history controls retain their existing contracts and owners |
| AI & models → Selected-text rewrite | Hold-key branch used old motion; practice always visible | Hold key folds with shortcut; practice moves into remembered Advanced; model download/runtime recovery remain visible while off; Manage opens transforms over this owner |
| Customize → Voice commands | Full-page editor replaced its caller | Voice-command sheet preserves Customize selection and restores the originating row; existing nested editing/confirmation dialogs keep #850 behavior |
| Customize → Styles | Delivery Advanced opened with a large inline app editor | Delivery Advanced opens its launch row; app overrides use a sheet, with explicit Delivery ownership; Back to Customize retains its prior route |
| Customize → Transforms | Rewrite owner and transforms editor routed through Text | Rewrite owner retained; Manage now opens transforms as a sheet over rewrite, retaining the selected owner and caller focus |
| Text & vocabulary → Vocabulary | Full-page scanned vocabulary | Sheet over Text; same bounded filter/sort workflow; close restores Vocabulary |
| Text & vocabulary → Aliases | Full-page alias editor | Sheet over Text; saved values and explicit destructive confirmation unchanged; close restores Aliases |
| Text & vocabulary → Knowledge | Full-page knowledge editor | Sheet over Text; nested confirmations/menus retain focus lifecycle; close restores Knowledge |
| Text & vocabulary → Manage project scan | Full-page scan editor; displayed path wrapped | Sheet over Text (or the pane owning the enabled sidebar launch); middle-truncated path; scan/cancel/change handlers unchanged; close restores its actual caller |

Additional existing app-editor rows: manual bundle-ID entry is a remembered,
lazy Advanced group owned by `app-overrides`; IDE roots and index actions fold
only while each app's IDE context is enabled. Roots and draft manual-entry text
survive hiding. An empty Modes binding opens this same app sheet over Modes;
closing preserves Modes, and adding a profile retains the existing return to
its selected Mode. No application profile, IDE index or private environment
value is copied into disclosure storage.

Hex's “Double-tap to lock”, “Double-tap only”, keyboard-model-switch mappings,
sound-set management and lab panes have no corresponding Murmur feature. They
were mapped to the actual recording-mode/timing, sound-cue, model, and optional
local-model controls above; no unrelated toggles, mappings or sound sets were
introduced. The behavior adaptation does not import Hex's visual dimensions.

## Search and capability boundaries

`settingsDisclosureRouting.ts` lists Advanced targets and dependent owners.
The static index includes folded controls even when their children are absent.
Search opens only Advanced UI state. If a feature is off, it pulses that parent
using #848's two 650ms pulses; it never enables a feature or grants permission.
Each request reveals once, so the user can close Advanced after searching.
Repeated searches and external navigation requests can reveal it again.
Manual bundle-ID search opens the existing app sheet and its own Advanced.
Unknown target strings cannot become CSS selectors or throw selector errors.

Only explicit existing capability facts suppress controls: the macOS-only
Smart destination row/search item, a confirmed absent NotchPill integration,
and `supported: false` for remote-speaker labeling. Unknown capability is not
interpreted as unsupported. Downloadable models remain offered, and temporary
model/language/runtime/recording unavailability retains the disabled contract.
No immutable backend setting, permission workflow or private-data boundary
was changed.

## Verification and native handoff

`SettingsBranch.test.tsx` renders production components and verifies saved
values/drafts, focus restoration, immediate inert/AX exclusion, 450ms disposal,
reversal, instant reduced motion, pane persistence and bounded invalid storage.
`SettingsPanel.test.tsx` exercises real search and verifies off-parent routing
without settings/permission writes. Existing owner navigation and model tests
remain in place.

`settings-folding.spec.ts` covers all changed folding/Advanced panes in Light
and Dark, all seven existing editor sheets and their caller focus, nested IDE
rows, both motion preferences, computed timing/row measurements, search pulse,
manual-entry search, persistence/reload, and enabled navigation. The original
22-pane fixtures, full visual suite, #850 keyboard/dialog tests, measurement
and imported-theme contrast matrices remain required. Darwin and Linux PNGs
have distinct platform suffixes; Linux runs in
`mcr.microsoft.com/playwright:v1.62.1-noble` on the Mac mini. The temporary
`playwright.849.config.ts` uses only port 14849 with server reuse disabled.
It is an evidence runner, not a replacement for the standard CI config.

Native verification is deliberately reserved for the lead's isolated Mac mini
build. See the implementation evidence directory for exact recording steps,
command logs, golden manifest and revision receipt. No GUI app, OS display or
permission setting, updater/OTA/canary, recognition benchmark, production data,
MacBook, remote push, PR or review was operated by this implementation worker.

Murmur Bench: N/A — this changes Settings presentation, disclosure UI state,
search routing and editor focus only; backend settings, capture, recognition,
transcript delivery and runtime execution paths are unchanged.
