//! Import confirmation owns the exact reviewed bytes, not a mutable path.
use super::backend::Backend;
use crate::knowledge::model::{Bulk, BulkAction, Filter};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, sync::Mutex};

/// Shipped wording for a file the person chose but Murmur could not read.
pub const UNREADABLE: &str = "Murmur could not read the selected knowledge import.";

#[derive(Default)]
pub struct Reviews(Mutex<BTreeMap<String, (Vec<u8>, u64)>>);
impl Reviews {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, (Vec<u8>, u64)>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// One review is open at a time; choosing another file replaces it.
    pub fn begin(&self, bytes: Vec<u8>, revision: u64) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let mut reviews = self.map();
        reviews.clear();
        reviews.insert(token.clone(), (bytes, revision));
        token
    }
    /// A review stays open until it succeeds or is cancelled, so a failed
    /// import can be retried or dismissed without choosing the file again.
    pub fn get(&self, token: &str) -> Result<(Vec<u8>, u64), &'static str> {
        self.map().get(token).cloned().ok_or(REVIEW_EXPIRED)
    }
    pub fn finish(&self, token: &str) {
        self.map().remove(token);
    }
    pub fn clear(&self) {
        self.map().clear();
    }
}

pub const REVIEW_EXPIRED: &str = "Import review expired";
pub const REVIEW_UNAVAILABLE: &str = "Import review unavailable";
// Shipped collection failures, each ending with the guarantee that holds.
pub const COLLECT_CHANGED: &str =
    "Knowledge changed while collecting matches. Refresh and try again. No records were changed.";
pub const COLLECT_PAGES: &str =
    "Knowledge pages changed. Refresh and try again. No records were changed.";
pub const COLLECT_MATCHES: &str =
    "Knowledge matches changed. Refresh and try again. No records were changed.";
/// Explanations this adapter raises itself, shown to the person verbatim.
pub const ADAPTER: [&str; 5] = [
    REVIEW_EXPIRED,
    REVIEW_UNAVAILABLE,
    COLLECT_CHANGED,
    COLLECT_PAGES,
    COLLECT_MATCHES,
];
/// What a record that could not be changed says when the owner's reason is
/// not one of the known, content-free ones.
pub const RECORD_FAILED: &str = "Murmur could not change this record.";
/// Shipped default name for a knowledge export.
pub const EXPORT_NAME: &str = "murmur-personal-knowledge.json";
const NOTICE_LIMIT: usize = 512;

pub fn import_notice(imported: u64, duplicates: u64) -> String {
    format!(
        "Imported {imported}; skipped {duplicates} duplicate{}.",
        if duplicates == 1 { "" } else { "s" }
    )
}

pub fn export_notice(count: u64) -> String {
    format!(
        "Exported {count} {}.",
        if count == 1 { "record" } else { "records" }
    )
}

/// Adds one page of the matching records to the snapshot being collected and
/// returns the next offset, or `None` when the snapshot is complete. A change
/// between pages fails the whole collection before any record is touched.
pub fn collect_page(
    bulk: &mut Option<Bulk>,
    filter: &Filter,
    offset: usize,
    result: &Value,
) -> Result<Option<usize>, String> {
    let unavailable = || "Knowledge unavailable".to_string();
    let revision = result["storeRevision"].as_u64().ok_or_else(unavailable)?;
    let total = result["total"].as_u64().ok_or_else(unavailable)? as usize;
    let entries = result["entries"]
        .as_array()
        .ok_or_else(unavailable)?
        .clone();
    let next = result["nextOffset"].as_u64().map(|n| n as usize);
    if bulk
        .as_ref()
        .is_some_and(|b| b.revision != revision || b.total != total)
    {
        return Err(COLLECT_CHANGED.into());
    }
    if bulk.is_none() {
        *bulk = Some(
            Bulk::new(filter.clone(), revision, total)
                .map_err(|_| "Knowledge exceeds supported size".to_string())?,
        );
    }
    let b = bulk.as_mut().ok_or_else(unavailable)?;
    let mut ids: std::collections::BTreeSet<&str> =
        b.entries.iter().filter_map(|v| v["id"].as_str()).collect();
    if !entries
        .iter()
        .all(|v| v["id"].as_str().is_some_and(|id| ids.insert(id)))
    {
        return Err(COLLECT_PAGES.into());
    }
    let collected = b.entries.len() + entries.len();
    match next {
        None if collected != total => return Err(COLLECT_MATCHES.into()),
        Some(n) if n != collected || n <= offset => return Err(COLLECT_PAGES.into()),
        _ => (),
    }
    b.append(revision, total, entries, next)
        .map_err(|_| COLLECT_PAGES.to_string())?;
    Ok(next)
}

