#!/usr/bin/env python3
"""Local accuracy corpus, metrics, and speech-model A/B for Murmur (issue #827).

Subcommands:

* ``export`` — opt-in, local-only copy of saved dictation recordings whose
  history text the user corrected into an accuracy corpus.
* ``synth`` — build a tiny synthetic corpus with macOS ``say`` and known text.
* ``score`` — compute WER, punctuation F1 (``. , ? -``), sentence-start
  capitalization, and named-term/homophone hit rate for a benchmark report.
* ``ab`` — run the internal headless benchmark over the documented model arms
  and print one content-free table per model with latency.

Nothing here uploads anything. Corpora and raw reports contain transcript
text and stay in Application Support; printed summaries contain only counts,
rates, and latencies.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from typing import Any, Iterable
import wave


REPO_ROOT = Path(__file__).resolve().parents[1]
APP_SUPPORT = Path.home() / "Library" / "Application Support"
DEFAULT_CORPUS_DIR = APP_SUPPORT / "Murmur Accuracy Corpus" / "v1"
DEFAULT_REPORT_DIR = APP_SUPPORT / "Murmur Bench" / "accuracy-reports"
DEFAULT_AUDIO_DIR = Path.home() / "Documents" / "Murmur"
CORPUS_ID = "accuracy-v1"
SAMPLE_RATE = 16_000
MAX_REFERENCE_BYTES = 4_000
PUNCTUATION_MARKS = (".", ",", "?", "-")
SENTENCE_TERMINATORS = frozenset(".?!")
# A default recommendation needs real speech; below this many real corrected
# clips the A/B only reports numbers.
MIN_RECOMMENDATION_FIXTURES = 20

# Model arms the issue asks to compare. `model` is a runtime catalog ID; arms
# without one are reported as not runnable with the documented reason.
ARMS: dict[str, dict[str, str | None]] = {
    "coreml-v3-auto": {
        "label": "Parakeet TDT 0.6b v3 Core ML (language auto)",
        "model": "parakeet-tdt-0.6b-v3-coreml",
        "note": "Production default backend; FluidAudio detects language itself.",
    },
    "coreml-v3-en": {
        "label": "Parakeet TDT 0.6b v3 Core ML (language en)",
        "model": None,
        "note": (
            "Not runnable: the pinned fluidaudio-rs transcribe_samples API takes "
            "no language hint and Murmur's Core ML backend ignores `language`, "
            "so this arm would be byte-identical to language auto."
        ),
    },
    "sherpa-v2-en": {
        "label": "Parakeet TDT 0.6b v2 English (sherpa-onnx CPU)",
        "model": "parakeet-tdt-0.6b-v2-fp16",
        "note": "English-only Parakeet v2 through the sherpa-onnx backend.",
    },
    "unified-en": {
        "label": "Parakeet Unified English 0.6b (Hex GGUF)",
        "model": None,
        "note": (
            "Not runnable: handy-computer/parakeet-unified-en-0.6b-gguf needs a "
            "GGUF Parakeet runtime; Murmur has no such backend."
        ),
    },
}
DEFAULT_ARMS = tuple(ARMS)

# Synthetic prompts: punctuation, sentence starts, and the homophone/name
# classes from the issue. `terms` must occur verbatim in `reference`.
SYNTHETIC_PROMPTS: tuple[dict[str, Any], ...] = (
    {
        "id": "syn-queue-claude",
        "reference": "Queue an update for Claude. Then check the CPU usage, please.",
        "terms": ["Queue", "Claude", "CPU"],
    },
    {
        "id": "syn-question",
        "reference": "Did you open the settings? No, I opened the OpenCode docs instead.",
        "terms": ["OpenCode"],
    },
    {
        "id": "syn-hyphen",
        "reference": "The real-time preview works, and the follow-up fix shipped today.",
        "terms": ["real-time", "follow-up"],
    },
    {
        "id": "syn-homophones",
        "reference": "Their team moved the meeting to Friday. It's at noon, not two.",
        "terms": ["Their", "Friday"],
    },
    {
        "id": "syn-names",
        "reference": "Send the Kubernetes logs to GitHub, then restart Postgres.",
        "terms": ["Kubernetes", "GitHub", "Postgres"],
    },
)


# --- Tokenization and alignment ---------------------------------------------

_WORD = re.compile(r"[A-Za-z0-9]+(?:['’][A-Za-z0-9]+)*")


@dataclasses.dataclass(frozen=True)
class Token:
    raw: str
    norm: str
    mark: str | None
    ends_sentence: bool


def tokenize(text: str) -> list[Token]:
    """Split text into words, attaching the scored punctuation mark that
    follows each word (first of ``. , ? -`` in the separator, if any)."""
    matches = list(_WORD.finditer(text))
    result: list[Token] = []
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        separator = text[match.end() : end]
        mark = next((c for c in separator if c in PUNCTUATION_MARKS), None)
        ends_sentence = any(c in SENTENCE_TERMINATORS for c in separator) and (
            index + 1 == len(matches) or any(c.isspace() for c in separator)
        )
        raw = match.group().replace("’", "'")
        result.append(Token(raw=raw, norm=raw.lower(), mark=mark, ends_sentence=ends_sentence))
    return result


def align(reference: list[Token], hypothesis: list[Token]) -> list[tuple[int | None, int | None]]:
    """Levenshtein alignment on normalized words with a deterministic
    backtrace (match/substitution, then deletion, then insertion)."""
    rows, cols = len(reference), len(hypothesis)
    cost = [[0] * (cols + 1) for _ in range(rows + 1)]
    for i in range(rows + 1):
        cost[i][0] = i
    for j in range(cols + 1):
        cost[0][j] = j
    for i in range(1, rows + 1):
        for j in range(1, cols + 1):
            substitution = int(reference[i - 1].norm != hypothesis[j - 1].norm)
            cost[i][j] = min(
                cost[i - 1][j - 1] + substitution,
                cost[i - 1][j] + 1,
                cost[i][j - 1] + 1,
            )
    ops: list[tuple[int | None, int | None]] = []
    i, j = rows, cols
    while i > 0 or j > 0:
        if i > 0 and j > 0:
            substitution = int(reference[i - 1].norm != hypothesis[j - 1].norm)
            if cost[i][j] == cost[i - 1][j - 1] + substitution:
                ops.append((i - 1, j - 1))
                i, j = i - 1, j - 1
                continue
        if i > 0 and cost[i][j] == cost[i - 1][j] + 1:
            ops.append((i - 1, None))
            i -= 1
        else:
            ops.append((None, j - 1))
            j -= 1
    ops.reverse()
    return ops


# --- Metrics -----------------------------------------------------------------


@dataclasses.dataclass
class PunctuationCounts:
    tp: dict[str, int] = dataclasses.field(default_factory=lambda: dict.fromkeys(PUNCTUATION_MARKS, 0))
    fp: dict[str, int] = dataclasses.field(default_factory=lambda: dict.fromkeys(PUNCTUATION_MARKS, 0))
    fn: dict[str, int] = dataclasses.field(default_factory=lambda: dict.fromkeys(PUNCTUATION_MARKS, 0))

    def add(self, other: "PunctuationCounts") -> None:
        for mark in PUNCTUATION_MARKS:
            self.tp[mark] += other.tp[mark]
            self.fp[mark] += other.fp[mark]
            self.fn[mark] += other.fn[mark]

    @staticmethod
    def _f1(tp: int, fp: int, fn: int) -> float | None:
        denominator = 2 * tp + fp + fn
        return None if denominator == 0 else 2 * tp / denominator

    def micro_f1(self) -> float | None:
        return self._f1(sum(self.tp.values()), sum(self.fp.values()), sum(self.fn.values()))

    def per_mark_f1(self) -> dict[str, float | None]:
        return {mark: self._f1(self.tp[mark], self.fp[mark], self.fn[mark]) for mark in PUNCTUATION_MARKS}


def punctuation_counts(reference: str, hypothesis: str) -> PunctuationCounts:
    """Score the mark after each aligned word. A matching mark is a true
    positive; a missing or different reference mark is a false negative; an
    extra or different hypothesis mark is a false positive."""
    ref, hyp = tokenize(reference), tokenize(hypothesis)
    counts = PunctuationCounts()
    for ref_index, hyp_index in align(ref, hyp):
        ref_mark = ref[ref_index].mark if ref_index is not None else None
        hyp_mark = hyp[hyp_index].mark if hyp_index is not None else None
        if ref_mark is not None and ref_mark == hyp_mark:
            counts.tp[ref_mark] += 1
            continue
        if ref_mark is not None:
            counts.fn[ref_mark] += 1
        if hyp_mark is not None:
            counts.fp[hyp_mark] += 1
    return counts


def sentence_start_capitalization(reference: str, hypothesis: str) -> tuple[int, int]:
    """Return (correct, total) over reference sentence starts that begin with a
    letter. A start is correct when its aligned hypothesis word is capitalized."""
    ref, hyp = tokenize(reference), tokenize(hypothesis)
    starts = {0} | {index + 1 for index, token in enumerate(ref[:-1]) if token.ends_sentence}
    starts = {index for index in starts if index < len(ref) and ref[index].raw[0].isalpha()}
    correct = 0
    for ref_index, hyp_index in align(ref, hyp):
        if ref_index in starts and hyp_index is not None and hyp[hyp_index].raw[0].isupper():
            correct += 1
    return correct, len(starts)


def _count_sequence(haystack: list[str], needle: list[str]) -> int:
    count, index = 0, 0
    while needle and index + len(needle) <= len(haystack):
        if haystack[index : index + len(needle)] == needle:
            count += 1
            index += len(needle)
        else:
            index += 1
    return count


@dataclasses.dataclass
class TermCounts:
    expected: int = 0
    hits: int = 0
    exact_case_hits: int = 0
    terms_missing_from_reference: int = 0

    def add(self, other: "TermCounts") -> None:
        self.expected += other.expected
        self.hits += other.hits
        self.exact_case_hits += other.exact_case_hits
        self.terms_missing_from_reference += other.terms_missing_from_reference


def term_hits(reference: str, hypothesis: str, terms: Iterable[str]) -> TermCounts:
    """Named-term/homophone hit rate. Each term occurrence in the reference is
    expected once; it is a hit when the hypothesis contains the same word
    sequence (case-insensitive), and an exact-case hit when spelling and case
    also match. Hits per term are capped at the reference count."""
    ref, hyp = tokenize(reference), tokenize(hypothesis)
    ref_norm, hyp_norm = [t.norm for t in ref], [t.norm for t in hyp]
    ref_raw, hyp_raw = [t.raw for t in ref], [t.raw for t in hyp]
    counts = TermCounts()
    for term in dict.fromkeys(terms):
        term_tokens = tokenize(term)
        if not term_tokens:
            continue
        expected = _count_sequence(ref_norm, [t.norm for t in term_tokens])
        if expected == 0:
            counts.terms_missing_from_reference += 1
            continue
        counts.expected += expected
        counts.hits += min(expected, _count_sequence(hyp_norm, [t.norm for t in term_tokens]))
        counts.exact_case_hits += min(
            _count_sequence(ref_raw, [t.raw for t in term_tokens]),
            _count_sequence(hyp_raw, [t.raw for t in term_tokens]),
        )
    return counts


def word_errors(reference: str, hypothesis: str) -> tuple[int, int]:
    """Raw WER numerator/denominator over this module's tokenization."""
    ref, hyp = tokenize(reference), tokenize(hypothesis)
    errors = sum(
        1
        for ref_index, hyp_index in align(ref, hyp)
        if ref_index is None or hyp_index is None or ref[ref_index].norm != hyp[hyp_index].norm
    )
    return errors, len(ref)


