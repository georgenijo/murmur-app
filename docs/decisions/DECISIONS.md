# Decisions Log

Running log of architectural, scope, and process decisions for this project. Newest entries at the top. Each entry is short — for deep rationale on a single locked decision, write an ADR alongside in `docs/decisions/YYYY-MM-DD-*.md` and reference it here.

Maintained via the `/decisions` skill. See `~/.claude/skills/decisions/SKILL.md` for the entry format and invocation rules.

---

## 2026-06-23: Post-model correction layer — 3 tiers, local-only Tier 3, no routing

**Decision:** Add a post-model TEXT correction layer that runs on every ASR backend, beside the existing cleanup + voice-command passes. Tier 1 = exact spoken→written term map (Aho-Corasick, single pass). Tier 2 = sounds-like match (Metaphone phonetic key + edit-distance, confidence cutoff, fires only near vocab). Both no-LLM, built on settings-change, target <1ms, logged as a `correction_ms` telemetry phase; one unified vocab config feeds both. Tier 3 = optional model cleanup using a **100% local** model only: Qwen2.5-1.5B-Instruct (Q4_K_M GGUF) via `llama-cpp-2` + Metal (Apache-2.0), with Qwen2.5-0.5B-Instruct as an optional "fast mode." The Tier 3 backend is a trait so a future BYO-OpenAI-compatible cloud option can plug in if approved. Smart per-input routing is dropped in favor of a single configurable backend.

**Rationale:** Vocab previously fed Whisper's `initial_prompt`, a silent no-op on the default Parakeet/sherpa engine (parakeet.rs:208) — moving correction post-model fixes it for every engine and is the only place that can do camelCase/abbrev orthography. Cursor "Composer 2.5 Fast" was the only permitted cloud model, but live probing of `api.cursor.com` with a valid Admin-scoped key confirmed there is NO headless chat/completions endpoint: `/v1/me`, `/v1/models`, `/v0/agents` return 200 but only expose account/model-metadata/repo-bound Cloud Agents; every inference verb (`/v1/chat/completions`, `/v1/responses`, `/v1/completions`, `/v1/chat`, `/v1/messages`, `/v1/generate`) 404s. Composer has "no external API." Per the "no substitute" directive, cloud is dropped (kept as a trait seam). Routing was dropped because its "large input → bigger model = faster" premise only held for cloud; locally a bigger model is *slower* on long input and multiple resident models waste RAM. Mitigations: optional 0.5B fast-mode + a length-guard that skips Tier 3 on very long inputs.

**Status:** active

**References:** branch `feat/post-model-correction`; parakeet.rs:208; recording.rs (post-transcription pipeline, insertion after voice_commands); Qwen2.5 GGUF (Apache-2.0) via `llama-cpp-2`; Cursor probe — `/v1/me`,`/v1/models`,`/v0/agents` = 200, all inference verbs = 404.

---
