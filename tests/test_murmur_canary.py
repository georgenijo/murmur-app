import importlib.util
from pathlib import Path
import subprocess
import json
import plistlib
import hashlib
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


@pytest.mark.parametrize("installed_version", ["0.31.4", "0.31.3"])
@pytest.mark.parametrize("transient_busy", [False, True])
def test_isolated_canary_verifies_installed_bundle_and_clears_dry_run(monkeypatch, tmp_path, installed_version, transient_busy):
    app = tmp_path / "Murmur Canary.app"
    (app / "Contents").mkdir(parents=True)
    with (app / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleShortVersionString": installed_version}, stream)
    monkeypatch.setattr(canary, "previous_release", lambda repo, tag: "v0.31.3")
    monkeypatch.setattr(canary, "install_previous_bundle", lambda *args, **kwargs: (app, str(app / "Contents/MacOS/ui")))
    monkeypatch.setenv("MURMUR_UPDATER_CANARY_DRY_RUN", "1")
    monkeypatch.setattr(canary, "prepare_isolated_profile", lambda root: (root / "home", root / "models"))
    environments = []

    launches = 0

    def launch(*args, **kwargs):
        nonlocal launches
        launches += 1
        environments.append(kwargs["env"])
        result = passing_result()
        if transient_busy and launches == 1:
            result.update(status="failed", error="Updater check already in progress.")
        (tmp_path / "result.json").write_text(json.dumps(result))
        return SimpleNamespace(pid=123, poll=lambda: None)

    monkeypatch.setattr(canary.subprocess, "Popen", launch)
    stopped = []
    monkeypatch.setattr(canary, "terminate_and_collect_stderr", lambda process: stopped.append(process.pid))
    args = SimpleNamespace(tag="v0.31.4", repository="owner/repo", root=str(tmp_path),
                           dry_run=False, isolated=True, timeout=1)
    if installed_version == "0.31.4":
        assert canary.run_remote(args) == 0
    else:
        with pytest.raises(canary.CanaryError, match="installed bundle version"):
            canary.run_remote(args)
    assert stopped == [123] * launches
    assert launches == (2 if transient_busy else 1)
    assert environments[0]["HOME"] == str(tmp_path / "home")
    assert environments[0]["CFFIXED_USER_HOME"] == str(tmp_path / "home")
    assert "MURMUR_UPDATER_CANARY_DRY_RUN" not in environments[0]
    assert environments[0]["WHISPER_MODEL_DIR"] == str(tmp_path / "models")


def test_isolated_profile_bootstraps_public_client_and_pins_model(monkeypatch, tmp_path):
    fixture = b"real model bytes stand-in for this unit test"
    monkeypatch.setattr(canary, "CANARY_MODEL_SHA256", hashlib.sha256(fixture).hexdigest())
    downloads = []

    def download(command, check):
        downloads.append(command)
        Path(command[-1]).write_bytes(fixture)

    monkeypatch.setattr(canary.subprocess, "run", download)
    home, models = canary.prepare_isolated_profile(tmp_path)
    settings = json.loads((home / "Library/Application Support/com.localdictation/settings.json").read_text())
    assert settings["launchOnboardingComplete"] is True
    assert settings["model"] == "tiny.en"
    assert settings["disabled"] is True
    assert (models / "ggml-tiny.en.bin").read_bytes() == fixture
    canary.prepare_isolated_profile(tmp_path)
    assert len(downloads) == 1


def test_isolated_profile_rejects_corrupt_model(monkeypatch, tmp_path):
    monkeypatch.setattr(canary.subprocess, "run", lambda command, check: Path(command[-1]).write_bytes(b"corrupt"))
    with pytest.raises(canary.CanaryError, match="checksum"):
        canary.prepare_isolated_profile(tmp_path)
    assert not (tmp_path / "models/ggml-tiny.en.bin").exists()


def test_busy_result_restarts_source_before_accepting_complete_result(monkeypatch, tmp_path):
    app = tmp_path / "App"
    (app / "Contents").mkdir(parents=True)
    with (app / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleShortVersionString": "0.31.4"}, stream)
    monkeypatch.setattr(canary, "previous_release", lambda repo, tag: "v0.31.3")
    monkeypatch.setattr(canary, "install_previous_bundle", lambda *args, **kwargs: (app, "unused"))
    launches = 0

    def launch(*args, **kwargs):
        nonlocal launches
        launches += 1
        result = passing_result()
        if launches == 1:
            result.update(status="failed", error=canary.BUSY_CHECK_ERROR)
        (tmp_path / "result.json").write_text(json.dumps(result))
        return SimpleNamespace(pid=123, poll=lambda: None)

    monkeypatch.setattr(canary.subprocess, "Popen", launch)
    stopped = []
    monkeypatch.setattr(canary, "terminate_and_collect_stderr", lambda process: stopped.append(process.pid) or "")
    ticks = iter([0, 0, 0, 2])
    monkeypatch.setattr(canary.time, "monotonic", lambda: next(ticks))
    monkeypatch.setattr(canary.time, "sleep", lambda seconds: None)
    args = SimpleNamespace(tag="v0.31.4", repository="owner/repo", root=str(tmp_path),
                           dry_run=False, isolated=False, timeout=1)
    assert canary.run_remote(args) == 0
    assert launches == 2
    assert stopped == [123, 123]


def test_busy_result_fails_after_bounded_source_restarts(monkeypatch, tmp_path):
    monkeypatch.setattr(canary, "previous_release", lambda repo, tag: "v0.31.3")
    monkeypatch.setattr(canary, "install_previous_bundle", lambda *args, **kwargs: (tmp_path / "App", "unused"))
    launches = 0

    def launch(*args, **kwargs):
        nonlocal launches
        launches += 1
        result = passing_result()
        result.update(status="failed", error=canary.BUSY_CHECK_ERROR)
        (tmp_path / "result.json").write_text(json.dumps(result))
        return SimpleNamespace(pid=123, poll=lambda: None)

    monkeypatch.setattr(canary.subprocess, "Popen", launch)
    monkeypatch.setattr(canary.time, "monotonic", lambda: 0)
    stopped = []
    monkeypatch.setattr(canary, "terminate_and_collect_stderr", lambda process: stopped.append(process.pid) or "")
    args = SimpleNamespace(tag="v0.31.4", repository="owner/repo", root=str(tmp_path),
                           dry_run=False, isolated=False, timeout=1)

    with pytest.raises(canary.CanaryError, match="every startup attempt"):
        canary.run_remote(args)
    assert launches == canary.MAX_BUSY_START_ATTEMPTS
    assert stopped == [123] * canary.MAX_BUSY_START_ATTEMPTS