def correction_terms(original: str, corrected: str) -> list[str]:
    """Words the user inserted or replaced when correcting `original`, grouped
    into contiguous phrases. These are the homophone/name targets."""
    # Case-sensitive on purpose: `github` -> `GitHub` is a name correction.
    before = [t.raw for t in tokenize(original)]
    after_tokens = tokenize(corrected)
    after = [t.raw for t in after_tokens]
    terms: list[str] = []
    matcher = difflib.SequenceMatcher(a=before, b=after, autojunk=False)
    for tag, _a0, _a1, b0, b1 in matcher.get_opcodes():
        if tag in ("replace", "insert") and b1 > b0:
            terms.append(" ".join(t.raw for t in after_tokens[b0:b1]))
    return list(dict.fromkeys(terms))


# --- Report scoring -----------------------------------------------------------


def _rate(numerator: int, denominator: int) -> float | None:
    return None if denominator == 0 else numerator / denominator


def load_corpus_terms(corpus_dir: Path | None) -> dict[str, list[str]]:
    if corpus_dir is None:
        return {}
    manifest_path = corpus_dir / "manifest.json"
    if not manifest_path.is_file():
        return {}
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    terms: dict[str, list[str]] = {}
    for recording in manifest.get("recordings") or []:
        if isinstance(recording, dict) and isinstance(recording.get("promptId"), str):
            values = recording.get("terms") or []
            terms[recording["promptId"]] = [v for v in values if isinstance(v, str)]
    return terms


