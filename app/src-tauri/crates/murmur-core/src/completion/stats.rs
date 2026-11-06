use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stats {
    pub total_words: u64,
    pub total_recordings: u64,
    pub total_duration_seconds: f64,
    #[serde(default)]
    pub wpm_samples: Vec<f64>,
    #[serde(default)]
    pub daily_buckets: BTreeMap<String, Day>,
    #[serde(default)]
    pub activity: Activity,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Day {
    pub words: u64,
    pub recordings: u64,
    pub recording_seconds: f64,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Meetings {
    pub count: u64,
    pub total_minutes: f64,
    pub summaries_generated: u64,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub runs: u64,
    pub approved: u64,
    pub undone: u64,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transforms {
    pub runs: u64,
    pub approved: u64,
    pub undone: u64,
    pub by_preset_name: BTreeMap<String, Counts>,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scopes {
    pub global: u64,
    pub app: u64,
    pub project: u64,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Corrections {
    pub proposed: u64,
    pub taught: u64,
    pub by_scope: Scopes,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Month {
    pub runs: u64,
    pub approved: u64,
    pub undone: u64,
    pub meetings: u64,
    pub meeting_minutes: f64,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Activity {
    pub meetings: Meetings,
    pub transforms: Transforms,
    pub corrections: Corrections,
    pub recordings_by_mode: BTreeMap<String, u64>,
    pub paste_last_uses: u64,
    pub monthly_buckets: BTreeMap<String, Month>,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatisticsEvent {
    Dictation {
        words: u64,
        seconds: f64,
        mode: Option<String>,
        day: String,
    },
    Meeting {
        seconds: f64,
        month: String,
    },
    CorrectionProposed,
    CorrectionTaught {
        scope: Scope,
    },
    PasteLast,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    App,
    Project,
}
fn cost(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}
fn date(v: &str, day: bool) -> bool {
    let b = v.as_bytes();
    b.len() == if day { 10 } else { 7 }
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && ("01"..="12").contains(&&v[5..7])
        && (!day || ("01"..="31").contains(&&v[8..10]))
}
impl StatisticsEvent {
    pub fn valid(&self) -> bool {
        match self {
            Self::Dictation { seconds, day, .. } => cost(*seconds) && date(day, true),
            Self::Meeting { seconds, month } => cost(*seconds) && date(month, false),
            _ => true,
        }
    }
}
impl Stats {
    pub fn valid(&self) -> bool {
        cost(self.total_duration_seconds)
            && self.wpm_samples.iter().all(|v| cost(*v))
            && self
                .daily_buckets
                .iter()
                .all(|(k, v)| date(k, true) && cost(v.recording_seconds))
            && cost(self.activity.meetings.total_minutes)
            && self
                .activity
                .monthly_buckets
                .iter()
                .all(|(k, v)| date(k, false) && cost(v.meeting_minutes))
    }
    pub fn apply(&mut self, event: &StatisticsEvent) {
        match event {
            StatisticsEvent::Dictation {
                words,
                seconds,
                mode,
                day,
            } => {
                self.total_words += words;
                self.total_recordings += 1;
                self.total_duration_seconds += seconds;
                if *words > 0 && *seconds > 0.0 {
                    self.wpm_samples.push(*words as f64 / seconds * 60.0);
                }
                if self.wpm_samples.len() > 100 {
                    self.wpm_samples.drain(..self.wpm_samples.len() - 100);
                }
                let bucket = self.daily_buckets.entry(day.clone()).or_default();
                bucket.words += words;
                bucket.recordings += 1;
                bucket.recording_seconds += seconds;
                if let Some(mode) = mode.as_ref().filter(|s| !s.is_empty()) {
                    *self
                        .activity
                        .recordings_by_mode
                        .entry(mode.clone())
                        .or_default() += 1;
                }
            }
            StatisticsEvent::Meeting { seconds, month } => {
                self.activity.meetings.count += 1;
                self.activity.meetings.total_minutes += seconds / 60.0;
                let bucket = self
                    .activity
                    .monthly_buckets
                    .entry(month.clone())
                    .or_default();
                bucket.meetings += 1;
                bucket.meeting_minutes += seconds / 60.0;
            }
            StatisticsEvent::CorrectionProposed => self.activity.corrections.proposed += 1,
            StatisticsEvent::CorrectionTaught { scope } => {
                self.activity.corrections.taught += 1;
                let scopes = &mut self.activity.corrections.by_scope;
                match scope {
                    Scope::Global => scopes.global += 1,
                    Scope::App => scopes.app += 1,
                    Scope::Project => scopes.project += 1,
                }
            }
            StatisticsEvent::PasteLast => self.activity.paste_last_uses += 1,
        }
    }
}
