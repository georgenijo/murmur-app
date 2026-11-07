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
pub fn local(text: &str) -> Result<(String, Vec<String>, Value), Problem> {
    let v = crate::json::jsonc(text, 256 * 1024)?;
    if v.get("colors").is_some() || v["tokenColors"].is_array() {
        let (label, dark, t) = vscode(&v)?;
        Ok((label, vec![if dark { "dark" } else { "light" }.into()], t))
    } else {
        let theme = if v["theme"].is_object() {
            v["theme"].clone()
        } else {
            v.clone()
        };
        if theme["version"] != 1 {
            return Err(Problem::Invalid);
        }
        let doc = murmur_core::state::normalize(
            "murmur-appearance",
            json!({"version":1,"revision":0,"mode":"system","theme":theme}),
        )
        .map_err(|_| Problem::Invalid)?;
        Ok((
            v["label"]
                .as_str()
                .unwrap_or("Imported theme")
                .chars()
                .take(64)
                .collect(),
            vec!["light".into(), "dark".into()],
            doc["theme"].clone(),
        ))
    }
}
