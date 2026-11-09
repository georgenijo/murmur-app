//! Pure projections between the service payloads and the Customize view.
//! The production host and the view share these so neither re-derives wire names.
use serde_json::{Value, json};

pub const CLIPBOARD_NOT_READ: &str =
    "Phrase matched, but clipboard preview was not explicitly enabled or readable.";

/// Scan view for the summary persisted as `codeVocabLastScan`; idle when absent.
pub fn restore(settings: &Value) -> Value {
    let summary = &settings["codeVocabLastScan"];
    if summary.is_object() {
        finished(None, summary.clone())
    } else {
        Value::Null
    }
}

/// Terminal scan view built from the `scan_code_vocab` summary. Keeps the
/// summary's own fields (`rankedTerms[].freq`, `capped`, ...) and adds the state.
pub fn finished(id: Option<&str>, summary: Value) -> Value {
    let mut view = summary;
    let state = if view["adopted"] == false {
        "superseded"
    } else if view["terms"].as_u64().unwrap_or(0) > 0 {
        "done"
    } else {
        "empty"
    };
    view["state"] = json!(state);
    view["filesRead"] = view["files"].clone();
    if let Some(id) = id {
        view["id"] = json!(id);
    }
    view
}

/// In-flight scan view from the latest `vocab-scan-progress` payload (null until
/// the first tick). Source paths in the payload are never projected.
pub fn running(id: &str, progress: &Value) -> Value {
    let count = |key: &str| progress[key].as_u64().unwrap_or(0);
    json!({
        "id": id,
        "state": "scanning",
        "filesRead": count("filesRead"),
        "dirsSkipped": count("dirsSkipped"),
        "terms": count("termsSoFar"),
        "rankedTerms": [],
    })
}

/// View for a scan that stopped with an error. Content-free by construction.
pub fn failed(id: &str) -> Value {
    json!({"id":id,"state":"failed","filesRead":0,"terms":0,"rankedTerms":[]})
}

/// Settings fields to persist after a finished scan, matching shipped
/// `runVocabScan`: an adopted summary is stored with its folder, a superseded
/// one clears any stale stored summary.
pub fn persist(settings: &Value, folder: &str, summary: &Value) -> Option<Value> {
    if summary["adopted"] == true {
        Some(json!({
            "codeVocabFolder": folder.trim(),
            "codeVocabEnabled": true,
            "codeVocabLastScan": summary,
        }))
    } else if settings["codeVocabLastScan"].is_null() {
        None
    } else {
        Some(json!({"codeVocabLastScan": null}))
    }
}

/// Header and detail copy for the scan strip, using the shipped wording. The
/// idle state has no detail: the page intro explains it.
pub fn status(scan: &Value) -> (&'static str, String) {
    let count = |key: &str| scan[key].as_u64().unwrap_or(0);
    match scan["state"].as_str() {
        Some("scanning") => (
            "Scanning…",
            format!(
                "{} files · {} dirs skipped · {} terms",
                count("filesRead"),
                count("dirsSkipped"),
                count("terms")
            ),
        ),
        Some("done") => (
            "Scan complete",
            format!("{} terms layered on top of built-ins", count("terms")),
        ),
        Some("empty") => (
            "No identifiers found",
            // Only a scan that read nothing means the folder had no source files.
            if count("files") > 0 {
                "Built-in dev terms still active.".into()
            } else {
                "Folder had no source files — built-in dev terms still active.".into()
            },
        ),
        Some("superseded") => (
            "Scan superseded",
            "Settings changed before this scan finished, so its terms were not applied.".into(),
        ),
        Some("failed") => (
            "Scan could not finish",
            "The folder could not be read. Check that it is still available, then scan again."
                .into(),
        ),
        // The page intro already carries the idle guidance.
        _ => ("Not scanned yet", String::new()),
    }
}

/// Thousands-grouped integer, like shipped `toLocaleString()`.
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Compact bytes, as shipped: "2.3 MB" from one MiB, else rounded "812 KB".
fn bytes(n: u64) -> String {
    if n == 0 {
        return "0 MB".into();
    }
    let mb = n as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{} KB", (n as f64 / 1024.0).round() as u64)
    }
}

