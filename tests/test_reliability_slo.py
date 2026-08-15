from __future__ import annotations

import importlib.util
import json
from datetime import datetime, timedelta, timezone
from pathlib import Path
import unittest
from unittest import mock


MODULE_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "reliability_slo.py"
)
SPEC = importlib.util.spec_from_file_location("reliability_slo", MODULE_PATH)
assert SPEC and SPEC.loader
slo = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(slo)

NOW = datetime(2026, 8, 19, 12, 0, tzinfo=timezone.utc)  # Wednesday
INSTALL = "internal-install-sentinel"


def stamp(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def event(code: str, when: datetime | str, recording_id: int | None = None, **data: object) -> dict:
    fields = {"event_code": code, **data}
    if recording_id is not None:
        fields["recording_id"] = recording_id
    return {
        "timestamp": stamp(when) if isinstance(when, datetime) else when,
        "stream": "pipeline",
        "level": "info",
        "summary": "untrusted summary",
        "data": fields,
    }


def audio(code: str, when: datetime, recording_id: int, **data: object) -> dict:
    item = event(
        code,
        when,
        recording_id,
        owner=recording_id,
        owner_kind="dictation",
        **data,
    )
    item["stream"] = "audio"
    return item


def request(when: datetime, recording_id: int, contract: object = 1) -> dict:
    return event(
        "pipeline.dictation_requested",
        when,
        recording_id,
        slo_contract=contract,
    )


def terminal(when: datetime, recording_id: int, outcome: str = "success") -> dict:
    return event(
        "pipeline.dictation_terminal",
        when,
        recording_id,
        outcome=outcome,
        error_code="none",
    )


def baseline(when: datetime) -> dict:
    return event("system.startup_baseline", when)


def complete_attempt(
    when: datetime,
    recording_id: int,
    latency_ms: float = 200,
    outcome: str = "success",
) -> list[dict]:
    return [
        request(when, recording_id),
        audio("audio.capture_started", when + timedelta(milliseconds=10), recording_id),
        audio(
            "audio.capture_ready",
            when + timedelta(milliseconds=latency_ms),
            recording_id,
        ),
        terminal(when + timedelta(seconds=2), recording_id, outcome),
    ]


def feed(items: list[dict], now: datetime = NOW, install: str = INSTALL) -> dict:
    evaluator = slo.ReliabilitySloEvaluator(now=now)
    for item in items:
        evaluator.observe(install, item)
    return evaluator.report()


def week(report: dict, index: int = 0) -> dict:
    return report["weeks"][index]


class ReliabilitySloContractTests(unittest.TestCase):
    def test_only_exact_numeric_contract_v1_requests_enter_the_denominator(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]
        for recording_id, contract in enumerate((None, 0, 2, True, 1.0, "1"), start=1):
            items.extend(complete_attempt(start + timedelta(minutes=recording_id), recording_id))
            items[-4]["data"]["slo_contract"] = contract
        items.extend(complete_attempt(start + timedelta(hours=1), 20))

        current = week(feed(items))

        self.assertEqual(current["counts"]["eligible_requests"], 1)
        self.assertEqual(current["counts"]["ready"], 1)

    def test_permission_pending_excludes_only_the_latency_denominator(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]
        items.extend(complete_attempt(start, 1, latency_ms=900, outcome="capture_init_failure"))
        items.insert(
            2,
            audio(
                "audio.permission_prompt_changed",
                start + timedelta(milliseconds=5),
                1,
                state="pending",
            ),
        )
        items.append(
            audio(
                "audio.permission_prompt_changed",
                start + timedelta(seconds=1),
                1,
                state="resolved",
                prompt_pending_ms=995,
            )
        )

        current = week(feed(items))

        self.assertEqual(current["counts"]["excluded_permission_prompts"], 1)
        self.assertEqual(current["counts"]["requested"], 1)
        self.assertEqual(current["counts"]["eligible_requests"], 0)
        self.assertEqual(current["counts"]["accepted"], 1)
        self.assertEqual(current["counts"]["ready"], 1)
        self.assertEqual(current["counts"]["failed"], 1)
        self.assertEqual(current["counts"]["within_400"], 0)
        self.assertEqual(current["startup_ms"]["sample_count"], 0)

    def test_retained_prompt_resolution_alone_proves_permission_exclusion(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]
        items.extend(complete_attempt(start, 1, latency_ms=900))
        items.append(
            audio(
                "audio.permission_prompt_changed",
                start + timedelta(seconds=1),
                1,
                state="resolved",
                prompt_pending_ms=500,
            )
        )

        current = week(feed(items))

        self.assertEqual(current["counts"]["requested"], 1)
        self.assertEqual(current["counts"]["excluded_permission_prompts"], 1)
        self.assertEqual(current["counts"]["eligible_requests"], 0)
        self.assertEqual(current["startup_ms"]["sample_count"], 0)

    def test_prompted_stuck_failure_still_fails_state_and_presentation_slos(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [
            baseline(start - timedelta(minutes=1)),
            request(start, 1),
            audio(
                "audio.permission_prompt_changed",
                start + timedelta(milliseconds=1),
                1,
                state="pending",
            ),
            event(
                "pipeline.dictation_state_changed",
                start + timedelta(seconds=1),
                1,
                **{"from": "starting", "to": "processing"},
            ),
            terminal(start + timedelta(seconds=2), 1, "capture_init_failure"),
            baseline(start + timedelta(minutes=1)),
        ]

        previous = week(feed(items), 1)
        self.assertEqual(previous["counts"]["eligible_requests"], 0)
        self.assertEqual(previous["counts"]["excluded_permission_prompts"], 1)
        self.assertEqual(previous["counts"]["failed"], 1)
        self.assertEqual(
            previous["counts"]["failures_without_actionable_presentation"], 1
        )
        self.assertEqual(previous["states"]["processing"]["restart_required"], 1)
        self.assertEqual(previous["sample_status"], "below_minimum")
        self.assertEqual(previous["verdict"], "fail")
        self.assertIn("restart_required_state", previous["reasons"])
        self.assertIn("failure_presentation_missing", previous["reasons"])

    def test_audio_ownership_must_match_dictation_and_recording_id(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1)), request(start, 1)]
        wrong_owner = audio("audio.capture_ready", start + timedelta(milliseconds=100), 1)
        wrong_owner["data"]["owner"] = 2
        transform = audio("audio.capture_started", start, 1)
        transform["data"]["owner_kind"] = "transform"
        items.extend([wrong_owner, transform, terminal(start + timedelta(seconds=1), 1)])

        current = week(feed(items))

        self.assertEqual(current["counts"]["accepted"], 0)
        self.assertEqual(current["counts"]["ready"], 0)
        self.assertEqual(current["startup_ms"]["within_400_fraction"], 0.0)

    def test_events_before_a_proven_startup_session_are_ignored(self) -> None:
        start = NOW - timedelta(days=1)
        items = complete_attempt(start, 1)
        items.extend([baseline(start + timedelta(hours=1)), *complete_attempt(start + timedelta(hours=2), 2)])

        self.assertEqual(week(feed(items))["counts"]["eligible_requests"], 1)

    def test_recording_ids_are_isolated_by_install_and_startup_session(self) -> None:
        start = NOW - timedelta(days=1)
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        first_session = [
            baseline(start - timedelta(minutes=2)),
            *complete_attempt(start, 1, outcome="pipeline_failure"),
        ]
        second_session = [
            baseline(start + timedelta(minutes=1)),
            *complete_attempt(start + timedelta(minutes=2), 1),
        ]
        other_install = [
            baseline(start - timedelta(minutes=1)),
            *complete_attempt(start + timedelta(minutes=3), 1),
        ]
        for item in [*first_session, *second_session]:
            evaluator.observe("install-a", item)
        for item in other_install:
            evaluator.observe("install-b", item)

        current = week(evaluator.report())
        self.assertEqual(current["counts"]["requested"], 3)
        self.assertEqual(current["counts"]["failed"], 1)
        self.assertEqual(current["counts"]["ready"], 3)


