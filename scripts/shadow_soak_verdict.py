#!/usr/bin/env python3
"""Evaluate the dictation-shadow exit criteria E1-E9 for each family (D-S2c-15 S6).

    python3 scripts/shadow_soak_verdict.py --expect-sha <40-hex candidate> --e7 e7.json \\
        --checklist c1.json --checklist c2.json --checklist c3.json  s1.json s2.json s3.json ...

Each positional argument is the `dictation-shadow-summary.json` of one native *session* (a
launch, copied after a clean exit, see docs/testing/dictation-shadow-soak.md). A checklist
(`--checklist`, one per completed native checklist) is a record that lists the sessions that
made it up, so two launches of one checklist are one pass, not two:

    {"label": "pass1", "date": "2026-11-18", "complete": true,
     "sessions": [{"pid": 69329, "started_unix_ms": 1795063000000}, ...]}

`--expect-sha` is the candidate commit the passes and the replay were meant to test: every
session's `session.git_sha` and the E7 dump's `git_sha` must be it. `--e7` is the file the E8
test writes when `MURMUR_E7_PER_ROW_OUT` is set (only after the test passes); run that test
with `MURMUR_GOLDEN_REPEAT=100` on a clean checkout of the candidate.

For every family the script prints one line per criterion, `PASS` or `FAIL` with the values
read, and exits 1 if any line is a FAIL. Every input is fail closed: a summary without the
`session` block, `matched` counters or conservation counters comes from a build before the
S1-S4 evidence and cannot count.

The row manifests (docs/testing/shadow-rows/<family>.json) list each family's C3 rows, the
rows a native pass cannot produce (with the golden or unit test that drives each), and the
rows a step of the runbook drives (those need 5 native `matched` closes of their own: replay
hits do not count for them). `delegated` rows (R201.4, R205.4: they decide a reply another site
returns, so they close as the announced no-reply close and have no `matched`) mirror
`replay::DELEGATED_ROWS` and are credited `matched + delegated`, never the header count. The summary
fields it reads are the ones the dictation shadow writes: `session`, `outstanding_at_exit`,
`per_row[..].matched`, `incomplete_closes`, `conservation` (`total` books with `balanced`,
headers per site, `non_family` by file:line), `core_divergences`, and `final` (true only on the
clean-exit write: a summary without it is a crash or a kill and taints the pass).
`named_no_reply` is an expected close kind and is not taint.

Numbers only: nothing here reads or prints transcript, audio, clipboard or selection content.
"""

from __future__ import annotations

import argparse
import datetime
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFESTS = ROOT / "docs/testing/shadow-rows"
FAMILIES = ("start", "lifecycle", "stop", "cancel")

# E-criteria thresholds (appendix C8.5).
E2_PASSES = 3
E2_DAYS = 2
E3_HITS = 5
E5_RATE = 0.001
E7_REPEAT = 100

# The counters whose non-zero value means the shadow gave up a flight it should have compared
# (E9). `unconsumed` is read at a clean exit, which every counted summary is.
TAINT_COUNTERS = (
    "abandoned_by_loss",
    "abandoned_by_panic",
    "skipped_incomplete",
    "late_message_discarded",
    "late_header_discarded",
    "unverifiable",
    "unconsumed",
    "outstanding_at_exit",
    "core_divergences",
)

# The shutdown record of a final write (`final: true`), as #1173 round 4 names it (E9). All six
# fields are required, and fail closed: a missing field is a build before it was written.
#   * `shutdown_quiesced`: `true` only if the wait finished before the deadline, no attempt
#     arrived while the file was being written, nothing was refused after the close and
#     nothing was unaccounted for. Must be `true`.
#   * `shutdown_unsettled`: attempts unaccounted for at the final read. Must be `0`.
#   * `dropped_after_shutdown`: publish attempts refused after the close. Must be `0`.
#   * `published_attempts`, `popped`, `push_lost`: every publish attempt of the session, the
#     messages the comparison thread took off the queue, and the attempts the queue itself
#     rejected. Required counts, information only, and the terms of the invariant below.
# Each attempt ends in exactly one place (`Transport::push`: refused after the close, rejected
# by the queue, or queued and later popped; the rest is unsettled), so the four terms do not
# overlap (`push_lost` is a part of the summary's separate `dropped`, not of
# `dropped_after_shutdown`), and all four are read once, attempts last:
#     popped + push_lost + dropped_after_shutdown + shutdown_unsettled == published_attempts
# A file for which it does not hold was not written from one consistent read: taint.
SHUTDOWN_QUIESCED = "shutdown_quiesced"
SHUTDOWN_ZERO_COUNTS = ("shutdown_unsettled", "dropped_after_shutdown")
SHUTDOWN_INFORMATION_COUNTS = ("published_attempts", "popped", "push_lost")
# `rejected_after_close` is not a field of the summary: #1173 calls the same thing
# `dropped_after_shutdown`. Kept only as an alias: when present and above 0 it is taint.
SHUTDOWN_ALIAS_COUNTS = ("rejected_after_close",)

