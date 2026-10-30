from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
import wave

from scripts import murmur_accuracy as accuracy


def write_wav(path: Path, seconds: float, rate: int = 16_000) -> None:
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(b"\x00\x00" * int(seconds * rate))


class TokenizeTests(unittest.TestCase):
    def test_marks_attach_to_the_preceding_word(self) -> None:
        tokens = accuracy.tokenize("Hi, real-time preview? Yes. It's done")
        self.assertEqual([t.norm for t in tokens], ["hi", "real", "time", "preview", "yes", "it's", "done"])
        self.assertEqual([t.mark for t in tokens], [",", "-", None, "?", ".", None, None])
        self.assertEqual([t.ends_sentence for t in tokens], [False, False, False, True, True, False, False])

    def test_decimal_point_is_not_a_sentence_end(self) -> None:
        tokens = accuracy.tokenize("Version 3.5 shipped.")
        self.assertFalse(tokens[1].ends_sentence)
        self.assertTrue(tokens[-1].ends_sentence)


class PunctuationTests(unittest.TestCase):
    def test_identical_text_scores_one(self) -> None:
        counts = accuracy.punctuation_counts("Hi, there. Ready?", "Hi, there. Ready?")
        self.assertEqual(counts.micro_f1(), 1.0)

    def test_missing_and_wrong_marks(self) -> None:
        # Reference marks: "," "." "?" ; hypothesis keeps "," drops "." and
        # turns "?" into "." -> tp=1, fn=2 ("." and "?"), fp=1 (".").
        counts = accuracy.punctuation_counts("Hi, there. Ready?", "Hi, there ready.")
        self.assertEqual(sum(counts.tp.values()), 1)
        self.assertEqual(sum(counts.fn.values()), 2)
        self.assertEqual(sum(counts.fp.values()), 1)
        self.assertAlmostEqual(counts.micro_f1(), 2 / 5)
        self.assertEqual(counts.per_mark_f1()["?"], 0.0)
        self.assertEqual(counts.per_mark_f1()[","], 1.0)

    def test_hyphen_is_scored(self) -> None:
        counts = accuracy.punctuation_counts("real-time", "real time")
        self.assertEqual(counts.fn["-"], 1)
        self.assertEqual(counts.micro_f1(), 0.0)

    def test_no_marks_anywhere_is_undefined(self) -> None:
        self.assertIsNone(accuracy.punctuation_counts("hello there", "hello there").micro_f1())

    def test_inserted_word_marks_are_false_positives(self) -> None:
        counts = accuracy.punctuation_counts("go now", "go, um, now")
        self.assertEqual(sum(counts.fp.values()), 2)
        self.assertEqual(sum(counts.tp.values()), 0)


class CapitalizationTests(unittest.TestCase):
    def test_counts_reference_sentence_starts(self) -> None:
        self.assertEqual(
            accuracy.sentence_start_capitalization("Queue it. Then go? Yes", "queue it. Then go? yes"),
            (1, 3),
        )

    def test_deleted_start_is_a_miss_and_digit_start_is_skipped(self) -> None:
        self.assertEqual(accuracy.sentence_start_capitalization("Hello. 5 cats", "cats"), (0, 1))


class TermHitTests(unittest.TestCase):
    def test_homophones_and_names(self) -> None:
        counts = accuracy.term_hits(
            "Queue an update for Claude on the CPU.",
            "cue an update for claude on the CPU.",
            ["Queue", "Claude", "CPU"],
        )
        self.assertEqual(counts.expected, 3)
        self.assertEqual(counts.hits, 2)  # claude (case-insensitive) + CPU
        self.assertEqual(counts.exact_case_hits, 1)  # only CPU

    def test_multiword_terms_repeats_and_missing_terms(self) -> None:
        counts = accuracy.term_hits(
            "open code and OpenCode and open code",
            "open code and open codes and open code open code",
            ["open code", "OpenCode", "never said"],
        )
        # "open code" expected twice; hypothesis has it three times -> capped at 2.
        # "OpenCode" expected once; missed.
        self.assertEqual(counts.expected, 3)
        self.assertEqual(counts.hits, 2)
        self.assertEqual(counts.terms_missing_from_reference, 1)


class WordErrorTests(unittest.TestCase):
    def test_word_errors(self) -> None:
        self.assertEqual(accuracy.word_errors("the cat sat", "the bat sat down"), (2, 3))
        self.assertEqual(accuracy.word_errors("A, b.", "a b"), (0, 2))


class CorrectionTermsTests(unittest.TestCase):
    def test_extracts_replaced_and_inserted_phrases(self) -> None:
        self.assertEqual(
            accuracy.correction_terms(
                "cue an update for basic cloud information",
                "Queue an update for basic Claude information",
            ),
            ["Queue", "Claude"],
        )
        self.assertEqual(
            accuracy.correction_terms("check the CPO size", "check the CPU size"), ["CPU"]
        )
        self.assertEqual(
            accuracy.correction_terms("open codes design", "OpenCode's design"), ["OpenCode's"]
        )
        self.assertEqual(
            accuracy.correction_terms("push to github today", "push to GitHub today"), ["GitHub"]
        )
        self.assertEqual(accuracy.correction_terms("same text", "same text"), [])


