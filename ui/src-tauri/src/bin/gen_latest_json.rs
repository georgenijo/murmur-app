/// Generates the `latest.json` update manifest consumed by tauri-plugin-updater.
///
/// Usage (called by the release workflow after tauri-action builds and signs artifacts):
///
///   gen_latest_json <version> <pub_date> <signature> <url> <notes>
///
/// All fields are required; pass an empty string for notes if there are none.
/// Output is written to stdout so the caller can redirect it to a file.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!(
            "Usage: gen_latest_json <version> <pub_date> <signature> <url> <notes>\n\
             Example:\n  gen_latest_json 0.4.0 2026-01-01T00:00:00Z dW50cnVzdGVk \\\n\
             https://github.com/owner/repo/releases/download/v0.4.0/App.app.tar.gz \\\n\
             'Bug fixes' > latest.json"
        );
        std::process::exit(1);
    }

    let version = &args[1];
    let pub_date = &args[2];
    let signature = &args[3];
    let url = &args[4];
    let notes = &args[5];

    println!("{}", make_latest_json(version, pub_date, signature, url, notes));
}

/// Builds the Tauri 2 update manifest JSON string.
///
/// `version` must NOT include a leading `v` — Tauri compares it against the
/// semver in `tauri.conf.json` and a prefix causes a permanent version mismatch.
pub fn make_latest_json(
    version: &str,
    pub_date: &str,
    signature: &str,
    url: &str,
    notes: &str,
) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": {
            "darwin-aarch64": {
                "signature": signature,
                "url": url
            }
        }
    }))
    .expect("serde_json serialization is infallible for this input")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FAKE_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHNpZ25hdHVyZQ==";
    const FAKE_URL: &str = "https://github.com/georgenijo/murmur-app/releases/download/v0.4.0/Local%20Dictation.app.tar.gz";

    fn parse(version: &str) -> Value {
        let raw = make_latest_json(version, "2026-01-01T00:00:00Z", FAKE_SIG, FAKE_URL, "notes");
        serde_json::from_str(&raw).expect("make_latest_json must produce valid JSON")
    }

    #[test]
    fn output_is_valid_json() {
        let raw = make_latest_json("0.4.0", "2026-01-01T00:00:00Z", FAKE_SIG, FAKE_URL, "");
        assert!(
            serde_json::from_str::<Value>(&raw).is_ok(),
            "output must be valid JSON"
        );
    }

    #[test]
    fn top_level_required_fields_present() {
        let json = parse("0.4.0");
        assert!(json.get("version").is_some(), "missing 'version'");
        assert!(json.get("pub_date").is_some(), "missing 'pub_date'");
        assert!(json.get("platforms").is_some(), "missing 'platforms'");
        assert!(json.get("notes").is_some(), "missing 'notes'");
    }

    #[test]
    fn platform_key_is_darwin_aarch64() {
        // Tauri matches the key against the running platform — wrong key = no updates
        let json = parse("0.4.0");
        assert!(
            json["platforms"]["darwin-aarch64"].is_object(),
            "platform key must be 'darwin-aarch64'"
        );
    }

    #[test]
    fn platform_has_signature_and_url() {
        let json = parse("0.4.0");
        let p = &json["platforms"]["darwin-aarch64"];
        assert!(p["signature"].is_string(), "missing platform 'signature'");
        assert!(p["url"].is_string(), "missing platform 'url'");
    }

    #[test]
    fn version_has_no_v_prefix() {
        // tauri-plugin-updater does semver comparison; a 'v' prefix causes a
        // permanent mismatch — the app always thinks it needs an update.
        let json = parse("0.4.0");
        let version = json["version"].as_str().unwrap();
        assert!(
            !version.starts_with('v'),
            "version must not start with 'v', got: {version}"
        );
    }

    #[test]
    fn version_is_preserved_exactly() {
        let json = parse("1.2.3");
        assert_eq!(json["version"].as_str().unwrap(), "1.2.3");
    }

    #[test]
    fn signature_is_preserved_exactly() {
        let sig = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHNpZ25hdHVyZQ==";
        let raw = make_latest_json("0.4.0", "2026-01-01T00:00:00Z", sig, FAKE_URL, "");
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["platforms"]["darwin-aarch64"]["signature"], sig);
    }

    #[test]
    fn url_is_preserved_exactly() {
        let url = "https://github.com/georgenijo/murmur-app/releases/download/v0.4.0/Local%20Dictation.app.tar.gz";
        let raw = make_latest_json("0.4.0", "2026-01-01T00:00:00Z", FAKE_SIG, url, "");
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["platforms"]["darwin-aarch64"]["url"], url);
    }

    #[test]
    fn notes_are_preserved_exactly() {
        let notes = "Fixes a crash on startup.\n\nSee changelog for details.";
        let raw = make_latest_json("0.4.0", "2026-01-01T00:00:00Z", FAKE_SIG, FAKE_URL, notes);
        let json: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["notes"].as_str().unwrap(), notes);
    }
}