def score_text_pair(reference: str, hypothesis: str, terms: list[str]) -> dict[str, Any]:
    punctuation = punctuation_counts(reference, hypothesis)
    caps_correct, caps_total = sentence_start_capitalization(reference, hypothesis)
    hits = term_hits(reference, hypothesis, terms)
    return {
        "punctuation": punctuation,
        "caps": (caps_correct, caps_total),
        "terms": hits,
    }


def _summarize(punctuation: PunctuationCounts, caps: tuple[int, int], terms: TermCounts) -> dict[str, Any]:
    return {
        "punctuationF1": punctuation.micro_f1(),
        "punctuationF1ByMark": punctuation.per_mark_f1(),
        "punctuationCounts": {
            "tp": punctuation.tp,
            "fp": punctuation.fp,
            "fn": punctuation.fn,
        },
        "sentenceStartCapitalization": _rate(*caps),
        "sentenceStarts": caps[1],
        "termHitRate": _rate(terms.hits, terms.expected),
        "termExactCaseHitRate": _rate(terms.exact_case_hits, terms.expected),
        "termsExpected": terms.expected,
        "termsMissingFromReference": terms.terms_missing_from_reference,
    }


def score_report(
    report: dict[str, Any],
    corpus_terms: dict[str, list[str]],
    global_terms: list[str],
) -> dict[str, Any]:
    """Content-free accuracy summary for every successful model in a report."""
    models = []
    for result in report.get("results") or []:
        if not isinstance(result, dict):
            continue
        row: dict[str, Any] = {
            "modelName": result.get("modelName"),
            "label": result.get("label"),
            "error": result.get("error"),
        }
        fixtures = result.get("fixtures") or []
        if result.get("error") or not fixtures:
            models.append(row)
            continue
        totals = {
            "raw": [PunctuationCounts(), [0, 0], TermCounts()],
            "delivered": [PunctuationCounts(), [0, 0], TermCounts()],
        }
        word = {"wordErrors": 0, "referenceWords": 0, "normalizedWordErrors": 0,
                "normalizedReferenceWords": 0, "deliveredNormalizedWordErrors": 0}
        for fixture in fixtures:
            reference = fixture.get("reference") or ""
            terms = list(dict.fromkeys([*corpus_terms.get(fixture.get("fixtureId"), []), *global_terms]))
            for key, text in (("raw", fixture.get("transcript") or ""),
                              ("delivered", fixture.get("deliveredTranscript") or "")):
                scored = score_text_pair(reference, text, terms)
                totals[key][0].add(scored["punctuation"])
                totals[key][1][0] += scored["caps"][0]
                totals[key][1][1] += scored["caps"][1]
                totals[key][2].add(scored["terms"])
            for field in word:
                value = fixture.get(field)
                if isinstance(value, int) and not isinstance(value, bool):
                    word[field] += value
        row.update(
            {
                "fixtures": len(fixtures),
                "wordErrorRate": _rate(word["wordErrors"], word["referenceWords"]),
                "normalizedWordErrorRate": _rate(word["normalizedWordErrors"], word["normalizedReferenceWords"]),
                "deliveredNormalizedWordErrorRate": _rate(
                    word["deliveredNormalizedWordErrors"], word["normalizedReferenceWords"]
                ),
                "raw": _summarize(totals["raw"][0], tuple(totals["raw"][1]), totals["raw"][2]),
                "delivered": _summarize(
                    totals["delivered"][0], tuple(totals["delivered"][1]), totals["delivered"][2]
                ),
                "modelLoadMs": result.get("modelLoadMs"),
                "firstInferenceMs": result.get("firstInferenceMs"),
                "warmMedianMs": result.get("warmMedianMs"),
                "warmP95Ms": result.get("warmP95Ms"),
                "realtimeFactor": result.get("realtimeFactor"),
            }
        )
        models.append(row)
    corpus = report.get("corpus") or {}
    return {
        "schemaVersion": 1,
        "corpusSource": corpus.get("source"),
        "corpusProvenance": corpus.get("provenance"),
        "fixtureCount": corpus.get("fixtureCount"),
        "preset": report.get("preset"),
        "iterations": report.get("iterations"),
        "models": models,
    }


