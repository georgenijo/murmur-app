#!/usr/bin/env python3
"""Evaluate the dictation-shadow exit criteria E1-E9 for each family (D-S2c-15 S6).

    python3 scripts/shadow_soak_verdict.py --e7 e7.json pass1.json pass2.json pass3.json
    python3 scripts/shadow_soak_verdict.py --family start --e7 e7.json pass*.json

Each positional argument is the `dictation-shadow-summary.json` of one native pass (copy
it after a clean exit, see docs/testing/dictation-shadow-soak.md). `--e7` is the file the E8
test writes when `MURMUR_E7_PER_ROW_OUT` is set; run that test with `MURMUR_GOLDEN_REPEAT=100`.

For every family the script prints one line per criterion, `PASS` or `FAIL` with the values
read, and exits 1 if any line is a FAIL. Every input is fail closed: a summary without the
`session` block, `matched` counters or conservation counters comes from a build before the
S1-S4 evidence and cannot count.

The row manifests (docs/testing/shadow-rows/<family>.json) list each family's C3 rows, the
rows a native pass cannot produce (with the golden or unit test that drives each), and the
rows a step of the runbook drives (those count as natively reachable: they need 5 hits). The summary
fields it reads are the ones the dictation shadow writes: `session`, `outstanding_at_exit`,
`per_row[..].matched`, `incomplete_closes`, `conservation`, `core_divergences`.

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
    for entry in manifest.get("runbook_driven", []):
        if entry["row"] not in rows:
            problems.append(f"{entry['row']}: runbook-driven but not a row of the family")
        if entry["row"] in reachable:
            problems.append(f"{entry['row']}: both runbook-driven and not natively reachable")
    if problems:
        raise ValueError(f"{family} manifest: " + "; ".join(problems))
    return manifest


def row_total(summary: dict, key: str, field: str) -> int | None:
    """`per_row[key][field]` of one summary: 0 when the row was never hit, `None` when the
    summary has per_row entries that lack `field` (a build before it was written)."""
    per_row = summary.get("per_row", {})
    entry = per_row.get(key)
    if entry is None:
        return 0
    if field not in entry:
        return None
    return entry[field]


def lost_for(summary: dict, sites: list[int]) -> int:
    by_site = summary.get("lost_by_site", {}) or {}
    return sum(num(by_site, str(site)) for site in sites)


def local_day(started_unix_ms: int) -> str:
    return datetime.datetime.fromtimestamp(started_unix_ms / 1000).strftime("%Y-%m-%d")


def evaluate_e2(summaries: list[dict], family: str) -> Verdict:
    problems = []
    sessions = []
    for summary in summaries:
        session = summary.get("session")
        name = Path(summary["_path"]).name
        if not isinstance(session, dict):
            problems.append(f"{name}: no `session` block (a build before S1)")
            continue
        if session.get("armed") is not True:
            problems.append(f"{name}: session.armed is not true")
        sha = session.get("git_sha")
        if not (isinstance(sha, str) and len(sha) == 40 and all(c in "0123456789abcdef" for c in sha)):
            problems.append(f"{name}: session.git_sha is {sha!r}, not a 40-hex commit")
        started = session.get("started_unix_ms")
        if not isinstance(started, int) or isinstance(started, bool) or started <= 0:
            problems.append(f"{name}: session.started_unix_ms is {started!r}")
        sessions.append(session)
    keys = [(s.get("pid"), s.get("started_unix_ms")) for s in sessions]
    if len(set(keys)) != len(keys):
        problems.append("two summaries come from the same session (same pid and start): one pass counted twice")
    builds = {(s.get("git_sha"), s.get("build_profile")) for s in sessions}
    if len(builds) > 1:
        problems.append(f"passes come from {len(builds)} builds, not one: {sorted(map(str, builds))}")
    days = {
        local_day(s["started_unix_ms"])
        for s in sessions
        if isinstance(s.get("started_unix_ms"), int) and s["started_unix_ms"] > 0
    }
    if len(summaries) < E2_PASSES:
        problems.append(f"{len(summaries)} passes, need at least {E2_PASSES}")
    if len(days) < E2_DAYS:
        problems.append(f"{len(days)} distinct day(s), need at least {E2_DAYS}")
    detail = f"passes={len(summaries)} days={len(days)} builds={len(builds)}"
    if problems:
        detail += "; " + "; ".join(problems)
    return Verdict(family, "E2", not problems, detail)


def evaluate_family(
    family: str,
    manifest: dict,
    summaries: list[dict],
    e7: dict | None,
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

    verdicts.append(evaluate_e2(summaries, family))

    # E3: every C3 row of the family hit >= 5 times (matched closes in the passes plus the E7
    # replay's compared count) unless it is on the not-natively-reachable list.
    waived = {entry["row"] for entry in manifest["not_natively_reachable"]}
    e7_rows = (e7 or {}).get("per_row", {})
    short = []
    stale = []
    for entry in manifest["rows"]:
        key = entry["row"]
        if key in waived:
            continue
        total = 0
        for summary in summaries:
            value = row_total(summary, key, "matched")
            if value is None:
                stale.append(Path(summary["_path"]).name)
                value = 0
            total += value
        total += e7_rows.get(key, 0) if isinstance(e7_rows.get(key, 0), int) else 0
        if total < E3_HITS:
            short.append(f"{entry['id']}={total}")
    checked = len(manifest["rows"]) - len(waived)
    detail = f"rows_checked={checked} waived={len(waived)} below_{E3_HITS}={len(short)}"
    if stale:
        detail += f"; per_row.matched missing in {sorted(set(stale))} (a build before S2)"
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
        verdicts.append(
            Verdict(
                family,
                "E7",
                isinstance(repeat, int) and repeat >= E7_REPEAT,
                f"golden_repeat={repeat} (>= {E7_REPEAT}) replayed_rows={len(e7_rows)}",
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
        conservation = summary.get("conservation") or {}
        for site, counts in conservation.items():
            if site == "non_family" or site not in site_keys:
                continue
            entries = counts.get("entries", 0)
            resolved = counts.get("headers", 0) + counts.get("no_decision", 0) + counts.get("unwound", 0)
            if entries != resolved:
                taint.append(f"{name}:conservation[{site}] entries={entries} != headers+no_decision+unwound={resolved}")
    # A family's imbalance counts for the family, the global counters for every family.
    verdicts.append(Verdict(family, "E9", not taint, "tainted: " + "; ".join(taint) if taint else "no taint"))
    return verdicts


def evaluate(
    summaries: list[dict],
    e7: dict | None,
    families: list[str],
    directory: Path = MANIFESTS,
) -> list[Verdict]:
    verdicts: list[Verdict] = []
    for family in families:
        verdicts.extend(evaluate_family(family, load_manifest(family, directory), summaries, e7))
    return verdicts


def load_e7(path: Path | None) -> dict | None:
    if path is None:
        return None
    dump = json.loads(path.read_text())
    if "per_row" not in dump:
        # The bare `{"<site>.<row>": n}` map.
        dump = {"per_row": dump}
    return dump


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("summaries", nargs="+", type=Path, help="one dictation-shadow-summary.json per native pass")
    parser.add_argument("--e7", type=Path, help="the file the E8 test writes (MURMUR_E7_PER_ROW_OUT)")
    parser.add_argument("--family", action="append", choices=FAMILIES, help="a family to evaluate (default: all)")
    parser.add_argument("--manifests", type=Path, default=MANIFESTS)
    arguments = parser.parse_args(argv)
    try:
        summaries = load_summaries(arguments.summaries)
        e7 = load_e7(arguments.e7)
        verdicts = evaluate(summaries, e7, arguments.family or list(FAMILIES), arguments.manifests)
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
