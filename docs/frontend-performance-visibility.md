# Frontend visibility and recording render work

This document describes the performance mechanisms and repeatable verification
workloads. It does not claim a measured whole-app speedup; compare matching
release builds and retain the resulting measurements with the change.

## Mechanism before and after

| Workload | Before | After |
| --- | --- | --- |
| Recording audio levels | `useRecordingState` calls `setAudioLevel` in `App` for every sample; the main tree can render at the sample cadence | Only the mounted, visible `RecordingWaveform` leaf subscribes and updates; lifecycle state stays in `App` |
| Hidden startup of Diagnostics window | Workspace immediately hydrates Events and performance history, subscribes to both streams, and starts the 1-second private-capture status poll | Workspace remains mounted but inactive until native visibility is known |
| Settings closed with Diagnostics selected | Category alone keeps diagnostics active behind the main surface | Settings surface visibility and native main-window visibility also gate activation |
| Diagnostics tab selection | Events and performance subscriptions remain active on every tab | Events subscribes only on Events; performance history subscribes only on Runs/Performance; capture badge remains active on every visible Diagnostics tab |
| Diagnostics resume | Cached performance rows merge with a new snapshot, including rows cleared elsewhere while hidden | Backend snapshot replaces the cached snapshot; live arrivals during the read are retained |
| Settings memo | Inline checklist callback changes on each parent render | Checklist callback is stable while its actual dependencies are unchanged |

The shared native `audio-level` emitter and `useSilenceAutoStop` are unchanged.
There is no sampling or throttling of that stream. The main header currently
sets `showRecordControls={false}`; the visible home waveform is the leaf owner.
The hidden waveform unsubscribes even while global hotkeys and recording
lifecycle listeners continue to run.

Native activity starts false, installs observers before hydrating, checks
`isVisible()` and `isMinimized()`, and rejects outdated replies. Focus loss
alone does not imply hidden: a visible unfocused window stays active. Close
invalidates reads immediately; focused reopen clears that close latch. Focus
and DOM visibility changes also schedule one recheck after 150 ms for native
transition ordering. Resize notifications debounce into that one recheck.
There is no recurring visibility poll. Native close remains owned by Rust;
the frontend observes `tauri://close-requested` directly rather than calling
the high-level close helper, which can destroy the window by default.

Native verification must include minimizing an already unfocused window and
restoring it, in addition to the focused-window flow. The frontend observes
DOM visibility and native focus/resize events; it does not add a native
miniaturization observer. Verify the production WKWebView delivers a wake-up
for both paths before considering this gate complete.

## Designed regression coverage

- `HomeRecordingBar.performance.test.tsx`: flush 120 alternating audio samples
  in separate React acts, verify the waveform changes and recording-owner render
  count does not increase; verify hidden waveform unsubscription and continuing
  recording status updates. Fake timers isolate audio from the duration timer.
- `useNativeWindowVisible.test.tsx`: hidden startup, no recurring hidden probes,
  visible unfocused state, close-to-hide, reopen, minimized-state settling,
  DOM visibility notifications, outdated responses, and late listener cleanup.
- `DiagnosticsWindowApp.test.tsx` and `SettingsPanel.test.tsx`: visibility is
  passed through to persistent diagnostics workspaces on hide/reopen.
- `DiagnosticsWorkspace.test.tsx`: hidden badge/pane polling stops, resume
  hydrates immediately, hidden transform reads are deferred, and tab selection
  gates data subscriptions.
- `usePerformanceDiagnostics.lifecycle.test.tsx`: hidden reads/subscriptions,
  queued callbacks after hide, backend clear while hidden, live arrivals during
  hydration, and stale responses from an earlier visibility generation.
- `usePerformanceHealth.test.tsx`: hidden polling stops, resume hydrates, and
  a pending hidden read cannot continue into a model-runtime query.

## Native verification and measurement workloads

Use the same production configuration, model, corpus, and capture backend for
baseline and candidate. Record immutable SHAs, app build identity, OS version,
and host. Do not compare a debug build with a release baseline.

1. **Idle, 60 seconds per state:** Diagnostics never opened; pop out Events then
   close it; open embedded Diagnostics then return Home; repeat with Performance
   and Dictation selected; minimize/restore both windows while focused and while
   already unfocused. Count frontend diagnostic IPC calls and listener delivery
   by webview, main/Diagnostics React commits, and CPU time for Murmur plus each
   owned WebContent process. Record median/peak RSS separately. Expected mechanism:
   hidden diagnostic IPC and data callbacks stop after pending work settles.
   Native visibility probes occur only at startup and transitions.
2. **Recording, 30 seconds:** replay the same speech/silence workload with Home
   visible, Settings covering Home, main window hidden, and main minimized.
   Count emitted audio samples, waveform commits, `App` commits, total React
   commit time, and p50/p95 commit duration. Record process CPU-seconds and RSS.
   Expected mechanism: audio-only samples cause zero `App` commits; only the
   visible recording waveform renders for audio. Duration and real state changes
   still legitimately commit the parent.
3. **Functional continuity:** finish/cancel recordings while hidden, restore and
   inspect duration/history/delivery; exercise hold, double-tap and silence
   auto-stop with identical input. Compare stop decision time and delivered text.
   Confirm no duplicate history or changed recording generation behavior.
4. **Diagnostics continuity:** hide while reads are pending, complete a run,
   clear history from the other window, and reopen. Confirm fresh backend rows,
   no resurrected cleared data, retained tab/filter/report selection and imported
   reports, and current private-capture state. Repeat close/reopen 20 times and
   check that active listener and timer counts return to their original values.
5. **Navigation:** record Home → Settings → Diagnostics → Home → Settings
   p50/p95 first-presented-frame latency under active recording and while idle.
   Count commits and long tasks (>50 ms). Keep all first-run/model hydration
   samples separate from warm navigation.

Run the targeted suites above, existing `useRecordingState`,
`useSilenceAutoStop`, `useEventStore`, `usePerformanceDiagnostics`,
`HomeDashboard`, and UI-latency suites, then the repository's scheduled TypeScript
and full frontend checks. Run native UI checks and corpus comparisons separately
from compilation and other benchmark processes to avoid host contention.