CRITERIA = ("E1", "E2", "E3", "E4", "E5", "E6", "E7", "E8", "E9")


class Verdict:
    def __init__(self, family: str, criterion: str, passed: bool, detail: str):
        self.family = family
        self.criterion = criterion
        self.passed = passed
        self.detail = detail

    def line(self) -> str:
        return f"{self.family:<9} {self.criterion}  {'PASS' if self.passed else 'FAIL'}  {self.detail}"


def num(summary: dict, key: str) -> int:
    value = summary.get(key, 0)
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def load_summaries(paths: list[Path]) -> list[dict]:
    summaries = []
    for path in paths:
        summary = json.loads(path.read_text())
        if not isinstance(summary, dict):
            raise ValueError(f"{path}: not a summary object")
        summary["_path"] = str(path)
        summaries.append(summary)
    return summaries


def load_checklist(path: Path) -> dict:
    record = json.loads(path.read_text())
    problems = []
    if not isinstance(record, dict):
        raise ValueError(f"{path}: not a checklist record")
    if not isinstance(record.get("complete"), bool):
        problems.append("`complete` is not true/false")
    date = record.get("date")
    try:
        datetime.date.fromisoformat(date)
    except (TypeError, ValueError):
        problems.append(f"`date` is {date!r}, not YYYY-MM-DD")
    sessions = record.get("sessions")
    if not isinstance(sessions, list) or not sessions:
        problems.append("`sessions` is empty")
    else:
        for index, entry in enumerate(sessions):
            if not (
                isinstance(entry, dict)
                and isinstance(entry.get("pid"), int)
                and isinstance(entry.get("started_unix_ms"), int)
            ):
                problems.append(f"sessions[{index}] needs integer `pid` and `started_unix_ms`")
    if problems:
        raise ValueError(f"{path}: " + "; ".join(problems))
    record["_path"] = str(path)
    return record


def load_manifest(family: str, directory: Path = MANIFESTS) -> dict:
    manifest = json.loads((directory / f"{family}.json").read_text())
    reachable = {entry["row"] for entry in manifest["not_natively_reachable"]}
    rows = {entry["row"] for entry in manifest["rows"]}
    problems = []
    if not reachable <= rows:
        problems.append(f"rows listed as not natively reachable but not in the family: {sorted(reachable - rows)}")
    for entry in manifest["not_natively_reachable"]:
        for field in ("test", "why"):
            if field == "test" and not (entry.get("file") and entry.get("name")):
                problems.append(f"{entry['row']}: names no test")
            if field == "why" and not str(entry.get("why", "")).strip():
                problems.append(f"{entry['row']}: gives no reason")
    for entry in manifest.get("delegated", []):
        if entry.get("row") not in rows:
            problems.append(f"{entry.get('row')}: delegated but not a row of the family")
        if not str(entry.get("why", "")).strip():
            problems.append(f"{entry.get('row')}: a delegated row gives a reason")
        if entry.get("row") in reachable:
            problems.append(f"{entry.get('row')}: both delegated and not natively reachable")
    for entry in manifest.get("runbook_driven", []):
        if entry["row"] not in rows:
            problems.append(f"{entry['row']}: runbook-driven but not a row of the family")
        if entry["row"] in reachable:
            problems.append(f"{entry['row']}: both runbook-driven and not natively reachable")
    if problems:
        raise ValueError(f"{family} manifest: " + "; ".join(problems))
    return manifest


def lost_for(summary: dict, sites: list[int]) -> int:
    by_site = summary.get("lost_by_site", {}) or {}
    return sum(num(by_site, str(site)) for site in sites)


def local_day(started_unix_ms: int) -> str:
    return datetime.datetime.fromtimestamp(started_unix_ms / 1000).strftime("%Y-%m-%d")


def _is_sha(value) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(c in "0123456789abcdef" for c in value)


