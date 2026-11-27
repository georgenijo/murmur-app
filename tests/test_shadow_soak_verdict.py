import contextlib
import copy
import io
import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts import shadow_soak_verdict as verdict

ROOT = Path(__file__).resolve().parents[1]
SHA = "0123456789abcdef0123456789abcdef01234567"
DAY = 24 * 3600 * 1000
T0 = 1_800_000_000_000  # a fixed instant; the days below are DAY apart


def manifest(family: str) -> dict:
    return verdict.load_manifest(family)


def natively_required(family: str) -> list[str]:
    data = manifest(family)
    waived = {entry["row"] for entry in data["not_natively_reachable"]}
    return [entry["row"] for entry in data["rows"] if entry["row"] not in waived]


def delegated_rows() -> set[str]:
    return {entry["row"] for family in verdict.FAMILIES for entry in manifest(family).get("delegated", [])}


def row_entry(matched: int = 0, delegated: int = 0, compared: int = 0) -> dict:
    """One row of the E7 dump, as the E8 test writes it."""
    return {"compared": compared, "matched": matched, "delegated": delegated, "e3_credit": matched + delegated}


_ABSENT = object()  # a field left out of a summary


def clean_summary(index: int = 0, day: int = 0, **overrides) -> dict:
    """A summary of one clean native pass: every natively required row matched 10 times."""
    per_row = {}
    for family in verdict.FAMILIES:
        for key in natively_required(family):
            if key in delegated_rows():
                # The reply is another site's: no `matched`, the verified delegated closes instead.
                per_row[key] = {"compared": 10, "matched": 0, "delegated": 10, "mismatched": 0, "pd_witness": 0}
            else:
                per_row[key] = {"compared": 10, "matched": 10, "delegated": 0, "mismatched": 0, "pd_witness": 0}
    sites = {site for family in verdict.FAMILIES for site in manifest(family)["sites"]}
    summary = {
        "session": {
            "started_unix_ms": T0 + day * DAY + index * 1000,
            "pid": 1000 + index,
            "git_sha": SHA,
            "armed": True,
            "build_profile": "shadow-soak",
        },
        "compared": 5000,
        "matched": 5000,
        "mismatched_root": 0,
        "followup": 0,
        "dropped": 0,
        "lost_open": 0,
        "loss_unattributed": 0,
        "lost_by_site": {},
        "shadow_panic": 0,
        "witness_overflow": 0,
        "abandoned_by_loss": 0,
        "abandoned_by_panic": 0,
        "skipped_incomplete": 0,
        "late_message_discarded": 0,
        "late_header_discarded": 0,
        "unverifiable": 0,
        "unconsumed": 0,
        "outstanding_at_exit": 0,
        "incomplete_closes": {},
        "core_divergences": 0,
        "final": True,
        # The final write's shutdown record, as #1173 round 4 names it: every attempt is
        # popped, rejected by the queue, refused after the close, or unsettled.
        "shutdown_quiesced": True,
        "shutdown_unsettled": 0,
        "dropped_after_shutdown": 0,
        "published_attempts": 1200,
        "popped": 1200,
        "push_lost": 0,
        "dropped_unavailable": 0,
        "named_no_reply": 2,
        "unclassified_err_replies": 3,
        # The shape PR #1173 writes: per-site objects hold only `headers`; the four books and
        # `balanced` are in `total`; `non_family` is keyed by file:line.
        "conservation": {str(site): {"headers": 6} for site in sorted(sites)}
        | {
            "total": {"entries": 8, "headers": 6, "no_decision": 1, "unwound": 1, "balanced": True},
            "non_family": {"app/src-tauri/src/keyboard.rs:120": 3},
            "unclassified": 0,
        },
        "per_row": per_row,
    }
    summary.update(overrides)
    return summary


def e7_dump(**overrides) -> dict:
    pds = {f"PD-{n}": 3 for n in (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 16)}
    dump = {"git_sha": SHA, "dirty": False, "golden_repeat": 100, "per_row": {}, "pd_witness": pds}
    dump.update(overrides)
    return dump


def three_passes() -> list[dict]:
    return [clean_summary(0, 0), clean_summary(1, 0), clean_summary(2, 1)]


def records(summaries: list[dict], groups=None) -> list[dict]:
    """Checklist records for `summaries`: by default one checklist per summary; `groups` lists
    the summary indexes of each checklist (two launches of one checklist are one group)."""
    if groups is None:
        groups = [[i] for i in range(len(summaries))]
    out = []
    for number, group in enumerate(groups):
        sessions = [summaries[i]["session"] for i in group if "session" in summaries[i]]
        if not sessions:
            continue
        first = min(session["started_unix_ms"] for session in sessions)
        out.append(
            {
                "label": f"checklist-{number}",
                "date": verdict.local_day(first),
                "complete": True,
                "sessions": [{"pid": x["pid"], "started_unix_ms": x["started_unix_ms"]} for x in sessions],
                "_path": f"checklist-{number}.json",
            }
        )
    return out


def evaluate(summaries: list[dict], e7: dict | None = None, families=None, checklists=None, expect_sha: str = SHA):
    with tempfile.TemporaryDirectory() as tmp:
        loaded = []
        for index, summary in enumerate(summaries):
            summary = dict(summary)
            summary["_path"] = f"{tmp}/pass{index}.json"
            loaded.append(summary)
        return verdict.evaluate(
            loaded,
            e7 if e7 is not None else e7_dump(),
            families or list(verdict.FAMILIES),
            checklists=records(summaries) if checklists is None else checklists,
            expect_sha=expect_sha,
        )


def by(verdicts, family: str, criterion: str):
    return next(v for v in verdicts if v.family == family and v.criterion == criterion)


class CleanRunTests(unittest.TestCase):
    def test_three_clean_passes_on_two_days_pass_every_criterion_of_every_family(self):
        verdicts = evaluate(three_passes())
        self.assertEqual(len(verdicts), 9 * len(verdict.FAMILIES))
        failed = [v.line() for v in verdicts if not v.passed]
        self.assertEqual(failed, [])
        for family in verdict.FAMILIES:
            self.assertEqual([v.criterion for v in verdicts if v.family == family], list(verdict.CRITERIA))

    def test_a_line_names_the_family_the_criterion_the_result_and_the_values(self):
        line = by(evaluate(three_passes()), "start", "E2").line()
        self.assertRegex(line, r"^start\s+E2\s+PASS\s+checklists=3 sessions=3 days=2 builds=1")


