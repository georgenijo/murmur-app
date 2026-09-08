#!/usr/bin/env python3
"""Score spike JSONL against RTTM and check repeat-run label stability."""

from __future__ import annotations

import argparse
import itertools
import json
from pathlib import Path


def parse_rttm(path: Path) -> list[tuple[float, float, str]]:
    intervals = []
    for line in path.read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if not fields or fields[0] != "SPEAKER":
            continue
        start = float(fields[3])
        intervals.append((start, start + float(fields[4]), fields[7]))
    return intervals


def parse_uem(path: Path) -> tuple[float, float]:
    fields = path.read_text(encoding="utf-8").split()
    return float(fields[2]), float(fields[3])


def canonical_segments(run: dict) -> list[tuple[int, int, int]]:
    ordered = sorted(
        run["segments"],
        key=lambda segment: (segment["startSeconds"], segment["endSeconds"]),
    )
    labels: dict[str, int] = {}
    canonical = []
    for segment in ordered:
        label = labels.setdefault(segment["speakerId"], len(labels))
        canonical.append(
            (
                label,
                round(segment["startSeconds"] * 1000),
                round(segment["endSeconds"] * 1000),
            )
        )
    return canonical


def best_mapping(
    pair_counts: dict[tuple[str, str], int], references: list[str], hypotheses: list[str]
) -> dict[str, str | None]:
    best_score = -1
    best: dict[str, str | None] = {}
    if len(hypotheses) <= len(references):
        for assigned in itertools.permutations(references, len(hypotheses)):
            candidate = dict(zip(hypotheses, assigned, strict=True))
            score = sum(pair_counts.get((reference, hypothesis), 0) for hypothesis, reference in candidate.items())
            if score > best_score:
                best_score, best = score, candidate
    else:
        for selected in itertools.combinations(hypotheses, len(references)):
            for assigned in itertools.permutations(references):
                candidate = {hypothesis: None for hypothesis in hypotheses}
                candidate.update(dict(zip(selected, assigned, strict=True)))
                score = sum(pair_counts.get((reference, hypothesis), 0) for hypothesis, reference in candidate.items() if reference)
                if score > best_score:
                    best_score, best = score, candidate
    return best


def score(run: dict, references: list[tuple[float, float, str]], uem: tuple[float, float]) -> dict:
    frame_seconds = 0.01
    frame_count = int((uem[1] - uem[0]) / frame_seconds)
    reference_frames = [set() for _ in range(frame_count)]
    hypothesis_frames = [set() for _ in range(frame_count)]
    ignored = [False] * frame_count

    def fill(frames: list[set[str]], start: float, end: float, label: str) -> None:
        first = max(0, int((start - uem[0]) / frame_seconds))
        last = min(frame_count, int((end - uem[0]) / frame_seconds) + 1)
        for index in range(first, last):
            midpoint = uem[0] + (index + 0.5) * frame_seconds
            if start <= midpoint < end:
                frames[index].add(label)

    for start, end, label in references:
        fill(reference_frames, start, end, label)
        for boundary in (start, end):
            first = max(0, int((boundary - 0.25 - uem[0]) / frame_seconds))
            last = min(frame_count, int((boundary + 0.25 - uem[0]) / frame_seconds) + 1)
            for index in range(first, last):
                ignored[index] = True
    for segment in run["segments"]:
        fill(
            hypothesis_frames,
            float(segment["startSeconds"]),
            float(segment["endSeconds"]),
            str(segment["speakerId"]),
        )

    reference_labels = sorted({label for _, _, label in references})
    hypothesis_labels = sorted({str(segment["speakerId"]) for segment in run["segments"]})
    pair_counts: dict[tuple[str, str], int] = {}
    for ref, hyp, skip in zip(reference_frames, hypothesis_frames, ignored, strict=True):
        if skip or len(ref) != 1 or len(hyp) != 1:
            continue
        pair = (next(iter(ref)), next(iter(hyp)))
        pair_counts[pair] = pair_counts.get(pair, 0) + 1
    mapping = best_mapping(pair_counts, reference_labels, hypothesis_labels)

    reference_speech = misses = false_alarms = speaker_errors = 0
    for ref, hyp, skip in zip(reference_frames, hypothesis_frames, ignored, strict=True):
        if skip or len(ref) > 1:
            continue
        if ref:
            reference_speech += 1
            if not hyp:
                misses += 1
            else:
                expected = next(iter(ref))
                if expected not in {mapping.get(label) for label in hyp}:
                    speaker_errors += 1
                false_alarms += max(0, len(hyp) - 1)
        else:
            false_alarms += len(hyp)

    return {
        "referenceSpeakerCount": len(reference_labels),
        "hypothesisSpeakerCount": len(hypothesis_labels),
        "mapping": mapping,
        "collarSeconds": 0.25,
        "ignoreReferenceOverlap": True,
        "missPercent": 100 * misses / reference_speech,
        "falseAlarmPercent": 100 * false_alarms / reference_speech,
        "speakerErrorPercent": 100 * speaker_errors / reference_speech,
        "approximateDerPercent": 100 * (misses + false_alarms + speaker_errors) / reference_speech,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("jsonl", type=Path)
    parser.add_argument("rttm", type=Path)
    parser.add_argument("uem", type=Path)
    parser.add_argument("--audio-seconds", type=float, required=True)
    arguments = parser.parse_args()

    records = [json.loads(line) for line in arguments.jsonl.read_text(encoding="utf-8").splitlines() if line.startswith("{")]
    runs = [record for record in records if record.get("kind") == "run"]
    if not runs:
        raise SystemExit("no run records found")
    stability = all(canonical_segments(run) == canonical_segments(runs[0]) for run in runs[1:])
    result = {
        "initializationSeconds": next(record["elapsedSeconds"] for record in records if record.get("kind") == "initialization"),
        "runs": [
            {
                "run": run["run"],
                "elapsedSeconds": run["elapsedSeconds"],
                "secondsPerAudioHour": run["elapsedSeconds"] * 3600 / arguments.audio_seconds,
                "rtfx": arguments.audio_seconds / run["elapsedSeconds"],
                "speakerCount": run["speakerCount"],
                "segmentCount": run["segmentCount"],
                "score": score(run, parse_rttm(arguments.rttm), parse_uem(arguments.uem)),
            }
            for run in runs
        ],
        "canonicalLabelsAndMillisecondBoundariesStableAcrossRuns": stability,
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
