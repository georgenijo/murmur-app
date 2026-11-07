//! Dependency-free sRGB/OKLab/OKLCH parity with appearance/color.ts.
pub type Color = [f64; 3];
pub type Lab = [f64; 3];
pub fn parse(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let h = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.into()
    };
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&h[0..2], 16).ok()? as f64,
        u8::from_str_radix(&h[2..4], 16).ok()? as f64,
        u8::from_str_radix(&h[4..6], 16).ok()? as f64,
    ])
}
pub fn hex(c: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        c[0].round().clamp(0., 255.) as u8,
        c[1].round().clamp(0., 255.) as u8,
        c[2].round().clamp(0., 255.) as u8
    )
}
pub fn linear(v: f64) -> f64 {
    let c = v / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
pub fn gamma(v: f64) -> f64 {
    (if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    })
    .clamp(0., 1.)
        * 255.
}
pub fn lab(c: Color) -> Lab {
    let [r, g, b] = c.map(linear);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ]
}
fn rgb([l, a, b]: Lab) -> Color {
    let x = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let y = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let z = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        4.0767416621 * x - 3.3077115913 * y + 0.2309699292 * z,
        -1.2684380046 * x + 2.6097574011 * y - 0.3413193965 * z,
        -0.0041960863 * x - 0.7034186147 * y + 1.707614701 * z,
    ]
}
pub fn lch(c: Color) -> Lab {
    let [l, a, b] = lab(c);
    [l, a.hypot(b), b.atan2(a)]
}
pub fn gamut([l, c, h]: Lab) -> Color {
    let make = |c| [l, c * h.cos(), c * h.sin()];
    let valid = |c| {
        rgb(make(c))
            .iter()
            .all(|v| *v >= -0.000001 && *v <= 1.000001)
    };
    let mut c = c;
    if !valid(c) {
        let mut lo = 0.;
        let mut hi = c.max(0.);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.;
            if valid(mid) { lo = mid } else { hi = mid }
        }
        c = lo
    }
    rgb(make(c)).map(|v| gamma(v).round())
}
pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let a = lab(a);
    let b = lab(b);
    let t = t.clamp(0., 1.);
    rgb(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)).map(|v| gamma(v).round())
}
pub fn composite(a: Color, b: Color, t: f64) -> Color {
    std::array::from_fn(|i| (a[i] * t + b[i] * (1. - t)).round())
}
pub fn luminance(c: Color) -> f64 {
    let [r, g, b] = c.map(linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}
pub fn ratio(a: Color, b: Color) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
pub fn solve(source: Color, passes: impl Fn(Color) -> bool) -> Color {
    if passes(source) {
        return source;
    }
    let [l, c, h] = lch(source);
    let mut best = None;
    for step in 0..=200 {
        let v = step as f64 / 200.;
        let candidate = gamut([v, c, h]);
        if passes(candidate)
            && best
                .as_ref()
                .is_none_or(|(_, distance)| ((v - l).abs()) < *distance)
        {
            best = Some((candidate, (v - l).abs()));
        }
    }
    if let Some((c, _)) = best {
        return c;
    }
    for p in [[0.; 3], [255.; 3]] {
        if passes(p) {
            return p;
        }
    }
    source
}
pub fn contrast(source: Color, backgrounds: &[Color], min: f64) -> Color {
    let c = solve(source, |c| backgrounds.iter().all(|b| ratio(c, *b) >= min));
    if backgrounds.iter().all(|b| ratio(c, *b) >= min) {
        c
    } else {
        let score = |c| {
            backgrounds
                .iter()
                .map(|b| ratio(c, *b))
                .fold(f64::INFINITY, f64::min)
        };
        if score([0.; 3]) >= score([255.; 3]) {
            [0.; 3]
        } else {
            [255.; 3]
        }
    }
}