class E1Tests(unittest.TestCase):
    def test_a_root_mismatch_fails_every_family(self):
        passes = three_passes()
        passes[1]["mismatched_root"] = 1
        verdicts = evaluate(passes)
        for family in verdict.FAMILIES:
            self.assertFalse(by(verdicts, family, "E1").passed, family)

    def test_a_row_mismatch_in_a_family_fails_it(self):
        passes = three_passes()
        key = natively_required("cancel")[0]
        passes[0]["per_row"][key]["mismatched"] = 2
        verdicts = evaluate(passes)
        self.assertIn("family_row_mismatches=2", by(verdicts, "cancel", "E1").detail)
        self.assertFalse(by(verdicts, "cancel", "E1").passed)
        self.assertIn("family_row_mismatches=0", by(verdicts, "start", "E1").detail)


class E2Tests(unittest.TestCase):
    def fails(self, passes, needle):
        result = by(evaluate(passes), "start", "E2")
        self.assertFalse(result.passed, result.detail)
        self.assertIn(needle, result.detail)

    def test_fewer_than_three_passes_fail(self):
        self.fails(three_passes()[:2], "completed checklists, need at least 3")

    def test_one_day_fails(self):
        self.fails([clean_summary(0, 0), clean_summary(1, 0), clean_summary(2, 0)], "need at least 2")

    def test_two_builds_fail(self):
        passes = three_passes()
        passes[2]["session"]["git_sha"] = "f" * 40
        self.fails(passes, "builds, not one")

    def test_a_summary_from_before_the_session_block_fails(self):
        passes = three_passes()
        del passes[0]["session"]
        self.fails(passes, "no `session` block")

    def test_an_unknown_sha_or_a_disarmed_session_fails(self):
        passes = three_passes()
        passes[0]["session"]["git_sha"] = "unknown"
        self.fails(passes, "not a 40-hex commit")
        passes = three_passes()
        passes[0]["session"]["armed"] = False
        self.fails(passes, "armed is not true")

    def test_the_same_session_twice_is_one_pass(self):
        passes = three_passes()
        passes[1] = copy.deepcopy(passes[0])
        self.fails(passes, "same session")

    def test_two_launches_of_one_checklist_are_one_pass_not_two(self):
        # Sol's plant: two launches on day one plus one on day two passed as three passes.
        passes = three_passes()
        checklists = records(passes, [[0, 1], [2]])
        result = by(evaluate(passes, checklists=checklists), "start", "E2")
        self.assertFalse(result.passed)
        self.assertIn("2 completed checklists, need at least 3", result.detail)
        # Three checklists, one of them two launches: counted three times, on two days.
        passes = three_passes() + [clean_summary(3, 1)]
        result = by(evaluate(passes, checklists=records(passes, [[0, 1], [2], [3]])), "start", "E2")
        self.assertTrue(result.passed, result.detail)
        self.assertIn("checklists=3 sessions=4", result.detail)

    def test_a_summary_no_checklist_claims_fails(self):
        passes = three_passes() + [clean_summary(3, 1)]
        self.fails_with(passes, records(passes[:3]), "belongs to no checklist record")

    def fails_with(self, passes, checklists, needle):
        result = by(evaluate(passes, checklists=checklists), "start", "E2")
        self.assertFalse(result.passed, result.detail)
        self.assertIn(needle, result.detail)

    def test_a_checklist_that_is_incomplete_or_lists_a_missing_or_shared_session_fails(self):
        passes = three_passes()
        checklists = records(passes)
        checklists[0]["complete"] = False
        self.fails_with(passes, checklists, "not marked complete")
        checklists = records(passes)
        checklists[0]["sessions"].append({"pid": 4242, "started_unix_ms": 1})
        self.fails_with(passes, checklists, "has no summary")
        checklists = records(passes)
        checklists[1]["sessions"] = checklists[0]["sessions"]
        self.fails_with(passes, checklists, "also in another checklist")

    def test_the_date_is_the_local_day_of_the_first_session(self):
        passes = three_passes()
        checklists = records(passes)
        checklists[2]["date"] = checklists[0]["date"]  # day two claimed as day one
        self.fails_with(passes, checklists, "is not the local day of its first session")

    def test_summaries_from_another_commit_fail_even_when_they_agree_with_each_other(self):
        passes = three_passes()
        for summary in passes:
            summary["session"]["git_sha"] = "f" * 40
        result = by(evaluate(passes), "start", "E2")
        self.assertFalse(result.passed)
        self.assertIn("is not the candidate 0123456789ab", result.detail)

    def test_an_expect_sha_that_is_not_a_commit_is_refused(self):
        with self.assertRaises(ValueError):
            evaluate(three_passes(), expect_sha="main")