def evaluate_e2(summaries: list[dict], checklists: list[dict], expect_sha: str, family: str) -> Verdict:
    """E2: at least 3 completed native checklists on at least 2 days, on one build that is the
    candidate. A checklist may be several launches: its record names the sessions, and each
    must be one of the summaries given. A summary no checklist claims is rejected."""
    problems = []
    by_key: dict[tuple, dict] = {}
    for summary in summaries:
        session = summary.get("session")
        name = Path(summary["_path"]).name
        if not isinstance(session, dict):
            problems.append(f"{name}: no `session` block (a build before S1)")
            continue
        if session.get("armed") is not True:
            problems.append(f"{name}: session.armed is not true")
        sha = session.get("git_sha")
        if not _is_sha(sha):
            problems.append(f"{name}: session.git_sha is {sha!r}, not a 40-hex commit")
        elif sha != expect_sha:
            problems.append(f"{name}: session.git_sha {sha[:12]} is not the candidate {expect_sha[:12]}")
        started = session.get("started_unix_ms")
        if not isinstance(started, int) or isinstance(started, bool) or started <= 0:
            problems.append(f"{name}: session.started_unix_ms is {started!r}")
            continue
        key = (session.get("pid"), started)
        if key in by_key:
            problems.append(f"{name}: the same session (pid and start) as {Path(by_key[key]['_path']).name}")
        by_key[key] = summary
    builds = {
        (s["session"].get("git_sha"), s["session"].get("build_profile"))
        for s in summaries
        if isinstance(s.get("session"), dict)
    }
    if len(builds) > 1:
        problems.append(f"sessions come from {len(builds)} builds, not one: {sorted(map(str, builds))}")
    claimed: set[tuple] = set()
    complete = 0
    dates: set[str] = set()
    for record in checklists:
        label = record.get("label") or Path(record["_path"]).name
        keys = [(entry["pid"], entry["started_unix_ms"]) for entry in record["sessions"]]
        ok = True
        if record["complete"] is not True:
            problems.append(f"checklist {label}: not marked complete")
            ok = False
        if len(set(keys)) != len(keys):
            problems.append(f"checklist {label}: lists a session twice")
            ok = False
        for key in keys:
            if key not in by_key:
                problems.append(f"checklist {label}: session pid={key[0]} start={key[1]} has no summary (or its summary is not valid)")
                ok = False
            elif key in claimed:
                problems.append(f"checklist {label}: session pid={key[0]} start={key[1]} is also in another checklist")
                ok = False
            claimed.add(key)
        # The record's date is the day the checklist began: its first session's local day.
        first = min(entry["started_unix_ms"] for entry in record["sessions"])
        if local_day(first) != record["date"]:
            problems.append(f"checklist {label}: date {record['date']} is not the local day of its first session ({local_day(first)})")
            ok = False
        if ok:
            complete += 1
            dates.add(record["date"])
    for key, summary in by_key.items():
        if key not in claimed:
            problems.append(f"{Path(summary['_path']).name}: belongs to no checklist record")
    if complete < E2_PASSES:
        problems.append(f"{complete} completed checklists, need at least {E2_PASSES}")
    if len(dates) < E2_DAYS:
        problems.append(f"{len(dates)} distinct day(s), need at least {E2_DAYS}")
    detail = f"checklists={complete} sessions={len(summaries)} days={len(dates)} builds={len(builds)}"
    if problems:
        detail += "; " + "; ".join(problems)
    return Verdict(family, "E2", not problems, detail)


