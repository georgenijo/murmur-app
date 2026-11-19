import contextlib
import copy
import io
import json
import re
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


def clean_summary(index: int = 0, day: int = 0, **overrides) -> dict:
    """A summary of one clean native pass: every natively required row matched 10 times."""
    per_row = {}
    for family in verdict.FAMILIES:
        for key in natively_required(family):
            per_row[key] = {"compared": 10, "matched": 10, "mismatched": 0, "pd_witness": 0}
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
        "conservation": {
            str(site): {"entries": 8, "headers": 6, "no_decision": 1, "unwound": 1} for site in sorted(sites)
        }
        | {"non_family": {"keyboard": 3}},
        "per_row": per_row,
    }
    summary.update(overrides)
    return summary


def e7_dump(**overrides) -> dict:
    pds = {f"PD-{n}": 3 for n in (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 16, 18)}
    dump = {"golden_repeat": 100, "per_row": {}, "pd_witness": pds}
    dump.update(overrides)
    return dump


def three_passes() -> list[dict]:
    return [clean_summary(0, 0), clean_summary(1, 0), clean_summary(2, 1)]


def evaluate(summaries: list[dict], e7: dict | None = None, families=None):
    with tempfile.TemporaryDirectory() as tmp:
        loaded = []
        for index, summary in enumerate(summaries):
            summary = dict(summary)
            summary["_path"] = f"{tmp}/pass{index}.json"
            loaded.append(summary)
        return verdict.evaluate(loaded, e7 if e7 is not None else e7_dump(), families or list(verdict.FAMILIES))


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
        self.assertRegex(line, r"^start\s+E2\s+PASS\s+passes=3 days=2 builds=1")


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
        self.fails(three_passes()[:2], "need at least 3")

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
        dump = e7_dump(per_row={key: 3})
        self.assertTrue(by(evaluate(passes, dump), "start", "E3").passed)  # 2 + 3

    def test_the_summary_counts_matched_closes_not_compared(self):
        passes = three_passes()
        key = natively_required("lifecycle")[0]
        for summary in passes:
            summary["per_row"][key].update(compared=50, matched=0)
        self.assertFalse(by(evaluate(passes), "lifecycle", "E3").passed)

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
        self.assertIn("before S2", result.detail)


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
            absent = verdict.evaluate(passes, None, ["start"])
            self.assertFalse(by(absent, "start", "E7").passed)
            self.assertFalse(by(absent, "start", "E8").passed)
        self.assertFalse(by(evaluate(three_passes(), e7_dump(golden_repeat=1)), "start", "E7").passed)
        self.assertTrue(by(evaluate(three_passes(), e7_dump(golden_repeat=100)), "start", "E7").passed)

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

    def test_a_conservation_imbalance_at_a_family_site_taints_that_family(self):
        passes = three_passes()
        passes[0]["conservation"]["101"]["entries"] = 9  # one entry never resolved
        verdicts = evaluate(passes)
        self.assertFalse(by(verdicts, "start", "E9").passed)
        self.assertIn("conservation[101]", by(verdicts, "start", "E9").detail)
        self.assertTrue(by(verdicts, "cancel", "E9").passed)

    def test_non_family_acquisitions_are_reported_not_judged(self):
        passes = three_passes()
        passes[0]["conservation"]["non_family"] = {"keyboard": 99}
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
        self.assertEqual(len(expected), 141)
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


class CommandLineTests(unittest.TestCase):
    def run_main(self, passes, e7, *extra):
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for index, summary in enumerate(passes):
                path = Path(tmp) / f"pass{index}.json"
                path.write_text(json.dumps(summary))
                paths.append(str(path))
            arguments = [*paths, *extra]
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

    def test_the_bare_per_row_map_is_accepted_for_e7(self):
        code, out, _ = self.run_main(three_passes(), {"101.15": 3})
        self.assertEqual(code, 1)  # no repeat count, so E7 and E8 fail closed
        self.assertRegex(out, r"start\s+E7\s+FAIL")

    def test_unreadable_input_exits_two(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = verdict.main(["/nonexistent/summary.json"])
        self.assertEqual(code, 2)
        self.assertIn("error:", err.getvalue())


if __name__ == "__main__":
    unittest.main()
