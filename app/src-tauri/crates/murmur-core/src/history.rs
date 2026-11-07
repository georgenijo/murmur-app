//! Bounded History presentation/query and explicit export boundary. No logging,
//! automatic serialization, storage, completion producer or statistics writes.
use serde_json::{json, Map, Value};
use std::ops::Range;

pub const MAX_ENTRIES: usize = 200;
pub const MAX_PINS: usize = 20;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct Entry {
    /// Stable service-issued row identity, distinct even for duplicate legacy IDs.
    pub key: u64,
    pub text: String,
    pub timestamp: i64,
    pub duration: f64,
    pub file: bool,
    pub source_name: String,
    pub pinned: bool,
    pub interrupted: bool,
    original: Value,
}
impl Entry {
    /// Only project validated completion-store rows, oldest first. The original
    /// value is retained solely for exact-target core mutations; never export it.
    pub fn project(key: u64, original: Value) -> Option<Self> {
        let id = original.get("id")?;
        if !(id.is_string() || id.is_number()) {
            return None;
        }
        let text = original.get("text")?.as_str()?;
        let time = original.get("timestamp")?.as_f64()?;
        if !time.is_finite() || !(0.0..=8_640_000_000_000_000.0).contains(&time) {
            return None;
        }
        let timestamp = time as i64;
        let duration = original.get("duration")?.as_f64()?;
        let source_name = original
            .get("sourceName")
            .and_then(Value::as_str)
            .unwrap_or("");
        if key == 0
            || text.len() > MAX_TEXT_BYTES
            || source_name.len() > 4096
            || timestamp < 0
            || !duration.is_finite()
            || duration < 0.0
            || original.to_string().len() > 2 * MAX_TEXT_BYTES
        {
            return None;
        }
        Some(Self {
            key,
            text: text.into(),
            timestamp,
            duration,
            file: original.get("source").and_then(Value::as_str) == Some("file"),
            source_name: source_name.into(),
            pinned: original.get("pinned") == Some(&Value::Bool(true)),
            interrupted: original.get("interruption").is_some(),
            original,
        })
    }
    fn time(&self) -> f64 {
        self.original["timestamp"]
            .as_f64()
            .unwrap_or(self.timestamp as f64)
    }
    pub fn target(&self) -> Value {
        self.original.clone()
    }
    pub fn retained_bytes(&self) -> usize {
        self.text.len() + self.source_name.len() + self.original.to_string().len() + 256
    }
    pub fn raw_text(&self) -> Option<&str> {
        self.original.get("rawText").and_then(Value::as_str)
    }
    pub fn source_label(&self) -> &str {
        if self.file {
            if self.source_name.is_empty() {
                "File"
            } else {
                &self.source_name
            }
        } else {
            "Mic"
        }
    }
    fn export_json(&self) -> Value {
        let mut v = json!({"schemaVersion":2,"id": self.original.get("id"), "pinned":self.pinned,
            "timestamp":self.original.get("timestamp"),"durationSeconds":self.duration,
            "source":if self.file {"file"} else {"recording"},"text":self.text});
        if !self.source_name.is_empty() {
            v["sourceName"] = json!(self.source_name);
        }
        if let Some(raw) = self.raw_text() {
            v["rawText"] = json!(raw);
        }
        // Pick each field; never spread source objects or local teaching scope.
        for (name, fields) in [
            (
                "recording",
                &["recordingId", "modelId", "modeId", "profileId"][..],
            ),
            ("derived", &["sourceEntryId", "modeId", "createdAt"][..]),
        ] {
            if let Some(source) = self.original.get(name).and_then(Value::as_object) {
                let mut out = Map::new();
                for field in fields {
                    if let Some(value) = source.get(*field) {
                        if value.is_string() || value.is_number() || value.is_null() {
                            out.insert((*field).into(), value.clone());
                        }
                    }
                }
                if let Some(stages) = source.get("stages").and_then(Value::as_array) {
                    out.insert(
                        "stages".into(),
                        Value::Array(
                            stages
                                .iter()
                                .map(|s| {
                                    let mut stage = Map::new();
                                    for field in ["stage", "outcome", "changed"] {
                                        if let Some(value) = s.get(field) {
                                            if value.is_string() || value.is_boolean() {
                                                stage.insert(field.into(), value.clone());
                                            }
                                        }
                                    }
                                    Value::Object(stage)
                                })
                                .collect(),
                        ),
                    );
                }
                v[name] = Value::Object(out);
            }
        }
        v
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Source {
    #[default]
    All,
    Spoken,
    Files,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Date {
    #[default]
    All,
    Today,
    Week,
    Month,
}
/// Local-calendar cutoffs supplied by the host clock (calendar arithmetic,
/// including DST); never subtract fixed 24-hour periods for date filters.
#[derive(Clone, Copy, Default)]
pub struct Clock {
    pub now: i64,
    pub today: i64,
    pub week: i64,
    pub month: i64,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub source: Source,
    pub date: Date,
    pub pinned: bool,
}
impl Query {
    pub fn narrowed(&self) -> bool {
        !self.text.trim().is_empty()
            || self.source != Source::All
            || self.date != Date::All
            || self.pinned
    }
    pub fn filter_label(&self) -> String {
        let mut parts = vec![];
        match self.source {
            Source::All => (),
            Source::Spoken => parts.push("Spoken"),
            Source::Files => parts.push("Files"),
        }
        match self.date {
            Date::All => (),
            Date::Today => parts.push("Today"),
            Date::Week => parts.push("Past 7 days"),
            Date::Month => parts.push("Past 30 days"),
        }
        if self.pinned {
            parts.push("Pinned only");
        }
        format!(
            "Filter transcripts: {}",
            if parts.is_empty() {
                "Everything".into()
            } else {
                parts.join(" · ")
            }
        )
    }
    /// Indices into the complete retained snapshot, newest first, with reverse
    /// stored order as the tie-breaker. Never restrict queries to painted rows.
    pub fn matching(&self, entries: &[Entry], clock: Clock) -> Vec<usize> {
        let tokens: Vec<_> = self
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let cutoff = match self.date {
            Date::All => None,
            Date::Today => Some(clock.today),
            Date::Week => Some(clock.week),
            Date::Month => Some(clock.month),
        };
        let mut found: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (self.source == Source::All || (self.source == Source::Files) == e.file)
                    && (!self.pinned || e.pinned)
                    && cutoff.is_none_or(|t| e.time() >= t as f64 && e.time() <= clock.now as f64)
                    && {
                        let hay = format!("{} {}", e.text, e.source_name).to_lowercase();
                        tokens.iter().all(|t| hay.contains(t))
                    }
            })
            .map(|(i, _)| i)
            .collect();
        found.sort_by(|&a, &b| {
            entries[b]
                .time()
                .total_cmp(&entries[a].time())
                .then(b.cmp(&a))
        });
        found
    }
}
/// Unicode lowercase can expand (İ). Map folded bytes back to whole original
/// scalars so highlights always reassemble exactly, without slicing a glyph.
pub fn highlights(text: &str, query: &str) -> Vec<Range<usize>> {
    let folded = text.to_lowercase();
    let mut offsets = vec![];
    for (start, c) in text.char_indices() {
        let lower = c.to_lowercase().to_string();
        offsets.extend(std::iter::repeat_n(
            start..start + c.len_utf8(),
            lower.len(),
        ));
    }
    let mut ranges = vec![];
    for token in query.split_whitespace().map(str::to_lowercase) {
        for (i, _) in folded.match_indices(&token) {
            if ranges.len() == 200 {
                break;
            }
            ranges.push(offsets[i].start..offsets[i + token.len() - 1].end);
        }
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = vec![];
    for r in ranges {
        if let Some(last) = merged.last_mut().filter(|last| r.start <= last.end) {
            last.end = last.end.max(r.end);
        } else {
            merged.push(r);
        }
    }
    merged
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Text,
    Json,
}
impl Format {
    pub const ALL: [Self; 3] = [Self::Markdown, Self::Text, Self::Json];
    pub fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Text => "Plain text",
            Self::Json => "JSON",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Json => "json",
        }
    }
}
/// Timestamp rendering belongs to the platform's local time zone. The renderer
/// receives fixed stamps, so deterministic tests do not depend on the OS zone.
pub fn export(
    entries: &[&Entry],
    format: Format,
    exported_at: &str,
    stamp: impl Fn(i64) -> String,
) -> Option<String> {
    if entries.len() > MAX_ENTRIES {
        return None;
    }
    let mut out = match format {
        Format::Json => serde_json::to_string_pretty(&json!({"schema":"murmur.history.v2","exportedAt":exported_at,"count":entries.len(),"entries":entries.iter().map(|e|e.export_json()).collect::<Vec<_>>() })).ok()?,
        Format::Markdown => format!("# Murmur transcript history\n\nExported {exported_at} · {} {}\n\n",entries.len(),if entries.len()==1 {"entry"} else {"entries"}),
        Format::Text => String::new(),
    };
    if format != Format::Json {
        for (i, e) in entries.iter().enumerate() {
            let seconds = e.duration.round().max(0.) as u64;
            let duration = if seconds < 60 {
                format!("{seconds}s")
            } else {
                format!("{}m {}s", seconds / 60, seconds % 60)
            };
            if format == Format::Markdown {
                out.push_str(&format!(
                    "## {}\n\n- Source: {}\n- Duration: {duration}\n\n{}\n\n",
                    stamp(e.timestamp),
                    e.source_label(),
                    e.text
                ));
            } else {
                if i > 0 {
                    out.push_str("\n\n");
                }
                out.push_str(&format!(
                    "[{} · {} · {duration}]\n{}",
                    stamp(e.timestamp),
                    e.source_label(),
                    e.text
                ));
            }
            if out.len() > MAX_EXPORT_BYTES {
                return None;
            }
        }
    }
    let out = format!("{}\n", out.trim_end());
    (out.len() <= MAX_EXPORT_BYTES).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(key: u64, text: &str, time: i64) -> Entry {
        Entry::project(key,json!({"id":"duplicate","text":text,"timestamp":time,"duration":65.,"teachingContext":{"projectRoot":"SYNTHETIC_PRIVATE"},
            "rawText":"synthetic raw", "recording":{"recordingId":7,"modelId":"synthetic-model","modeId":null,"profileId":null,"teachingContext":"SYNTHETIC_PRIVATE","stages":[{"stage":"synthetic","outcome":"applied","changed":true,"private":"SYNTHETIC_PRIVATE"}]}})).unwrap()
    }
    #[test]
    fn query_orders_duplicate_legacy_ids_and_timestamp_ties_by_exact_position() {
        let entries = vec![
            entry(1, "violet lighthouse", 1),
            entry(2, "violet lighthouse", 1),
            entry(3, "violet", 2),
        ];
        let q = Query {
            text: "VIOLET LIGHTHOUSE".into(),
            ..Default::default()
        };
        assert_eq!(q.matching(&entries, Clock::default()), vec![1, 0]);
        assert_ne!(entries[0].key, entries[1].key);
        assert_eq!(entries[0].target(), entries[1].target());
    }
    #[test]
    fn fractional_legacy_timestamp_is_preserved_in_explicit_json() {
        let e = Entry::project(
            1,
            json!({"id":"synthetic","text":"synthetic","timestamp":1.5,"duration":1}),
        )
        .unwrap();
        let text = export(&[&e], Format::Json, "synthetic", |_| "synthetic".into()).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["entries"][0]["timestamp"], 1.5);
    }
    #[test]
    fn local_calendar_cutoffs_exclude_future_rows_and_include_boundary() {
        let entries = vec![
            entry(1, "synthetic", 100),
            entry(2, "synthetic", 99),
            entry(3, "synthetic", 201),
        ];
        let clock = Clock {
            now: 200,
            today: 100,
            week: 50,
            month: 0,
        };
        let q = Query {
            date: Date::Today,
            ..Default::default()
        };
        assert_eq!(q.matching(&entries, clock), vec![0]);
    }
    #[test]
    fn export_whitelists_nested_metadata_and_never_exports_scope_or_raw_in_prose() {
        let e = entry(1, "synthetic delivered", 1);
        for f in Format::ALL {
            let text = export(&[&e], f, "2026-11-07T00:00:00Z", |_| {
                "2026-11-07 00:00:00".into()
            })
            .unwrap();
            assert!(!text.contains("SYNTHETIC_PRIVATE"));
            assert!(!text.contains("teachingContext"));
            assert_eq!(text.contains("synthetic raw"), f == Format::Json);
            assert_eq!(text.contains("synthetic-model"), f == Format::Json);
        }
    }
    #[test]
    fn unicode_highlights_reassemble_exactly_and_merge_overlaps() {
        assert_eq!(highlights("therein", "there here"), vec![0..5]);
        let text = "İstanbul 猫 CAFÉ café";
        let ranges = highlights(text, "i café 猫");
        assert_eq!(ranges, vec![0..2, 10..13, 14..19, 20..25]);
        let mut result = String::new();
        let mut cursor = 0;
        for r in ranges {
            result.push_str(&text[cursor..r.start]);
            result.push_str(&text[r.clone()]);
            cursor = r.end;
        }
        result.push_str(&text[cursor..]);
        assert_eq!(result, text);
    }
    #[test]
    fn projection_and_export_enforce_bounds_without_logging_rejected_text() {
        assert!(Entry::project(
            0,
            json!({"id":"x","text":"synthetic","timestamp":1,"duration":1})
        )
        .is_none());
        assert!(Entry::project(
            1,
            json!({"id":{"private":"synthetic"},"text":"synthetic","timestamp":1,"duration":1})
        )
        .is_none());
        let e = entry(1, &"x".repeat(MAX_TEXT_BYTES), 1);
        assert!(
            export(&vec![&e; MAX_ENTRIES], Format::Text, "synthetic", |_| {
                "synthetic".into()
            })
            .is_none()
        );
    }
}
