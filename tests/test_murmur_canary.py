import importlib.util
from pathlib import Path
import subprocess
from types import SimpleNamespace

import pytest


SCRIPT = Path(__file__).parents[1] / "scripts" / "murmur_canary_fleet.py"
SPEC = importlib.util.spec_from_file_location("murmur_canary_fleet", SCRIPT)
assert SPEC and SPEC.loader
canary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(canary)


def passing_result() -> dict:
    return {
        "schemaVersion": 1,
        "status": "passed",
        "checkedVersion": "0.31.3",
        "offeredVersion": "0.31.4",
        "forced": False,
        "dryRun": False,
        "stages": {stage: "passed" for stage in canary.STAGES},
        "error": None,
    }


def test_evaluate_accepts_complete_pass() -> None:
    canary.evaluate_canary_result(passing_result(), "0.31.3", "0.31.4")


@pytest.mark.parametrize(
    "mutate, message",
    [
        (lambda result: result.update(status="failed"), "status"),
        (lambda result: result["stages"].update(install="failed"), "stages"),
        (lambda result: result.update(offeredVersion="0.31.5"), "offeredVersion"),
        (lambda result: result.update(checkedVersion="0.31.4"), "previous version"),
        (lambda result: result.update(schemaVersion=2), "schemaVersion"),
    ],
)
def test_evaluate_rejects_incomplete_or_mismatched_result(mutate, message: str) -> None:
    result = passing_result()
    mutate(result)
    with pytest.raises(canary.CanaryError, match=message):
        canary.evaluate_canary_result(result, "0.31.3", "0.31.4")


def test_evaluate_requires_all_stage_names() -> None:
    result = passing_result()
    del result["stages"]["signatureVerify"]
    with pytest.raises(canary.CanaryError, match="signatureVerify"):
        canary.evaluate_canary_result(result, "0.31.3", "0.31.4")


@pytest.mark.parametrize("field, value", [("forced", "false"), ("dryRun", 0), ("checkedVersion", None), ("error", 3)])
def test_evaluate_rejects_wrong_required_field_types(field, value) -> None:
    result = passing_result()
    result[field] = value
    with pytest.raises(canary.CanaryError, match=field):
        canary.evaluate_canary_result(result, "0.31.3", "0.31.4")


def test_evaluate_rejects_missing_required_field() -> None:
    result = passing_result()
    del result["dryRun"]
    with pytest.raises(canary.CanaryError, match="dryRun"):
        canary.evaluate_canary_result(result, "0.31.3", "0.31.4")


def test_evaluate_requires_exact_previous_version() -> None:
    with pytest.raises(canary.CanaryError, match="checkedVersion"):
        canary.evaluate_canary_result(passing_result(), "0.31.2", "0.31.4")


def test_evaluate_accepts_identifiable_dry_run() -> None:
    result = passing_result()
    result["status"] = "dry-run"
    result["dryRun"] = True
    for stage in canary.STAGES[2:]:
        result["stages"][stage] = "pending"
    canary.evaluate_dry_run_result(result, "0.31.3", "0.31.4")


def test_terminate_process_group_before_collecting_stderr(monkeypatch) -> None:
    events = []

    class FakeProcess:
        pid = 42

        def wait(self, timeout):
            events.append(("wait", timeout))
            return 0

        def communicate(self, timeout):
            events.append(("communicate", timeout))
            return (b"", b"bounded failure detail")

    monkeypatch.setattr(canary.os, "killpg", lambda pid, sig: events.append(("killpg", pid, sig)))

    stderr = canary.terminate_and_collect_stderr(FakeProcess())

    assert stderr == "bounded failure detail"
    assert events[0][0] == "killpg"
    assert events[1] == ("wait", 5)
    assert events[2] == ("communicate", 1)


def test_terminate_process_group_escalates_before_collecting_stderr(monkeypatch) -> None:
    events = []

    class FakeProcess:
        pid = 43
        waits = 0

        def wait(self, timeout):
            self.waits += 1
            events.append(("wait", timeout))
            if self.waits == 1:
                raise subprocess.TimeoutExpired("canary", timeout)
            return -9

        def communicate(self, timeout):
            events.append(("communicate", timeout))
            return (b"", b"")

    monkeypatch.setattr(canary.os, "killpg", lambda pid, sig: events.append(("killpg", pid, sig)))

    canary.terminate_and_collect_stderr(FakeProcess())

    assert [event[0] for event in events] == ["killpg", "wait", "killpg", "wait", "communicate"]
    assert events[2][2] == canary.signal.SIGKILL


def test_fleet_wrapper_normalizes_remote_failure_to_nonzero(monkeypatch) -> None:
    monkeypatch.setattr(
        canary.subprocess,
        "run",
        lambda command, check: SimpleNamespace(returncode=17),
    )

    assert canary.run_fleet_canary(["fleet", "exec"]) == 1


def test_fleet_wrapper_preserves_success(monkeypatch) -> None:
    monkeypatch.setattr(
        canary.subprocess,
        "run",
        lambda command, check: SimpleNamespace(returncode=0),
    )

    assert canary.run_fleet_canary(["fleet", "exec"]) == 0
