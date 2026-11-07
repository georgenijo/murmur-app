use crate::{section,workspace::Projection};
use chrono::{Datelike,Days,NaiveDate};
use std::collections::BTreeMap;
pub use crate::workspace::{Event,Problem,Session,Ticket};
#[derive(Clone)]
pub struct Snapshot{pub revision:u64,pub stats:murmur_core::completion::Stats,pub today:NaiveDate,pub mode_names:BTreeMap<String,String>}
#[derive(Clone,Debug,PartialEq)]pub struct Day{pub date:NaiveDate,pub words:u64,pub recordings:u64,pub wpm:f64,pub future:bool}
impl Snapshot{
 pub fn recent(&self,n:usize)->Vec<Day>{(0..n.min(56)).rev().map(|i|self.day(self.today.checked_sub_days(Days::new(i as u64)).unwrap())).collect()}
 pub fn day(&self,date:NaiveDate)->Day{let b=self.stats.daily_buckets.get(&date.to_string()).filter(|_|date<=self.today);Day{date,words:b.map_or(0,|b|b.words),recordings:b.map_or(0,|b|b.recordings),wpm:b.map_or(0.,|b|if b.recording_seconds>0.{(b.words as f64*60./b.recording_seconds).round()}else{0.}),future:date>self.today}}
 pub fn heatmap(&self)->Vec<Day>{let start=self.today.checked_sub_days(Days::new(self.today.weekday().num_days_from_sunday() as u64+49)).unwrap();(0..56).map(|i|self.day(start.checked_add_days(Days::new(i)).unwrap())).collect()}
 pub fn streak(&self)->usize{let mut date=self.today;if self.day(date).recordings==0{date=date.pred_opt().unwrap()};let mut n=0;while self.day(date).recordings>0{n+=1;let Some(prior)=date.pred_opt() else{break};date=prior;}n}
 pub fn totals(&self)->Vec<(&'static str,String,String)>{let s=&self.stats;let a=&s.activity;let month=self.today.format("%Y-%m").to_string();let m=a.monthly_buckets.get(&month);let most=a.recordings_by_mode.iter().filter(|(_,n)|**n>0).min_by(|(ka,a),(kb,b)|b.cmp(a).then(ka.cmp(kb)));let mode=most.map_or("None yet",|(id,_)|self.mode_names.get(id).map_or("Removed mode",String::as_str));let average=if s.wpm_samples.is_empty(){"—".into()}else{format!("{:.0}",s.wpm_samples.iter().sum::<f64>()/s.wpm_samples.len() as f64)};let minutes=s.total_duration_seconds/60.;let recordings_month:u64=s.daily_buckets.iter().filter(|(d,_)|d.starts_with(&month)).map(|(_,b)|b.recordings).sum();vec![
 ("Total words",s.total_words.to_string(),"all time".into()),("Average speed",average,"wpm".into()),("Recordings",s.total_recordings.to_string(),format!("{recordings_month} this month")),("Current streak",self.streak().to_string(),if self.streak()==1{"day"}else{"days"}.into()),("Total dictation time",format!("{minutes:.1} min"),"all time".into()),("Estimated typing time saved",format!("{:.1} min",(s.total_words as f64/40.-minutes).max(0.)),"At 40 typing wpm, minus dictation time".into()),("Most-used mode",mode.into(),format!("{} recordings",most.map_or(0,|(_,n)|*n))),("Meetings this month",m.map_or(0,|m|m.meetings).to_string(),format!("{:.1} minutes",m.map_or(0.,|m|m.meeting_minutes))),("Corrections taught",a.corrections.taught.to_string(),format!("{} proposed",a.corrections.proposed)),("Paste last",a.paste_last_uses.to_string(),"successful uses".into())]}
}
impl Projection for Snapshot{
 fn revision(&self)->u64{self.revision}fn bytes(&self)->usize{serde_json::to_value(&self.stats).map_or(usize::MAX,|v|section::bytes(&v)).saturating_add(self.mode_names.iter().map(|(k,v)|k.len()+v.len()).sum::<usize>())}
 fn valid(&self)->bool{self.stats.valid()&&self.bytes()<=1024*1024&&self.stats.daily_buckets.len()<=10000&&self.mode_names.len()<=107&&self.mode_names.iter().all(|(k,v)|section::text(k,128)&&section::text(v,128))&&self.today.year()>=1971&&self.today.year()<=9998}
}