class E3Tests(unittest.TestCase):
    def test_a_required_row_below_five_hits_fails_its_family_only(self):
        passes = three_passes()
        key = natively_required("start")[0]
        for summary in passes:
            summary["per_row"][key]["matched"] = 1
        verdicts = evaluate(passes)
        result = by(verdicts, "start", "E3")
        self.assertFalse(result.passed)
        self.assertIn("below_5=1", result.detail)
        self.assertTrue(by(verdicts, "stop", "E3").passed)

    def test_hits_are_summed_over_passes_and_the_golden_replay(self):
        passes = [clean_summary(i, i // 2) for i in range(3)]
        key = natively_required("start")[0]
        for index, summary in enumerate(passes):
            summary["per_row"][key]["matched"] = 1 if index < 2 else 0
        self.assertFalse(by(evaluate(passes), "start", "E3").passed)  # 2 hits
        dump = e7_dump(per_row={key: row_entry(matched=3)})
        self.assertTrue(by(evaluate(passes, dump), "start", "E3").passed)  # 2 + 3

    def test_the_summary_counts_matched_closes_not_compared(self):
        passes = three_passes()
        key = natively_required("lifecycle")[0]
        for summary in passes:
            summary["per_row"][key].update(compared=50, matched=0)
        self.assertFalse(by(evaluate(passes), "lifecycle", "E3").passed)

    def test_replay_hits_never_stand_in_for_a_runbook_driven_row(self):
        # Sol's plant: zero native R209.5 and five replay hits passed.
        for key, family in (("209.5", "lifecycle"), ("101.5", "start")):
            passes = three_passes()
            for summary in passes:
                summary["per_row"].pop(key, None)
            result = by(evaluate(passes, e7_dump(per_row={key: row_entry(matched=5)})), family, "E3")
            self.assertFalse(result.passed, key)
            self.assertIn("native only", result.detail)
            passes = three_passes()
            for index, summary in enumerate(passes):
                summary["per_row"][key]["matched"] = 2 if index < 2 else 1  # five native closes
            self.assertTrue(by(evaluate(passes), family, "E3").passed, key)

    def test_the_replay_credit_is_matched_not_compared(self):
        passes = three_passes()
        key = natively_required("start")[0]
        for summary in passes:
            summary["per_row"][key]["matched"] = 0
        self.assertTrue(by(evaluate(passes, e7_dump(per_row={key: row_entry(matched=5)})), "start", "E3").passed)
        self.assertFalse(by(evaluate(passes, e7_dump(per_row={key: row_entry(matched=4)})), "start", "E3").passed)
        self.assertFalse(by(evaluate(passes, e7_dump(per_row={key: row_entry(matched=0, compared=500)})), "start", "E3").passed)

    def test_a_delegated_row_is_credited_matched_plus_delegated_and_other_rows_matched_only(self):
        # R201.4 and R205.4 decide a reply another site returns: no `matched`, a `delegated` close.
        for key, family in (("201.4", "stop"), ("205.4", "cancel")):
            passes = three_passes()
            for summary in passes:
                summary["per_row"][key] = {"compared": 99, "matched": 0, "delegated": 0}
            self.assertFalse(by(evaluate(passes), family, "E3").passed, key)
            for index, summary in enumerate(passes):
                summary["per_row"][key]["delegated"] = 2 if index < 2 else 1
            self.assertTrue(by(evaluate(passes), family, "E3").passed, key)
            # The replay credits it the same way.
            passes = three_passes()
            for summary in passes:
                summary["per_row"][key] = {"compared": 0, "matched": 0, "delegated": 0}
            dump = e7_dump(per_row={key: row_entry(delegated=5)})
            self.assertTrue(by(evaluate(passes, dump), family, "E3").passed, key)
        # An ordinary row's `delegated` count is not credit.
        passes = three_passes()
        key = natively_required("start")[0]
        for summary in passes:
            summary["per_row"][key].update(matched=0, delegated=50)
        self.assertFalse(by(evaluate(passes), "start", "E3").passed)

    def test_an_absent_matched_or_delegated_counter_fails_closed(self):
        key = "205.4"  # delegated: both counters are read
        for field in ("matched", "delegated"):
            passes = three_passes()
            del passes[0]["per_row"][key][field]
            result = by(evaluate(passes), "cancel", "E3")
            self.assertFalse(result.passed, field)
            self.assertIn("fails closed", result.detail)
        # The E7 dump too: a bare count or an entry without `matched` is from an older build.
        passes = three_passes()
        ordinary = natively_required("start")[0]
        for summary in passes:
            summary["per_row"][ordinary]["matched"] = 0
        for entry in (5, {"compared": 5}):
            result = by(evaluate(passes, e7_dump(per_row={ordinary: entry})), "start", "E3")
            self.assertFalse(result.passed, entry)
            self.assertIn("the E7 dump", result.detail)

    def test_a_bare_integer_replay_row_cannot_stand_in_for_matched(self):
        # Sol's plant (round 2, blocker 1): the old dump format keyed each row to its
        # `compared` count. `"101.15": 5` with no native match must FAIL, not pass as 5 closes.
        passes = three_passes()
        for summary in passes:
            summary["per_row"]["101.15"]["matched"] = 0
        self.assertFalse(by(evaluate(passes, e7_dump(per_row={"101.15": 5})), "start", "E3").passed)
        result = by(evaluate(passes, e7_dump(per_row={"101.15": 5})), "start", "E3")
        self.assertIn("the E7 dump", result.detail)
        # Only an explicit non-negative integer `matched` counts, and it must reach the bar.
        for entry in ({"compared": 5}, {"matched": "5"}, {"matched": True}, {"matched": -5}, {"matched": 5.0}, [5]):
            self.assertFalse(by(evaluate(passes, e7_dump(per_row={"101.15": entry})), "start", "E3").passed, entry)
        self.assertTrue(by(evaluate(passes, e7_dump(per_row={"101.15": row_entry(matched=5)})), "start", "E3").passed)

    def test_the_manifests_mirror_the_rust_delegated_rows(self):
        text = (ROOT / "app/src-tauri/src/dictation_shadow/replay.rs").read_text()
        match = re.search(r"DELEGATED_ROWS: &\[\(u16, u16\)\] = &\[(.*?)\];", text, re.S)
        self.assertIsNotNone(match, "replay::DELEGATED_ROWS")
        rust = {f"{site}.{row}" for site, row in re.findall(r"\((\d+),\s*(\d+)\)", match.group(1))}
        self.assertEqual(rust, delegated_rows())
        self.assertTrue(rust)
        for family in verdict.FAMILIES:
            data = manifest(family)
            waived = {entry["row"] for entry in data["not_natively_reachable"]}
            for entry in data.get("delegated", []):
                self.assertNotIn(entry["row"], waived)
                self.assertIn(entry["row"], {row["row"] for row in data["rows"]})

    def test_a_delegated_entry_needs_a_reason_and_must_not_be_waived(self):
        data = copy.deepcopy(manifest("cancel"))
        with tempfile.TemporaryDirectory() as tmp:
            data["delegated"][0]["why"] = ""
            (Path(tmp) / "cancel.json").write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                verdict.load_manifest("cancel", Path(tmp))

    def test_a_row_on_the_not_natively_reachable_list_needs_no_hits(self):
        data = manifest("stop")
        waived = data["not_natively_reachable"][0]["row"]
        passes = three_passes()
        for summary in passes:
            summary["per_row"].pop(waived, None)
        self.assertTrue(by(evaluate(passes), "stop", "E3").passed)

    def test_a_runbook_driven_row_is_natively_required(self):
        passes = three_passes()
        for summary in passes:
            summary["per_row"].pop("101.5", None)
        result = by(evaluate(passes), "start", "E3")
        self.assertFalse(result.passed)
        self.assertIn("R101.5=0", result.detail)

    def test_a_summary_without_matched_counters_cannot_count(self):
        passes = three_passes()
        for entry in passes[0]["per_row"].values():
            del entry["matched"]
        result = by(evaluate(passes), "start", "E3")
        self.assertFalse(result.passed)
        self.assertIn("fails closed", result.detail)


class LossAndPanicTests(unittest.TestCase):
    def test_a_lost_message_at_a_family_site_fails_that_family_only(self):
        passes = three_passes()
        passes[0]["lost_by_site"] = {"205": 1}
        verdicts = evaluate(passes)
        self.assertFalse(by(verdicts, "cancel", "E4").passed)
        self.assertTrue(by(verdicts, "start", "E4").passed)

    def test_an_unattributed_or_dropped_message_fails_every_family(self):
        for key in ("dropped", "lost_open", "loss_unattributed"):
            passes = three_passes()
            passes[2][key] = 1
            verdicts = evaluate(passes)
            for family in verdict.FAMILIES:
                self.assertFalse(by(verdicts, family, "E4").passed, f"{key} {family}")

    def test_the_loss_rate_is_overall_and_at_most_a_tenth_of_a_percent(self):
        passes = three_passes()
        passes[0]["lost_by_site"] = {"999": 20}  # a site of no family: E4 clean, E5 sees it
        verdicts = evaluate(passes)
        self.assertTrue(by(verdicts, "start", "E4").passed)
        self.assertFalse(by(verdicts, "start", "E5").passed)  # 20 / 15020 = 0.0013
        passes[0]["lost_by_site"] = {"999": 10}
        self.assertTrue(by(evaluate(passes), "start", "E5").passed)  # 10 / 15010 = 0.00067

    def test_a_shadow_panic_or_a_witness_overflow_fails_e6(self):
        for key in ("shadow_panic", "witness_overflow"):
            passes = three_passes()
            passes[1][key] = 1
            self.assertFalse(by(evaluate(passes), "start", "E6").passed, key)


class E7E8Tests(unittest.TestCase):
    def test_no_e7_file_or_fewer_than_100_repeats_fails_e7(self):
        with tempfile.TemporaryDirectory() as tmp:
            passes = []
            for index, summary in enumerate(three_passes()):
                summary["_path"] = f"{tmp}/p{index}.json"
                passes.append(summary)
            absent = verdict.evaluate(passes, None, ["start"], checklists=records(passes), expect_sha=SHA)
            self.assertFalse(by(absent, "start", "E7").passed)
            self.assertFalse(by(absent, "start", "E8").passed)
        self.assertFalse(by(evaluate(three_passes(), e7_dump(golden_repeat=1)), "start", "E7").passed)
        self.assertTrue(by(evaluate(three_passes(), e7_dump(golden_repeat=100)), "start", "E7").passed)

    def test_a_dump_without_a_revision_from_another_commit_or_from_a_dirty_tree_fails_e7(self):
        for change, needle in (
            ({"git_sha": None}, "no git_sha"),
            ({"git_sha": "f" * 40}, "not the candidate"),
            ({"dirty": True}, "clean checkout"),
            ({"dirty": None}, "clean checkout"),
        ):
            dump = e7_dump()
            if change.get("git_sha", 1) is None:
                del dump["git_sha"]
            else:
                dump.update(change)
            result = by(evaluate(three_passes(), dump), "start", "E7")
            self.assertFalse(result.passed, change)
            self.assertIn(needle, result.detail)

    def test_a_family_pd_never_witnessed_fails_e8_for_the_families_that_own_it(self):
        pds = e7_dump()["pd_witness"]
        pds["PD-16"] = 0  # the cancel family's
        verdicts = evaluate(three_passes(), e7_dump(pd_witness=pds))
        self.assertFalse(by(verdicts, "cancel", "E8").passed)
        self.assertIn("PD-16", by(verdicts, "cancel", "E8").detail)
        self.assertTrue(by(verdicts, "start", "E8").passed)
        self.assertTrue(by(verdicts, "lifecycle", "E8").passed)


class E9Tests(unittest.TestCase):
    def taint(self, **change):
        passes = three_passes()
        passes[1].update(change)
        return by(evaluate(passes), "start", "E9")

    def test_each_taint_counter_fails_e9(self):
        for key in (
            "abandoned_by_loss",
            "abandoned_by_panic",
            "skipped_incomplete",
            "late_message_discarded",
            "late_header_discarded",
            "unverifiable",
            "unconsumed",
            "outstanding_at_exit",
            "core_divergences",
        ):
            result = self.taint(**{key: 1})
            self.assertFalse(result.passed, key)
            self.assertIn(f"{key}=1", result.detail)

    def test_an_unexpected_incomplete_close_is_taint(self):
        result = self.taint(incomplete_closes={"flight_dropped_mid_run": 2})
        self.assertFalse(result.passed)
        self.assertIn("incomplete_closes[flight_dropped_mid_run]=2", result.detail)
        self.assertTrue(self.taint(incomplete_closes={"flight_dropped_mid_run": 0}).passed)

    def test_a_conservation_imbalance_in_the_total_books_taints_every_family(self):
        passes = three_passes()
        passes[0]["conservation"]["total"]["entries"] = 9  # one entry never resolved
        passes[0]["conservation"]["total"]["balanced"] = False  # what the build writes then
        verdicts = evaluate(passes)
        for family in verdict.FAMILIES:
            self.assertFalse(by(verdicts, family, "E9").passed, family)
        self.assertIn("conservation.total", by(verdicts, "start", "E9").detail)

    def test_balanced_false_taints_even_when_the_numbers_add_up(self):
        passes = three_passes()
        passes[0]["conservation"]["total"]["balanced"] = False
        result = by(evaluate(passes), "start", "E9")
        self.assertFalse(result.passed)
        self.assertIn("balanced=False", result.detail)

    def test_balanced_true_does_not_hide_numbers_that_do_not_add_up(self):
        passes = three_passes()
        passes[0]["conservation"]["total"]["entries"] = 9
        result = by(evaluate(passes), "start", "E9")
        self.assertFalse(result.passed)
        self.assertIn("entries=9", result.detail)

    def test_a_summary_without_the_total_books_is_taint(self):
        passes = three_passes()
        del passes[0]["conservation"]["total"]
        self.assertFalse(by(evaluate(passes), "start", "E9").passed)

    def test_unclassified_acquisitions_are_taint(self):
        passes = three_passes()
        passes[0]["conservation"]["unclassified"] = 1
        self.assertFalse(by(evaluate(passes), "start", "E9").passed)

    def test_per_site_entries_are_not_read_and_non_family_is_informational(self):
        passes = three_passes()
        passes[0]["conservation"]["101"] = {"headers": 999}
        passes[0]["conservation"]["non_family"] = {"app/src-tauri/src/keyboard.rs:120": 99}
        self.assertTrue(by(evaluate(passes), "start", "E9").passed)

    def test_a_summary_that_is_not_final_is_taint(self):
        for value in (False, None):
            passes = three_passes()
            passes[1]["final"] = value
            result = by(evaluate(passes), "start", "E9")
            self.assertFalse(result.passed, value)
            self.assertIn("not a final summary", result.detail)
        passes = three_passes()
        del passes[1]["final"]
        self.assertFalse(by(evaluate(passes), "start", "E9").passed)

    # The shutdown record, one test per fail-closed condition (#1173 round 3).

    def shutdown(self, **change):
        passes = three_passes()
        for key, value in change.items():
            if value is _ABSENT:
                del passes[1][key]
            else:
                passes[1][key] = value
        return by(evaluate(passes), "start", "E9")

    def assert_tainted(self, result, text):
        self.assertFalse(result.passed, result.detail)
        self.assertIn(text, result.detail)

    def test_a_clean_shutdown_record_passes(self):
        self.assertTrue(self.shutdown().passed)
        # `published_attempts`, `popped` and `push_lost` are information only: any counts
        # that add up pass, zero included.
        self.assertTrue(self.shutdown(published_attempts=0, popped=0, push_lost=0).passed)
        self.assertTrue(self.shutdown(published_attempts=10**6, popped=10**6, push_lost=0).passed)
        self.assertTrue(self.shutdown(published_attempts=7, popped=0, push_lost=7, dropped=7).passed)

    def test_a_non_final_write_fails_closed(self):
        self.assert_tainted(self.shutdown(final=False), "not a final summary")

    def test_a_missing_shutdown_quiesced_fails_closed(self):
        self.assert_tainted(self.shutdown(shutdown_quiesced=_ABSENT), "shutdown_quiesced")

    def test_a_missing_shutdown_unsettled_fails_closed(self):
        self.assert_tainted(self.shutdown(shutdown_unsettled=_ABSENT), "no `shutdown_unsettled`")

    def test_a_missing_published_attempts_fails_closed(self):
        self.assert_tainted(self.shutdown(published_attempts=_ABSENT), "no `published_attempts`")

    def test_a_missing_popped_fails_closed(self):
        self.assert_tainted(self.shutdown(popped=_ABSENT), "no `popped`")

    def test_a_missing_push_lost_fails_closed(self):
        self.assert_tainted(self.shutdown(push_lost=_ABSENT), "no `push_lost`")

    def test_a_broken_accounting_invariant_is_taint(self):
        # popped + push_lost + dropped_after_shutdown + shutdown_unsettled == published_attempts,
        # read once: a file where it fails was not written from one consistent read.
        for change in (
            {"popped": 1199},  # an attempt nobody accounts for
            {"popped": 1201},  # an attempt counted twice
            {"push_lost": 1, "dropped": 1},
            {"published_attempts": 1201},
            {"published_attempts": 1199},
        ):
            result = self.shutdown(**change)
            self.assert_tainted(result, "popped+push_lost+dropped_after_shutdown+shutdown_unsettled")
            self.assertIn("!= published_attempts", result.detail)
        # Every term counts: a refused or unsettled attempt balances only against attempts.
        balanced = self.shutdown(dropped_after_shutdown=1, popped=1199)
        self.assert_tainted(balanced, "dropped_after_shutdown=1")
        self.assertNotIn("!= published_attempts", balanced.detail)
        balanced = self.shutdown(shutdown_unsettled=2, popped=1198)
        self.assertNotIn("!= published_attempts", balanced.detail)
        # The terms do not overlap: counting a queue rejection in `dropped_after_shutdown` as well
        # (a `push_lost` of 10 and a `dropped_after_shutdown` of 10 for the same 10 attempts) breaks it.
        overlapping = self.shutdown(dropped_after_shutdown=10, push_lost=10, popped=1190, dropped=10)
        self.assertIn("!= published_attempts", overlapping.detail)

    def test_push_lost_above_the_folded_drops_is_an_unfolded_loss(self):
        # `push_lost` is a part of the older `dropped`: `dropped` is `push_lost` plus tickets
        # dropped unsent plus `dropped_unavailable`, so `push_lost <= dropped - dropped_unavailable`.
        # `dropped` is folded at the last drain and `push_lost` is read later; a rejection landing
        # between the two shows as `push_lost` above the bound.
        holds = self.shutdown(push_lost=3, dropped=5, dropped_unavailable=2, popped=1197)
        self.assertNotIn("unfolded loss", holds.detail)
        self.assertNotIn("not all counts", holds.detail)
        self.assertNotIn("!= published_attempts", holds.detail)
        # The bound met with equality holds too.
        self.assertNotIn("unfolded loss", self.shutdown(push_lost=3, dropped=3, popped=1197).detail)
        # One above the bound: the accounting still adds up, the E9 taint is the unfolded loss.
        fails = self.shutdown(push_lost=4, dropped=5, dropped_unavailable=2, popped=1196)
        self.assert_tainted(fails, "push_lost=4 > dropped-dropped_unavailable=3")
        self.assertNotIn("!= published_attempts", fails.detail)
        # `dropped_unavailable` is not the queue's: dropped=2 of which 2 unavailable leaves a bound of 0.
        self.assert_tainted(self.shutdown(push_lost=1, dropped=2, dropped_unavailable=2, popped=1199), "unfolded loss")

    def test_a_missing_or_malformed_bound_term_fails_closed(self):
        for field in ("dropped", "dropped_unavailable"):
            self.assert_tainted(self.shutdown(**{field: _ABSENT}), "not all counts")
            for value in ("0", None, True, -1, 1.0):
                self.assert_tainted(self.shutdown(**{field: value}), "not all counts")

    def test_a_missing_dropped_after_shutdown_fails_closed(self):
        self.assert_tainted(self.shutdown(dropped_after_shutdown=_ABSENT), "no `dropped_after_shutdown`")

    def test_a_shutdown_that_did_not_quiesce_is_taint(self):
        for value in (False, None, "true", 1):
            self.assert_tainted(self.shutdown(shutdown_quiesced=value), "shutdown_quiesced")

    def test_unsettled_attempts_at_the_final_sample_are_taint(self):
        for value in (1, 5):
            self.assert_tainted(self.shutdown(shutdown_unsettled=value), f"shutdown_unsettled={value}")

    def test_attempts_refused_after_the_close_are_taint(self):
        for value in (1, 7):
            self.assert_tainted(self.shutdown(dropped_after_shutdown=value), f"dropped_after_shutdown={value}")

    def test_a_shutdown_counter_that_is_not_a_count_is_taint(self):
        for field in ("shutdown_unsettled", "dropped_after_shutdown", "published_attempts", "popped", "push_lost"):
            for value in ("0", None, True, -1, 0.0):
                self.assert_tainted(self.shutdown(**{field: value}), f"{field}={value!r}")

    def test_rejected_after_close_is_only_an_alias_that_fails_when_above_zero(self):
        # #1173 has no such field (it is `dropped_after_shutdown`); absent is fine, zero is
        # fine, above zero fails.
        self.assertTrue(self.shutdown(rejected_after_close=0).passed)
        self.assert_tainted(self.shutdown(rejected_after_close=2), "rejected_after_close=2")

    def test_the_named_no_reply_close_is_expected_not_taint(self):
        passes = three_passes()
        passes[0]["named_no_reply"] = 40
        self.assertTrue(by(evaluate(passes), "start", "E9").passed)

    def test_a_summary_without_the_new_taint_fields_cannot_count(self):
        for field in ("outstanding_at_exit", "conservation", "incomplete_closes", "core_divergences"):
            passes = three_passes()
            del passes[0][field]
            result = by(evaluate(passes), "start", "E9")
            self.assertFalse(result.passed, field)
            self.assertIn(f"no `{field}`", result.detail)


class ManifestTests(unittest.TestCase):
    """The manifests name the C3 rows of the code and the tests that drive them."""

    @classmethod
    def c3_rows(cls) -> list[tuple[str, int]]:
        core = ROOT / "app/src-tauri/crates/dictation-core/src"
        codes = {
            m.group(1): int(m.group(2))
            for m in re.finditer(r"Self::(\w+) => (\d+),", (core / "decision.rs").read_text())
        }
        rows = re.findall(r'\("(R\d{3}[ab]?\.\d+[a-er]?)", SiteId::(\w+)\)', (core / "rows.rs").read_text())
        found = []
        for rid, site in rows:
            tail = rid.split(".", 1)[1]
            digits = re.match(r"\d+", tail).group()
            step = {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5, "r": 6}.get(tail[len(digits)][:1] if len(tail) > len(digits) else "", 0)
            found.append((f"{codes[site]}.{int(digits) + 100 * step}", codes[site]))
        return found

    def test_the_four_manifests_together_hold_every_c3_row_once(self):
        expected = self.c3_rows()
        self.assertEqual(len(expected), 142)
        seen = []
        for family in verdict.FAMILIES:
            data = manifest(family)
            seen.extend(entry["row"] for entry in data["rows"])
            for entry in data["rows"]:
                site = int(entry["row"].split(".")[0])
                self.assertIn(site, data["sites"], f"{entry['row']} is not a site of {family}")
        self.assertEqual(sorted(seen), sorted(key for key, _ in expected))

    def test_every_not_natively_reachable_row_names_an_existing_test(self):
        for family in verdict.FAMILIES:
            data = manifest(family)
            for entry in data["not_natively_reachable"]:
                self.assertIn(entry["kind"], ("golden", "unit"))
                text = (ROOT / entry["file"]).read_text()
                self.assertRegex(
                    text, rf"fn {re.escape(entry['name'])}\b", f"{family} {entry['row']}: {entry['file']}"
                )
                self.assertTrue(entry["why"].strip())

    @staticmethod
    def function_body(text: str, name: str) -> str:
        match = re.search(rf"\bfn {re.escape(name)}\b", text)
        start = text.index("{", match.end())
        depth = 0
        for index in range(start, len(text)):
            depth += {"{": 1, "}": -1}.get(text[index], 0)
            if depth == 0:
                return text[match.start() : index + 1]
        raise AssertionError(name)

    @staticmethod
    def strip_comments(text: str) -> str:
        """Rust source without `//` and `/* */` comments. Strings and char literals are kept
        whole, so a `//` inside one is not a comment. (Raw strings do not occur in the cited
        tests.)"""
        out: list[str] = []
        i, n = 0, len(text)
        while i < n:
            two = text[i : i + 2]
            if two == "//":
                while i < n and text[i] != "\n":
                    i += 1
            elif two == "/*":
                depth = 1
                i += 2
                while i < n and depth:
                    pair = text[i : i + 2]
                    if pair == "/*":
                        depth, i = depth + 1, i + 2
                    elif pair == "*/":
                        depth, i = depth - 1, i + 2
                    else:
                        i += 1
                out.append(" ")
            elif text[i] == '"':
                j = i + 1
                while j < n and text[j] != '"':
                    j += 2 if text[j] == "\\" else 1
                out.append(text[i : j + 1])
                i = j + 1
            elif text[i] == "'" and re.match(r"'(\\.|[^'\\])'", text[i:]):
                end = text.index("'", i + 2 if text[i + 1] == "\\" else i + 1) + 1
                out.append(text[i:end])
                i = end
            else:
                out.append(text[i])
                i += 1
        return "".join(out)

    DECIDES = re.compile(r"\b(decide\w*|d_\d+\w*|start|tail|stop|expect_\w+|assert_pure_refusal)\(")
    NOT_A_DECISION = re.compile(r"ordinal|C3_ROWS|Split::|allowlist")
    # Calls whose arguments only print or format: a row id there is not an assertion.
    NOT_AN_ASSERTION = {
        "print", "println", "eprint", "eprintln", "dbg", "format", "write", "writeln", "panic",
        "info", "warn", "debug", "trace", "error", "log",
    }

    @classmethod
    def waiver_problem(cls, body: str, shown: str, row_id: str, name: str) -> str | None:
        """Why a cited unit test does not execute the row's assertion (None when it does).
        Comments are dropped first, so a case commented out of the table, or a row named only
        in a comment, does not count. The row id must be a string in code, inside parentheses
        (a row-asserting call or macro, a decision call's argument, or a case tuple of the table
        the loop asserts over), and the test must call a decision. It is not an execution
        tracer: it rules out a row that survives only in a comment or outside any call."""
        code = cls.strip_comments(body)
        literal = f'"{shown}"'
        # The literal must sit inside parentheses (a row-asserting call or macro, a decision
        # call's argument, or a case tuple), and none of the calls around it may be a print or
        # a log: `eprintln!("R203.2r")` names the row without asserting anything.
        stack: list[str] = []
        inside = False
        for match in re.finditer(r'"(?:[^"\\]|\\.)*"|[()]|[A-Za-z_]\w*!?(?=\s*\()', code):
            token = match.group()
            if token == "(":
                before = code[: match.start()].rstrip()
                opener = re.search(r"([A-Za-z_]\w*)!?$", before)
                stack.append(opener.group(1) if opener else "")
            elif token == ")":
                if stack:
                    stack.pop()
            elif token == literal:
                if stack and not any(call in cls.NOT_AN_ASSERTION for call in stack):
                    inside = True
        if not inside:
            return f"{row_id}: {name} never names the row in an assertion, a decision argument or a case table (a comment or a log line does not count)"
        if not cls.DECIDES.search(code):
            return f"{row_id}: {name} calls no decision"
        if cls.NOT_A_DECISION.search(code):
            return f"{row_id}: {name} is a registry or conversion test"
        return None

    def test_a_cited_unit_test_drives_the_decision_and_asserts_the_row(self):
        # Sol round 1, finding 7, and round 2, minor 6: a waiver that cites a row-number
        # conversion test or an allowlist test says nothing about the decision, and a row named
        # only in a comment (a case commented out) is not asserted. The cited test names the
        # row's id as a string in code and calls and asserts a decision, and is not a registry,
        # ordinal or split-signature test.
        bad = []
        for family in verdict.FAMILIES:
            for entry in manifest(family)["not_natively_reachable"]:
                if entry["kind"] != "unit":
                    continue
                body = self.function_body((ROOT / entry["file"]).read_text(), entry["name"])
                # A status-blind alias (R202.2 is R202.1, R203.10 is R203.9) is never produced as a
                # decision row; its test asserts the aliased row and names the alias.
                shown = entry.get("alias_of", entry["id"])
                problem = self.waiver_problem(body, shown, entry["id"], entry["name"])
                if problem:
                    bad.append(problem)
        self.assertEqual(bad, [])

    def cited_body(self, family: str, row_id: str) -> tuple[str, dict]:
        entry = next(e for e in manifest(family)["not_natively_reachable"] if e["id"] == row_id)
        return self.function_body((ROOT / entry["file"]).read_text(), entry["name"]), entry

    def test_a_case_commented_out_of_the_cited_test_loses_the_waiver(self):
        # Sol's plant: the `Recovering` case of R203.2r commented out. The row string stays in
        # the comment, and the decision call for the other cases still runs.
        body, entry = self.cited_body("stop", "R203.2r")
        self.assertIsNone(self.waiver_problem(body, entry["id"], entry["id"], entry["name"]))
        line = next(l for l in body.splitlines() if '"R203.2r"' in l and not l.lstrip().startswith("//"))
        planted = body.replace(line, "        // " + line.strip())
        self.assertNotEqual(body, planted)
        self.assertIn("never names the row in an assertion", self.waiver_problem(planted, entry["id"], entry["id"], entry["name"]))
        block = body.replace(line, "        /* " + line.strip() + " */")
        self.assertIn("never names the row in an assertion", self.waiver_problem(block, entry["id"], entry["id"], entry["name"]))

    def test_a_log_line_naming_the_row_does_not_keep_the_waiver(self):
        # Sol round 3, minor 4: `eprintln!("R203.2r")` beside the commented-out case.
        body, entry = self.cited_body("stop", "R203.2r")
        line = next(l for l in body.splitlines() if '"R203.2r"' in l and not l.lstrip().startswith("//"))
        for logged in ('eprintln!("R203.2r");', 'println!("R203.2r");', 'dbg!("R203.2r");', 'tracing::info!("R203.2r");', 'let _ = format!("{}", "R203.2r");'):
            planted = body.replace(line, "        // " + line.strip()).replace("    for (status, row) in cases {", f"    {logged}\n    for (status, row) in cases {{")
            self.assertNotEqual(body, planted)
            self.assertIn(
                "never names the row in an assertion",
                self.waiver_problem(planted, entry["id"], entry["id"], entry["name"]),
                logged,
            )
        # An assertion or a decision argument still counts.
        for good in ('assert_eq!(row, "R203.2r");', 'expect_203(&d, "R203.2r", term, true, FAIL_TAIL, status);'):
            planted = body.replace(line, "        // " + line.strip()).replace("    for (status, row) in cases {", f"    {good}\n    for (status, row) in cases {{")
            self.assertIsNone(self.waiver_problem(planted, entry["id"], entry["id"], entry["name"]), good)

    def test_a_row_named_only_outside_a_call_or_case_is_not_asserted(self):
        body, entry = self.cited_body("stop", "R203.2r")
        loose = 'fn x() {\n    let unused = "R203.2r";\n    let d = d_203(S, RID, E);\n    assert_eq!(d.reply, None);\n}\n'
        self.assertIn("never names the row", self.waiver_problem(loose, entry["id"], entry["id"], "x"))
        stripped = self.strip_comments('let u = "a // not a comment"; // gone\n/* gone /* nested */ */ let v = 1;')
        self.assertIn('"a // not a comment"', stripped)
        self.assertNotIn("gone", stripped)
        self.assertIn("let v = 1;", stripped)

    def test_a_cited_golden_is_a_scenario_or_a_pd_witness_test(self):
        for family in verdict.FAMILIES:
            for entry in manifest(family)["not_natively_reachable"]:
                if entry["kind"] == "golden":
                    self.assertRegex(entry["name"], r"^(golden_s\d+_|pd\d+)", f"{family} {entry['row']}")

    def test_a_row_the_native_pass_reached_is_not_on_the_waiver_list(self):
        # The rows pass 1 hit at least five times need their hits, not a waiver.
        hit = {
            "101.4", "101.11", "101.15", "102.1", "103.1", "104.1", "104.2", "104.7", "201.8",
            "2021.1", "203.6", "203.9", "2041.1", "2041.2", "2042.1", "2042.6", "205.4", "205.5",
            "206.1", "207.4", "207.5", "209.3", "209.4", "210.2", "210.4", "211.6", "212.1",
        }
        for family in verdict.FAMILIES:
            waived = {e["row"] for e in manifest(family)["not_natively_reachable"]}
            self.assertEqual(waived & hit, set(), family)

    def test_the_rows_pass_1_missed_are_waived_with_a_test_or_driven_by_a_runbook_step(self):
        missed = [
            "101.8", "101.9", "101.10", "203.4", "203.5", "205.2", "205.3", "209.2", "209.6",
            "209.7", "210.3", "2041.205", "2042.204",
            *[f"{site}.{n}" for site, ns in ((213, (1, 2)), (214, (1, 2)), (215, (1, 2, 3)), (216, (1, 2, 3, 4, 5)), (208, (1, 2))) for n in ns],
        ]
        runbook = (ROOT / "docs/testing/dictation-shadow-soak.md").read_text()
        for key in missed + ["101.5", "209.5"]:
            family = next(f for f in verdict.FAMILIES if key in {e["row"] for e in manifest(f)["rows"]})
            data = manifest(family)
            waived = {e["row"] for e in data["not_natively_reachable"]}
            driven = {e["row"]: e["step"] for e in data["runbook_driven"]}
            self.assertTrue(key in waived or key in driven, f"{key} is neither waived nor driven")
            if key in driven:
                self.assertIn(driven[key], runbook, f"{key}: the runbook has no step named {driven[key]!r}")

    def test_runbook_driven_rows_must_be_rows_of_the_family_and_not_waived(self):
        with tempfile.TemporaryDirectory() as tmp:
            for family in verdict.FAMILIES:
                data = copy.deepcopy(manifest(family))
                (Path(tmp) / f"{family}.json").write_text(json.dumps(data))
            bad = json.loads((Path(tmp) / "start.json").read_text())
            bad["runbook_driven"] = [{"row": bad["not_natively_reachable"][0]["row"], "step": "x"}]
            (Path(tmp) / "start.json").write_text(json.dumps(bad))
            with self.assertRaises(ValueError):
                verdict.load_manifest("start", Path(tmp))
            bad["runbook_driven"] = [{"row": "999.9", "step": "x"}]
            (Path(tmp) / "start.json").write_text(json.dumps(bad))
            with self.assertRaises(ValueError):
                verdict.load_manifest("start", Path(tmp))

    def test_a_waiver_with_no_test_or_no_reason_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            for field in ("name", "why"):
                data = copy.deepcopy(manifest("cancel"))
                data["not_natively_reachable"][0][field] = ""
                (Path(tmp) / "cancel.json").write_text(json.dumps(data))
                with self.assertRaises(ValueError, msg=field):
                    verdict.load_manifest("cancel", Path(tmp))


class RunbookE7StepTests(unittest.TestCase):
    """Sol round 3, finding 1: a failed E7 retry must not leave a passing dump to point at."""

    @staticmethod
    def snippet() -> str:
        text = (ROOT / "docs/testing/dictation-shadow-soak.md").read_text()
        start = text.index("unset E7_OUT\n")
        return text[start : text.index("# Site order (C8.7)", start)]

    def attempt(self, tmp: Path, *, passes: bool, soak: Path) -> tuple[str, str]:
        """Run the runbook's E7 step with a stub `cargo`; return (stdout, E7_OUT afterwards)."""
        stub = tmp / "bin"
        stub.mkdir(exist_ok=True)
        cargo = stub / "cargo"
        cargo.write_text(
            "#!/bin/sh\n"
            + ('echo "{}" > "$MURMUR_E7_PER_ROW_OUT"\n' if passes else "exit 101\n")
        )
        cargo.chmod(0o755)
        script = f'SOAK="{soak}"\n{self.snippet()}\necho "E7_OUT=${{E7_OUT-<unset>}}"\n'
        result = subprocess.run(
            ["bash", "-c", script],
            capture_output=True,
            text=True,
            env={"PATH": f"{stub}:{os.environ['PATH']}", "HOME": str(tmp)},
        )
        out = result.stdout
        return out, out.strip().splitlines()[-1].removeprefix("E7_OUT=")

    def test_a_failed_retry_never_leaves_an_e7_out_and_attempts_do_not_share_a_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            soak = tmp / "soak"
            soak.mkdir()
            out, first = self.attempt(tmp, passes=True, soak=soak)
            self.assertTrue(Path(first).is_file(), out)
            self.assertIn(f"--e7 {first}", out)
            # The retry fails (exit 101): no path is printed and E7_OUT is not the first one.
            out, second = self.attempt(tmp, passes=False, soak=soak)
            self.assertEqual(second, "<unset>", out)
            self.assertNotIn("pass this file", out)
            self.assertIn("attempt failed", out)
            # A passing retry is a different directory from the first attempt's.
            _, third = self.attempt(tmp, passes=True, soak=soak)
            self.assertNotEqual(first, third)
            self.assertTrue(Path(third).is_file())

    def test_the_verdict_example_refuses_an_unset_dump(self):
        text = (ROOT / "docs/testing/dictation-shadow-soak.md").read_text()
        self.assertIn('--e7 "${E7_OUT:?', text)
        self.assertNotIn("e7-per-row.json\"", text.replace('"$E7_DIR/e7-per-row.json"', ""))


class CommandLineTests(unittest.TestCase):
    def run_main(self, passes, e7, *extra):
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for index, summary in enumerate(passes):
                path = Path(tmp) / f"pass{index}.json"
                path.write_text(json.dumps(summary))
                paths.append(str(path))
            arguments = [*paths, *extra]
            if "--expect-sha" not in extra:
                arguments += ["--expect-sha", SHA]
            if "--checklist" not in extra:
                for number, record in enumerate(records(passes)):
                    record = {k: v for k, v in record.items() if k != "_path"}
                    path = Path(tmp) / f"checklist{number}.json"
                    path.write_text(json.dumps(record))
                    arguments += ["--checklist", str(path)]
            if e7 is not None:
                path = Path(tmp) / "e7.json"
                path.write_text(json.dumps(e7))
                arguments += ["--e7", str(path)]
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = verdict.main(arguments)
            return code, out.getvalue(), err.getvalue()

    def test_a_clean_run_exits_zero_and_prints_every_line(self):
        code, out, err = self.run_main(three_passes(), e7_dump())
        self.assertEqual(code, 0, out + err)
        self.assertEqual(out.count(" PASS "), 36)
        self.assertIn("36 PASS, 0 FAIL", out)

    def test_a_failing_criterion_exits_one(self):
        passes = three_passes()
        passes[0]["shadow_panic"] = 1
        code, out, _ = self.run_main(passes, e7_dump(), "--family", "stop")
        self.assertEqual(code, 1)
        self.assertRegex(out, r"stop\s+E6\s+FAIL")
        self.assertNotIn("start ", out)

    def test_a_bare_per_row_map_is_not_an_e7_dump(self):
        code, _, err = self.run_main(three_passes(), {"101.15": 3})
        self.assertEqual(code, 2)  # no revision, no repeat count: refused
        self.assertIn("not an E7 dump", err)

    def test_the_expect_sha_is_required_and_binds_the_evidence(self):
        code, _, err = self.run_main(three_passes(), e7_dump(), "--expect-sha", "f" * 40)
        self.assertEqual(code, 1)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err), self.assertRaises(SystemExit):
            verdict.main(["x.json"])

    def test_unreadable_input_exits_two(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = verdict.main(["/nonexistent/summary.json", "--expect-sha", SHA])
        self.assertEqual(code, 2)
        self.assertIn("error:", err.getvalue())


if __name__ == "__main__":
    unittest.main()
