from __future__ import annotations

import unittest

from scripts.murmur_bench import validate_comparability


def report(
    *,
    model_order: tuple[str, ...] = ("medium.en",),
    shared_init_ms: float = 350.0,
) -> dict:
    return {
        "environment": {
            "os": "macOS",
            "osVersion": "26.5.1",
            "architecture": "aarch64",
            "hardwareModel": "Mac16,10",
            "chip": "Apple M4",
        },
        "corpus": {
            "source": "personal",
            "fixtureIds": ["prompt-01", "prompt-02"],
        },
        "configuration": {
            "vadThreshold": 0.5,
            "executionPath": "full-buffer final transcription after recording stops",
            "transcriptTransformProfile": "default local delivery pipeline",
            "percentileMethod": "nearest-rank over measured warm iterations",
            "modelRunOrder": list(model_order),
            "sharedInitOrder": [model_order[0]],
        },
        "preset": "thorough",
        "iterations": 3,
        "sharedInitMs": shared_init_ms,
        "results": [{"modelName": model_name} for model_name in model_order],
    }


def metadata(*, cache_policy: str = "conditioned-timed-path-v4") -> dict:
    return {
        "cachePolicy": cache_policy,
        "conditioningPreset": "quick",
        "conditioningStages": [
            "all-selected-quick",
            "shared-init-targets-quick-x2",
        ],
        "gitCommit": "4" * 40,
        "gitDirty": False,
        "conditioningGitCommit": "4" * 40,
        "targetConditioningModelOrder": ["medium.en"],
        "targetConditioningSharedInitMs": [1_500.0, 1_550.0],
        "powerSource": "AC Power",
        "lowPowerMode": False,
        "thermalState": "nominal",
        "hostAfterMeasurement": {
            "powerSource": "AC Power",
            "lowPowerMode": False,
            "thermalState": "nominal",
        },
        "idleCpuLimitPercent": 20.0,
        "idleConsecutiveSamples": 3,
        "idleSampleIntervalSeconds": 5.0,
    }


class MurmurBenchComparabilityTests(unittest.TestCase):
    def test_rejects_different_model_sets_or_run_orders(self) -> None:
        with self.assertRaisesRegex(ValueError, "model set or run order"):
            validate_comparability(
                report(model_order=("tiny.en", "medium.en")),
                report(model_order=("medium.en", "tiny.en")),
                metadata(),
                metadata(),
            )

    def test_rejects_mismatched_conditioning_policy(self) -> None:
        with self.assertRaisesRegex(ValueError, "cache conditioning policy"):
            validate_comparability(
                report(),
                report(),
                metadata(),
                metadata(cache_policy="unconditioned"),
            )

    def test_accepts_cold_warm_skew_outside_the_timed_product_path(self) -> None:
        validate_comparability(
            report(shared_init_ms=1_500.0),
            report(shared_init_ms=14_000.0),
            metadata(),
            metadata(),
        )

    def test_rejects_conditioning_a_different_commit(self) -> None:
        candidate_metadata = metadata()
        candidate_metadata["conditioningGitCommit"] = "5" * 40
        with self.assertRaisesRegex(ValueError, "conditioning commit differs"):
            validate_comparability(
                report(),
                report(),
                metadata(),
                candidate_metadata,
            )

    def test_rejects_changed_host_settling_configuration(self) -> None:
        candidate_metadata = metadata()
        candidate_metadata["idleCpuLimitPercent"] = 30.0
        with self.assertRaisesRegex(ValueError, "host-settling configuration"):
            validate_comparability(
                report(),
                report(),
                metadata(),
                candidate_metadata,
            )

    def test_rejects_missing_target_conditioning_pass(self) -> None:
        candidate_metadata = metadata()
        candidate_metadata["targetConditioningSharedInitMs"] = [1_500.0]
        with self.assertRaisesRegex(ValueError, "target conditioning evidence"):
            validate_comparability(
                report(),
                report(),
                metadata(),
                candidate_metadata,
            )

    def test_accepts_conditioned_reports_with_equivalent_shared_init(self) -> None:
        validate_comparability(
            report(shared_init_ms=1_500.0),
            report(shared_init_ms=1_590.0),
            metadata(),
            metadata(),
        )

    def test_legacy_reports_without_conditioning_metadata_still_compare(self) -> None:
        validate_comparability(report(), report(), None, None)


if __name__ == "__main__":
    unittest.main()