def fixture(reference: str, transcript: str, delivered: str, fixture_id: str = "syn-a") -> dict:
    ref_words = len(reference.split())
    return {
        "fixtureId": fixture_id,
        "reference": reference,
        "transcript": transcript,
        "deliveredTranscript": delivered,
        "wordErrors": 1,
        "referenceWords": ref_words,
        "normalizedWordErrors": 1,
        "normalizedReferenceWords": ref_words,
        "deliveredNormalizedWordErrors": 0,
    }


class ScoreReportTests(unittest.TestCase):
    def report(self) -> dict:
        return {
            "preset": "standard",
            "iterations": 1,
            "corpus": {"source": "accuracy", "fixtureCount": 1, "provenance": "Synthetic"},
            "results": [
                {
                    "modelName": "parakeet-tdt-0.6b-v3-coreml",
                    "label": "Parakeet Core ML",
                    "warmMedianMs": 120.0,
                    "warmP95Ms": 140.0,
                    "realtimeFactor": 0.05,
                    "modelLoadMs": 900.0,
                    "fixtures": [
                        fixture(
                            "Queue it for Claude. Done?",
                            "cue it for claude done",
                            "Queue it for Claude. Done?",
                        )
                    ],
                },
                {"modelName": "base.en", "error": "not installed", "fixtures": []},
            ],
        }

    def test_scores_raw_and_delivered_without_copying_text(self) -> None:
        summary = accuracy.score_report(self.report(), {"syn-a": ["Queue", "Claude"]}, ["Done"])
        model = summary["models"][0]
        self.assertEqual(model["fixtures"], 1)
        self.assertEqual(model["delivered"]["punctuationF1"], 1.0)
        self.assertEqual(model["raw"]["punctuationF1"], 0.0)
        self.assertEqual(model["delivered"]["termHitRate"], 1.0)
        self.assertAlmostEqual(model["raw"]["termHitRate"], 2 / 3)
        self.assertEqual(model["raw"]["sentenceStartCapitalization"], 0.0)
        self.assertEqual(model["deliveredNormalizedWordErrorRate"], 0.0)
        self.assertEqual(model["normalizedWordErrorRate"], 1 / 5)
        self.assertEqual(summary["models"][1]["error"], "not installed")
        serialized = json.dumps(summary)
        for text in ("Queue it", "claude done", "Claude"):
            self.assertNotIn(text, serialized)
        table = accuracy.render_table(summary, [("unified-en", "Not runnable: no backend")])
        self.assertIn("parakeet-tdt-0.6b-v3-coreml", table)
        self.assertIn("- unified-en: Not runnable", table)
        self.assertNotIn("Queue it", table)

    def test_recommendation_requires_a_real_corpus_of_adequate_size(self) -> None:
        summary = accuracy.score_report(self.report(), {}, [])
        self.assertTrue(accuracy.recommend_default(summary, False).startswith("none — synthetic"))
        self.assertTrue(accuracy.recommend_default(summary, True).startswith("none — 1 real clips"))
        summary["fixtureCount"] = 25
        self.assertTrue(
            accuracy.recommend_default(summary, True).startswith("parakeet-tdt-0.6b-v3-coreml")
        )

    def test_a_perfect_wer_wins_the_recommendation(self) -> None:
        def model(name: str, wer: float, median: float) -> dict:
            return {
                "modelName": name,
                "fixtures": 25,
                "deliveredNormalizedWordErrorRate": wer,
                "warmMedianMs": median,
                "delivered": {"termHitRate": 0.5, "punctuationF1": 0.5},
            }

        summary = {"fixtureCount": 25, "models": [model("slow-perfect", 0.0, 900.0), model("fast", 0.1, 50.0)]}
        self.assertTrue(accuracy.recommend_default(summary, True).startswith("slow-perfect"))


class ArmTests(unittest.TestCase):
    def test_documented_arms_split_into_runs_and_skips(self) -> None:
        runs, skips = accuracy.resolve_arms(list(accuracy.DEFAULT_ARMS))
        self.assertEqual(
            runs,
            [
                ("coreml-v3-auto", "parakeet-tdt-0.6b-v3-coreml"),
                ("sherpa-v2-en", "parakeet-tdt-0.6b-v2-fp16"),
            ],
        )
        self.assertEqual([arm for arm, _ in skips], ["coreml-v3-en", "unified-en"])

    def test_raw_model_ids_and_duplicate_models(self) -> None:
        self.assertEqual(accuracy.resolve_arms(["base.en"])[0], [("base.en", "base.en")])
        with self.assertRaises(ValueError):
            accuracy.resolve_arms(["coreml-v3-auto", "parakeet-tdt-0.6b-v3-coreml"])

    def test_synthetic_terms_occur_in_their_references(self) -> None:
        for prompt in accuracy.SYNTHETIC_PROMPTS:
            counts = accuracy.term_hits(prompt["reference"], prompt["reference"], prompt["terms"])
            self.assertEqual(counts.terms_missing_from_reference, 0, prompt["id"])
            self.assertEqual(counts.exact_case_hits, counts.expected, prompt["id"])


class ExportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)
        self.audio = self.root / "Murmur"
        self.audio.mkdir()

    def tearDown(self) -> None:
        self.scratch.cleanup()

    def saved(self, number: int, seconds: float, delivered: str | None, mtime: float) -> None:
        wav = self.audio / f"murmur-{number:04d}.wav"
        write_wav(wav, seconds)
        if delivered is not None:
            (self.audio / f"murmur-{number:04d}.txt").write_text(delivered, encoding="utf-8")
        os.utime(wav, (mtime, mtime))

    def history(self, entries: list[dict]) -> Path:
        path = self.root / "history.json"
        path.write_text(json.dumps({"schema": "murmur.history.v2", "entries": entries}), encoding="utf-8")
        return path

    def args(self, history: Path | None, out: Path, dry_run: bool = False):
        return accuracy.parser().parse_args(
            ["export", "--audio-dir", str(self.audio), "--out", str(out)]
            + (["--history", str(history)] if history else [])
            + (["--dry-run"] if dry_run else [])
        )

    def test_exports_only_corrected_unambiguous_recordings(self) -> None:
        base = 1_800_000_000.0
        self.saved(1, 2.0, "cue an update", base)  # corrected -> exported
        self.saved(2, 2.0, "all good", base + 100)  # uncorrected
        self.saved(3, 2.0, "no history", base + 500)  # unpaired
        self.saved(4, 3.0, "twin one", base + 900)  # ambiguous: two entries
        self.saved(5, 1.5, None, base + 1300)  # no delivered .txt
        write_wav(self.audio / "murmur-0007.wav", 1.0, rate=44_100)  # wrong rate
        history = self.history(
            [
                {"timestamp": (base + 1) * 1000, "durationSeconds": 2.0, "source": "recording", "text": "Queue an update"},
                {"timestamp": (base + 101) * 1000, "durationSeconds": 2.0, "source": "recording", "text": "all good"},
                {"timestamp": (base + 901) * 1000, "durationSeconds": 3.0, "source": "recording", "text": "Twin one"},
                {"timestamp": (base + 902) * 1000, "durationSeconds": 3.1, "source": "recording", "text": "Twin 1"},
                {"timestamp": (base + 1301) * 1000, "durationSeconds": 1.5, "source": "recording", "text": "Fixed"},
                {"timestamp": (base + 1) * 1000, "durationSeconds": 2.0, "source": "file", "text": "Imported"},
            ]
        )
        out = self.root / "corpus"
        self.assertEqual(accuracy.export_corpus(self.args(history, out)), 0)
        manifest = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["corpusId"], "accuracy-v1")
        self.assertEqual(manifest["source"], "corrected-history-export")
        self.assertTrue(manifest["containsRealUserData"])
        references = [r["reference"] for r in manifest["recordings"]]
        self.assertEqual(references, ["Queue an update"])
        first = manifest["recordings"][0]
        self.assertEqual(first["promptId"], "corrected-0001")
        self.assertEqual(first["terms"], ["Queue"])
        self.assertEqual(first["durationMs"], 2000)
        self.assertEqual(first["sha256"], accuracy._sha256(out / "audio" / first["fileName"]))
        self.assertEqual(oct(out.stat().st_mode & 0o777), "0o700")

    def test_dry_run_copies_nothing_and_existing_output_is_refused(self) -> None:
        self.saved(1, 2.0, "cue", 1_800_000_000.0)
        history = self.history(
            [{"timestamp": 1_800_000_001_000, "durationSeconds": 2.0, "source": "recording", "text": "Queue"}]
        )
        out = self.root / "corpus"
        accuracy.export_corpus(self.args(history, out, dry_run=True))
        self.assertFalse(out.exists())
        out.mkdir()
        (out / "keep.txt").write_text("x", encoding="utf-8")
        with self.assertRaises(ValueError):
            accuracy.export_corpus(self.args(history, out))

    def test_refuses_to_write_inside_the_repository(self) -> None:
        with self.assertRaises(ValueError):
            accuracy.prepare_output_dir(accuracy.REPO_ROOT / "corpus-should-not-exist")
        self.assertFalse((accuracy.REPO_ROOT / "corpus-should-not-exist").exists())


if __name__ == "__main__":
    unittest.main()
