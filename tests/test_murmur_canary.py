import importlib.util
from pathlib import Path

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
        "stages": {stage: "passed" for stage in canary.STAGES},
        "error": None,
    }


def test_evaluate_accepts_complete_pass() -> None:
    canary.evaluate_canary_result(passing_result(), "0.31.4")


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
        canary.evaluate_canary_result(result, "0.31.4")


def test_evaluate_requires_all_stage_names() -> None:
    result = passing_result()
    del result["stages"]["signatureVerify"]
    with pytest.raises(canary.CanaryError, match="signatureVerify"):
        canary.evaluate_canary_result(result, "0.31.4")