def recommend_default(summary: dict[str, Any], contains_real_user_data: bool) -> str:
    """Name a candidate default only for a real corpus of adequate size:
    lowest delivered normalized WER, then higher term hit rate, then higher
    delivered punctuation F1, then lower warm median latency."""
    if not contains_real_user_data:
        return "none — synthetic corpus; a default recommendation needs a real corrected-recording run"
    fixtures = summary.get("fixtureCount") or 0
    if fixtures < MIN_RECOMMENDATION_FIXTURES:
        return (
            f"none — {fixtures} real clips is below the {MIN_RECOMMENDATION_FIXTURES}-clip minimum "
            "for a default recommendation"
        )
    candidates = [m for m in summary["models"] if not m.get("error") and m.get("fixtures")]
    if not candidates:
        return "none — no successful model results"

    def lower_is_better(value: Any) -> float:
        return float("inf") if value is None else value

    def higher_is_better(value: Any) -> float:
        return 0.0 if value is None else -value

    def key(model: dict[str, Any]) -> tuple[float, float, float, float]:
        delivered = model["delivered"]
        return (
            lower_is_better(model.get("deliveredNormalizedWordErrorRate")),
            higher_is_better(delivered.get("termHitRate")),
            higher_is_better(delivered.get("punctuationF1")),
            lower_is_better(model.get("warmMedianMs")),
        )

    best = min(candidates, key=key)
    return f"{best['modelName']} (candidate; confirm before changing the shipped default)"