class ReliabilitySloTimingTests(unittest.TestCase):
    def test_four_hundred_milliseconds_is_inclusive_and_uses_request_time(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]
        items.extend(complete_attempt(start, 1, latency_ms=400))
        items.extend(complete_attempt(start + timedelta(minutes=1), 2, latency_ms=401))

        current = week(feed(items))

        self.assertEqual(current["counts"]["ready"], 2)
        self.assertEqual(current["counts"]["within_400"], 1)
        self.assertEqual(current["startup_ms"]["within_400_fraction"], 0.5)

    def test_nearest_rank_percentiles_and_maximum_are_exact(self) -> None:
        start = NOW - timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]
        for recording_id, latency in enumerate(range(10, 210, 10), start=1):
            items.extend(
                complete_attempt(
                    start + timedelta(minutes=recording_id),
                    recording_id,
                    latency_ms=latency,
                )
            )

        metrics = week(feed(items))["startup_ms"]

        self.assertEqual(metrics["sample_count"], 20)
        self.assertEqual(metrics["p50"], 100.0)
        self.assertEqual(metrics["p95"], 190.0)
        self.assertEqual(metrics["max"], 200.0)

    def test_invalid_or_reversed_startup_timing_is_indeterminate(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        report = feed(
            [
                baseline(start - timedelta(minutes=1)),
                request(start, 1),
                audio("audio.capture_ready", start - timedelta(milliseconds=1), 1),
                terminal(start + timedelta(seconds=1), 1),
            ]
        )

        previous = week(report, 1)
        self.assertEqual(previous["counts"]["ready"], 1)
        self.assertEqual(previous["counts"]["invalid_startup_timings"], 1)
        self.assertEqual(previous["verdict"], "indeterminate")
        self.assertIn("invalid_startup_timing", previous["reasons"])

    def test_strict_utc_parser_rejects_offsets_and_invalid_calendar_values(self) -> None:
        self.assertIsNotNone(slo.parse_utc_timestamp("2026-08-17T00:00:00.123456789Z"))
        self.assertIsNone(slo.parse_utc_timestamp("2026-08-17T00:00:00+00:00"))
        self.assertIsNone(slo.parse_utc_timestamp("2026-02-30T00:00:00Z"))
        self.assertIsNone(slo.parse_utc_timestamp("not-a-time"))

    def test_unassignable_contract_request_is_visible_and_blocks_two_week_proof(self) -> None:
        newest = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        older = newest - timedelta(weeks=1)
        events = ReliabilitySloVerdictTests.completed_week_events(
            older,
            200,
            first_id=1,
        )
        events.extend(
            ReliabilitySloVerdictTests.completed_week_events(
                newest,
                200,
                first_id=1_000,
            )
        )
        events.append(request(newest, 9_999))
        events[-1]["timestamp"] = "not-a-timestamp"

        report = feed(events)

        self.assertEqual(
            report["integrity"],
            {
                "status": "indeterminate",
                "unassigned_contract_requests": 1,
                "overflowed_events": 0,
                "malformed_source_lines": 0,
            },
        )
        self.assertFalse(report["two_consecutive_complete_weeks_pass"])

    def test_future_contract_request_is_unassigned_but_expired_history_is_not(self) -> None:
        oldest_retained = datetime(2026, 6, 22, 0, 0, tzinfo=timezone.utc)
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        evaluator.observe(INSTALL, baseline(oldest_retained - timedelta(weeks=2)))
        evaluator.observe(INSTALL, request(oldest_retained - timedelta(microseconds=1), 1))
        evaluator.observe(INSTALL, request(NOW + timedelta(seconds=1), 2))

        report = evaluator.report()

        self.assertEqual(report["integrity"]["unassigned_contract_requests"], 1)
        self.assertEqual(report["integrity"]["status"], "indeterminate")
        self.assertTrue(
            all(item["counts"]["requested"] == 0 for item in report["weeks"])
        )

    def test_malformed_retained_source_line_blocks_two_week_proof(self) -> None:
        newest = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        older = newest - timedelta(weeks=1)
        events = ReliabilitySloVerdictTests.completed_week_events(older, 200)
        events.extend(
            ReliabilitySloVerdictTests.completed_week_events(
                newest,
                200,
                first_id=1_000,
            )
        )
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        for item in events:
            evaluator.observe(INSTALL, item)
        with mock.patch.object(slo, "MAX_AGGREGATE_COUNT", 1):
            evaluator.observe_malformed_source_line()
            evaluator.observe_malformed_source_line()

        report = evaluator.report()

        self.assertEqual(report["integrity"]["malformed_source_lines"], 1)
        self.assertEqual(report["integrity"]["status"], "indeterminate")
        self.assertFalse(report["two_consecutive_complete_weeks_pass"])

    def test_weeks_are_monday_utc_half_open_and_include_eight_complete_weeks(self) -> None:
        current_start = datetime(2026, 8, 17, tzinfo=timezone.utc)
        previous_start = current_start - timedelta(weeks=1)
        items = [baseline(previous_start - timedelta(minutes=1))]
        items.extend(complete_attempt(previous_start, 1))
        items.extend(
            complete_attempt(
                current_start - timedelta(microseconds=1),
                2,
            )
        )
        items.extend(complete_attempt(current_start, 3))

        report = feed(items)

        self.assertEqual(len(report["weeks"]), 9)
        self.assertEqual(report["weeks"][0]["week_start"], "2026-08-17T00:00:00Z")
        self.assertFalse(report["weeks"][0]["complete"])
        self.assertEqual(report["weeks"][0]["counts"]["eligible_requests"], 1)
        self.assertEqual(report["weeks"][1]["counts"]["eligible_requests"], 2)


class ReliabilitySloLifecycleTests(unittest.TestCase):
    def test_out_of_order_terminal_still_joins_by_session_and_recording(self) -> None:
        start = NOW - timedelta(days=1)
        report = feed(
            [
                baseline(start - timedelta(minutes=1)),
                terminal(start + timedelta(seconds=1), 9),
                audio("audio.capture_ready", start + timedelta(milliseconds=300), 9),
                request(start, 9),
                audio("audio.capture_started", start + timedelta(milliseconds=10), 9),
            ]
        )

        current = week(report)
        self.assertEqual(current["counts"]["eligible_requests"], 1)
        self.assertEqual(current["counts"]["missing_terminals"], 0)
        self.assertEqual(current["counts"]["ready"], 1)
        self.assertEqual(current["counts"]["ready_without_accepted"], 0)

    def test_pre_request_state_evidence_is_retained_until_contract_request_arrives(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [
            baseline(start - timedelta(minutes=1)),
            event(
                "pipeline.dictation_state_changed",
                start - timedelta(milliseconds=1),
                8,
                **{"from": "starting", "to": "recovering"},
            ),
            request(start, 8),
            event(
                "pipeline.dictation_state_changed",
                start + timedelta(seconds=2),
                8,
                **{"from": "recovering", "to": "idle"},
            ),
            terminal(start + timedelta(seconds=3), 8),
        ]

        previous = week(feed(items), 1)
        self.assertEqual(previous["states"]["recovering"]["self_recovered"], 1)
        self.assertEqual(
            previous["states"]["recovering"]["duration_ms"]["max"],
            2001.0,
        )

    def test_ready_without_eventually_correlated_acceptance_is_indeterminate(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        previous = week(
            feed(
                [
                    baseline(start - timedelta(minutes=1)),
                    request(start, 1),
                    audio("audio.capture_ready", start + timedelta(milliseconds=200), 1),
                    terminal(start + timedelta(seconds=1), 1),
                ]
            ),
            1,
        )
        self.assertEqual(previous["counts"]["ready_without_accepted"], 1)
        self.assertEqual(previous["verdict"], "indeterminate")
        self.assertIn("ready_without_accepted", previous["reasons"])

    def test_missing_duplicate_unknown_terminals_are_indeterminate(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1))]
        items.append(request(start, 1))
        items.extend([request(start, 2), terminal(start, 2), terminal(start, 2)])
        items.extend([request(start, 3), terminal(start, 3, "private-outcome")])

        current = week(feed(items), 1)

        self.assertEqual(current["counts"]["missing_terminals"], 1)
        self.assertEqual(current["counts"]["duplicate_terminals"], 1)
        self.assertEqual(current["counts"]["unknown_terminals"], 1)
        self.assertEqual(current["verdict"], "indeterminate")
        self.assertEqual(
            current["reasons"][:3],
            ["missing_terminal", "duplicate_terminal", "unknown_terminal"],
        )
        self.assertNotIn("private-outcome", json.dumps(current))

    def test_state_intervals_distinguish_recovery_restart_and_open_unknown(self) -> None:
        complete_week = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        session_start = complete_week - timedelta(hours=1)
        items = [baseline(session_start)]
        items.extend(complete_attempt(complete_week, 1))
        items.extend(
            [
                event(
                    "pipeline.dictation_state_changed",
                    complete_week + timedelta(seconds=3),
                    1,
                    **{"from": "recording", "to": "recovering"},
                ),
                event(
                    "pipeline.dictation_state_changed",
                    complete_week + timedelta(seconds=5),
                    1,
                    **{"from": "recovering", "to": "idle"},
                ),
            ]
        )
        items.extend(complete_attempt(complete_week + timedelta(minutes=1), 2))
        items.append(
            event(
                "pipeline.dictation_state_changed",
                complete_week + timedelta(minutes=1, seconds=3),
                2,
                **{"from": "recording", "to": "processing"},
            )
        )
        items.append(baseline(complete_week + timedelta(minutes=3)))
        items.extend(complete_attempt(complete_week + timedelta(minutes=4), 3))
        items.append(
            event(
                "pipeline.dictation_state_changed",
                complete_week + timedelta(minutes=4, seconds=3),
                3,
                **{"from": "recording", "to": "recovering"},
            )
        )

        previous = week(feed(items), 1)

        self.assertEqual(previous["states"]["recovering"]["self_recovered"], 1)
        self.assertEqual(previous["states"]["recovering"]["indeterminate"], 1)
        self.assertEqual(previous["states"]["recovering"]["duration_ms"]["p50"], 2000.0)
        self.assertEqual(previous["states"]["processing"]["restart_required"], 1)
        self.assertEqual(previous["states"]["processing"]["duration_ms"]["max"], 117000.0)
        self.assertEqual(previous["verdict"], "indeterminate")

    def test_orphan_state_exit_is_never_silently_healthy(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1)), *complete_attempt(start, 1)]
        items.append(
            event(
                "pipeline.dictation_state_changed",
                start + timedelta(seconds=3),
                1,
                **{"from": "processing", "to": "idle"},
            )
        )

        current = week(feed(items), 1)
        self.assertEqual(current["states"]["processing"]["indeterminate"], 1)
        self.assertEqual(current["verdict"], "indeterminate")

    def test_valid_state_chain_uses_the_prior_target_as_the_next_source(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1)), *complete_attempt(start, 1)]
        for offset, previous, current in [
            (3, "idle", "starting"),
            (4, "starting", "recording"),
            (5, "recording", "processing"),
            (7, "processing", "idle"),
        ]:
            items.append(
                event(
                    "pipeline.dictation_state_changed",
                    start + timedelta(seconds=offset),
                    1,
                    **{"from": previous, "to": current},
                )
            )

        current = week(feed(items), 1)

        self.assertEqual(current["counts"]["invalid_state_transitions"], 0)
        self.assertEqual(current["states"]["processing"]["self_recovered"], 1)

    def test_overlapping_out_of_order_state_chain_is_indeterminate(self) -> None:
        start = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        items = ReliabilitySloVerdictTests.completed_week_events(start, 200)
        for offset, previous, current in [
            (3, "recording", "recovering"),
            (4, "recording", "processing"),
            (5, "recovering", "idle"),
            (6, "processing", "idle"),
        ]:
            items.append(
                event(
                    "pipeline.dictation_state_changed",
                    start + timedelta(seconds=offset),
                    1,
                    **{"from": previous, "to": current},
                )
            )

        current = week(feed(items), 1)

        self.assertGreater(current["counts"]["invalid_state_transitions"], 0)
        self.assertIn("invalid_state_transition", current["reasons"])
        self.assertEqual(current["verdict"], "indeterminate")

    def test_future_state_exit_cannot_turn_a_complete_week_healthy(self) -> None:
        start = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        items = ReliabilitySloVerdictTests.completed_week_events(start, 200)
        items.extend(
            [
                event(
                    "pipeline.dictation_state_changed",
                    start + timedelta(seconds=3),
                    1,
                    **{"from": "recording", "to": "recovering"},
                ),
                event(
                    "pipeline.dictation_state_changed",
                    NOW + timedelta(days=1),
                    1,
                    **{"from": "recovering", "to": "idle"},
                ),
            ]
        )

        previous = week(feed(items), 1)

        self.assertEqual(previous["verdict"], "indeterminate")
        self.assertEqual(previous["counts"]["invalid_evidence_timestamps"], 1)
        self.assertEqual(previous["states"]["recovering"]["self_recovered"], 0)
        self.assertEqual(previous["states"]["recovering"]["indeterminate"], 1)
        self.assertIn("invalid_evidence_timestamp", previous["reasons"])

    def test_future_or_invalid_evidence_cannot_cure_any_attempt_clause(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        future = NOW + timedelta(days=1)
        items = [baseline(start - timedelta(minutes=1))]

        items.extend([request(start, 1), audio("audio.capture_started", future, 1), terminal(start + timedelta(seconds=2), 1)])
        items.extend([request(start + timedelta(minutes=1), 2), audio("audio.capture_started", start + timedelta(minutes=1, milliseconds=10), 2), audio("audio.capture_ready", future, 2), terminal(start + timedelta(minutes=1, seconds=2), 2)])
        items.extend(complete_attempt(start + timedelta(minutes=2), 3, latency_ms=900))
        items.append(audio("audio.permission_prompt_changed", future, 3, state="resolved", prompt_pending_ms=500))
        items.extend([request(start + timedelta(minutes=3), 4), audio("audio.capture_started", start + timedelta(minutes=3, milliseconds=10), 4), audio("audio.capture_ready", start + timedelta(minutes=3, milliseconds=200), 4), terminal(future, 4)])
        items.extend(complete_attempt(start + timedelta(minutes=4), 5, outcome="capture_init_failure"))
        items.append(event("pipeline.dictation_presentation", future, 5, status_code="microphone_initialization_failed", action_code="retry"))
        items.extend(complete_attempt(start + timedelta(minutes=5), 6))
        items.extend([
            event("pipeline.dictation_state_changed", start + timedelta(minutes=5, seconds=3), 6, **{"from": "recording", "to": "recovering"}),
            event("pipeline.dictation_state_changed", future, 6, **{"from": "recovering", "to": "idle"}),
        ])
        items.append(request(start + timedelta(minutes=6), 7))
        items.append(event("pipeline.dictation_terminal", "not-a-time", 7, outcome="success"))

        previous = week(feed(items), 1)

        self.assertEqual(previous["counts"]["invalid_evidence_timestamps"], 7)
        self.assertEqual(previous["counts"]["accepted"], 5)
        self.assertEqual(previous["counts"]["ready"], 4)
        self.assertEqual(previous["counts"]["excluded_permission_prompts"], 0)
        self.assertEqual(previous["counts"]["missing_terminals"], 2)
        self.assertEqual(
            previous["counts"]["failures_without_actionable_presentation"],
            1,
        )
        self.assertEqual(previous["states"]["recovering"]["self_recovered"], 0)
        self.assertEqual(previous["verdict"], "indeterminate")

    def test_invalid_restart_boundary_makes_open_state_indeterminate(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1)), *complete_attempt(start, 1)]
        items.extend(
            [
                event(
                    "pipeline.dictation_state_changed",
                    start + timedelta(seconds=3),
                    1,
                    **{"from": "recording", "to": "recovering"},
                ),
                event("system.startup_baseline", "not-a-timestamp"),
            ]
        )
        previous = week(feed(items), 1)
        self.assertEqual(previous["states"]["recovering"]["restart_required"], 0)
        self.assertEqual(previous["states"]["recovering"]["indeterminate"], 1)
        self.assertEqual(previous["verdict"], "indeterminate")

    def test_future_restart_boundary_cannot_fabricate_restart_duration(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1)), *complete_attempt(start, 1)]
        items.extend(
            [
                event(
                    "pipeline.dictation_state_changed",
                    start + timedelta(seconds=3),
                    1,
                    **{"from": "recording", "to": "recovering"},
                ),
                baseline(NOW + timedelta(days=1)),
            ]
        )

        previous = week(feed(items), 1)

        self.assertEqual(previous["states"]["recovering"]["restart_required"], 0)
        self.assertEqual(previous["states"]["recovering"]["indeterminate"], 1)
        self.assertEqual(
            previous["states"]["recovering"]["duration_ms"]["sample_count"],
            0,
        )
        self.assertEqual(previous["verdict"], "indeterminate")

    def test_actionable_failure_coverage_uses_exact_outcome_pairs(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1))]
        failures = [
            (1, "capture_init_failure", "microphone_initialization_failed", "retry"),
            (2, "runtime_interruption", "microphone_interrupted", "wait_for_partial_transcription"),
            (3, "stop_failure", "microphone_cleanup_stalled", "restart_app"),
            (4, "pipeline_failure", "private-status", "private-action"),
        ]
        for rid, outcome, status, action in failures:
            items.extend(complete_attempt(start + timedelta(minutes=rid), rid, outcome=outcome))
            items.append(
                event(
                    "pipeline.dictation_presentation",
                    start + timedelta(minutes=rid, seconds=3),
                    rid,
                    status_code=status,
                    action_code=action,
                )
            )
        # An immediate stop failure may have no correlated stalled-cleanup UI;
        # it must remain visible as missing presentation evidence.
        items.extend(
            complete_attempt(
                start + timedelta(minutes=5),
                5,
                outcome="stop_failure",
            )
        )
        items.extend(
            complete_attempt(
                start + timedelta(minutes=6),
                6,
                outcome="capture_init_failure",
            )
        )
        items.append(
            event(
                "pipeline.dictation_presentation",
                start + timedelta(minutes=6, seconds=3),
                6,
                status_code="microphone_initialization_failed",
                action_code="choose_microphone",
            )
        )

        current = week(feed(items), 1)

        self.assertEqual(current["counts"]["failed"], 6)
        self.assertEqual(current["counts"]["failures_with_actionable_presentation"], 4)
        self.assertEqual(current["counts"]["failures_without_actionable_presentation"], 2)
        self.assertEqual(current["verdict"], "fail")
        rendered = json.dumps(current)
        self.assertNotIn("private-status", rendered)
        self.assertNotIn("private-action", rendered)

    def test_cancelled_and_neutral_terminals_have_stable_bounded_counts(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        items = [baseline(start - timedelta(minutes=1))]
        outcomes = [
            "user_cancelled_starting",
            "user_cancelled_recording",
            "user_cancelled_processing",
            "superseded",
            "no_speech",
            "too_short",
        ]
        for rid, outcome in enumerate(outcomes, start=1):
            items.extend(
                complete_attempt(
                    start + timedelta(minutes=rid),
                    rid,
                    outcome=outcome,
                )
            )

        previous = week(feed(items), 1)
        self.assertEqual(previous["counts"]["cancelled"], 4)
        self.assertEqual(previous["counts"]["failed"], 0)
        self.assertEqual(previous["counts"]["missing_terminals"], 0)


class ReliabilitySloVerdictTests(unittest.TestCase):
    @staticmethod
    def completed_week_events(
        start: datetime,
        count: int,
        misses: int = 0,
        first_id: int = 1,
    ) -> list[dict]:
        items = [baseline(start - timedelta(minutes=1))]
        for offset in range(count):
            latency = 401 if offset < misses else 200
            items.extend(
                complete_attempt(
                    start + timedelta(seconds=offset * 10),
                    first_id + offset,
                    latency_ms=latency,
                )
            )
        return items

    def test_exact_minimum_and_995_percent_threshold_pass(self) -> None:
        start = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        report = feed(self.completed_week_events(start, 200, misses=1))

        previous = week(report, 1)
        self.assertEqual(previous["counts"]["eligible_requests"], 200)
        self.assertEqual(previous["startup_ms"]["within_400_fraction"], 0.995)
        self.assertEqual(previous["verdict"], "pass")

    def test_verdict_precedence_is_indeterminate_fail_insufficient_pass(self) -> None:
        start = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)

        indeterminate_items = self.completed_week_events(start, 1, misses=1)
        indeterminate_items[-1] = request(start + timedelta(minutes=1), 999)
        self.assertEqual(week(feed(indeterminate_items), 1)["verdict"], "indeterminate")

        failed = week(feed(self.completed_week_events(start, 1, misses=1)), 1)
        self.assertEqual(failed["verdict"], "fail")

        insufficient = week(feed(self.completed_week_events(start, 199)), 1)
        self.assertEqual(insufficient["verdict"], "insufficient")

        passed = week(feed(self.completed_week_events(start, 200)), 1)
        self.assertEqual(passed["verdict"], "pass")

    def test_current_partial_week_can_never_pass(self) -> None:
        start = datetime(2026, 8, 17, 0, 0, tzinfo=timezone.utc)
        current = week(feed(self.completed_week_events(start, 200)))
        self.assertEqual(current["verdict"], "insufficient")
        self.assertEqual(current["reasons"], ["partial_week"])

    def test_two_consecutive_complete_weeks_requires_the_newest_two(self) -> None:
        newest = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        older = newest - timedelta(weeks=1)
        events = self.completed_week_events(older, 200, first_id=1)
        # A startup boundary separates reused recording IDs and also proves no
        # state interval leaked between application sessions.
        events.extend(self.completed_week_events(newest, 200, first_id=1000))
        report = feed(events)
        self.assertTrue(report["two_consecutive_complete_weeks_pass"])

        not_enough = self.completed_week_events(older, 200)
        not_enough.extend(self.completed_week_events(newest, 199, first_id=1000))
        self.assertFalse(feed(not_enough)["two_consecutive_complete_weeks_pass"])

    def test_full_rescan_late_arrival_revises_a_closed_week(self) -> None:
        start = datetime(2026, 8, 10, 0, 0, tzinfo=timezone.utc)
        first_scan = [baseline(start - timedelta(minutes=1)), request(start, 1)]
        self.assertEqual(week(feed(first_scan), 1)["verdict"], "indeterminate")

        second_scan = [
            baseline(start - timedelta(minutes=1)),
            request(start, 1),
            audio("audio.capture_started", start + timedelta(milliseconds=10), 1),
            audio("audio.capture_ready", start + timedelta(milliseconds=200), 1),
            terminal(start + timedelta(seconds=1), 1),
        ]
        revised = week(feed(second_scan), 1)
        self.assertEqual(revised["counts"]["missing_terminals"], 0)
        self.assertEqual(revised["verdict"], "insufficient")


