//! Explicit, credential-free Open VSX discovery and read-only theme import.
use super::{archive::Archive, import};
use crate::section::Problem;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    time::Duration,
};
pub const LICENSES: [&str; 9] = [
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unlicense",
];
pub fn trusted(raw: &str) -> Result<url::Url, Problem> {
    let u = url::Url::parse(raw).map_err(|_| Problem::Invalid)?;
    if u.scheme() != "https"
        || u.host_str() != Some("open-vsx.org")
        || u.port().is_some_and(|p| p != 443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err(Problem::Invalid);
    }
    Ok(u)
}
pub trait Fetch {
    fn get(&self, url: &str, max: usize, current: &dyn Fn() -> bool) -> Result<Vec<u8>, Problem>;
}
pub struct Network {
    client: reqwest::blocking::Client,
}
impl Network {
    pub fn new() -> Result<Self, Problem> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .map_err(|_| Problem::Unavailable)?,
        })
    }
}
impl Fetch for Network {
    fn get(&self, url: &str, max: usize, current: &dyn Fn() -> bool) -> Result<Vec<u8>, Problem> {
        if !current() {
            return Err(Problem::Conflict);
        }
        let url = trusted(url)?;
        let mut response = self
            .client
            .get(url)
            .send()
            .map_err(|_| Problem::Unavailable)?;
        if !current() {
            return Err(Problem::Conflict);
        }
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n > max as u64)
        {
            return Err(Problem::Invalid);
        }
        let mut out = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            if !current() {
                return Err(Problem::Conflict);
            }
            let n = response
                .read(&mut chunk)
                .map_err(|_| Problem::Unavailable)?;
            if n == 0 {
                break;
            }
            if out.len() + n > max {
                return Err(Problem::Invalid);
            }
            out.extend_from_slice(&chunk[..n]);
        }
        if !current() {
            return Err(Problem::Conflict);
        }
        Ok(out)
    }
}
#[derive(Clone, Debug)]
pub struct Extension {
    pub id: String,
    pub publisher: String,
    pub name: String,
    pub label: String,
    pub version: String,
    pub license: String,
    pub manifest: String,
    pub checksum: String,
    pub archive: String,
    pub source: Option<String>,
}
impl Extension {
    pub fn valid(&self) -> bool {
        identifier(&self.publisher)
            && identifier(&self.name)
            && self
                .id
                .eq_ignore_ascii_case(&format!("{}.{}", self.publisher, self.name))
            && self.id.len() <= 128
            && !self.version.is_empty()
            && self.version.len() <= 64
            && self.label.chars().count() <= 64
            && LICENSES.contains(&self.license.as_str())
            && [&self.manifest, &self.checksum, &self.archive]
                .iter()
                .all(|s| s.len() <= 4096 && trusted(s).is_ok())
            && self.source.as_ref().is_none_or(|s| {
                s.len() <= 2048
                    && url::Url::parse(s).is_ok_and(|u| {
                        u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
                    })
            })
    }
}
fn short(raw: &str) -> String {
    Sha256::digest(raw.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn collection(id: &str) -> String {
    let s = format!("open-vsx:{}", id.to_lowercase());
    if s.len() <= 128 {
        s
    } else {
        format!("open-vsx:{}", short(id))
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn json(bytes: Vec<u8>, max: usize) -> Result<Value, Problem> {
    crate::json::jsonc(
        std::str::from_utf8(&bytes).map_err(|_| Problem::Invalid)?,
        max,
    )
}
fn detail(v: &Value) -> Result<Option<Extension>, Problem> {
    let s = |k| v[k].as_str().ok_or(Problem::Invalid);
    let publisher = s("namespace")?;
    let name = s("name")?;
    let version = s("version")?;
    let license = s("license")?;
    if !identifier(publisher) || !identifier(name) || version.is_empty() || version.len() > 64 {
        return Err(Problem::Invalid);
    }
    if !LICENSES.contains(&license) {
        return Ok(None);
    }
    let file = |k| {
        let s = v["files"][k].as_str().ok_or(Problem::Invalid)?;
        trusted(s)?;
        Ok(s.to_string())
    };
    let source = [
        v["repository"]["url"].as_str(),
        v["repository"].as_str(),
        v["homepage"].as_str(),
        v["url"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find(|s| {
        s.len() <= 2048
            && url::Url::parse(s).is_ok_and(|u| {
                u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
            })
    })
    .map(String::from);
    Ok(Some(Extension {
        id: format!("{publisher}.{name}"),
        publisher: publisher.into(),
        name: name.into(),
        label: v["displayName"]
            .as_str()
            .unwrap_or(name)
            .chars()
            .take(64)
            .collect(),
        version: version.into(),
        license: license.into(),
        manifest: file("manifest")?,
        checksum: file("sha256")?,
        archive: file("download")?,
        source,
    }))
}
fn contributions(v: &Value) -> Result<&Vec<Value>, Problem> {
    v["contributes"]["themes"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 40)
        .ok_or(Problem::Invalid)
}
fn identity(v: &Value, e: &Extension) -> bool {
    v["publisher"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case(&e.publisher))
        && v["name"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(&e.name))
        && v["version"] == e.version
        && v["license"]
            .as_str()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case(&e.license))
}
pub fn search(
    fetch: &dyn Fetch,
    query: &str,
    sort: &str,
    current: &dyn Fn() -> bool,
) -> Result<Vec<Extension>, Problem> {
    if query.len() > 256 || !["downloadCount", "rating", "timestamp", "relevance"].contains(&sort) {
        return Err(Problem::Invalid);
    }
    let mut u = trusted("https://open-vsx.org/api/-/search")?;
    u.query_pairs_mut()
        .append_pair("query", query.trim())
        .append_pair("category", "Themes")
        .append_pair("size", "16")
        .append_pair("sortBy", sort);
    let result = json(fetch.get(u.as_str(), 512 * 1024, current)?, 512 * 1024)?;
    let results = result["extensions"]
        .as_array()
        .filter(|a| a.len() <= 16)
        .ok_or(Problem::Invalid)?;
    let mut out = vec![];
    let mut completed = 0;
    for entry in results {
        if !current() {
            return Err(Problem::Conflict);
        }
        let publisher = entry["namespace"].as_str().unwrap_or("");
        let name = entry["name"].as_str().unwrap_or("");
        if !identifier(publisher) || !identifier(name) {
            continue;
        }
        let mut u = trusted("https://open-vsx.org/api/")?;
        u.path_segments_mut()
            .map_err(|_| Problem::Invalid)?
            .pop_if_empty()
            .push(publisher)
            .push(name);
        let result: Result<Option<Extension>, Problem> = (|| {
            let details = json(fetch.get(u.as_str(), 256 * 1024, current)?, 256 * 1024)?;
            let Some(e) = detail(&details)? else {
                return Ok(None);
            };
            let manifest = json(fetch.get(&e.manifest, 256 * 1024, current)?, 256 * 1024)?;
            if !identity(&manifest, &e) || contributions(&manifest).is_err() {
                return Ok(None);
            }
            Ok(Some(e))
        })();
        if result.is_ok() {
            completed += 1;
        }
        if let Ok(Some(e)) = result {
            out.push(e)
        }
        if out.len() == 8 {
            break;
        }
    }
    if !current() {
        return Err(Problem::Conflict);
    }
    if !results.is_empty() && completed == 0 {
        return Err(Problem::Unavailable);
    }
    Ok(out)
}
fn path(raw: &str, parent: &str) -> Result<String, Problem> {
    if raw.len() > 1024 || raw.is_empty() || raw.starts_with('/') || raw.contains(['\\', ':', '\0'])
    {
        return Err(Problem::Invalid);
    }
    let mut parts: Vec<_> = parent.split('/').collect();
    parts.pop();
    for p in raw.split('/') {
        match p {
            "" | "." => (),
            ".." => {
                if parts.len() <= 1 {
                    return Err(Problem::Invalid);
                }
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    if parts.first() != Some(&"extension") {
        return Err(Problem::Invalid);
    }
    let path = parts.join("/");
    if !super::archive::safe_path(&path) {
        return Err(Problem::Invalid);
    }
    Ok(path)
}
fn resolve(
    archive: &Archive<'_>,
    name: &str,
    depth: usize,
    visiting: &mut BTreeSet<String>,
    count: &mut usize,
) -> Result<Value, Problem> {
    if depth >= 8 || *count >= 320 || !visiting.insert(name.into()) {
        return Err(Problem::Invalid);
    }
    *count += 1;
    let mut value = json(archive.read(name, 256 * 1024)?, 256 * 1024)?;
    if !value.is_object() {
        return Err(Problem::Invalid);
    }
    if let Some(include) = value.get("include") {
        let include = include.as_str().ok_or(Problem::Invalid)?;
        let parent = resolve(archive, &path(include, name)?, depth + 1, visiting, count)?;
        let mut colors = parent["colors"].as_object().cloned().unwrap_or_default();
        if let Some(own) = value["colors"].as_object() {
            for (k, v) in own {
                colors.insert(k.clone(), v.clone());
            }
        }
        value["colors"] = Value::Object(colors);
    }
    visiting.remove(name);
    Ok(value)
}
pub fn download(
    fetch: &dyn Fetch,
    e: &Extension,
    current: &dyn Fn() -> bool,
) -> Result<Vec<Value>, Problem> {
    if !e.valid() {
        return Err(Problem::Invalid);
    }
    let checksum = fetch.get(&e.checksum, 1024, current)?;
    let checksum = std::str::from_utf8(&checksum)
        .map_err(|_| Problem::Invalid)?
        .split_whitespace()
        .next()
        .ok_or(Problem::Invalid)?;
    if checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Problem::Invalid);
    }
    let bytes = fetch.get(&e.archive, 20 * 1024 * 1024, current)?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(checksum) {
        return Err(Problem::Invalid);
    }
    let archive = Archive::new(&bytes)?;
    let manifest = json(
        archive.read("extension/package.json", 256 * 1024)?,
        256 * 1024,
    )?;
    if !identity(&manifest, e) {
        return Err(Problem::Invalid);
    }
    let mut count = 0;
    let mut converted = vec![];
    for contribution in contributions(&manifest)? {
        if !current() {
            return Err(Problem::Conflict);
        }
        let source = path(
            contribution["path"].as_str().ok_or(Problem::Invalid)?,
            "extension/package.json",
        )?;
        let mut v = resolve(&archive, &source, 0, &mut BTreeSet::new(), &mut count)?;
        let appearance = match contribution["uiTheme"].as_str() {
            Some("vs" | "hc-light") => "light",
            Some("vs-dark" | "hc-black") => "dark",
            _ => return Err(Problem::Invalid),
        };
        v["type"] = json!(appearance);
        if let Some(label) = contribution["label"].as_str() {
            v["name"] = json!(label.chars().take(64).collect::<String>());
        }
        let (label, dark, theme) = import::vscode(&v)?;
        converted.push((label, dark, theme, source));
    }
    // Pair corresponding light/dark names, preserving deterministic source identity.
    let stem = |label: &str| {
        label
            .split_whitespace()
            .filter(|w| !["light", "dark"].contains(&w.to_lowercase().as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for item in converted {
        groups
            .entry(stem(&item.0).to_lowercase())
            .or_default()
            .push(item);
    }
    let mut out = vec![];
    for (_, items) in groups {
        let pairs: Vec<Vec<(String, bool, Value, String)>> =
            if items.len() == 2 && items[0].1 != items[1].1 {
                vec![items]
            } else {
                items.into_iter().map(|item| vec![item]).collect()
            };
        for pair in pairs {
            let mut paths: Vec<_> = pair.iter().map(|i| i.3.clone()).collect();
            paths.sort();
            let id = format!("ovx-{}", short(&format!("{}:{}", e.id, paths.join(":"))));
            let mut theme = json!({"version":1,"presetId":"custom"});
            let mut modes = vec![];
            for (_, dark, t, _) in &pair {
                let mode = if *dark { "dark" } else { "light" };
                theme[mode] = super::compile::value(&super::compile::resolve(t, *dark));
                modes.push(mode);
            }
            modes.sort_by_key(|m| if *m == "light" { 0 } else { 1 });
            let label = if pair.len() == 2 {
                stem(&pair[0].0)
            } else {
                pair[0].0.clone()
            };
            let mut source = json!({"kind":"open-vsx","extensionId":e.id,"version":e.version,"license":e.license});
            if let Some(url) = &e.source {
                source["sourceUrl"] = json!(url)
            }
            out.push(json!({"version":1,"id":id,"label":if label.trim().is_empty(){e.label.clone()}else{label},"modes":modes,"theme":theme,"source":source,"collection":{"id":collection(&e.id),"label":e.label}}));
        }
    }
    if !current() {
        return Err(Problem::Conflict);
    }
    Ok(out)
}