def _fmt(value: Any, *, percent: bool = False, digits: int = 1) -> str:
    if value is None:
        return "n/a"
    if percent:
        return f"{value * 100:.{digits}f}%"
    return f"{value:.{digits}f}"


def render_table(summary: dict[str, Any], arm_notes: list[tuple[str, str]]) -> str:
    header = (
        "| Model | Clips | nWER | Delivered nWER | Punct F1 (raw / delivered) | "
        "Sentence caps | Term hits (exact case) | Warm median ms | p95 ms | RTF | Load ms |"
    )
    lines = [header, "|" + "---|" * 11]
    for model in summary["models"]:
        if model.get("error") or not model.get("fixtures"):
            lines.append(f"| {model.get('modelName')} | error | | | | | | | | | |")
            continue
        raw, delivered = model["raw"], model["delivered"]
        lines.append(
            "| {name} | {clips} | {nwer} | {dwer} | {praw} / {pdel} | {caps} | {terms} ({exact}) | "
            "{median} | {p95} | {rtf} | {load} |".format(
                name=model["modelName"],
                clips=model["fixtures"],
                nwer=_fmt(model["normalizedWordErrorRate"], percent=True),
                dwer=_fmt(model["deliveredNormalizedWordErrorRate"], percent=True),
                praw=_fmt(raw["punctuationF1"], digits=3),
                pdel=_fmt(delivered["punctuationF1"], digits=3),
                caps=_fmt(delivered["sentenceStartCapitalization"], percent=True),
                terms=_fmt(delivered["termHitRate"], percent=True),
                exact=_fmt(delivered["termExactCaseHitRate"], percent=True),
                median=_fmt(model["warmMedianMs"]),
                p95=_fmt(model["warmP95Ms"]),
                rtf=_fmt(model["realtimeFactor"], digits=3),
                load=_fmt(model["modelLoadMs"]),
            )
        )
    if arm_notes:
        lines.append("")
        lines.append("Not run:")
        lines.extend(f"- {arm}: {note}" for arm, note in arm_notes)
    return "\n".join(lines)


# --- Corpus writing -----------------------------------------------------------


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def wav_info(path: Path) -> tuple[int, int, int, float] | None:
    """(sample rate, channels, sample width, seconds) or None if unreadable."""
    try:
        with wave.open(str(path), "rb") as handle:
            frames = handle.getnframes()
            rate = handle.getframerate()
            return rate, handle.getnchannels(), handle.getsampwidth(), frames / rate if rate else 0.0
    except (wave.Error, EOFError, OSError):
        return None


def _inside(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def prepare_output_dir(out: Path) -> Path:
    out = out.expanduser()
    if not out.is_absolute():
        raise ValueError("--out must be an absolute path")
    if _inside(out, REPO_ROOT):
        raise ValueError("refusing to write a corpus inside the Git repository")
    if out.exists() and any(out.iterdir()):
        raise ValueError(f"{out} already exists and is not empty; remove it or choose another --out")
    (out / "audio").mkdir(parents=True, exist_ok=True)
    os.chmod(out, 0o700)
    return out


def write_manifest(out: Path, source: str, real: bool, recordings: list[dict[str, Any]]) -> None:
    now = dt.datetime.now(dt.timezone.utc).isoformat()
    manifest = {
        "schemaVersion": 1,
        "corpusId": CORPUS_ID,
        "containsRealUserData": real,
        "source": source,
        "sampleRate": SAMPLE_RATE,
        "createdAt": now,
        "updatedAt": now,
        "recordings": recordings,
    }
    temporary = out / ".manifest.json.tmp"
    temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, out / "manifest.json")