fn duration(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn terminal(scan: &Value) -> bool {
    matches!(
        scan["state"].as_str(),
        Some("done" | "empty" | "superseded")
    )
}

/// Result numbers for a finished scan, as `(value, caption)` pairs in the
/// shipped order. Nothing for idle, running, or failed scans.
pub fn result_stats(scan: &Value) -> Vec<(String, &'static str)> {
    if !terminal(scan) {
        return vec![];
    }
    let count = |key: &str| scan[key].as_u64().unwrap_or(0);
    vec![
        (group(count("terms")), "terms"),
        (group(count("files")), "files read"),
        (group(count("skipped")), "dirs skipped"),
        (bytes(count("bytes")), "scanned"),
        (duration(count("ms")), "duration"),
    ]
}

/// Shipped warning copy for a finished scan: not applied, cap reached, and the
/// empty guidance, which depends on whether any file was read.
pub fn warnings(scan: &Value) -> Vec<String> {
    let mut out = vec![];
    if !terminal(scan) {
        return out;
    }
    let files = scan["files"].as_u64().unwrap_or(0);
    if scan["state"] == "superseded" {
        out.push(
            "This result was not applied because the selected folder, toggle, or active scan \
             changed while it was running. Rescan the current folder to use its terms."
                .into(),
        );
    }
    let capped = scan["capped"] == true;
    if capped {
        out.push(
            "Hit the 1,000-file cap before finishing — some folders weren't indexed. Point at \
             a single project (e.g. /code/murmur-app) for full coverage."
                .into(),
        );
    }
    if !capped && scan["state"] == "empty" {
        out.push(if files > 0 {
            format!(
                "Scanned {} files but found 0 identifiers — built-in dev terms still apply.",
                group(files)
            )
        } else {
            "Found 0 source files. Wrong folder, or everything was in skipped dirs \
             (node_modules, target, .git)."
                .into()
        });
    }
    out
}

pub fn is_scanning(scan: &Value) -> bool {
    scan["state"] == "scanning"
}

/// Preview text and the shipped note for a matched phrase whose clipboard was
/// not read. A response without output is unusable, so it is an error.
pub fn command_preview(result: &Value) -> Result<(String, Option<&'static str>), String> {
    let output = result["output"]
        .as_str()
        .ok_or("Voice command preview unavailable")?;
    let note = (result["clipboardRequired"] == true && result["clipboardRead"] != true)
        .then_some(CLIPBOARD_NOT_READ);
    Ok((output.to_owned(), note))
}

/// Frequency shown beside a ranked term; the producer serializes `freq`.
pub fn frequency(term: &Value) -> u64 {
    term["freq"].as_u64().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(adopted: bool, terms: u64) -> Value {
        json!({"files":87,"skipped":6,"terms":terms,"bytes":2400000,"capped":false,"ms":610,
            "sampleTerms":["SyntheticTerm"],
            "rankedTerms":[{"term":"SyntheticTerm","freq":42},{"term":"FixtureSymbol","freq":31}],
            "whisperCount":2,"adopted":adopted})
    }

    #[test]
    fn persisted_summary_restores_through_the_settings_normalizer() {
        let mut settings = murmur_core::state::settings_defaults();
        assert!(restore(&murmur_core::state::settings(settings.clone()).unwrap()).is_null());
        settings["codeVocabLastScan"] = summary(true, 2);
        let settings = murmur_core::state::settings(settings).unwrap();
        let scan = restore(&settings);
        assert_eq!(scan["state"], "done");
        assert_eq!(scan["filesRead"], 87);
        assert_eq!(frequency(&scan["rankedTerms"][0]), 42);
        assert_eq!(status(&scan).1, "2 terms layered on top of built-ins");
    }

    #[test]
    fn an_empty_or_superseded_summary_keeps_its_own_state() {
        assert_eq!(finished(None, summary(true, 0))["state"], "empty");
        assert_eq!(finished(None, summary(false, 2))["state"], "superseded");
        assert_eq!(
            status(&finished(None, summary(true, 0))).0,
            "No identifiers found"
        );
        assert_eq!(
            status(&finished(Some("a"), summary(false, 2))).0,
            "Scan superseded"
        );
    }

    #[test]
    fn finished_scans_show_the_shipped_result_numbers() {
        let scan = finished(Some("a"), summary(true, 1234));
        let stats: Vec<_> = result_stats(&scan)
            .into_iter()
            .map(|(n, c)| format!("{n} {c}"))
            .collect();
        assert_eq!(
            stats,
            [
                "1,234 terms",
                "87 files read",
                "6 dirs skipped",
                "2.3 MB scanned",
                "0.6s duration"
            ]
        );
        let small = finished(
            None,
            json!({"files":3,"skipped":0,"terms":1,"bytes":831488,"ms":0,"adopted":true}),
        );
        assert_eq!(result_stats(&small)[3].0, "812 KB");
        assert_eq!(result_stats(&small)[4].0, "0.0s");
        assert!(result_stats(&running("a", &Value::Null)).is_empty());
        assert!(result_stats(&failed("a")).is_empty());
        assert!(result_stats(&Value::Null).is_empty());
    }

    #[test]
    fn empty_scan_guidance_depends_on_whether_files_were_read() {
        let mut read = summary(true, 0);
        read["files"] = json!(12);
        let read = finished(None, read);
        assert_eq!(status(&read).1, "Built-in dev terms still active.");
        assert_eq!(
            warnings(&read),
            ["Scanned 12 files but found 0 identifiers — built-in dev terms still apply."]
        );
        let mut none = summary(true, 0);
        none["files"] = json!(0);
        let none = finished(None, none);
        assert!(status(&none).1.starts_with("Folder had no source files"));
        assert_eq!(warnings(&none).len(), 1);
        assert!(warnings(&none)[0].starts_with("Found 0 source files."));
        assert!(!warnings(&read)[0].contains("Found 0 source files"));
    }

    #[test]
    fn superseded_and_capped_scans_carry_their_shipped_warnings() {
        let superseded = warnings(&finished(Some("a"), summary(false, 2)));
        assert_eq!(superseded.len(), 1);
        assert!(superseded[0].contains("not applied"));
        let mut capped = summary(true, 0);
        capped["capped"] = json!(true);
        let capped = warnings(&finished(None, capped));
        assert_eq!(capped.len(), 1, "cap replaces the empty guidance");
        assert!(capped[0].contains("1,000-file cap"));
        assert!(warnings(&finished(None, summary(true, 2))).is_empty());
        assert!(warnings(&failed("a")).is_empty());
    }

    #[test]
    fn ranked_terms_use_the_producers_freq_field() {
        let scan = finished(Some("scan-1"), summary(true, 2));
        let counts: Vec<_> = scan["rankedTerms"]
            .as_array()
            .unwrap()
            .iter()
            .map(frequency)
            .collect();
        assert_eq!(counts, [42, 31]);
        assert_eq!(frequency(&json!({"term":"x","frequency":9})), 0);
    }

    #[test]
    fn running_scan_is_cancellable_before_the_first_tick_and_hides_paths() {
        let first = running("scan-1", &Value::Null);
        assert!(is_scanning(&first));
        assert_eq!(first["id"], "scan-1");
        let tick = running(
            "scan-1",
            &json!({"scanId":"scan-1","currentPath":"/synthetic/src/a.rs","filesRead":12,
                "dirsSkipped":2,"termsSoFar":30,"done":false,"adopted":false}),
        );
        assert_eq!(status(&tick).1, "12 files · 2 dirs skipped · 30 terms");
        assert!(!tick.to_string().contains("/synthetic"));
        assert!(!is_scanning(&failed("scan-1")));
        assert_eq!(status(&failed("scan-1")).0, "Scan could not finish");
    }

    #[test]
    fn adopted_scans_persist_folder_and_summary_and_superseded_ones_clear_it() {
        let mut settings =
            murmur_core::state::settings(murmur_core::state::settings_defaults()).unwrap();
        let adopted = summary(true, 2);
        let patch = persist(&settings, " /synthetic/project ", &adopted).unwrap();
        assert_eq!(patch["codeVocabFolder"], "/synthetic/project");
        assert_eq!(patch["codeVocabLastScan"], adopted);
        // The service accepts exactly this intent: it normalizes to itself.
        let mut next = settings.clone();
        for (key, value) in patch.as_object().unwrap() {
            next[key] = value.clone();
        }
        let normalized = murmur_core::state::settings(next).unwrap();
        for (key, value) in patch.as_object().unwrap() {
            assert_eq!(&normalized[key], value, "{key}");
        }
        let superseded = summary(false, 2);
        assert!(persist(&settings, "/synthetic/project", &superseded).is_none());
        settings["codeVocabLastScan"] = adopted;
        assert_eq!(
            persist(&settings, "/synthetic/project", &superseded),
            Some(json!({"codeVocabLastScan":null}))
        );
    }

    #[test]
    fn matched_clipboard_command_explains_an_unread_clipboard() {
        let (text, note) = command_preview(
            &json!({"output":"Hello ","matched":true,"clipboardRequired":true,"clipboardRead":false}),
        )
        .unwrap();
        assert_eq!(text, "Hello ");
        assert_eq!(note, Some(CLIPBOARD_NOT_READ));
        let read = json!({"output":"Hello x","matched":true,"clipboardRequired":true,"clipboardRead":true});
        assert_eq!(command_preview(&read).unwrap().1, None);
        let plain =
            json!({"output":"Hi","matched":true,"clipboardRequired":false,"clipboardRead":false});
        assert_eq!(command_preview(&plain).unwrap().1, None);
        assert!(command_preview(&json!({"matched":true})).is_err());
    }
}