def evaluate_family(
    family: str,
    manifest: dict,
    summaries: list[dict],
    e7: dict | None,
    checklists: list[dict],
    expect_sha: str,
) -> list[Verdict]:
    sites = manifest["sites"]
    site_keys = {str(site) for site in sites}
    verdicts: list[Verdict] = []

    # E1: no root mismatch anywhere in the checklist, and none credited to a family row.
    root = sum(num(s, "mismatched_root") for s in summaries)
    family_mismatched = sum(
        entry.get("mismatched", 0)
        for s in summaries
        for key, entry in (s.get("per_row") or {}).items()
        if key.split(".", 1)[0] in site_keys
    )
    verdicts.append(
        Verdict(
            family,
            "E1",
            root == 0 and family_mismatched == 0,
            f"mismatched_root={root} family_row_mismatches={family_mismatched}",
        )
    )

    verdicts.append(evaluate_e2(summaries, checklists, expect_sha, family))

    # E3: every C3 row of the family hit >= 5 times: `matched` closes in the sessions, plus
    # the E7 replay's `matched` for the rows a replay may credit, unless the row is on the
    # not-natively-reachable list. A row a runbook step drives (R101.5, R209.5) needs its five
    # native closes of its own: the replay's hits never stand in for the step.
    waived = {entry["row"] for entry in manifest["not_natively_reachable"]}
    driven = {entry["row"] for entry in manifest.get("runbook_driven", [])}
    delegated = {entry["row"] for entry in manifest.get("delegated", [])}
    e7_rows = (e7 or {}).get("per_row", {})
    stale_in: list[str] = []

    def credit(entry: dict | None, key: str) -> int | None:
        """E3 credit of one per-row entry: `matched`, plus `delegated` for a delegated row (its
        reply is another site's, so it has no `matched` of its own). `None` when a counter is
        absent or not a non-negative integer: the evidence is from a build before it was written and fails closed."""
        if entry is None:
            return 0
        if not isinstance(entry, dict):
            return None
        fields = ("matched", "delegated") if key in delegated else ("matched",)
        values = [entry.get(field) for field in fields]
        if not all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in values):
            return None
        return sum(values)

    def native(key: str) -> int:
        total = 0
        for summary in summaries:
            value = credit((summary.get("per_row") or {}).get(key), key)
            if value is None:
                stale_in.append(Path(summary["_path"]).name)
                value = 0
            total += value
        return total

    def replay(key: str) -> int:
        value = credit(e7_rows.get(key), key)
        if value is None:
            stale_in.append("the E7 dump")
            return 0
        return value

    short = []
    for entry in manifest["rows"]:
        key = entry["row"]
        if key in waived:
            continue
        total = native(key)
        if key not in driven:
            total += replay(key)
        if total < E3_HITS:
            short.append(f"{entry['id']}={total}" + (" (native only: a runbook step drives it)" if key in driven else ""))
    stale = sorted(set(stale_in))
    checked = len(manifest["rows"]) - len(waived)
    detail = f"rows_checked={checked} waived={len(waived)} below_{E3_HITS}={len(short)}"
    if stale:
        detail += f"; per_row `matched` (or `delegated`, for a delegated row) missing in {stale}: evidence from a build before it was written fails closed"
    if short:
        detail += ": " + ", ".join(short)
    verdicts.append(Verdict(family, "E3", not short and not stale, detail))

    # E4: a lost message is an unverified input.
    lost = sum(
        num(s, "dropped") + num(s, "lost_open") + num(s, "loss_unattributed") + lost_for(s, sites)
        for s in summaries
    )
    verdicts.append(Verdict(family, "E4", lost == 0, f"dropped+lost_open+loss_unattributed+lost_by_site{sites}={lost}"))

    # E5: the loss rate overall (every site), over all passes.
    all_lost = sum(
        num(s, "dropped")
        + num(s, "lost_open")
        + num(s, "loss_unattributed")
        + sum(num(s.get("lost_by_site", {}) or {}, k) for k in (s.get("lost_by_site") or {}))
        for s in summaries
    )
    compared = sum(num(s, "compared") for s in summaries)
    denominator = compared + all_lost
    rate = (all_lost / denominator) if denominator else 1.0
    verdicts.append(
        Verdict(
            family,
            "E5",
            denominator > 0 and rate <= E5_RATE,
            f"lost={all_lost} compared={compared} rate={rate:.6f} (<= {E5_RATE})",
        )
    )

    # E6
    panic = sum(num(s, "shadow_panic") for s in summaries)
    overflow = sum(num(s, "witness_overflow") for s in summaries)
    verdicts.append(Verdict(family, "E6", panic == 0 and overflow == 0, f"shadow_panic={panic} witness_overflow={overflow}"))

    # E7: the E8 test ran with the goldens repeated 100 times, with no sub-run diverging (a
    # divergence fails the test before the file is written).
    if e7 is None:
        verdicts.append(Verdict(family, "E7", False, "no --e7 file: run the E8 test with MURMUR_E7_PER_ROW_OUT"))
    else:
        repeat = e7.get("golden_repeat", 0)
        sha = e7.get("git_sha")
        problems = []
        if not (isinstance(repeat, int) and not isinstance(repeat, bool) and repeat >= E7_REPEAT):
            problems.append(f"golden_repeat={repeat} (< {E7_REPEAT})")
        if not _is_sha(sha):
            problems.append(f"the dump has no git_sha ({sha!r}): it is from a build before the revision was written")
        elif sha != expect_sha:
            problems.append(f"the dump is from {sha[:12]}, not the candidate {expect_sha[:12]}")
        if e7.get("dirty") is not False:
            problems.append(f"dirty={e7.get('dirty')!r}: the replay must run on a clean checkout of the candidate")
        verdicts.append(
            Verdict(
                family,
                "E7",
                not problems,
                f"golden_repeat={repeat} git_sha={str(sha)[:12]} dirty={e7.get('dirty')} replayed_rows={len(e7_rows)}"
                + ("; " + "; ".join(problems) if problems else ""),
            )
        )

    # E8: every preserved defect whose row belongs to the family was witnessed.
    pd_witness = (e7 or {}).get("pd_witness", {})
    missing_pds = [f"PD-{n}" for n in manifest["pds"] if not pd_witness.get(f"PD-{n}", 0)]
    verdicts.append(
        Verdict(
            family,
            "E8",
            e7 is not None and not missing_pds,
            "pd_witness " + ", ".join(f"PD-{n}={pd_witness.get(f'PD-{n}', 0)}" for n in manifest["pds"])
            + (f"; never witnessed: {missing_pds}" if missing_pds else ""),
        )
    )

    # E9: a tainted run is repeated. A run is tainted by a flight the shadow gave up, a flight
    # still open at exit, an unexpected incomplete close, a decision section the counters do
    # not balance, or a divergence of the persisted core.
    taint = []
    for summary in summaries:
        name = Path(summary["_path"]).name
        for counter in TAINT_COUNTERS:
            if num(summary, counter):
                taint.append(f"{name}:{counter}={num(summary, counter)}")
        for field in ("outstanding_at_exit", "conservation", "incomplete_closes", "core_divergences"):
            if field not in summary:
                taint.append(f"{name}: no `{field}` (a build before S2-S4)")
        for reason, count in (summary.get("incomplete_closes") or {}).items():
            if count:
                taint.append(f"{name}:incomplete_closes[{reason}]={count}")
        # A summary that is not the clean-exit write (`final`) comes from a crash or a kill:
        # its counters stop at the last periodic write, so the pass is tainted.
        if summary.get("final") is not True:
            taint.append(f"{name}: not a final summary (`final` is {summary.get('final')!r}: a crash or a kill)")
        # The shutdown (a final write): quiesced, nothing unsettled, nothing dropped after the
        # close, and the attempt count present. Each fails closed.
        if summary.get(SHUTDOWN_QUIESCED) is not True:
            taint.append(f"{name}:{SHUTDOWN_QUIESCED}={summary.get(SHUTDOWN_QUIESCED)!r} (absent or not true: the transport did not go quiet)")
        for counter in (*SHUTDOWN_ZERO_COUNTS, *SHUTDOWN_INFORMATION_COUNTS, *SHUTDOWN_ALIAS_COUNTS):
            if counter not in summary:
                if counter not in SHUTDOWN_ALIAS_COUNTS:
                    taint.append(f"{name}: no `{counter}` (a build before the shutdown record)")
                continue
            value = summary[counter]
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                taint.append(f"{name}:{counter}={value!r} is not a count")
            elif value > 0 and counter not in SHUTDOWN_INFORMATION_COUNTS:
                taint.append(f"{name}:{counter}={value}")
        terms = [summary.get(k) for k in ("popped", "push_lost", "dropped_after_shutdown", "shutdown_unsettled", "published_attempts")]
        if all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in terms):
            popped, lost, dropped, unsettled, published = terms
            if popped + lost + dropped + unsettled != published:
                taint.append(
                    f"{name}: popped+push_lost+dropped_after_shutdown+shutdown_unsettled="
                    f"{popped + lost + dropped + unsettled} != published_attempts={published} "
                    "(the accounting is not one consistent read)"
                )
        # `push_lost` is a part of the older top-level `dropped` (not of `dropped_after_shutdown`):
        # `dropped` is `push_lost` plus tickets dropped unsent plus `dropped_unavailable`. So
        # `push_lost <= dropped - dropped_unavailable`. `dropped` is folded at the last drain and
        # `push_lost` read later: a queue rejection landing between the two shows as `push_lost`
        # above the bound, and is a real unfolded loss. Fails closed (a missing or malformed
        # term is taint too).
        bound = [summary.get(k) for k in ("push_lost", "dropped", "dropped_unavailable")]
        if not all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in bound):
            taint.append(f"{name}: push_lost/dropped/dropped_unavailable are not all counts: {bound!r}")
        elif bound[0] > bound[1] - bound[2]:
            taint.append(
                f"{name}: push_lost={bound[0]} > dropped-dropped_unavailable={bound[1] - bound[2]} "
                "(a queue rejection after the last drain's fold: an unfolded loss)"
            )
        # The books are kept over all family sites together (`conservation.total`); only the
        # headers are per site, and `non_family` (keyed by file:line) is informational.
        total = (summary.get("conservation") or {}).get("total")
        if not isinstance(total, dict):
            if "conservation" in summary:
                taint.append(f"{name}: conservation has no `total` books")
        else:
            books = [total.get(k) for k in ("entries", "headers", "no_decision", "unwound")]
            if not all(isinstance(v, int) and not isinstance(v, bool) for v in books):
                taint.append(f"{name}: conservation.total lacks integer entries/headers/no_decision/unwound")
            elif books[0] != books[1] + books[2] + books[3]:
                taint.append(
                    f"{name}:conservation.total entries={books[0]} != "
                    f"headers+no_decision+unwound={books[1] + books[2] + books[3]}"
                )
            if total.get("balanced") is not True:
                taint.append(f"{name}:conservation.total.balanced={total.get('balanced')!r}")
        unclassified = (summary.get("conservation") or {}).get("unclassified", 0)
        if unclassified:
            taint.append(f"{name}:conservation.unclassified={unclassified}")
    # The books and the global counters are not per family: they taint every family. The
    # named no-reply close (`named_no_reply`) is an expected kind and is never taint.
    # Live error replies with dynamic text (Smart Auto, shell) match no fixed class: counted,
    # reported, and not taint by themselves.
    errs = [s.get("unclassified_err_replies") for s in summaries]
    err_text = (
        f"unclassified_err_replies={sum(e for e in errs if isinstance(e, int))} (informational, not taint)"
        if all(isinstance(e, int) and not isinstance(e, bool) for e in errs)
        else "unclassified_err_replies=n/a (a summary lacks it)"
    )
    verdicts.append(
        Verdict(family, "E9", not taint, ("tainted: " + "; ".join(taint) if taint else "no taint") + "; " + err_text)
    )
    return verdicts