/// One record a bulk change could not apply: title, id, the owner's reason.
pub type Failure = (String, String, String);

/// The shipped bulk summary, bounded so the projection accepts it. Failures
/// that do not fit are counted rather than dropped silently.
pub fn bulk_notice(
    action: BulkAction,
    succeeded: usize,
    unchanged: usize,
    failures: &[Failure],
) -> String {
    let past = |a: BulkAction| {
        if a == BulkAction::Enable {
            "enabled"
        } else {
            "disabled"
        }
    };
    let mut text = format!(
        "{succeeded} records {}; {} failed.",
        match action {
            BulkAction::Delete => "removed",
            other => past(other),
        },
        failures.len()
    );
    if action != BulkAction::Delete {
        text.push_str(&format!(" {unchanged} already {}.", past(action)));
    }
    let mut shown = 0;
    for (title, id, message) in failures {
        let line = format!("\n{title} · {id}: {message}");
        let reserve = if shown + 1 < failures.len() { 40 } else { 0 };
        if text.len() + line.len() + reserve > NOTICE_LIMIT {
            break;
        }
        text.push_str(&line);
        shown += 1;
    }
    if shown < failures.len() {
        text.push_str(&format!("\n…and {} more failed.", failures.len() - shown));
    }
    text
}

/// Reads the chosen file, hiding the operating system's detail.
pub fn read_import(path: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|_| UNREADABLE)?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE)?;
    Ok(bytes)
}

