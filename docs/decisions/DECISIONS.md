# Decisions Log

Running log of architectural, scope, and process decisions for this project. Newest entries at the top. Each entry is short — for deep rationale on a single locked decision, write an ADR alongside in `docs/decisions/YYYY-MM-DD-*.md` and reference it here.

Maintained via the `/decisions` skill. See `~/.claude/skills/decisions/SKILL.md` for the entry format and invocation rules.

---

## 2026-07-20: Overlay geometry & lifecycle contract locked (#280)

**Decision:** Five locked outcomes of the overlay architecture review (issue #280; PRs #290, #299, #301):
1. **Rust is sole author of overlay geometry.** All dimensions derive from `geometry_for()` in `commands/overlay.rs` returning `OverlayGeometry`; the frontend consumes it at runtime (`get_overlay_geometry`, `overlay-geometry-changed`) and contains no geometry pixel constants. Motion timing (ms/easing) is frontend-owned in `lib/overlayMotion.ts`, with the shrink delay derived from the height-transition token — never free-standing.
2. **Contract enforced by a shared checked-in fixture** (`app/src/components/overlay/overlay-geometry.fixture.json`) asserted from both `cargo test` and vitest. No codegen.
3. **Hover expansion is one serialized 4-phase controller** (`useOverlayExpansion`: collapsed/opening/open/closing) with grow-then-reveal / hide-then-shrink ordering, applied-frame acks from `set_overlay_expanded`, and a generation-guarded writer owning every surface resize. No hook may own half of this lifecycle.
4. **Contract + controller + split land before any visual rehaul** (PR4); the rehaul may not touch geometry derivation except via `geometry_for()`.
5. **Cross-window settings stay localStorage + `settings-changed` events**, wrapped in per-window hooks; all overlay settings access goes through `loadSettings()` — no Rust settings store.

**Rationale:** TS and Rust were independent authors of overlay geometry (divergent no-notch fallbacks 185/140/200, hand-mirrored 44px drop height) and the expand choreography was un-acknowledged (CSS could animate into a window that had not grown). Two independent architecture reviews converged on the diagnosis; runtime Rust-owned geometry beats a shared-constants file because it shares the *derivation*, not just values, making formula drift structurally impossible rather than test-guarded.

**Status:** active

**References:** issue #280 (review memo + drift note), PR #290 (PR1 geometry contract), PR #299 (replacement PR2 expansion controller), PR #301 (replacement PR3 component split), `docs/features/overlay.md`.

---

## 2026-06-23: In-process Tier 3 abandoned (ggml ABI clash); deferred to a sidecar

**Decision:** Tiers 1–2 (no-LLM post-model correction) ship as planned. Tier 3 (local-LLM cleanup) is NOT shipped in-process — the `llama-cpp-2` integration and dormant settings/module were removed from the app crate. Tier 3 is deferred to a future sidecar-process design. This supersedes the Tier-3 portion of the entry below (Tiers 1–2 portion still stands).

**Rationale:** `whisper-rs` and `llama-cpp-2` each statically vendor their own `ggml`. They link (matching symbol names) but **SIGSEGV at runtime** — an ABI mismatch between two ggml versions in one process — reproduced in both CPU (`MURMUR_T3_GPU_LAYERS=0`) and Metal modes during model load. Proven by isolation: a standalone binary linking only `llama-cpp-2` loads the GGUF and generates text fine (`MODEL LOADED OK`); the same code inside the app (which also links whisper) crashes. The only viable local-LLM path is a separate sidecar process (proven to run in isolation), which is a substantial new subsystem (persistent helper binary, IPC, lifecycle, Tauri `externalBin` bundling + codesign, model-download UX) with a CI signing/bundling path that can't be validated locally before pushing — too much risk for this release. A secondary finding: the 0.5B model wrapped its output in a ```` ```php ```` code fence (weak instruction-following), so Tier 3 would also need the 1.5B variant + output sanitization + prompt tuning. Working inference code is parked for the sidecar effort.

**Status:** active

**References:** branch `feat/post-model-correction`; commit removing in-process Tier 3; scratchpad `t3probe` (standalone proof) + `correction_model.rs.sidecar-ref` (parked impl).

---

## 2026-06-23: Post-model correction layer — 3 tiers, local-only Tier 3, no routing

**Status (Tier 3 portion):** superseded by 2026-06-23 (in-process Tier 3 abandoned). Tiers 1–2 portion active.

**Decision:** Add a post-model TEXT correction layer that runs on every ASR backend, beside the existing cleanup + voice-command passes. Tier 1 = exact spoken→written term map (Aho-Corasick, single pass). Tier 2 = sounds-like match (Metaphone phonetic key + edit-distance, confidence cutoff, fires only near vocab). Both no-LLM, built on settings-change, target <1ms, logged as a `correction_ms` telemetry phase; one unified vocab config feeds both. Tier 3 = optional model cleanup using a **100% local** model only: Qwen2.5-1.5B-Instruct (Q4_K_M GGUF) via `llama-cpp-2` + Metal (Apache-2.0), with Qwen2.5-0.5B-Instruct as an optional "fast mode." The Tier 3 backend is a trait so a future BYO-OpenAI-compatible cloud option can plug in if approved. Smart per-input routing is dropped in favor of a single configurable backend.

**Rationale:** Vocab previously fed Whisper's `initial_prompt`, a silent no-op on the default Parakeet/sherpa engine (parakeet.rs:208) — moving correction post-model fixes it for every engine and is the only place that can do camelCase/abbrev orthography. Cursor "Composer 2.5 Fast" was the only permitted cloud model, but live probing of `api.cursor.com` with a valid Admin-scoped key confirmed there is NO headless chat/completions endpoint: `/v1/me`, `/v1/models`, `/v0/agents` return 200 but only expose account/model-metadata/repo-bound Cloud Agents; every inference verb (`/v1/chat/completions`, `/v1/responses`, `/v1/completions`, `/v1/chat`, `/v1/messages`, `/v1/generate`) 404s. Composer has "no external API." Per the "no substitute" directive, cloud is dropped (kept as a trait seam). Routing was dropped because its "large input → bigger model = faster" premise only held for cloud; locally a bigger model is *slower* on long input and multiple resident models waste RAM. Mitigations: optional 0.5B fast-mode + a length-guard that skips Tier 3 on very long inputs.

**Status:** active

**References:** branch `feat/post-model-correction`; parakeet.rs:208; recording.rs (post-transcription pipeline, insertion after voice_commands); Qwen2.5 GGUF (Apache-2.0) via `llama-cpp-2`; Cursor probe — `/v1/me`,`/v1/models`,`/v0/agents` = 200, all inference verbs = 404.

---
