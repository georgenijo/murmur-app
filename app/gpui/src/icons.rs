//! Decorative Lucide glyphs. Accessible names belong to the owning control.
use gpui::{Styled, px};
use gpui_component::Icon;
#[derive(Clone, Copy)]
pub enum Glyph {
    House,
    Calendar,
    Chart,
    Mic,
    Stop,
    Play,
    Pause,
    Spinner,
    Settings,
    PanelLeft,
    Search,
    Filter,
    More,
    Copy,
    ChevronRight,
    ChevronLeft,
    ChevronDown,
    Download,
    Refresh,
    Close,
    Check,
    Keyboard,
    Palette,
    Sliders,
    Type,
    Cpu,
    Audio,
    Send,
    Wand,
    Gauge,
    Bug,
    Message,
    File,
    Trash,
    Plus,
    Upload,
    Sun,
    Moon,
    Pencil,
    ClipboardCheck,
    CircleDot,
    Panels,
    Alert,
    SettingsDelivery,
    SettingsText,
    SettingsModes,
    SettingsDiagnostics,
}
pub fn icon(glyph: Glyph) -> Icon {
    let bytes: &'static [u8] = match glyph {
        Glyph::SettingsDelivery => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="5" y="4" width="14" height="16" rx="2"/><path d="m9 12 2 2 4-5"/></svg>"#,
        Glyph::SettingsText => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M5 5h14M8 5v14M5 19h6M15 10h4M15 14h4"/></svg>"#,
        Glyph::SettingsModes => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="4" y="5" width="11" height="7" rx="1.5"/><rect x="9" y="12" width="11" height="7" rx="1.5"/><path d="M7 8.5h5M12 15.5h5"/></svg>"#,
        Glyph::SettingsDiagnostics => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 4h16v16H4zM8 9h8M8 13h5M8 17h3"/></svg>"#,

        Glyph::Alert => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/circle-alert.svg")
        }
        Glyph::Pencil => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/pencil.svg")
        }
        Glyph::ClipboardCheck => include_bytes!(
            "../vendor/gpui-component/crates/assets/assets/icons/clipboard-check.svg"
        ),
        Glyph::CircleDot => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/circle-dot.svg")
        }
        Glyph::Panels => include_bytes!(
            "../vendor/gpui-component/crates/assets/assets/icons/panels-top-left.svg"
        ),
        Glyph::File => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/file.svg")
        }
        Glyph::Trash => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/trash.svg")
        }
        Glyph::Plus => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/plus.svg")
        }
        Glyph::Upload => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/upload.svg")
        }
        Glyph::Sun => include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/sun.svg"),
        Glyph::Moon => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/moon.svg")
        }
        Glyph::House => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/house.svg")
        }
        Glyph::Calendar => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/calendar.svg")
        }
        Glyph::Chart => include_bytes!(
            "../vendor/gpui-component/crates/assets/assets/icons/chart-no-axes-column.svg"
        ),
        Glyph::Mic => include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/mic.svg"),
        Glyph::Play => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/play.svg")
        }
        Glyph::Pause => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/pause.svg")
        }
        Glyph::Stop => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/square.svg")
        }
        Glyph::Spinner => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/loader-circle.svg")
        }
        Glyph::Settings => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/settings.svg")
        }
        Glyph::PanelLeft => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/panel-left.svg")
        }
        Glyph::Search => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/search.svg")
        }
        Glyph::Filter => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/list-filter.svg")
        }
        Glyph::More => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/ellipsis.svg")
        }
        Glyph::Copy => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/copy.svg")
        }
        Glyph::ChevronRight => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/chevron-right.svg")
        }
        Glyph::ChevronLeft => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/chevron-left.svg")
        }
        Glyph::ChevronDown => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/chevron-down.svg")
        }
        Glyph::Download => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/download.svg")
        }
        Glyph::Refresh => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/refresh-cw.svg")
        }
        Glyph::Close => include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/x.svg"),
        Glyph::Check => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/check.svg")
        }
        Glyph::Keyboard => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/keyboard.svg")
        }
        Glyph::Palette => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/palette.svg")
        }
        Glyph::Sliders => include_bytes!(
            "../vendor/gpui-component/crates/assets/assets/icons/sliders-horizontal.svg"
        ),
        Glyph::Type => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/type.svg")
        }
        Glyph::Cpu => include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/cpu.svg"),
        Glyph::Audio => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/audio-lines.svg")
        }
        Glyph::Send => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/send.svg")
        }
        Glyph::Wand => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/wand-sparkles.svg")
        }
        Glyph::Gauge => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/gauge.svg")
        }
        Glyph::Bug => include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/bug.svg"),
        Glyph::Message => {
            include_bytes!("../vendor/gpui-component/crates/assets/assets/icons/message-square.svg")
        }
    };
    Icon::default().data(bytes).size(px(16.))
}

pub fn settings_glyph(label: &str) -> Glyph {
    match label {
        "General" => Glyph::CircleDot,
        "Recording" => Glyph::Mic,
        "Customize" => Glyph::Pencil,
        "Delivery" => Glyph::SettingsDelivery,
        "Meetings" => Glyph::Calendar,
        "Text & vocabulary" => Glyph::SettingsText,
        "AI & models" => Glyph::Sun,
        "Modes" => Glyph::SettingsModes,
        "Appearance" => Glyph::Palette,
        "Performance Lab" => Glyph::Chart,
        "Diagnostics" => Glyph::SettingsDiagnostics,
        "Keyboard shortcuts" => Glyph::Keyboard,
        _ => Glyph::Audio,
    }
}
