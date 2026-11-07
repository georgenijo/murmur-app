use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use std::cmp::Ordering;

pub const FEED: &str =
    "https://github.com/georgenijo/murmur-app/releases/latest/download/latest-v2.json";
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEE1NUQ4ODNFNUE2MjQ4RDcKUldUWFNHSmFQb2hkcFRWNUpGSXRCZ3pITjFrc3Qvd01aczJSdDVvTEJROWw5bCt0bnRTY3d3SDIK";
pub const MAX_NOTES: usize = 1_000_000;
pub const MAX_FEED: u64 = 16 * 1024 * 1024;
pub const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

pub fn version(s: &str) -> Option<Version> {
    Version::parse(s.trim().strip_prefix('v').unwrap_or(s.trim())).ok()
}
/// The installed policy compares the numeric triple, ignoring prerelease/build.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (version(a)?, version(b)?);
    Some((a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch)))
}
pub fn forced(current: &str, minimum: &Value) -> bool {
    minimum
        .as_str()
        .is_some_and(|min| compare(current, min).is_none_or(|c| c == Ordering::Less))
}
pub fn platform() -> String {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    format!("{os}-{}", std::env::consts::ARCH)
}
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: String,
    pub notes: String,
    pub forced: bool,
    pub policy_verified: bool,
    pub url: String,
    pub signature: String,
    pub discovered: u64,
}
/// Same static feed lookup order as installed Tauri clients: bundle-specific
/// key first, then platform key. A different platform is never a fallback.
pub fn parse_feed(bytes: &[u8], current: &str, platform: &str, now: u64) -> Result<Option<Offer>> {
    let raw: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid update manifest")?;
    let target = raw["version"].as_str().ok_or("Missing update version")?;
    let new = version(target).ok_or("Invalid update version")?;
    let old = version(current).ok_or("Invalid installed version")?;
    if new <= old {
        return Ok(None);
    }
    let platforms = raw["platforms"]
        .as_object()
        .ok_or("Missing update platforms")?;
    let key = format!("{platform}-app");
    let artifact = platforms
        .get(&key)
        .or_else(|| platforms.get(platform))
        .ok_or("Update does not support this platform")?;
    let url = artifact["url"].as_str().ok_or("Missing update URL")?;
    let signature = artifact["signature"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing update signature")?;
    let latest = raw["notes"].as_str().unwrap_or("");
    Ok(Some(Offer {
        version: new.to_string(),
        notes: release_notes(&raw, current, target, latest),
        forced: forced(current, &raw["min_version"]),
        policy_verified: raw.get("min_version").is_none() || raw["min_version"].is_string(),
        url: url.into(),
        signature: signature.into(),
        discovered: now,
    }))
}
fn stable(s: &str) -> bool {
    version(s).is_some_and(|v| v.pre.is_empty() && v.build.is_empty())
}
fn release_notes(raw: &Value, current: &str, target: &str, latest: &str) -> String {
    let mut history = std::collections::BTreeMap::new();
    if stable(current) && stable(target) && compare(current, target) == Some(Ordering::Less) {
        for item in raw["release_history"].as_array().into_iter().flatten() {
            if let (Some(v), Some(notes)) = (item["version"].as_str(), item["notes"].as_str()) {
                if stable(v)
                    && !notes.trim().is_empty()
                    && compare(v, current) == Some(Ordering::Greater)
                    && compare(v, target).is_some_and(|c| c != Ordering::Greater)
                {
                    history.insert(version(v).unwrap(), notes.trim().to_string());
                }
            }
        }
    }
    let target = version(target).unwrap();
    history.insert(target, latest.trim().into());
    let notes = if history.len() <= 1 {
        latest.to_owned()
    } else {
        history
            .into_iter()
            .rev()
            .map(|(v, n)| format!("# v{v}\n\n{n}"))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    };
    bound_notes(&notes)
}
pub fn bound_notes(s: &str) -> String {
    s.chars().take(MAX_NOTES).collect()
}
fn decode(s: &str) -> Result<String> {
    String::from_utf8(
        STANDARD
            .decode(s.trim())
            .map_err(|_| "Invalid signature encoding")?,
    )
    .map_err(|_| "Invalid signature text".into())
}
pub fn verify(bytes: &[u8], signature: &str, public_key: &str) -> Result<()> {
    let key = PublicKey::decode(&decode(public_key)?).map_err(|_| "Invalid updater key")?;
    let sig = Signature::decode(&decode(signature)?).map_err(|_| "Invalid update signature")?;
    // true matches installed plugin clients, accepting legacy Ed and prehashed ED.
    key.verify(bytes, &sig, true)
        .map_err(|_| "Update signature verification failed".into())
}
pub fn translocated(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == "AppTranslocation")
}
pub fn normalize_pending(v: Value) -> crate::completion::Result<Value> {
    if v.is_null() {
        return Ok(v);
    }
    let version = v["version"]
        .as_str()
        .filter(|s| version(s).is_some())
        .ok_or(crate::completion::Error::Invalid)?;
    let notes = v["notes"]
        .as_str()
        .ok_or(crate::completion::Error::Invalid)?;
    Ok(json!({"version":version,"notes":bound_notes(notes)}))
}
pub fn pending_for(v: &Value, current: &str) -> Option<CompletedUpdate> {
    let pending = normalize_pending(v.clone()).ok()?;
    let stored = pending["version"].as_str()?;
    // Preserve build and prerelease identity, unlike minimum-policy comparison.
    if stored.trim().trim_start_matches('v') != current.trim().trim_start_matches('v') {
        return None;
    }
    Some(CompletedUpdate {
        version: stored.into(),
        notes: pending["notes"].as_str()?.into(),
    })
}