def evaluate(
    summaries: list[dict],
    e7: dict | None,
    families: list[str],
    directory: Path = MANIFESTS,
    *,
    checklists: list[dict],
    expect_sha: str,
) -> list[Verdict]:
    if not _is_sha(expect_sha):
        raise ValueError(f"--expect-sha {expect_sha!r} is not a 40-hex commit")
    verdicts: list[Verdict] = []
    for family in families:
        verdicts.extend(evaluate_family(family, load_manifest(family, directory), summaries, e7, checklists, expect_sha))
    return verdicts


def load_e7(path: Path | None) -> dict | None:
    if path is None:
        return None
    dump = json.loads(path.read_text())
    if not isinstance(dump, dict) or not isinstance(dump.get("per_row"), dict):
        raise ValueError(f"{path}: not an E7 dump (no per_row)")
    return dump


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("summaries", nargs="+", type=Path, help="one dictation-shadow-summary.json per native session")
    parser.add_argument("--expect-sha", required=True, help="the candidate commit (40 hex) the passes and the replay must be on")
    parser.add_argument("--checklist", action="append", type=Path, default=[], help="a completed-checklist record (repeat once per checklist)")
    parser.add_argument("--e7", type=Path, help="the file the E8 test writes (MURMUR_E7_PER_ROW_OUT)")
    parser.add_argument("--family", action="append", choices=FAMILIES, help="a family to evaluate (default: all)")
    parser.add_argument("--manifests", type=Path, default=MANIFESTS)
    arguments = parser.parse_args(argv)
    try:
        summaries = load_summaries(arguments.summaries)
        e7 = load_e7(arguments.e7)
        checklists = [load_checklist(path) for path in arguments.checklist]
        verdicts = evaluate(
            summaries,
            e7,
            arguments.family or list(FAMILIES),
            arguments.manifests,
            checklists=checklists,
            expect_sha=arguments.expect_sha,
        )
    except (OSError, ValueError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    for verdict in verdicts:
        print(verdict.line())
    failed = [v for v in verdicts if not v.passed]
    print(f"{len(verdicts) - len(failed)} PASS, {len(failed)} FAIL")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