def _entry(index: int, prompt_id: str, label: str, reference: str, file_name: str,
           path: Path, seconds: float, terms: list[str], recorded_at: str) -> dict[str, Any]:
    return {
        "entryId": f"{prompt_id}-take-01",
        "promptIndex": index,
        "promptId": prompt_id,
        "label": label,
        "reference": reference,
        "take": 1,
        "selected": True,
        "fileName": file_name,
        "sha256": _sha256(path),
        "recordedAt": recorded_at,
        "sampleRate": SAMPLE_RATE,
        "durationMs": int(round(seconds * 1000)),
        "terms": terms,
    }


_SAVED_WAV = re.compile(r"^murmur-(\d+)\.wav$")


@dataclasses.dataclass
class SavedRecording:
    path: Path
    mtime: float
    seconds: float
    delivered: str | None


def scan_saved_recordings(audio_dir: Path) -> tuple[list[SavedRecording], int]:
    """Find Save Audio outputs (`murmur-NNNN.wav` plus the optional delivered
    `murmur-NNNN.txt`)."""
    recordings, invalid = [], 0
    for path in sorted(audio_dir.iterdir()):
        if not _SAVED_WAV.match(path.name) or not path.is_file() or path.is_symlink():
            continue
        info = wav_info(path)
        if info is None or info[0] != SAMPLE_RATE or info[1] != 1 or info[2] != 2:
            invalid += 1
            continue
        transcript = path.with_suffix(".txt")
        recordings.append(
            SavedRecording(
                path=path,
                mtime=path.stat().st_mtime,
                seconds=info[3],
                delivered=transcript.read_text(encoding="utf-8").strip() if transcript.is_file() else None,
            )
        )
    return recordings, invalid


def pair_history(
    recordings: list[SavedRecording],
    entries: list[dict[str, Any]],
    max_skew_seconds: float,
) -> dict[int, dict[str, Any]]:
    """Pair recordings with history entries by completion time and duration.
    Ambiguous pairs (several candidates, or one entry claimed by several
    recordings) are dropped rather than guessed."""
    live = [
        e for e in entries
        if isinstance(e, dict) and e.get("source", "recording") == "recording"
        and isinstance(e.get("timestamp"), (int, float)) and isinstance(e.get("text"), str)
        and isinstance(e.get("durationSeconds"), (int, float))
    ]
    candidates: dict[int, list[int]] = {}
    for index, recording in enumerate(recordings):
        tolerance = max(1.0, 0.15 * recording.seconds)
        candidates[index] = [
            position for position, entry in enumerate(live)
            if abs(entry["timestamp"] / 1000.0 - recording.mtime) <= max_skew_seconds
            and abs(entry["durationSeconds"] - recording.seconds) <= tolerance
        ]
    claims: dict[int, int] = {}
    for matches in candidates.values():
        if len(matches) == 1:
            claims[matches[0]] = claims.get(matches[0], 0) + 1
    return {
        index: live[matches[0]]
        for index, matches in candidates.items()
        if len(matches) == 1 and claims[matches[0]] == 1
    }


def _normalized_text(text: str) -> str:
    return " ".join(text.split())


