//! Headless Performance Lab benchmark runner.
//!
//! Runs `benchmark::run` outside the app UI, using `tauri::test`'s mock
//! runtime to satisfy the `AppHandle` that `emit_progress` needs (no other
//! part of the benchmark pipeline touches the AppHandle -- model directory
//! resolution goes through `dirs::` directly, see `transcriber::whisper`,
//! `transcriber::coreml`, `transcriber::parakeet`).
//!
//! `#[ignore]`d because it loads real models and does real inference; run it
//! explicitly:
//!
//! ```sh
//! MURMUR_BENCH_OUT=/tmp/report.json MURMUR_BENCH_PRESET=quick MURMUR_BENCH_MODELS=tiny.en \
//!     cargo test --test headless_benchmark -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Env vars:
//! - `MURMUR_BENCH_OUT` (required): path to write the `BenchmarkReport` JSON to.
//! - `MURMUR_BENCH_MODELS` (optional): comma-separated model names (e.g.
//!   `tiny.en,base.en`). Defaults to every model `benchmark_models()` reports
//!   as installed on this machine.
//! - `MURMUR_BENCH_PRESET` (optional): `quick` | `standard` | `thorough`.
//!   Defaults to `standard`.

use std::path::PathBuf;
use ui_lib::benchmark::{self, BenchmarkCoordinator, BenchmarkPreset, BenchmarkRequest};

/// Parse `MURMUR_BENCH_PRESET`. Missing -> `Standard`; unrecognized -> error.
fn parse_preset(value: Option<&str>) -> Result<BenchmarkPreset, String> {
    match value.map(str::to_lowercase).as_deref() {
        None => Ok(BenchmarkPreset::Standard),
        Some("quick") => Ok(BenchmarkPreset::Quick),
        Some("standard") => Ok(BenchmarkPreset::Standard),
        Some("thorough") => Ok(BenchmarkPreset::Thorough),
        Some(other) => Err(format!(
            "Unknown MURMUR_BENCH_PRESET '{other}' (expected quick|standard|thorough)"
        )),
    }
}

/// Parse `MURMUR_BENCH_MODELS`. Missing -> every entry of `installed`;
/// present but empty (after trimming/splitting) -> error; installed empty
/// with no override -> error (nothing to benchmark).
fn parse_models(value: Option<&str>, installed: &[String]) -> Result<Vec<String>, String> {
    match value {
        None => {
            if installed.is_empty() {
                Err(
                    "No installed benchmark models found; set MURMUR_BENCH_MODELS explicitly"
                        .to_string(),
                )
            } else {
                Ok(installed.to_vec())
            }
        }
        Some(raw) => {
            let names = raw
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if names.is_empty() {
                Err("MURMUR_BENCH_MODELS was set but contained no model names".to_string())
            } else {
                Ok(names)
            }
        }
    }
}

/// Parse `MURMUR_BENCH_OUT`. Required; missing or blank -> error.
fn parse_out_path(value: Option<&str>) -> Result<PathBuf, String> {
    match value {
        Some(raw) if !raw.trim().is_empty() => Ok(PathBuf::from(raw)),
        _ => Err("MURMUR_BENCH_OUT is required (output JSON path)".to_string()),
    }
}

#[test]
fn preset_env_var_maps_to_expected_preset_or_errors() {
    assert!(matches!(parse_preset(None), Ok(BenchmarkPreset::Standard)));
    assert!(matches!(
        parse_preset(Some("quick")),
        Ok(BenchmarkPreset::Quick)
    ));
    assert!(matches!(
        parse_preset(Some("QUICK")),
        Ok(BenchmarkPreset::Quick)
    ));
    assert!(matches!(
        parse_preset(Some("standard")),
        Ok(BenchmarkPreset::Standard)
    ));
    assert!(matches!(
        parse_preset(Some("thorough")),
        Ok(BenchmarkPreset::Thorough)
    ));
    assert!(parse_preset(Some("bogus")).is_err());
}

#[test]
fn models_env_var_defaults_to_installed_list_or_parses_csv() {
    let installed = vec!["tiny.en".to_string(), "base.en".to_string()];
    assert_eq!(parse_models(None, &installed), Ok(installed.clone()));
    assert_eq!(
        parse_models(Some("tiny.en, small.en"), &installed),
        Ok(vec!["tiny.en".to_string(), "small.en".to_string()])
    );
    assert!(parse_models(Some(""), &installed).is_err());
    assert!(parse_models(Some(" , "), &installed).is_err());
    assert!(parse_models(None, &[]).is_err());
}

#[test]
fn out_path_is_required() {
    assert!(parse_out_path(None).is_err());
    assert!(parse_out_path(Some("")).is_err());
    assert!(parse_out_path(Some("  ")).is_err());
    assert_eq!(
        parse_out_path(Some("/tmp/report.json")),
        Ok(PathBuf::from("/tmp/report.json"))
    );
}

/// Runs the full Performance Lab benchmark headlessly and writes the
/// resulting `BenchmarkReport` JSON to `MURMUR_BENCH_OUT`. See module docs
/// for the exact invocation and env vars.
#[test]
#[ignore]
fn headless_benchmark() {
    let out_path = parse_out_path(std::env::var("MURMUR_BENCH_OUT").ok().as_deref())
        .expect("MURMUR_BENCH_OUT");
    let preset = parse_preset(std::env::var("MURMUR_BENCH_PRESET").ok().as_deref())
        .expect("MURMUR_BENCH_PRESET");

    let catalog = benchmark::benchmark_models();
    let installed = catalog
        .iter()
        .filter(|model| model.installed)
        .map(|model| model.model_name.clone())
        .collect::<Vec<_>>();
    let model_names = parse_models(std::env::var("MURMUR_BENCH_MODELS").ok().as_deref(), &installed)
        .expect("MURMUR_BENCH_MODELS");

    println!("headless benchmark: preset={preset:?} models={model_names:?}");

    let request = BenchmarkRequest {
        model_names,
        preset,
    };

    let app = tauri::test::mock_app();
    let handle = app.handle();

    let coordinator = BenchmarkCoordinator::new();
    assert!(
        coordinator.try_start(),
        "fresh BenchmarkCoordinator should always start"
    );

    let started = std::time::Instant::now();
    let report = benchmark::run(handle, &coordinator, request).expect("benchmark run failed");
    coordinator.finish();
    println!(
        "headless benchmark completed in {:.1}s",
        started.elapsed().as_secs_f64()
    );

    let json = serde_json::to_string_pretty(&report).expect("serialize benchmark report");
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).expect("create MURMUR_BENCH_OUT parent directory");
        }
    }
    std::fs::write(&out_path, &json).expect("write MURMUR_BENCH_OUT");
    println!("wrote benchmark report to {}", out_path.display());
}
