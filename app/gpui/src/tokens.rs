//! Geometry maps to --ui-* in styles.css and settings.css. Color roles are
//! supplied by gpui-component's theme; screen code must never choose literals.
use crate::preferences;
use gpui::{App, Hsla, Pixels, px};
use gpui_component::ActiveTheme;

pub const CONTROL_RADIUS: Pixels = px(8.);
pub const CARD_RADIUS: Pixels = px(12.);
pub const MENU_RADIUS: Pixels = px(4.);
pub const INPUT_HEIGHT: Pixels = px(30.);
pub const STANDARD_HEIGHT: Pixels = px(32.);
pub const COMPACT_HEIGHT: Pixels = px(28.);
pub const XS_HEIGHT: Pixels = px(24.);
pub const CONTROL_INSET: Pixels = px(11.);
pub const CAPTION_SIZE: Pixels = px(11.);
pub const CAPTION_LEADING: Pixels = px(16.);
pub const BODY_SIZE: Pixels = px(14.);
pub const BODY_LEADING: Pixels = px(20.);
pub const DISABLED_OPACITY: f32 = 0.64;
pub const FOCUS_WIDTH: Pixels = px(3.);
pub const SELECT_MIN: Pixels = px(144.);
pub const SELECT_MAX: Pixels = px(320.);
pub const LIST_GAP: Pixels = px(5.);
pub const SWITCH_WIDTH: Pixels = px(30.);
pub const SWITCH_HEIGHT: Pixels = px(18.);
pub const SWITCH_THUMB: Pixels = px(14.);

#[derive(Clone, Copy)]
pub struct Palette {
    pub surface: Hsla,
    pub raised: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub focus: Hsla,
}
impl Palette {
    /// Overlay utility windows follow system appearance; notch is always dark.
    pub fn overlay(floating: bool, dark: bool, cx: &App) -> Self {
        let dark = !floating || dark;
        let color = |value| Hsla::from(gpui::rgb(value));
        let (surface, text) = if dark {
            (color(0x1c1c1e), color(0xf6f6f8))
        } else {
            (color(0xf6f6f8), color(0x1c1c1e))
        };
        let high = preferences::current(cx).increase_contrast;
        Self {
            surface,
            raised: surface.blend(text.opacity(0.08)),
            text,
            muted: text.opacity(if high { 1. } else { 0.85 }),
            border: text.opacity(if high { 1. } else { 0.2 }),
            accent: text,
            on_accent: surface,
            danger: color(0xef4444),
            focus: text,
        }
    }

    pub fn get(cx: &App) -> Self {
        let theme = cx.theme();
        let high_contrast = preferences::current(cx).increase_contrast;
        Self {
            surface: theme.background,
            raised: theme.secondary,
            text: theme.foreground,
            muted: if high_contrast {
                theme.foreground
            } else {
                theme.muted_foreground
            },
            border: if high_contrast {
                theme.foreground
            } else {
                theme.input
            },
            accent: theme.primary,
            on_accent: theme.primary_foreground,
            danger: theme.danger,
            focus: theme.ring,
        }
    }
    pub fn popover(self, cx: &App) -> Hsla {
        if preferences::current(cx).reduce_transparency {
            self.surface
        } else {
            self.surface.opacity(0.8)
        }
    }
}

/// Seed the gallery/default native theme with Murmur's Sonic semantic roles.
/// The later appearance port can replace these roles through Theme::update;
/// controls only read Palette and never persist or alter a theme document.
pub fn apply_appearance(dark: bool, cx: &mut App) {
    use gpui::rgb;
    use gpui_component::{Theme, ThemeMode};
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        // app/src/lib/appearance/palettes.ts, SONIC_LIGHT / SONIC_DARK.
        let colors = if dark {
            [
                0x0b0f11, 0x1e2529, 0xdbe4e9, 0xabb3b9, 0x586065, 0x92dbfe, 0x00394b, 0xfa746f,
                0x0f1315,
            ]
        } else {
            [
                0xf7fafc, 0xe9eff3, 0x2b3438, 0x586065, 0xabb3b9, 0x036785, 0xf3faff, 0xa83836,
                0xffffff,
            ]
        };
        let c = &mut theme.colors;
        c.background = rgb(colors[0]).into();
        c.secondary = rgb(colors[1]).into();
        c.foreground = rgb(colors[2]).into();
        c.muted_foreground = rgb(colors[3]).into();
        c.border = rgb(colors[4]).into();
        c.input = c.border;
        c.primary = rgb(colors[5]).into();
        c.ring = c.primary;
        c.primary_foreground = rgb(colors[6]).into();
        c.danger = rgb(colors[7]).into();
        c.popover = rgb(colors[8]).into();
        c.popover_foreground = c.foreground;
        theme.font_family = ".SystemUIFont".into();
        theme.font_size = BODY_SIZE;
        theme.radius = CONTROL_RADIUS;
        theme.radius_lg = CARD_RADIUS;
    });
}

/// The notch stays dark regardless of the main appearance document.
pub fn overlay_surface() -> Hsla {
    Hsla::from(gpui::rgb(0x141414)).opacity(0.92)
}

/// Live Words notch card; Reduce Transparency is completely opaque.
pub fn preview_surface(opaque: bool) -> Hsla {
    Hsla::from(gpui::rgb(0x141414)).opacity(if opaque { 1. } else { 0.95 })
}