impl Backend {
    pub async fn inspect_knowledge(&self, path: String) -> Result<Value, String> {
        let bytes = read_import(&path)?;
        let (mut summary, revision) = self.services.inspect_knowledge(&bytes)?;
        summary["token"] = json!(self.knowledge_reviews.begin(bytes, revision));
        Ok(summary)
    }
    /// Trigger conflicts are reported in the review, not a reason to refuse:
    /// only a same-ID conflict rejects an import, and the owner decides that.
    pub async fn import_knowledge(&self, preview: Value) -> Result<(u64, u64), String> {
        let token = preview["token"].as_str().ok_or(REVIEW_UNAVAILABLE)?;
        let (bytes, revision) = self.knowledge_reviews.get(token)?;
        let counts = self.services.import_knowledge(&bytes, revision)?;
        self.knowledge_reviews.finish(token);
        self.events
            .publish("main", "native-services-changed", "null");
        Ok(counts)
    }
    pub fn discard_knowledge_review(&self) {
        self.knowledge_reviews.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_reports_the_shipped_line_without_the_path() {
        let missing = std::env::temp_dir().join("murmur-synthetic-missing-knowledge-import.json");
        let e = read_import(missing.to_str().unwrap()).unwrap_err();
        assert_eq!(e, UNREADABLE);
        assert!(!e.contains("murmur-synthetic"));
        let dir = std::env::temp_dir();
        assert_eq!(read_import(dir.to_str().unwrap()).unwrap_err(), UNREADABLE);
    }

    fn page(
        revision: u64,
        total: usize,
        ids: std::ops::Range<usize>,
        next: Option<usize>,
    ) -> Value {
        json!({
            "storeRevision": revision,
            "total": total,
            "entries": ids.map(crate::knowledge::fixtures::entry).collect::<Vec<_>>(),
            "nextOffset": next,
        })
    }

    #[test]
    fn collection_gathers_every_page_and_refuses_each_shipped_inconsistency() {
        let filter = Filter::default();
        let mut bulk = None;
        assert_eq!(
            collect_page(&mut bulk, &filter, 0, &page(4, 60, 0..50, Some(50))),
            Ok(Some(50))
        );
        assert_eq!(
            collect_page(&mut bulk, &filter, 50, &page(4, 60, 50..60, None)),
            Ok(None)
        );
        assert_eq!(bulk.as_ref().unwrap().entries.len(), 60);

        let first = || {
            let mut bulk = None;
            collect_page(&mut bulk, &filter, 0, &page(4, 60, 0..50, Some(50))).unwrap();
            bulk
        };
        // The store moved on between pages.
        let mut moved = first();
        assert_eq!(
            collect_page(&mut moved, &filter, 50, &page(5, 60, 50..60, None)),
            Err(COLLECT_CHANGED.to_string())
        );
        let mut resized = first();
        assert_eq!(
            collect_page(&mut resized, &filter, 50, &page(4, 61, 50..60, None)),
            Err(COLLECT_CHANGED.to_string())
        );
        // A record repeated on a later page.
        let mut repeated = first();
        assert_eq!(
            collect_page(&mut repeated, &filter, 50, &page(4, 60, 40..50, None)),
            Err(COLLECT_PAGES.to_string())
        );
        // The last page leaves records unaccounted for.
        let mut short = first();
        assert_eq!(
            collect_page(&mut short, &filter, 50, &page(4, 60, 50..55, None)),
            Err(COLLECT_MATCHES.to_string())
        );
        // A next offset that does not follow the records collected so far.
        let mut skipped = first();
        assert_eq!(
            collect_page(&mut skipped, &filter, 50, &page(4, 60, 50..55, Some(58))),
            Err(COLLECT_PAGES.to_string())
        );
        // A response with no store revision is unavailable, never a partial snapshot.
        let mut bad = None;
        assert_eq!(
            collect_page(&mut bad, &filter, 0, &json!({"total": 1, "entries": []})),
            Err("Knowledge unavailable".to_string())
        );
        // Every refusal is explained in the person's words, not a generic failure.
        for line in [COLLECT_CHANGED, COLLECT_PAGES, COLLECT_MATCHES] {
            assert_eq!(super::super::explain::operation(line), Some(line), "{line}");
        }
    }

    #[test]
    fn the_bulk_summary_uses_the_shipped_wording_for_each_action() {
        assert_eq!(
            bulk_notice(BulkAction::Delete, 3, 0, &[]),
            "3 records removed; 0 failed."
        );
        assert_eq!(
            bulk_notice(BulkAction::Enable, 2, 1, &[]),
            "2 records enabled; 0 failed. 1 already enabled."
        );
        let failed = vec![(
            "Synthetic phrase 1".to_string(),
            "synthetic-1".to_string(),
            "This knowledge record changed in another window. Refresh and try again.".to_string(),
        )];
        assert_eq!(
            bulk_notice(BulkAction::Disable, 2, 0, &failed),
            "2 records disabled; 1 failed. 0 already disabled.\nSynthetic phrase 1 · synthetic-1: This knowledge record changed in another window. Refresh and try again."
        );
    }

    #[test]
    fn a_long_failure_list_is_bounded_and_counts_what_it_omits() {
        let failures: Vec<Failure> = (0..200)
            .map(|i| {
                (
                    format!("Synthetic phrase {i}"),
                    format!("synthetic-{i}"),
                    RECORD_FAILED.to_string(),
                )
            })
            .collect();
        let notice = bulk_notice(BulkAction::Delete, 0, 0, &failures);
        assert!(crate::section::text(&notice, 512), "{} bytes", notice.len());
        let shown = notice
            .lines()
            .filter(|l| l.contains(" · synthetic-"))
            .count();
        assert!(shown > 0 && shown < 200);
        assert!(
            notice.ends_with(&format!("\n…and {} more failed.", 200 - shown)),
            "{notice}"
        );
        assert!(notice.starts_with("0 records removed; 200 failed."));
        // Nothing is dropped silently when everything fits.
        let few = bulk_notice(BulkAction::Delete, 1, 0, &failures[..2]);
        assert!(!few.contains("more failed"));
    }

    #[test]
    fn import_and_export_notices_report_counts_with_correct_plurals() {
        assert_eq!(import_notice(2, 1), "Imported 2; skipped 1 duplicate.");
        assert_eq!(import_notice(0, 0), "Imported 0; skipped 0 duplicates.");
        assert_eq!(import_notice(1, 2), "Imported 1; skipped 2 duplicates.");
        assert_eq!(export_notice(1), "Exported 1 record.");
        assert_eq!(export_notice(0), "Exported 0 records.");
    }

    #[test]
    fn a_review_survives_a_failed_import_and_ends_on_success_or_cancel() {
        let reviews = Reviews::default();
        let first = reviews.begin(b"one".to_vec(), 7);
        assert_eq!(reviews.get(&first).unwrap(), (b"one".to_vec(), 7));
        // A failed import leaves the review open for a retry.
        assert_eq!(reviews.get(&first).unwrap().1, 7);
        reviews.finish(&first);
        assert_eq!(reviews.get(&first), Err(REVIEW_EXPIRED));
        let second = reviews.begin(b"two".to_vec(), 8);
        let third = reviews.begin(b"three".to_vec(), 9);
        assert!(reviews.get(&second).is_err(), "a new review replaces it");
        reviews.clear();
        assert!(reviews.get(&third).is_err(), "cancel drops the review");
    }
}
