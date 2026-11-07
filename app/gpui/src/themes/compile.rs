use super::color::*;
use crate::section::Problem;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const TOKENS: [&str; 16] = [
    "background",
    "surface",
    "surface-container-low",
    "surface-container",
    "surface-container-high",
    "surface-container-lowest",
    "surface-container-highest",
    "primary",
    "primary-dim",
    "on-primary",
    "on-surface",
    "on-surface-variant",
    "outline-variant",
    "error",
    "success",
    "warning",
];
pub type Tokens = BTreeMap<String, Color>;
const FLOORS: [(&str, f64); 5] = [
    ("surface-container-low", 0.05),
    ("surface-container-lowest", 0.085),
    ("surface-container", 0.12),
    ("surface-container-high", 0.16),
    ("surface-container-highest", 0.2),
];
pub fn sonic(dark: bool) -> Tokens {
    let v: Value = serde_json::from_str(include_str!("palettes.json")).unwrap();
    from_value(&v[if dark { "sonic_dark" } else { "sonic_light" }]).unwrap()
}
pub fn from_value(v: &Value) -> Result<Tokens, Problem> {
    let mut out = Tokens::new();
    for k in TOKENS {
        out.insert(
            k.into(),
            parse(v[k].as_str().ok_or(Problem::Invalid)?).ok_or(Problem::Invalid)?,
        );
    }
    Ok(out)
}
pub fn value(t: &Tokens) -> Value {
    Value::Object(t.iter().map(|(k, v)| (k.clone(), json!(hex(*v)))).collect())
}
fn pole(background: Color) -> Color {
    let black = [0.; 3];
    let white = [255.; 3];
    if ratio(black, composite(black, background, 0.15))
        >= ratio(white, composite(white, background, 0.15))
    {
        black
    } else {
        white
    }
}
fn repair(surface: Color, fg: Color) -> Color {
    solve(surface, |c| ratio(fg, composite(fg, c, 0.15)) >= 7.)
}
fn ladder(bg: Color, contrast: f64) -> Tokens {
    let fg = pole(bg);
    let bg = repair(bg, fg);
    let lowest = if fg == [0.; 3] { [255.; 3] } else { [0.; 3] };
    let strength = 1. + contrast / 100.;
    let mut out = Tokens::new();
    for (k, a, target) in [
        ("background", 0., fg),
        ("surface", 0., fg),
        ("surface-container-low", 0.035, fg),
        ("surface-container", 0.065, fg),
        ("surface-container-high", 0.1, fg),
        ("surface-container-lowest", 0.02, lowest),
        ("surface-container-highest", 0.14, fg),
    ] {
        out.insert(k.into(), repair(mix(bg, target, a * strength), fg));
    }
    if fg == [255.; 3] {
        for (k, a) in FLOORS {
            out.insert(
                k.into(),
                repair(composite(fg, bg, a * strength.max(1.)), fg),
            );
        }
    }
    out
}
fn passes(t: &Tokens, token: &str, c: Color) -> bool {
    let surfaces: Vec<_> = TOKENS[..7].iter().map(|k| t[*k]).collect();
    match token {
        "on-primary" => ["primary", "primary-dim", "error"]
            .iter()
            .all(|k| ratio(c, t[*k]) >= 4.5),
        "on-surface" | "on-surface-variant" => surfaces.iter().all(|b| {
            ratio(c, *b) >= 4.5
                && (token != "on-surface"
                    || [0.05, 0.1, 0.15]
                        .iter()
                        .all(|a| ratio(c, composite(t["primary"], *b, *a)) >= 4.5))
        }),
        "error" | "success" | "warning" => surfaces
            .iter()
            .all(|b| ratio(c, *b) >= 4.5 && ratio(c, composite(c, *b, 0.1)) >= 4.5),
        "primary" => surfaces.iter().all(|b| ratio(c, *b) >= 4.5),
        _ => true,
    }
}
pub fn failures(t: &Tokens) -> bool {
    [
        "primary",
        "error",
        "success",
        "warning",
        "on-surface",
        "on-surface-variant",
        "on-primary",
    ]
    .iter()
    .any(|k| !passes(t, k, t[*k]))
}
#[derive(Clone)]
pub struct Compiled {
    pub light: Tokens,
    pub dark: Tokens,
    pub adjustments: usize,
}
pub fn resolve(theme: &Value, dark: bool) -> Tokens {
    let mut t = sonic(dark);
    let mutable = theme["presetId"] == "custom"
        || [
            "accent",
            "background",
            "foreground",
            "contrast",
            "light",
            "dark",
        ]
        .iter()
        .any(|k| theme.get(*k).is_some());
    if !mutable {
        return t;
    }
    let color = |k| theme[k].as_str().and_then(parse);
    if color("background").is_some() || theme.get("contrast").is_some() {
        t.extend(ladder(
            color("background").unwrap_or(t["background"]),
            theme["contrast"].as_f64().unwrap_or(0.).clamp(-100., 100.),
        ));
    }
    if let Some(accent) = color("accent") {
        let [l, c, h] = lch(accent);
        let l = if dark { l.max(0.78) } else { l.min(0.52) };
        let primary = gamut([l, c.clamp(0.045, 0.22), h]);
        let dim = gamut([
            (l - if dark { 0.045 } else { 0.055 }).max(0.),
            c.clamp(0.04, 0.22),
            h,
        ]);
        t.insert("primary".into(), primary);
        t.insert("primary-dim".into(), dim);
        t.insert(
            "on-primary".into(),
            contrast(
                if dark { [0., 18., 22.] } else { [255.; 3] },
                &[primary, dim],
                4.5,
            ),
        );
    }
    if let Some(fg) = color("foreground") {
        t.insert("on-surface".into(), fg);
        t.insert("on-surface-variant".into(), mix(fg, t["background"], 0.22));
    }
    for k in TOKENS {
        if let Some(c) = theme[if dark { "dark" } else { "light" }][k]
            .as_str()
            .and_then(parse)
        {
            t.insert(k.into(), c);
        }
    }
    let fg = pole(t["background"]);
    for k in &TOKENS[..7] {
        let mut surface = t[*k];
        if fg == [255.; 3]
            && let Some((_, a)) = FLOORS.iter().find(|(name, _)| name == k)
        {
            let [l, c, h] = lch(surface);
            let floor = lab(composite(fg, t["background"], *a))[0];
            if l < floor {
                let raised = repair(gamut([floor, c, h]), fg);
                if lab(raised)[0] > l + 0.005 {
                    surface = raised
                }
            }
        }
        t.insert((*k).into(), repair(surface, fg));
    }
    let primary = solve(t["primary"], |c| passes(&t, "primary", c));
    t.insert("primary".into(), primary);
    let [l, _, _] = lch(primary);
    let [_, c, h] = lch(t["primary-dim"]);
    let on = if ratio([0.; 3], primary) >= ratio([255.; 3], primary) {
        [0.; 3]
    } else {
        [255.; 3]
    };
    let mut dim = contrast(gamut([(l - 0.045).clamp(0., 1.), c, h]), &[on], 4.5);
    if dim == primary {
        dim = gamut([
            (l + if on == [0.; 3] { 0.02 } else { -0.02 }).clamp(0., 1.),
            c,
            h,
        ]);
    }
    t.insert("primary-dim".into(), dim);
    for k in [
        "error",
        "success",
        "warning",
        "on-surface",
        "on-surface-variant",
        "on-primary",
    ] {
        let c = solve(t[k], |c| passes(&t, k, c));
        t.insert(k.into(), c);
    }
    if failures(&t) {
        let base = if dark { [0.; 3] } else { [255.; 3] };
        for k in &TOKENS[..7] {
            t.insert((*k).into(), base);
        }
        for (k, light, dark_color) in [
            ("primary", "#005a75", "#92dbfe"),
            ("primary-dim", "#004a60", "#84cdef"),
            ("on-primary", "#ffffff", "#00394b"),
            ("on-surface", "#111111", "#eeeeee"),
            ("on-surface-variant", "#333333", "#cccccc"),
            ("outline-variant", "#595959", "#777777"),
            ("error", "#8b1a1a", "#ff9a95"),
            ("success", "#176b3a", "#66d99a"),
            ("warning", "#704d00", "#f4bd65"),
        ] {
            t.insert(
                k.into(),
                parse(if dark { dark_color } else { light }).unwrap(),
            );
        }
    }
    t
}
pub fn compile(theme: &Value) -> Compiled {
    let light = resolve(theme, false);
    let dark = resolve(theme, true);
    let adjustments = [(&light, "light"), (&dark, "dark")]
        .iter()
        .map(|(t, mode)| {
            TOKENS
                .iter()
                .filter(|k| {
                    theme[*mode][**k]
                        .as_str()
                        .and_then(parse)
                        .is_some_and(|c| c != t[**k])
                })
                .count()
        })
        .sum();
    Compiled {
        light,
        dark,
        adjustments,
    }
}
pub fn apply(tokens: &Tokens, dark: bool, cx: &mut gpui::App) {
    crate::tokens::apply_appearance(dark, cx);
    gpui_component::Theme::update(cx, |theme| {
        let c = |k| {
            let [r, g, b] = tokens[k];
            gpui::Hsla::from(gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32))
        };
        let t = &mut theme.colors;
        t.background = c("background");
        t.foreground = c("on-surface");
        t.secondary = c("surface-container");
        t.sidebar = c("surface-container-low");
        t.success = c("success");
        t.muted_foreground = c("on-surface-variant");
        t.border = c("outline-variant");
        t.input = t.border;
        t.primary = c("primary");
        t.primary_foreground = c("on-primary");
        t.ring = t.primary;
        t.danger = c("error");
        t.popover = c("surface-container-lowest");
        t.popover_foreground = t.foreground;
    });
}
