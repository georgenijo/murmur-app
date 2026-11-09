use super::{color::*, compile};
use crate::section::Problem;
use serde_json::{Value, json};
pub fn color(raw: &str, base: Color) -> Option<Color> {
    let s = raw.trim();
    let h = s.strip_prefix('#').unwrap_or(s);
    if [3, 4, 6, 8].contains(&h.len()) && h.bytes().all(|c| c.is_ascii_hexdigit()) {
        let full = if h.len() <= 4 {
            h.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            h.into()
        };
        let c = parse(&format!("#{}", &full[..6]))?;
        let alpha = if full.len() == 8 {
            u8::from_str_radix(&full[6..], 16).ok()? as f64 / 255.
        } else {
            1.
        };
        return Some(composite(c, base, alpha));
    }
    let body = s.strip_prefix("color(")?.strip_suffix(')')?;
    let mut pieces = body.split('/');
    let mut words = pieces.next()?.split_whitespace();
    let space = words.next()?;
    let number = |s: &str| {
        s.strip_suffix('%')
            .map_or_else(
                || s.parse::<f64>().ok(),
                |s| s.parse::<f64>().ok().map(|n| n / 100.),
            )
            .filter(|n| n.is_finite())
    };
    let channels: Vec<_> = words.map(number).collect::<Option<_>>()?;
    if channels.len() != 3 {
        return None;
    }
    let alpha = pieces
        .next()
        .map(str::trim)
        .map(number)
        .unwrap_or(Some(1.))?
        .clamp(0., 1.);
    if pieces.next().is_some() {
        return None;
    }
    let c = match space {
        "srgb" => channels.iter().map(|n| n * 255.).collect::<Vec<_>>(),
        "display-p3" => {
            let r = linear(channels[0] * 255.);
            let g = linear(channels[1] * 255.);
            let b = linear(channels[2] * 255.);
            vec![
                gamma(1.2249401762805 * r - 0.2249401762805 * g),
                gamma(-0.042056961239 * r + 1.042056961239 * g),
                gamma(-0.0196375547643 * r - 0.0786360655012 * g + 1.0982736202656 * b),
            ]
        }
        _ => return None,
    };
    Some(composite([c[0], c[1], c[2]], base, alpha))
}
fn canvas_color(raw: &str) -> Option<Color> {
    let raw = raw.trim();
    let h = raw.strip_prefix('#').unwrap_or(raw);
    if h.len() == 4 && h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return color(&format!("#{}", &h[..3]), [0.; 3]);
    }
    if h.len() == 8 && h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return color(&format!("#{}", &h[..6]), [0.; 3]);
    }
    if raw.starts_with("color(")
        && let Some((before, _)) = raw.split_once('/')
    {
        return color(&format!("{})", before.trim()), [0.; 3]);
    }
    color(raw, [0.; 3])
}
pub fn vscode(v: &Value) -> Result<(String, bool, Value), Problem> {
    let colors = &v["colors"];
    let pick = |keys: &[&str], base| {
        keys.iter().find_map(|k| {
            colors[*k]
                .as_str()
                .filter(|s| s.len() <= 128)
                .and_then(|s| color(s, base))
        })
    };
    let canvas = ["editor.background", "editorPane.background"]
        .iter()
        .find_map(|key| {
            colors[*key]
                .as_str()
                .filter(|s| s.len() <= 128)
                .and_then(canvas_color)
        })
        .ok_or(Problem::Invalid)?;
    let dark = match v["type"].as_str() {
        Some("dark" | "hc-black") => true,
        Some("light" | "hc-light") => false,
        _ => luminance(canvas) < 0.179,
    };
    let accent = pick(
        &[
            "focusBorder",
            "button.background",
            "textLink.foreground",
            "activityBarBadge.background",
            "progressBar.background",
            "badge.background",
        ],
        canvas,
    )
    .unwrap_or(canvas);
    let fg = pick(&["editor.foreground", "foreground"], canvas)
        .filter(|c| ratio(*c, canvas) >= 4.5)
        .unwrap_or(if luminance(canvas) < 0.179 {
            [255.; 3]
        } else {
            [0.; 3]
        });
    let mut theme = json!({"version":1,"presetId":"custom","background":hex(canvas),"foreground":hex(fg),"accent":hex(accent)});
    let derived = compile::resolve(&theme, dark);
    let mut t = derived.clone();
    t.insert("background".into(), canvas);
    t.insert(
        "surface".into(),
        pick(&["editorPane.background"], canvas).unwrap_or(canvas),
    );
    t.insert("surface-container-lowest".into(), canvas);
    for (k, keys) in [
        (
            "surface-container-low",
            vec!["sideBar.background", "activityBar.background"],
        ),
        (
            "surface-container",
            vec!["panel.background", "editorWidget.background"],
        ),
        (
            "surface-container-high",
            vec![
                "menu.background",
                "quickInput.background",
                "dropdown.background",
            ],
        ),
        (
            "surface-container-highest",
            vec!["list.activeSelectionBackground"],
        ),
        (
            "outline-variant",
            vec!["panel.border", "sideBar.border", "contrastBorder"],
        ),
    ] {
        if let Some(c) = pick(&keys, canvas) {
            t.insert(k.into(), c);
        }
    }
    t.insert("primary".into(), accent);
    let action = pick(&["button.background"], canvas).unwrap_or(accent);
    t.insert("primary-dim".into(), action);
    for (k, keys, base) in [
        ("on-primary", vec!["button.foreground"], action),
        (
            "on-surface",
            vec!["editor.foreground", "foreground"],
            canvas,
        ),
        (
            "on-surface-variant",
            vec![
                "sideBar.foreground",
                "descriptionForeground",
                "disabledForeground",
            ],
            t["surface-container-low"],
        ),
        (
            "error",
            vec!["editorError.foreground", "errorForeground"],
            canvas,
        ),
        ("warning", vec!["editorWarning.foreground"], canvas),
    ] {
        let fallback = derived[k];
        let c = pick(&keys, base)
            .filter(|c| ratio(*c, base) >= 4.5)
            .unwrap_or_else(|| {
                if ratio(fallback, base) >= 4.5 {
                    fallback
                } else if luminance(base) < 0.179 {
                    [255.; 3]
                } else {
                    [0.; 3]
                }
            });
        t.insert(k.into(), c);
    }
    theme[if dark { "dark" } else { "light" }] = compile::value(&t);
    let label = v["displayName"]
        .as_str()
        .or(v["name"].as_str())
        .unwrap_or("VS Code theme")
        .chars()
        .take(64)
        .collect();
    Ok((label, dark, theme))
}
/// The shipped explanations for a theme file exchange. `$wrapped` lists each
/// reason as the person sees it, with the shipped failure prefix, so the
/// service-failure allow list can show it verbatim.
macro_rules! reasons {
    ($wrapped:ident, $prefix:literal; $($name:ident = $text:literal),+ $(,)?) => {
        $(pub const $name: &str = $text;)+
        pub const $wrapped: &[&str] = &[$(concat!($prefix, $text)),+];
    };
}
pub mod reason {
    pub const IMPORT_PREFIX: &str = "Failed to import theme: ";
    pub const EXPORT_PREFIX: &str = "Failed to export theme: ";
    // What the shipped theme parser reports about the chosen file.
    reasons! { IMPORT_PARSE, "Failed to import theme: ";
        TOO_LARGE = "Theme file exceeds the 256 KiB limit.",
        NOT_JSON = "Theme file is not valid JSON.",
        NO_CANVAS = "That VS Code theme has no \"editor.background\" color.",
        BAD_LIBRARY_SHAPE = "Theme file has an invalid library-theme shape.",
        NO_APPEARANCE = "Theme file has no supported appearance.",
        BAD_VERSION = "Theme file has an unsupported version.",
        BAD_MODE = "Theme file has an invalid appearance mode.",
        BAD_THEME = "Theme file has an unsupported theme version.",
    }
    // What reading the chosen file can report (the shipped read_theme_file command).
    reasons! { IMPORT_READ, "Failed to import theme: ";
        UNREADABLE = "Murmur could not read the theme file.",
        NOT_A_FILE = "Choose a valid theme file.",
        FILE_TOO_LARGE = "Theme files must be 256 KiB or smaller.",
        NOT_UTF8 = "Theme files must be valid UTF-8.",
    }
    // What writing the export can report (the shipped write_theme_file command).
    reasons! { EXPORT_WRITE, "Failed to export theme: ";
        BAD_DESTINATION = "Choose a valid theme export destination.",
        WRITE_FAILED = "Murmur could not write the theme export.",
        PUBLISH_FAILED = "Murmur could not publish the theme export.",
        EXPORT_TOO_LARGE = "Theme exports must be 64 KiB or smaller.",
        SYNC_FAILED = "Theme export was written, but Murmur could not finish syncing its folder.",
    }
}
/// The shipped line for a failed import, from a file-read or parse reason.
/// Anything the person has no shipped line for reads as an unreadable file.
pub fn import_failure(reason: &str) -> &'static str {
    [reason::IMPORT_PARSE, reason::IMPORT_READ]
        .into_iter()
        .flatten()
        .copied()
        .find(|line| line.strip_prefix(reason::IMPORT_PREFIX) == Some(reason))
        .unwrap_or(concat!(
            "Failed to import theme: ",
            "Murmur could not read the theme file."
        ))
}
/// The shipped line for a failed export.
pub fn export_failure(reason: &str) -> &'static str {
    reason::EXPORT_WRITE
        .iter()
        .copied()
        .find(|line| line.strip_prefix(reason::EXPORT_PREFIX) == Some(reason))
        .unwrap_or(concat!(
            "Failed to export theme: ",
            "Murmur could not write the theme export."
        ))
}
fn is_vscode(v: &Value) -> bool {
    v["colors"]
        .as_object()
        .is_some_and(|colors| colors.keys().any(|k| k.contains('.')))
        || v["tokenColors"].is_array()
}
/// The theme with only the fields the shipped importer keeps; whatever else
/// the file carried is not stored or shared again.
fn plain_theme(theme: &Value) -> Option<Value> {
    let doc = murmur_core::state::appearance_export(
        "murmur-appearance",
        json!({"version":1,"revision":0,"mode":"system","theme":theme}),
    )
    .ok()?;
    Some(doc["theme"].clone())
}
/// A theme file as the shipped importer reads it: the label, the appearances it
/// supports and the theme. A failure is the shipped reason, not the file.
pub fn local(text: &str) -> Result<(String, Vec<String>, Value), &'static str> {
    use reason::*;
    if text.len() > 256 * 1024 {
        return Err(TOO_LARGE);
    }
    let v = crate::json::jsonc(text, 256 * 1024).map_err(|_| NOT_JSON)?;
    if is_vscode(&v) {
        let (label, dark, t) = vscode(&v).map_err(|_| NO_CANVAS)?;
        return Ok((label, vec![if dark { "dark" } else { "light" }.into()], t));
    }
    if v["version"] == 2 {
        let name = v["name"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.chars().count() <= 64);
        let (Some(name), Some(listed), true) = (
            name,
            v["modes"].as_array(),
            v["theme"].is_object() && v["theme"]["version"] == 1,
        ) else {
            return Err(BAD_LIBRARY_SHAPE);
        };
        let modes: Vec<String> = ["light", "dark"]
            .into_iter()
            .filter(|mode| listed.iter().any(|m| m == mode))
            .map(String::from)
            .collect();
        if modes.is_empty() {
            return Err(NO_APPEARANCE);
        }
        let theme = plain_theme(&v["theme"]).ok_or(BAD_LIBRARY_SHAPE)?;
        return Ok((name.into(), modes, theme));
    }
    if v["version"] != 1 {
        return Err(BAD_VERSION);
    }
    if !["system", "light", "dark"]
        .iter()
        .any(|mode| v["mode"] == *mode)
    {
        return Err(BAD_MODE);
    }
    if !v["theme"].is_object() || v["theme"]["version"] != 1 {
        return Err(BAD_THEME);
    }
    let theme = plain_theme(&v["theme"]).ok_or(BAD_THEME)?;
    Ok((
        "Imported theme".into(),
        vec!["light".into(), "dark".into()],
        theme,
    ))
}
fn pretty(file: Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(&file).unwrap_or_default()
    )
}
/// The portable v1 file for the current appearance (`appearance` is the
/// allow-listed export of the appearance document).
pub fn current_file(appearance: &Value) -> String {
    pretty(json!({"version":1,"mode":appearance["mode"],"theme":appearance["theme"]}))
}
/// The portable v2 file for one library theme (`entry` is from the allow-listed export).
pub fn entry_file(entry: &Value) -> String {
    pretty(json!({"version":2,"name":entry["label"],"modes":entry["modes"],"theme":entry["theme"]}))
}
