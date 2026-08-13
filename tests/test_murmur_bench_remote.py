from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import murmur_bench_remote as remote


NOMINAL_SNAPSHOT = {
    "powerSource": "AC Power",
    "lowPowerMode": False,
    "thermalState": "nominal",
    "normalizedCpuPercent": 5.0,
    "loadAverage1m": 0.5,
    "conflictingProcesses": [],
}


class MurmurBenchRemoteTests(unittest.TestCase):
    def test_benchmark_environment_namespaces_cargo_cache_by_ref(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first_lock = root / "first.lock"
            second_lock = root / "second.lock"
            first_lock.write_text("first", encoding="utf-8")
            second_lock.write_text("second", encoding="utf-8")

            first = remote.benchmark_environment(root, first_lock, "1" * 40)[
                "CARGO_TARGET_DIR"
            ]
            repeated = remote.benchmark_environment(root, first_lock, "1" * 40)[
                "CARGO_TARGET_DIR"
            ]
            new_ref = remote.benchmark_environment(root, first_lock, "2" * 40)[
                "CARGO_TARGET_DIR"
            ]
            new_lock = remote.benchmark_environment(root, second_lock, "1" * 40)[
                "CARGO_TARGET_DIR"
            ]

            self.assertEqual(first, repeated)
            self.assertNotEqual(first, new_ref)
            self.assertNotEqual(first, new_lock)
            self.assertEqual(Path(first).parent, root / "cargo-target")

    def test_preflight_rejects_battery_low_power_and_thermal_pressure(self) -> None:
        cases = (
            ({**NOMINAL_SNAPSHOT, "powerSource": "Battery Power"}, "AC power"),
            ({**NOMINAL_SNAPSHOT, "lowPowerMode": True}, "Low Power Mode"),
            ({**NOMINAL_SNAPSHOT, "thermalState": "warning"}, "thermal"),
        )
        for snapshot, message in cases:
            with self.subTest(snapshot=snapshot):
                with self.assertRaisesRegex(ValueError, message):
                    remote.require_host_preflight(snapshot)

    def test_wait_for_stable_host_requires_consecutive_idle_samples(self) -> None:
        snapshots = iter(
            [
                {**NOMINAL_SNAPSHOT, "normalizedCpuPercent": 35.0},
                {**NOMINAL_SNAPSHOT, "normalizedCpuPercent": 8.0},
                {**NOMINAL_SNAPSHOT, "normalizedCpuPercent": 7.0},
                {**NOMINAL_SNAPSHOT, "normalizedCpuPercent": 6.0},
            ]
        )
        with mock.patch.object(remote.time, "monotonic", side_effect=range(20)):
            settled = remote.wait_for_stable_host(
                timeout_seconds=10.0,
                interval_seconds=0.0,
                max_cpu_percent=20.0,
                consecutive_samples=3,
                snapshot_fn=lambda: next(snapshots),
                sleep_fn=lambda _: None,
            )
        self.assertEqual(settled["normalizedCpuPercent"], 6.0)

    def test_wait_for_stable_host_waits_out_a_conflicting_build(self) -> None:
        snapshots = iter(
            [
                {**NOMINAL_SNAPSHOT, "conflictingProcesses": ["200:cargo"]},
                NOMINAL_SNAPSHOT,
            ]
        )
        with mock.patch.object(remote.time, "monotonic", side_effect=range(20)):
            settled = remote.wait_for_stable_host(
                timeout_seconds=10.0,
                interval_seconds=0.0,
                max_cpu_percent=20.0,
                consecutive_samples=1,
                snapshot_fn=lambda: next(snapshots),
                sleep_fn=lambda _: None,
            )
        self.assertEqual(settled, NOMINAL_SNAPSHOT)

    def test_preflight_rejects_another_headless_benchmark(self) -> None:
        with self.assertRaisesRegex(ValueError, "conflicting benchmark process"):
            remote.require_host_preflight(
                {**NOMINAL_SNAPSHOT, "conflictingProcesses": ["headless_benchmark"]}
            )

    def test_process_monitor_allows_only_the_benchmark_process_tree(self) -> None:
        processes = {
            100: (1, "Python"),
            101: (100, "cargo"),
            102: (101, "headless_benchmark"),
            200: (1, "cargo"),
            201: (200, "rustc"),
            300: (1, "baseline-headless_benchmark"),
        }
        self.assertEqual(remote.descendant_pids(processes, 100), {100, 101, 102})
        self.assertEqual(
            remote.conflicting_processes(processes, allowed_root_pid=100),
            ["200:cargo", "201:rustc", "300:baseline-headless_benchmark"],
        )

    def test_benchmark_stops_immediately_on_unrelated_build_process(self) -> None:
        process = mock.Mock(pid=100)
        with (
            mock.patch.object(remote.subprocess, "Popen", return_value=process),
            mock.patch.object(
                remote,
                "process_table",
                return_value={100: (1, "Python"), 200: (1, "cargo")},
            ),
            mock.patch.object(remote, "run") as run,
            mock.patch.object(remote, "terminate_process_group") as terminate,
        ):
            with self.assertRaisesRegex(ValueError, "unrelated build process"):
                remote.run_benchmark(
                    runner=Path("/tmp/murmur_bench.py"),
                    worktree=Path("/tmp/worktree"),
                    output=Path("/tmp/report.json"),
                    corpus_dir=Path("/tmp/corpus"),
                    preset="thorough",
                    models="medium.en",
                    machine_label="mac-mini",
                    environment={},
                )
        terminate.assert_called_once_with(process)
        run.assert_called_once_with(
            ["cargo", "clean", "--release", "-p", "ui"],
            cwd=Path("/tmp/worktree/app/src-tauri"),
            env={},
        )

    def test_run_lock_rejects_a_concurrent_comparison(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory)
            first = remote.acquire_run_lock(cache_root)
            try:
                with self.assertRaisesRegex(ValueError, "another benchmark comparison"):
                    remote.acquire_run_lock(cache_root)
            finally:
                first.close()

    def test_conditioned_run_primes_then_measures_same_ref_and_annotates_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worktree = root / "worktree"
            cache_root = root / "cache"
            report_root = root / "reports"
            worktree.mkdir()
            cache_root.mkdir()
            report_root.mkdir()
            report_path = report_root / "candidate.json"
            calls: list[tuple[str, Path, object]] = []

            def fake_run_benchmark(**kwargs: object) -> None:
                preset = str(kwargs["preset"])
                output = Path(kwargs["output"])
                calls.append((preset, output, kwargs["models"]))
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text(
                    json.dumps(
                        {
                            "results": [],
                            "sharedInitMs": 350.0,
                            "configuration": {
                                "modelRunOrder": ["medium.en"],
                                "sharedInitOrder": ["medium.en"],
                            },
                        }
                    ),
                    encoding="utf-8",
                )
                output.with_suffix(".meta.json").write_text(
                    json.dumps({"gitCommit": "4" * 40, "durationSeconds": 1.0}),
                    encoding="utf-8",
                )

            with (
                mock.patch.object(remote, "run_benchmark", side_effect=fake_run_benchmark),
                mock.patch.object(
                    remote,
                    "wait_for_stable_host",
                    return_value=NOMINAL_SNAPSHOT,
                ) as wait_for_stable_host,
                mock.patch.object(
                    remote,
                    "host_snapshot",
                    return_value=NOMINAL_SNAPSHOT,
                ),
            ):
                remote.run_conditioned_benchmark(
                    runner=root / "murmur_bench.py",
                    worktree=worktree,
                    report=report_path,
                    cache_root=cache_root,
                    corpus_dir=root / "corpus",
                    preset="thorough",
                    conditioning_preset="quick",
                    models="medium.en",
                    machine_label="mac-mini",
                    environment={},
                    role="candidate",
                    ref="4" * 40,
                    sha="4" * 40,
                    order_position=1,
                    idle_timeout_seconds=30.0,
                    idle_interval_seconds=1.0,
                    idle_cpu_percent=20.0,
                    idle_samples=3,
                )

            self.assertEqual(
                [preset for preset, _, _ in calls],
                ["quick", "quick", "quick", "thorough"],
            )
            self.assertEqual(
                [models for _, _, models in calls],
                ["medium.en", "medium.en", "medium.en", "medium.en"],
            )
            self.assertEqual(wait_for_stable_host.call_count, 2)
            self.assertNotEqual(calls[0][1], report_path)
            self.assertFalse(calls[0][1].exists(), "conditioning report must be temporary")
            meta = json.loads(report_path.with_suffix(".meta.json").read_text())
            self.assertEqual(meta["cachePolicy"], "conditioned-timed-path-v3")
            self.assertEqual(meta["conditioningPreset"], "quick")
            self.assertEqual(
                meta["conditioningStages"],
                ["all-selected-quick", "shared-init-targets-quick-x2"],
            )
            self.assertEqual(meta["fullConditioningSharedInitMs"], 350.0)
            self.assertEqual(meta["targetConditioningModelOrder"], ["medium.en"])
            self.assertEqual(meta["targetConditioningSharedInitMs"], [350.0, 350.0])
            self.assertEqual(meta["conditioningGitCommit"], "4" * 40)
            self.assertEqual(meta["powerSource"], "AC Power")
            self.assertFalse(meta["lowPowerMode"])
            self.assertEqual(meta["thermalState"], "nominal")
            self.assertEqual(meta["runnerOrderPosition"], 1)
            self.assertEqual(meta["idleSampleIntervalSeconds"], 1.0)

if __name__ == "__main__":
    unittest.main()