def export_corpus(args: argparse.Namespace) -> int:
    audio_dir = args.audio_dir.expanduser()
    if not audio_dir.is_dir():
        raise ValueError(f"{audio_dir} is not a directory (enable Save audio and Save transcript first)")
    entries: list[dict[str, Any]] = []
    if args.history:
        history = json.loads(args.history.expanduser().read_text(encoding="utf-8"))
        if not isinstance(history, dict) or history.get("schema") != "murmur.history.v2":
            raise ValueError("--history must be a Murmur History JSON export (murmur.history.v2)")
        entries = history.get("entries") or []
    recordings, invalid = scan_saved_recordings(audio_dir)
    pairs = pair_history(recordings, entries, args.max_skew_seconds)

    skipped = {"invalidAudio": invalid, "unpaired": 0, "uncorrected": 0, "noDeliveredText": 0, "invalidReference": 0}
    selected: list[tuple[SavedRecording, str, str]] = []
    for index, recording in enumerate(recordings):
        entry = pairs.get(index)
        if entry is None:
            skipped["unpaired"] += 1
            continue
        if recording.delivered is None:
            skipped["noDeliveredText"] += 1
            continue
        corrected = entry["text"].strip()
        if _normalized_text(corrected) == _normalized_text(recording.delivered):
            skipped["uncorrected"] += 1
            continue
        selected.append((recording, corrected, recording.delivered))

    valid = []
    for recording, reference, original in selected:
        if not reference.strip() or len(reference.encode("utf-8")) > MAX_REFERENCE_BYTES:
            skipped["invalidReference"] += 1
            continue
        valid.append((recording, reference, original))

    summary = {"scanned": len(recordings), "exported": len(valid), "skipped": skipped}
    if args.dry_run or not valid:
        print(json.dumps(summary, indent=2))
        if not valid:
            print("Nothing to export.", file=sys.stderr)
        return 0

    out = prepare_output_dir(args.out)
    manifest_entries = []
    for number, (recording, reference, original) in enumerate(valid, start=1):
        prompt_id = f"corrected-{number:04d}"
        file_name = f"{prompt_id}.wav"
        destination = out / "audio" / file_name
        shutil.copyfile(recording.path, destination)
        terms = correction_terms(original, reference)
        recorded_at = dt.datetime.fromtimestamp(recording.mtime, dt.timezone.utc).isoformat()
        manifest_entries.append(
            _entry(number, prompt_id, f"Corrected recording {number}", reference,
                   file_name, destination, recording.seconds, terms, recorded_at)
        )
    write_manifest(out, "corrected-history-export", True, manifest_entries)
    print(json.dumps(summary, indent=2))
    print(f"Corpus: {out}")
    return 0


def synth_corpus(args: argparse.Namespace) -> int:
    out = prepare_output_dir(args.out)
    entries = []
    with tempfile.TemporaryDirectory() as scratch:
        for number, prompt in enumerate(SYNTHETIC_PROMPTS, start=1):
            aiff = Path(scratch) / f"{prompt['id']}.aiff"
            file_name = f"{prompt['id']}.wav"
            destination = out / "audio" / file_name
            subprocess.run(["say", "-v", args.voice, "-o", str(aiff), prompt["reference"]], check=True)
            subprocess.run(
                ["afconvert", "-f", "WAVE", "-d", f"LEI16@{SAMPLE_RATE}", "-c", "1", str(aiff), str(destination)],
                check=True,
            )
            info = wav_info(destination)
            if info is None:
                raise ValueError("afconvert produced an unreadable WAV")
            entries.append(
                _entry(number, prompt["id"], f"Synthetic {number}", prompt["reference"], file_name,
                       destination, info[3], list(prompt["terms"]),
                       dt.datetime.now(dt.timezone.utc).isoformat())
            )
    write_manifest(out, "synthetic-say", False, entries)
    print(f"Synthetic corpus ({len(entries)} clips): {out}")
    return 0


def _read_terms_file(path: Path | None) -> list[str]:
    if path is None:
        return []
    return [line.strip() for line in path.expanduser().read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.startswith("#")]


