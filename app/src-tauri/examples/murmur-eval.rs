use std::path::PathBuf;

use ui_lib::evaluation::{self, EvaluationTier, RunOptions};

fn usage() -> &'static str {
    "Usage: cargo run --example murmur-eval -- <deterministic|hardware> [--fixtures DIR] [--output FILE] [--workspace-root DIR] [--machine-label LABEL]"
}

fn main() {
    if let Err(error) = run() {
        eprintln!("murmur-eval: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let tier = match args.next().as_deref() {
        Some("deterministic") => EvaluationTier::Deterministic,
        Some("hardware") => EvaluationTier::Hardware,
        Some("--help" | "-h") => {
            println!("{}", usage());
            return Ok(());
        }
        _ => return Err(usage().to_string()),
    };

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut fixtures = manifest_dir.join("eval/fixtures").join(tier.as_str());
    let mut output = manifest_dir
        .join("target/murmur-eval")
        .join(format!("{}-report.json", tier.as_str()));
    let mut workspace_root = manifest_dir.join("../..");
    let mut machine_label = "local-machine".to_string();

    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}; {}", usage()))?;
        match flag.as_str() {
            "--fixtures" => fixtures = PathBuf::from(value),
            "--output" => output = PathBuf::from(value),
            "--workspace-root" => workspace_root = PathBuf::from(value),
            "--machine-label" => machine_label = value,
            _ => return Err(format!("unknown option '{flag}'; {}", usage())),
        }
    }

    let report = evaluation::run(&RunOptions {
        tier,
        fixtures_dir: fixtures,
        workspace_root,
        machine_label,
    })?;
    evaluation::write_report(&report, &output)?;
    println!(
        "murmur-eval {}: {} passed, {} failed, {} skipped; report {}",
        tier.as_str(),
        report.summary.passed,
        report.summary.failed,
        report.summary.skipped,
        output.display()
    );
    if report.summary.failed > 0 {
        return Err("evaluation failures were reported".to_string());
    }
    Ok(())
}