class ReliabilitySloPrivacyAndApiTests(unittest.TestCase):
    def test_attempt_cap_fails_closed_with_bounded_overflow_evidence(self) -> None:
        start = NOW - timedelta(days=1)
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        evaluator.observe(INSTALL, baseline(start - timedelta(minutes=1)))
        with mock.patch.object(slo, "MAX_TRACKED_ATTEMPTS", 2):
            for recording_id in range(1, 4):
                evaluator.observe(
                    INSTALL,
                    request(start + timedelta(minutes=recording_id), recording_id),
                )

        report = evaluator.report()

        self.assertEqual(week(report)["counts"]["requested"], 2)
        self.assertEqual(report["integrity"]["overflowed_events"], 1)
        self.assertEqual(report["integrity"]["status"], "indeterminate")
        self.assertFalse(report["two_consecutive_complete_weeks_pass"])

    def test_install_cap_fails_closed_without_allocating_unbounded_sessions(self) -> None:
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        with mock.patch.object(slo, "MAX_TRACKED_INSTALLS", 1):
            evaluator.observe("install-a", baseline(NOW - timedelta(minutes=2)))
            evaluator.observe("install-b", baseline(NOW - timedelta(minutes=1)))

        report = evaluator.report()

        self.assertEqual(len(evaluator._install_sessions), 1)
        self.assertEqual(report["integrity"]["overflowed_events"], 1)
        self.assertEqual(report["integrity"]["status"], "indeterminate")

    def test_repeated_evidence_and_state_intervals_have_fixed_caps(self) -> None:
        start = datetime(2026, 8, 11, 12, tzinfo=timezone.utc)
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        evaluator.observe(INSTALL, baseline(start - timedelta(minutes=1)))
        for _ in range(20):
            evaluator.observe(INSTALL, request(start, 1))
            evaluator.observe(
                INSTALL,
                audio("audio.capture_ready", start + timedelta(milliseconds=200), 1),
            )
            evaluator.observe(INSTALL, terminal(start + timedelta(seconds=1), 1))
        transitions = [
            ("idle", "starting"),
            ("starting", "recording"),
            ("recording", "processing"),
            ("processing", "idle"),
            ("idle", "starting"),
            ("starting", "recording"),
            ("recording", "processing"),
            ("processing", "idle"),
        ]
        with mock.patch.object(slo, "MAX_STATE_INTERVALS_PER_ATTEMPT", 1):
            for offset, (previous, current) in enumerate(transitions, start=2):
                evaluator.observe(
                    INSTALL,
                    event(
                        "pipeline.dictation_state_changed",
                        start + timedelta(seconds=offset),
                        1,
                        **{"from": previous, "to": current},
                    ),
                )

        attempt = next(iter(evaluator._attempts.values()))
        report = evaluator.report()

        self.assertEqual(attempt["request_count"], 2)
        self.assertEqual(attempt["terminal_count"], 2)
        self.assertIsInstance(attempt["requested_at"], datetime)
        self.assertIsInstance(attempt["ready_at"], datetime)
        self.assertEqual(len(attempt["state_intervals"]["processing"]), 1)
        self.assertEqual(report["integrity"]["overflowed_events"], 1)
        self.assertEqual(report["integrity"]["status"], "indeterminate")

    def test_old_events_skip_allocation_and_restart_index_is_session_local(self) -> None:
        retained = NOW - timedelta(weeks=7)
        expired = NOW - timedelta(weeks=10)
        evaluator = slo.ReliabilitySloEvaluator(now=NOW)
        evaluator.observe("install-a", baseline(expired - timedelta(minutes=1)))
        evaluator.observe("install-a", request(expired, 99))
        self.assertEqual(evaluator._attempts, {})

        evaluator.observe("install-a", request(retained, 1))
        evaluator.observe("install-b", baseline(retained - timedelta(minutes=1)))
        evaluator.observe("install-b", request(retained, 1))
        self.assertIn(("install-a", 1), evaluator._session_attempt_keys)
        self.assertIn(("install-b", 1), evaluator._session_attempt_keys)

        evaluator.observe("install-a", baseline(retained + timedelta(minutes=1)))

        self.assertNotIn(("install-a", 1), evaluator._session_attempt_keys)
        self.assertIn(("install-b", 1), evaluator._session_attempt_keys)
        self.assertEqual(len(evaluator._attempts), 2)

    def test_convenience_feed_api_and_report_shape_are_aggregate_only(self) -> None:
        start = NOW - timedelta(days=1)
        secret_install = "PRIVATE-INSTALL-UUID"
        secret_summary = "PRIVATE TRANSCRIPT CONTENT"
        items = [baseline(start - timedelta(minutes=1)), *complete_attempt(start, 1)]
        for item in items:
            item["summary"] = secret_summary
            item["ingest_app_version"] = "99.99-private"
            item["data"]["device_path"] = "/private/device/path"
        report = slo.evaluate_fleet(
            [(secret_install, item) for item in items],
            now=NOW,
        )
        rendered = json.dumps(report, sort_keys=True)

        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["report"], "murmur-reliability-slo/v1")
        self.assertEqual(report["privacy"], "aggregate_only")
        self.assertNotIn(secret_install, rendered)
        self.assertNotIn(secret_summary, rendered)
        self.assertNotIn("99.99-private", rendered)
        self.assertNotIn("/private/device/path", rendered)
        for forbidden_key in (
            "install_id",
            "app_version",
            "device",
            "content",
            "path",
            "raw",
        ):
            self.assertNotIn('"%s"' % forbidden_key, rendered)

    def test_constructor_requires_utc_and_at_least_eight_complete_weeks(self) -> None:
        with self.assertRaises(ValueError):
            slo.ReliabilitySloEvaluator(now=datetime(2026, 8, 19))
        with self.assertRaises(ValueError):
            slo.ReliabilitySloEvaluator(
                now=datetime(2026, 8, 19, tzinfo=timezone(timedelta(hours=1)))
            )
        with self.assertRaises(ValueError):
            slo.ReliabilitySloEvaluator(now=NOW, complete_weeks=7)
        with self.assertRaises(ValueError):
            slo.ReliabilitySloEvaluator(now=NOW, complete_weeks=9)


if __name__ == "__main__":
    unittest.main()