def _write_summary(summary: dict[str, Any], output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")


def _corpus_is_real(corpus_dir: Path | None, report: dict[str, Any]) -> bool:
    if corpus_dir is not None and (corpus_dir / "manifest.json").is_file():
        manifest = json.loads((corpus_dir / "manifest.json").read_text(encoding="utf-8"))
        return manifest.get("containsRealUserData") is True
    return (report.get("corpus") or {}).get("source") == "personal"


def score_command(args: argparse.Namespace) -> int:
    report_path = args.report.expanduser().resolve()
    report = json.loads(report_path.read_text(encoding="utf-8"))
    corpus_dir = args.corpus_dir.expanduser() if args.corpus_dir else None
    summary = score_report(report, load_corpus_terms(corpus_dir), _read_terms_file(args.terms))
    summary["recommendedDefault"] = recommend_default(summary, _corpus_is_real(corpus_dir, report))
    output = args.output or report_path.with_suffix(".accuracy.json")
    _write_summary(summary, output)
    print(render_table(summary, []))
    print(f"\nRecommended default: {summary['recommendedDefault']}")
    print(f"Summary: {output}")
    return 0


def resolve_arms(names: list[str]) -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    """Split requested arms into (arm, model) runs and (arm, reason) skips.
    Unknown names are treated as raw runtime model IDs."""
    runs, skips = [], []
    for name in names:
        arm = ARMS.get(name)
        if arm is None:
            runs.append((name, name))
        elif arm["model"] is None:
            skips.append((name, str(arm["note"])))
        else:
            runs.append((name, str(arm["model"])))
    if len({model for _, model in runs}) != len(runs):
        raise ValueError("two arms resolve to the same model")
    return runs, skips


def ab_command(args: argparse.Namespace) -> int:
    names = [name.strip() for name in args.arms.split(",") if name.strip()]
    runs, skips = resolve_arms(names)
    if not runs:
        raise ValueError("no runnable arms selected")
    corpus_dir = args.corpus_dir.expanduser().resolve()
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    report_path = (args.output_dir.expanduser() / f"{stamp}-{args.corpus}-ab.json").resolve()
    if _inside(report_path, REPO_ROOT):
        raise ValueError("refusing to write a raw report inside the Git repository")

    sys.path.insert(0, str(REPO_ROOT))
    from scripts import murmur_bench  # noqa: PLC0415 - reuse the release runner

    bench_args = argparse.Namespace(
        repo=REPO_ROOT,
        output=report_path,
        corpus=args.corpus,
        corpus_dir=corpus_dir,
        preset=args.preset,
        models=",".join(model for _, model in runs),
        machine_label=args.machine_label,
    )
    murmur_bench.run_benchmark(bench_args)
    report = json.loads(report_path.read_text(encoding="utf-8"))
    terms_dir = corpus_dir if args.corpus == "accuracy" else None
    summary = score_report(report, load_corpus_terms(terms_dir), _read_terms_file(args.terms))
    summary["arms"] = [{"arm": arm, "model": model} for arm, model in runs]
    summary["notRunnable"] = [{"arm": arm, "reason": reason} for arm, reason in skips]
    summary["recommendedDefault"] = recommend_default(summary, _corpus_is_real(terms_dir, report))
    output = report_path.with_suffix(".accuracy.json")
    _write_summary(summary, output)
    print()
    print(render_table(summary, skips))
    print(f"\nRecommended default: {summary['recommendedDefault']}")
    print(f"Content-free summary: {output}")
    print(f"Raw report (contains transcript text; keep local): {report_path}")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = root.add_subparsers(dest="command", required=True)

    export = commands.add_parser("export", help="opt-in local export of corrected recordings")
    export.add_argument("--audio-dir", type=Path, default=DEFAULT_AUDIO_DIR,
                        help="Save audio/transcript output folder (default ~/Documents/Murmur)")
    export.add_argument("--history", type=Path, help="History JSON export (History → Export → JSON)")
    export.add_argument("--out", type=Path, default=DEFAULT_CORPUS_DIR)
    export.add_argument("--max-skew-seconds", type=float, default=20.0)
    export.add_argument("--dry-run", action="store_true", help="print counts only; copy nothing")
    export.set_defaults(handler=export_corpus)

    synth = commands.add_parser("synth", help="build a synthetic `say` corpus with known text")
    synth.add_argument("--out", type=Path, required=True)
    synth.add_argument("--voice", default="Samantha")
    synth.set_defaults(handler=synth_corpus)

    score = commands.add_parser("score", help="score an existing benchmark report")
    score.add_argument("report", type=Path)
    score.add_argument("--corpus-dir", type=Path, help="accuracy corpus whose manifest lists terms")
    score.add_argument("--terms", type=Path, help="extra named terms, one per line")
    score.add_argument("--output", type=Path)
    score.set_defaults(handler=score_command)

    ab = commands.add_parser("ab", help="run the speech-model A/B and print the report table")
    ab.add_argument("--corpus", choices=("accuracy", "personal"), default="accuracy")
    ab.add_argument("--corpus-dir", type=Path, default=DEFAULT_CORPUS_DIR)
    ab.add_argument("--arms", default=",".join(DEFAULT_ARMS),
                    help=f"comma-separated arms or model IDs (default: {','.join(DEFAULT_ARMS)})")
    ab.add_argument("--preset", choices=("quick", "standard", "thorough"), default="standard")
    ab.add_argument("--terms", type=Path, help="extra named terms, one per line")
    ab.add_argument("--output-dir", type=Path, default=DEFAULT_REPORT_DIR)
    ab.add_argument("--machine-label", default=None)
    ab.set_defaults(handler=ab_command)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return args.handler(args)
    except (OSError, subprocess.CalledProcessError, ValueError, json.JSONDecodeError) as error:
        print(f"murmur-accuracy: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
