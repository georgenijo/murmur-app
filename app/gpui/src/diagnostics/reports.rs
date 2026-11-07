//! Strict, bounded, session-only portable reports. Validation mirrors the
//! retained React parser. Neither raw input nor parser errors leave this module.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMPORTS: usize = 20;
/// UTF-8 bytes, including text that will subsequently be stripped.
pub const MAX_STRING_BYTES: usize = 4096;
pub const MAX_COMPARISON_METRICS: usize = 8192;
const MAX_COMPARISON_SCOPE_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMPARISON_WORK: usize = 1_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportError {
    Empty,
    Oversized,
    MalformedJson,
    UnsupportedVersion,
    SchemaMismatch,
    CollectionLimit,
    StringLimit,
    SessionLimit,
    ReadFailed,
}
impl ImportError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "The selected diagnostic report is empty.",
            Self::Oversized => "Diagnostic reports are limited to 8 MiB.",
            Self::MalformedJson => "The selected file is not valid JSON.",
            Self::UnsupportedVersion => {
                "This diagnostic report version is not supported by this Murmur build."
            }
            Self::SchemaMismatch => {
                "The selected JSON is not a supported Murmur benchmark or evaluation report."
            }
            Self::CollectionLimit => {
                "The selected report exceeds supported diagnostic collection limits."
            }
            Self::StringLimit => "Diagnostic report strings are limited to 4 KiB each.",
            Self::SessionLimit => {
                "Clear session imports before importing more reports (20 maximum)."
            }
            Self::ReadFailed => "The selected diagnostic report could not be read.",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Benchmark,
    Evaluation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Imported,
    Local,
}
#[derive(Clone)]
pub struct Report {
    pub kind: Kind,
    pub source: Source,
    id: String,
    data: Arc<Value>,
}
#[derive(Clone, Copy)]
enum Rule {
    Str,
    NullableStr,
    Num,
    NullableNum,
    Int,
    NullableInt,
    SignedInt,
    Bool,
    NullableBool,
}
use Rule::*;
// Check the entire tree before schema validation or any identity formatting.
// This includes nullable fields, arrays, unknown fields and object keys.
fn bounded_strings(v: &Value) -> bool {
    match v {
        Value::String(s) => s.len() <= MAX_STRING_BYTES,
        Value::Array(a) => a.iter().all(bounded_strings),
        Value::Object(o) => o
            .iter()
            .all(|(k, v)| k.len() <= MAX_STRING_BYTES && bounded_strings(v)),
        _ => true,
    }
}
fn valid(v: &Value, rule: Rule) -> bool {
    let num = || v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.);
    let int = || {
        v.as_f64()
            .is_some_and(|n| n.is_finite() && n.fract() == 0. && n.abs() <= 9_007_199_254_740_991.)
    };
    match rule {
        Str => v.is_string(),
        NullableStr => v.is_null() || v.is_string(),
        Num => num(),
        NullableNum => v.is_null() || num(),
        Int => num() && int(),
        NullableInt => v.is_null() || num() && int(),
        SignedInt => v.is_null() || int(),
        Bool => v.is_boolean(),
        NullableBool => v.is_null() || v.is_boolean(),
    }
}
fn fields(v: &Value, keys: &[(&str, Rule)], nested: &[&str], optional: &[(&str, Rule)]) -> bool {
    let Some(object) = v.as_object() else {
        return false;
    };
    object.keys().all(|key| {
        keys.iter().chain(optional).any(|(k, _)| key == k) || nested.contains(&key.as_str())
    }) && keys
        .iter()
        .all(|(key, rule)| object.get(*key).is_some_and(|v| valid(v, *rule)))
        && nested.iter().all(|key| object.contains_key(*key))
        && optional
            .iter()
            .all(|(key, rule)| object.get(*key).is_none_or(|v| valid(v, *rule)))
}
fn array(v: &Value, max: usize) -> Result<&[Value], ImportError> {
    let a = v.as_array().ok_or(ImportError::SchemaMismatch)?;
    if a.len() > max {
        Err(ImportError::CollectionLimit)
    } else {
        Ok(a)
    }
}
fn strings(v: &Value) -> Result<bool, ImportError> {
    Ok(array(v, 512)?.iter().all(Value::is_string))
}
fn unique<'a>(entries: impl Iterator<Item = &'a Value>, field: &str) -> bool {
    let values: Vec<_> = entries.map(|v| v[field].to_string()).collect();
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn date(v: &Value) -> bool {
    // Wire dates are ISO timestamps. The native importer does not guess locale.
    v.as_str()
        .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
}
const MODEL_METRICS: &[&str] = &[
    "modelLoadMs",
    "firstInferenceMs",
    "warmMedianMs",
    "warmP95Ms",
    "realtimeFactor",
    "wordErrorRate",
    "normalizedWordErrorRate",
    "deliveredWordErrorRate",
    "deliveredNormalizedWordErrorRate",
];
fn complete_model(v: &Value) -> bool {
    MODEL_METRICS.iter().all(|k| valid(&v[*k], Num))
        && v["fixtures"].as_array().is_some_and(|a| !a.is_empty())
}
fn benchmark(v: &Value) -> Result<bool, ImportError> {
    let versioned = v.get("reportVersion").is_some();
    let mut nested = vec!["recommendations", "results"];
    let mut keys = vec![
        ("createdAt", Str),
        ("appVersion", Str),
        ("platform", Str),
        ("preset", Str),
        ("iterations", Int),
        ("sharedInitMs", Num),
    ];
    if versioned {
        keys.push(("reportVersion", Int));
        nested.extend(["environment", "corpus", "configuration"]);
    }
    if !fields(v, &keys, &nested, &[])
        || !date(&v["createdAt"])
        || !["quick", "standard", "thorough"].contains(&v["preset"].as_str().unwrap_or(""))
        || v["iterations"].as_u64().unwrap_or(0) == 0
        || !fields(
            &v["recommendations"],
            &[
                ("fastest", NullableStr),
                ("mostAccurate", NullableStr),
                ("balanced", NullableStr),
            ],
            &[],
            &[],
        )
    {
        return Ok(false);
    }
    let models = array(&v["results"], 64)?;
    let mut identities = BTreeSet::new();
    for m in models {
        let mut keys = vec![
            ("modelName", Str),
            ("label", Str),
            ("backend", Str),
            ("accelerator", Str),
            ("memoryDeltaMb", Num),
            ("error", NullableStr),
        ];
        keys.extend(MODEL_METRICS.iter().map(|k| (*k, NullableNum)));
        if !fields(m, &keys, &["fixtures"], &[("downloadSize", Str)])
            || !identities.insert((
                m["modelName"].as_str(),
                m["backend"].as_str(),
                m["accelerator"].as_str(),
            ))
        {
            return Ok(false);
        }
        let fixtures = array(&m["fixtures"], 512)?;
        if !unique(fixtures.iter(), "fixtureId") {
            return Ok(false);
        }
        for f in fixtures {
            if !fields(
                f,
                &[
                    ("fixtureId", Str),
                    ("label", Str),
                    ("audioSeconds", Num),
                    ("warmMedianMs", Num),
                    ("warmP95Ms", Num),
                    ("realtimeFactor", Num),
                    ("wordErrorRate", Num),
                    ("wordErrors", Int),
                    ("referenceWords", Int),
                    ("normalizedWordErrorRate", Num),
                    ("normalizedWordErrors", Int),
                    ("normalizedReferenceWords", Int),
                    ("reference", Str),
                    ("transcript", Str),
                    ("deliveredTranscript", Str),
                    ("deliveredWordErrorRate", Num),
                    ("deliveredWordErrors", Int),
                    ("deliveredNormalizedWordErrorRate", Num),
                    ("deliveredNormalizedWordErrors", Int),
                    ("deliveredTransformFailed", Bool),
                ],
                &[],
                &[],
            ) {
                return Ok(false);
            }
        }
        if m["error"].is_null() && !complete_model(m) {
            return Ok(false);
        }
    }
    if versioned {
        let e = &v["environment"];
        let c = &v["corpus"];
        let conf = &v["configuration"];
        if !fields(
            e,
            &[
                ("os", Str),
                ("osVersion", NullableStr),
                ("architecture", Str),
                ("hardwareModel", NullableStr),
                ("chip", NullableStr),
                ("memoryMb", NullableInt),
            ],
            &[],
            &[],
        ) || !fields(
            c,
            &[
                ("language", Str),
                ("fixtureCount", Int),
                ("referenceWords", Int),
                ("provenance", Str),
                ("limitation", Str),
            ],
            &["fixtureIds"],
            &[],
        ) || !strings(&c["fixtureIds"])?
            || c["fixtureCount"].as_u64() != Some(array(&c["fixtureIds"], 512)?.len() as u64)
            || c["fixtureIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(Value::to_string)
                .collect::<BTreeSet<_>>()
                .len()
                != c["fixtureIds"].as_array().unwrap().len()
            || !fields(
                conf,
                &[
                    ("vadThreshold", Num),
                    ("executionPath", Str),
                    ("transcriptTransformProfile", Str),
                    ("percentileMethod", Str),
                ],
                &["modelRunOrder", "sharedInitOrder"],
                &[],
            )
            || !strings(&conf["modelRunOrder"])?
            || !strings(&conf["sharedInitOrder"])?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn recognition_complete(r: &Value) -> bool {
    ["expectedRaw", "actualRaw"]
        .iter()
        .all(|k| r[*k].is_string())
        && [
            "rawWordErrors",
            "normalizedWordErrors",
            "referenceWords",
            "normalizedReferenceWords",
            "referenceCharacters",
            "characterErrors",
        ]
        .iter()
        .all(|k| valid(&r[*k], Int))
        && [
            "referenceWords",
            "normalizedReferenceWords",
            "referenceCharacters",
        ]
        .iter()
        .all(|k| r[*k].as_u64().unwrap_or(0) > 0)
        && ["rawWer", "normalizedWer", "cer"]
            .iter()
            .all(|k| valid(&r[*k], Num))
        && r["boundedAlternativeMatch"] == true
}
fn case_complete(c: &Value) -> bool {
    c["status"] == "passed"
        && c["failures"].as_array().is_some_and(|a| a.is_empty())
        && recognition_complete(&c["recognition"])
        && c["transformation"]["actualFinal"].is_string()
        && c["transformation"]["exactMatch"] == true
        && c["transformation"]["commandExactMatch"] != false
        && c["transformation"]["noChangePreserved"] != false
        && c["transformation"]["stages"]
            .as_array()
            .is_some_and(|a| a.iter().all(|s| s["expectationMatch"] == true))
        && c["delivery"]["delivered"].is_string()
        && c["delivery"]["exactMatch"] == true
        && c["delivery"]["attempts"].as_u64().unwrap_or(0) > 0
}
fn evaluation(v: &Value) -> Result<bool, ImportError> {
    if !fields(
        v,
        &[
            ("reportVersion", Int),
            ("fixtureVersion", Int),
            ("generatedAt", Str),
            ("tier", Str),
        ],
        &["privacy", "environment", "summary", "cases"],
        &[],
    ) || !date(&v["generatedAt"])
        || !["deterministic", "hardware"].contains(&v["tier"].as_str().unwrap_or(""))
    {
        return Ok(false);
    }
    let p = &v["privacy"];
    let e = &v["environment"];
    let s = &v["summary"];
    if !fields(
        p,
        &[
            ("localOnly", Bool),
            ("historyIngestion", Bool),
            ("networkUsed", Bool),
            ("systemClipboardUsed", Bool),
            ("fixtureProvenanceRequired", Bool),
        ],
        &[],
        &[],
    ) || p["localOnly"] != true
        || p["historyIngestion"] != false
        || p["networkUsed"] != false
        || p["systemClipboardUsed"] != false
        || p["fixtureProvenanceRequired"] != true
        || !fields(
            e,
            &[
                ("appVersion", Str),
                ("os", Str),
                ("arch", Str),
                ("machineLabel", Str),
                ("logicalCpus", NullableInt),
            ],
            &[],
            &[],
        )
        || !fields(
            s,
            &[
                ("total", Int),
                ("passed", Int),
                ("failed", Int),
                ("skipped", Int),
                ("aggregateRawWer", NullableNum),
                ("aggregateNormalizedWer", NullableNum),
                ("aggregateCer", NullableNum),
                ("transformationMatchRate", NullableNum),
                ("commandExactMatchRate", NullableNum),
                ("noChangePreservationRate", NullableNum),
                ("deliveryMatchRate", NullableNum),
            ],
            &[],
            &[],
        )
    {
        return Ok(false);
    }
    let cases = array(&v["cases"], 10000)?;
    if !unique(cases.iter(), "id")
        || s["total"].as_u64() != Some(cases.len() as u64)
        || s["total"].as_f64()
            != Some(
                s["passed"].as_f64().unwrap()
                    + s["failed"].as_f64().unwrap()
                    + s["skipped"].as_f64().unwrap(),
            )
    {
        return Ok(false);
    }
    if s["passed"].as_u64().unwrap_or(0) > 0
        && ![
            "aggregateRawWer",
            "aggregateNormalizedWer",
            "aggregateCer",
            "transformationMatchRate",
            "deliveryMatchRate",
        ]
        .iter()
        .all(|k| valid(&s[*k], Num))
    {
        return Ok(false);
    }
    for c in cases {
        if !fields(
            c,
            &[("id", Str), ("status", Str)],
            &[
                "failures",
                "provenance",
                "context",
                "model",
                "recognition",
                "transformation",
                "delivery",
                "latency",
                "runtime",
            ],
            &[],
        ) || !["passed", "failed", "skipped"].contains(&c["status"].as_str().unwrap_or(""))
            || !strings(&c["failures"])?
        {
            return Ok(false);
        }
        let p = &c["provenance"];
        let ctx = &c["context"];
        let m = &c["model"];
        let r = &c["recognition"];
        let t = &c["transformation"];
        let d = &c["delivery"];
        let l = &c["latency"];
        let rt = &c["runtime"];
        if !fields(
            p,
            &[
                ("kind", Str),
                ("source", Str),
                ("containsRealUserData", Bool),
                ("deletion", Str),
            ],
            &[],
            &[],
        ) || p["containsRealUserData"] != false
            || !fields(
                ctx,
                &[
                    ("bundleId", NullableStr),
                    ("matchedProfile", NullableStr),
                    ("fixtureOnly", Bool),
                ],
                &[],
                &[],
            )
            || (v["tier"] == "deterministic") != m.is_null()
            || (!m.is_null()
                && (!fields(
                    m,
                    &[
                        ("name", Str),
                        ("backend", Str),
                        ("accelerator", Str),
                        ("audioPath", Str),
                        ("sampleRateHz", Int),
                        ("channels", Int),
                    ],
                    &[],
                    &[],
                ) || m["sampleRateHz"].as_u64().unwrap_or(0) == 0
                    || m["channels"].as_u64().unwrap_or(0) == 0))
            || !fields(
                r,
                &[
                    ("expectedRaw", NullableStr),
                    ("actualRaw", NullableStr),
                    ("rawWordErrors", NullableInt),
                    ("normalizedWordErrors", NullableInt),
                    ("referenceWords", NullableInt),
                    ("normalizedReferenceWords", NullableInt),
                    ("referenceCharacters", NullableInt),
                    ("characterErrors", NullableInt),
                    ("rawWer", NullableNum),
                    ("normalizedWer", NullableNum),
                    ("cer", NullableNum),
                    ("boundedAlternativeMatch", Bool),
                ],
                &[],
                &[],
            )
            || !fields(
                t,
                &[
                    ("expectedFinal", Str),
                    ("actualFinal", NullableStr),
                    ("exactMatch", Bool),
                    ("commandExactMatch", NullableBool),
                    ("noChangePreserved", NullableBool),
                ],
                &["stages"],
                &[],
            )
            || !fields(
                d,
                &[
                    ("expected", Str),
                    ("delivered", NullableStr),
                    ("exactMatch", Bool),
                    ("attempts", Int),
                    ("partialCount", Int),
                    ("firstPartialMs", NullableInt),
                    ("firstPartialApplicability", Str),
                    ("finalOnly", Bool),
                ],
                &[],
                &[],
            )
            || !fields(
                l,
                &[
                    ("rawAsrMs", Int),
                    ("transformationMs", Int),
                    ("finalizationMs", Int),
                    ("deliveryMs", Int),
                    ("totalMs", Int),
                ],
                &[],
                &[],
            )
            || !fields(
                rt,
                &[
                    ("incrementalCompletion", Str),
                    ("fallbackUsed", Bool),
                    ("memoryBeforeMb", NullableInt),
                    ("memoryAfterMb", NullableInt),
                    ("memoryDeltaMb", SignedInt),
                ],
                &["fallbackStages"],
                &[],
            )
            || !strings(&rt["fallbackStages"])?
        {
            return Ok(false);
        }
        for stage in array(&t["stages"], 128)? {
            if !fields(
                stage,
                &[
                    ("name", Str),
                    ("outcome", Str),
                    ("changed", Bool),
                    ("durationUs", Int),
                    ("text", Str),
                    ("expectationMatch", Bool),
                ],
                &[],
                &[],
            ) {
                return Ok(false);
            }
        }
        if c["status"] == "passed" {
            if !case_complete(c) {
                return Ok(false);
            }
        } else if c["failures"].as_array().unwrap().is_empty() {
            return Ok(false);
        }
    }
    Ok(true)
}
fn remove(v: &mut Value, keys: &[&str]) {
    if let Some(o) = v.as_object_mut() {
        for key in keys {
            o.remove(*key);
        }
    }
}
impl Report {
    pub fn parse(bytes: &[u8], declared_bytes: usize, source: Source) -> Result<Self, ImportError> {
        if declared_bytes > MAX_BYTES || bytes.len() > MAX_BYTES {
            return Err(ImportError::Oversized);
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Err(ImportError::Empty);
        }
        let mut v: Value = serde_json::from_slice(bytes).map_err(|_| ImportError::MalformedJson)?;
        if !bounded_strings(&v) {
            return Err(ImportError::StringLimit);
        }
        let kind = if ["createdAt", "results", "recommendations"]
            .iter()
            .any(|k| v.get(*k).is_some())
        {
            Kind::Benchmark
        } else if ["generatedAt", "fixtureVersion", "cases"]
            .iter()
            .any(|k| v.get(*k).is_some())
        {
            Kind::Evaluation
        } else {
            return Err(ImportError::SchemaMismatch);
        };
        let version = v.get("reportVersion");
        if (kind == Kind::Benchmark && version.is_some_and(|v| *v != 2))
            || (kind == Kind::Evaluation
                && (version.is_some_and(|v| *v != 1)
                    || v.get("fixtureVersion").is_some_and(|v| *v != 1)))
        {
            return Err(ImportError::UnsupportedVersion);
        }
        if !match kind {
            Kind::Benchmark => benchmark(&v)?,
            Kind::Evaluation => evaluation(&v)?,
        } {
            return Err(ImportError::SchemaMismatch);
        }
        match kind {
            Kind::Benchmark => {
                remove(&mut v["corpus"], &["provenance", "limitation"]);
                for m in v["results"].as_array_mut().unwrap() {
                    let succeeded = m["error"].is_null() && complete_model(m);
                    remove(m, &["error"]);
                    m["succeeded"] = json!(succeeded);
                    for f in m["fixtures"].as_array_mut().unwrap() {
                        remove(
                            f,
                            &[
                                "label",
                                "reference",
                                "transcript",
                                "deliveredTranscript",
                                "wordErrors",
                                "referenceWords",
                                "normalizedWordErrors",
                                "normalizedReferenceWords",
                                "deliveredWordErrors",
                                "deliveredNormalizedWordErrors",
                            ],
                        );
                    }
                }
            }
            Kind::Evaluation => {
                remove(&mut v, &["privacy"]);
                for c in v["cases"].as_array_mut().unwrap() {
                    let complete = case_complete(c);
                    let fixture_only = c["context"]["fixtureOnly"].clone();
                    remove(c, &["failures", "provenance", "context"]);
                    c["complete"] = json!(complete);
                    c["fixtureOnly"] = fixture_only;
                    remove(&mut c["model"], &["audioPath", "sampleRateHz", "channels"]);
                    remove(&mut c["recognition"], &["expectedRaw", "actualRaw"]);
                    remove(&mut c["transformation"], &["expectedFinal", "actualFinal"]);
                    for s in c["transformation"]["stages"].as_array_mut().unwrap() {
                        remove(s, &["text"]);
                    }
                    remove(&mut c["delivery"], &["expected", "delivered"]);
                }
            }
        }
        // Content identity is independent of list ordering and re-parsing. Only
        // normalized, content-free data enters the digest; source separates a
        // saved report from a session import of the same report.
        let id = format!(
            "{:?}:{:x}",
            source,
            Sha256::digest(v.to_string().as_bytes())
        );
        Ok(Self {
            kind,
            source,
            id,
            data: Arc::new(v),
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    #[cfg(test)]
    pub(crate) fn normalized_for_test(&self) -> String {
        self.data.to_string()
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.data.to_string().len() + 64
    }
    pub fn title(&self) -> String {
        format!(
            "{} · {} · {}",
            match self.kind {
                Kind::Benchmark => "Benchmark",
                Kind::Evaluation => "Evaluation",
            },
            self.data[if self.kind == Kind::Benchmark {
                "createdAt"
            } else {
                "generatedAt"
            }]
            .as_str()
            .unwrap_or(""),
            match self.source {
                Source::Imported => "Imported",
                Source::Local => "Local",
            }
        )
    }
    pub fn metadata(&self) -> Vec<(String, String)> {
        let d = &self.data;
        let keys = if self.kind == Kind::Benchmark {
            vec![
                "reportVersion",
                "appVersion",
                "platform",
                "preset",
                "iterations",
                "environment",
                "corpus",
                "configuration",
            ]
        } else {
            vec![
                "reportVersion",
                "fixtureVersion",
                "tier",
                "environment",
                "summary",
            ]
        };
        keys.into_iter()
            .map(|k| {
                (
                    k.into(),
                    if d[k].is_null() {
                        "Legacy / unavailable".into()
                    } else {
                        d[k].to_string()
                    },
                )
            })
            .collect()
    }
    pub fn privacy_notice(&self) -> &'static str {
        match self.kind {
            Kind::Benchmark if self.data.get("reportVersion").is_none() => {
                "Legacy context is incomplete; comparison is blocked."
            }
            Kind::Benchmark => {
                "Transcript and fixture text are stripped. Imports stay in this session."
            }
            Kind::Evaluation => {
                "Fixture text, failures, provenance, context and paths are stripped. Imports stay in this session."
            }
        }
    }
}
#[derive(Clone)]
pub struct Finding {
    pub blocker: bool,
    pub code: &'static str,
    pub message: &'static str,
}
#[derive(Clone)]
pub struct Metric {
    pub scope: Arc<str>,
    pub label: &'static str,
    pub unit: &'static str,
    pub baseline: f64,
    pub candidate: f64,
    pub delta: f64,
    pub percent: Option<f64>,
}
pub struct Comparison {
    pub findings: Vec<Finding>,
    pub metrics: Vec<Metric>,
    pub recommendations: Option<Value>,
}
impl Comparison {
    pub fn blocked(&self) -> bool {
        self.findings.iter().any(|f| f.blocker)
    }
    pub fn status(&self) -> &'static str {
        if self.blocked() {
            "Blocked"
        } else if !self.findings.is_empty() {
            "Warning"
        } else {
            "Compatible"
        }
    }
}
fn identity(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| v[*k].to_string())
        .collect::<Vec<_>>()
        .join("\0")
}
fn set(v: &Value, key: &str) -> BTreeSet<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v[key].to_string())
        .collect()
}
fn add_metric(
    metrics: &mut Vec<Metric>,
    scope: &Arc<str>,
    label: &'static str,
    a: &Value,
    b: &Value,
) {
    let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) else {
        return;
    };
    let unit = if label.ends_with("Ms") {
        "ms"
    } else if label.ends_with("Us") {
        "µs"
    } else if label.ends_with("Mb") {
        "MB"
    } else {
        ""
    };
    metrics.push(Metric {
        scope: scope.clone(),
        label,
        unit,
        baseline: a,
        candidate: b,
        delta: b - a,
        percent: if a == 0. {
            None
        } else {
            Some((b - a) / a.abs() * 100.)
        },
    });
}
// Conservative upper bounds, checked before compatibility's matching loops or
// metric allocation. Counts include unavailable metrics. Shared scopes still
// have a separate byte budget so long legal identities cannot amplify memory.
fn comparison_bounded(v: &Value) -> bool {
    let (mut metrics, mut scopes, mut work) = (0usize, 0usize, 0usize);
    if let Some(models) = v["results"].as_array() {
        metrics = 1;
        work += models.len() * models.len();
        for m in models {
            let scope = identity(m, &["modelName", "backend", "accelerator"]);
            let fixtures = m["fixtures"].as_array().unwrap();
            metrics += MODEL_METRICS.len() + 1 + fixtures.len() * (MODEL_METRICS.len() - 2);
            work += fixtures.len() * fixtures.len();
            scopes += scope.len() + 16;
            for f in fixtures {
                scopes += scope.len() + 16 + f["fixtureId"].to_string().len();
            }
        }
    } else {
        let cases = v["cases"].as_array().unwrap();
        metrics += 10;
        work += cases.len() * cases.len();
        for c in cases {
            let scope = c["id"].as_str().unwrap();
            let stages = c["transformation"]["stages"].as_array().unwrap();
            metrics += 9 + stages.len();
            scopes += scope.len();
            for stage in stages {
                scopes += scope.len() + 8 + stage["name"].to_string().len();
            }
        }
    }
    metrics <= MAX_COMPARISON_METRICS
        && scopes <= MAX_COMPARISON_SCOPE_BYTES
        && work <= MAX_COMPARISON_WORK
}
fn same_identity(a: &Value, b: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|k| a[*k] == b[*k])
}
pub fn compare(a: &Report, b: &Report) -> Comparison {
    let mut out = Comparison {
        findings: vec![],
        metrics: vec![],
        recommendations: None,
    };
    let mut issue = |blocker, code, message| {
        if !out.findings.iter().any(|f| f.code == code) {
            out.findings.push(Finding {
                blocker,
                code,
                message,
            });
        }
    };
    if a.kind != b.kind {
        issue(
            true,
            "report_type_mismatch",
            "Benchmark and evaluation metrics have different semantics.",
        );
        return out;
    }
    let (a, b) = (&*a.data, &*b.data);
    if !comparison_bounded(a) || !comparison_bounded(b) {
        issue(
            true,
            "comparison_limit",
            "Reports exceed the bounded comparison budget. Choose smaller reports.",
        );
        return out;
    }
    if a.get("reportVersion") != b.get("reportVersion")
        || a.get("fixtureVersion") != b.get("fixtureVersion")
    {
        issue(true, "schema_mismatch", "Report schemas differ.");
    }
    if a.get("results").is_some() {
        if a.get("reportVersion").is_none() || b.get("reportVersion").is_none() {
            issue(
                true,
                "legacy_context_missing",
                "Legacy metadata cannot prove a like-for-like comparison.",
            );
        }
        for (key, code, msg) in [
            ("preset", "preset_mismatch", "Benchmark presets differ."),
            (
                "iterations",
                "iteration_mismatch",
                "Measured iteration counts differ.",
            ),
            (
                "corpus",
                "corpus_mismatch",
                "Corpus identities or reference counts differ.",
            ),
            (
                "configuration",
                "configuration_mismatch",
                "Execution, VAD, transform, percentile or order differs.",
            ),
        ] {
            if a[key] != b[key] {
                issue(true, code, msg);
            }
        }
        let left = a["results"].as_array().unwrap();
        let right = b["results"].as_array().unwrap();
        let keys = ["modelName", "backend", "accelerator"];
        if left
            .iter()
            .map(|v| identity(v, &keys))
            .collect::<BTreeSet<_>>()
            != right
                .iter()
                .map(|v| identity(v, &keys))
                .collect::<BTreeSet<_>>()
        {
            issue(
                true,
                "model_set_mismatch",
                "Model, backend or accelerator identities differ.",
            );
        }
        for l in left {
            if let Some(r) = right.iter().find(|r| same_identity(l, r, &keys)) {
                if l["succeeded"] != true || r["succeeded"] != true {
                    issue(
                        true,
                        "benchmark_result_incomplete",
                        "A benchmark model result is incomplete or failed.",
                    );
                }
                if set(&l["fixtures"], "fixtureId") != set(&r["fixtures"], "fixtureId") {
                    issue(
                        true,
                        "corpus_mismatch",
                        "Per-model fixture identities differ.",
                    );
                }
            }
        }
        if a["appVersion"] != b["appVersion"] {
            issue(
                false,
                "app_version_mismatch",
                "Murmur versions differ; implementation changes may affect results.",
            );
        }
        if a["platform"] != b["platform"] || a["environment"] != b["environment"] {
            issue(
                false,
                "machine_mismatch",
                "Machine or operating-system metadata differs.",
            );
        }
    } else {
        if a["tier"] != b["tier"] {
            issue(
                true,
                "evaluation_tier_mismatch",
                "Deterministic and hardware tiers cannot be compared.",
            );
        }
        let left = a["cases"].as_array().unwrap();
        let right = b["cases"].as_array().unwrap();
        if left
            .iter()
            .chain(right)
            .any(|v| v["complete"] != true || v["status"] != "passed")
        {
            issue(
                true,
                "evaluation_result_incomplete",
                "An evaluation result is incomplete, failed or skipped.",
            );
        }
        if set(&a["cases"], "id") != set(&b["cases"], "id") {
            issue(
                true,
                "fixture_set_mismatch",
                "Evaluation fixture IDs differ.",
            );
        }
        for l in left {
            if let Some(r) = right.iter().find(|r| l["id"] == r["id"]) {
                if l["model"] != r["model"] {
                    issue(
                        true,
                        "evaluation_model_mismatch",
                        "Evaluation model, backend or accelerator differs.",
                    );
                }
                let signature = |v: &Value| {
                    json!([
                        v["fixtureOnly"],
                        v["transformation"]["stages"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|s| s["name"].clone())
                            .collect::<Vec<_>>(),
                        v["delivery"]["firstPartialApplicability"],
                        v["delivery"]["finalOnly"],
                        v["runtime"]["incrementalCompletion"]
                    ])
                };
                if signature(l) != signature(r) {
                    issue(
                        true,
                        "evaluation_execution_mismatch",
                        "Evaluation execution or stage semantics differ.",
                    );
                }
            }
        }
        if a["environment"]["appVersion"] != b["environment"]["appVersion"] {
            issue(
                false,
                "app_version_mismatch",
                "Murmur versions differ; implementation changes may affect results.",
            );
        }
        if ["os", "arch", "machineLabel", "logicalCpus"]
            .iter()
            .any(|k| a["environment"][*k] != b["environment"][*k])
        {
            issue(
                false,
                "machine_mismatch",
                "Machine or operating-system metadata differs.",
            );
        }
    }
    if out.blocked() {
        return out;
    }
    if a.get("results").is_some() {
        add_metric(
            &mut out.metrics,
            &Arc::from("report"),
            "sharedInitMs",
            &a["sharedInitMs"],
            &b["sharedInitMs"],
        );
        for l in a["results"].as_array().unwrap() {
            let r = b["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| same_identity(l, r, &["modelName", "backend", "accelerator"]))
                .unwrap();
            let scope: Arc<str> = identity(l, &["modelName", "backend", "accelerator"])
                .replace('\0', " · ")
                .into();
            for key in MODEL_METRICS.iter().chain(["memoryDeltaMb"].iter()) {
                add_metric(&mut out.metrics, &scope, key, &l[*key], &r[*key]);
            }
            for f in l["fixtures"].as_array().unwrap() {
                let rf = r["fixtures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| f["fixtureId"] == r["fixtureId"])
                    .unwrap();
                let fixture_scope: Arc<str> = format!("{scope} · {}", f["fixtureId"]).into();
                for key in &MODEL_METRICS[2..] {
                    add_metric(&mut out.metrics, &fixture_scope, key, &f[*key], &rf[*key]);
                }
            }
        }
        if out.findings.is_empty() {
            out.recommendations = Some(b["recommendations"].clone());
        }
    } else {
        for key in [
            "passed",
            "failed",
            "skipped",
            "aggregateRawWer",
            "aggregateNormalizedWer",
            "aggregateCer",
            "transformationMatchRate",
            "commandExactMatchRate",
            "noChangePreservationRate",
            "deliveryMatchRate",
        ] {
            add_metric(
                &mut out.metrics,
                &Arc::from("summary"),
                key,
                &a["summary"][key],
                &b["summary"][key],
            );
        }
        for l in a["cases"].as_array().unwrap() {
            let r = b["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| l["id"] == r["id"])
                .unwrap();
            let scope: Arc<str> = l["id"].as_str().unwrap().into();
            for (section, keys) in [
                ("recognition", &["rawWer", "normalizedWer", "cer"][..]),
                (
                    "latency",
                    &[
                        "rawAsrMs",
                        "transformationMs",
                        "finalizationMs",
                        "deliveryMs",
                        "totalMs",
                    ][..],
                ),
                ("runtime", &["memoryDeltaMb"][..]),
            ] {
                for key in keys {
                    add_metric(
                        &mut out.metrics,
                        &scope,
                        key,
                        &l[section][*key],
                        &r[section][*key],
                    );
                }
            }
            for (ls, rs) in l["transformation"]["stages"]
                .as_array()
                .unwrap()
                .iter()
                .zip(r["transformation"]["stages"].as_array().unwrap())
            {
                add_metric(
                    &mut out.metrics,
                    &Arc::from(format!("{scope} · {}", ls["name"])),
                    "durationUs",
                    &ls["durationUs"],
                    &rs["durationUs"],
                );
            }
        }
    }
    out
}
