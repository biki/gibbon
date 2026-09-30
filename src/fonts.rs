//! The interface and code fonts to choose from in Settings.
//!
//! Bundled fonts are embedded and registered at startup (SIL OFL 1.1,
//! licenses in `assets/fonts/`, fetched by `scripts/fetch-fonts.sh`).
//! SF Pro and SF Mono come from macOS itself.

use std::borrow::Cow;

use gpui_kit::App;

pub struct FontChoice {
    /// Stored in the settings.
    pub id: &'static str,
    /// Shown in Settings.
    pub name: &'static str,
    /// The family name the text system resolves.
    pub family: &'static str,
    pub note: &'static str,
}

const fn choice(
    id: &'static str,
    name: &'static str,
    family: &'static str,
    note: &'static str,
) -> FontChoice {
    FontChoice {
        id,
        name,
        family,
        note,
    }
}

pub const UI_FONTS: &[FontChoice] = &[
    // GPUI's name for the platform UI font.
    choice("sf", "SF Pro", ".SystemUIFont", "macOS system"),
    choice("geist", "Geist", "Geist", "crisp"),
    choice("plex", "IBM Plex Sans", "IBM Plex Sans", "engineered"),
    choice("manrope", "Manrope", "Manrope", "rounded"),
    choice("dmsans", "DM Sans", "DM Sans", "geometric"),
    choice("figtree", "Figtree", "Figtree", "open"),
    choice("instrument", "Instrument Sans", "Instrument Sans", "compact"),
    choice("inter", "Inter", "Inter", "neutral"),
];

pub const CODE_FONTS: &[FontChoice] = &[
    choice("sfmono", "SF Mono", "SF Mono", "macOS system"),
    choice("geistmono", "Geist Mono", "Geist Mono", "clean"),
    choice("plexmono", "IBM Plex Mono", "IBM Plex Mono", "classic"),
    choice("fira", "Fira Code", "Fira Code", "ligatures"),
    choice("source", "Source Code Pro", "Source Code Pro", "airy"),
    choice("dmmono", "DM Mono", "DM Mono", "soft"),
    choice("martian", "Martian Mono", "Martian Mono", "wide"),
    choice("jetbrains", "JetBrains Mono", "JetBrains Mono", "tall"),
];

pub const DEFAULT_UI: &str = "inter";
pub const DEFAULT_CODE: &str = "jetbrains";

pub fn ui(id: &str) -> &'static FontChoice {
    UI_FONTS
        .iter()
        .find(|f| f.id == id)
        .or_else(|| UI_FONTS.iter().find(|f| f.id == DEFAULT_UI))
        .expect("default interface font")
}

pub fn code(id: &str) -> &'static FontChoice {
    CODE_FONTS
        .iter()
        .find(|f| f.id == id)
        .or_else(|| CODE_FONTS.iter().find(|f| f.id == DEFAULT_CODE))
        .expect("default code font")
}

macro_rules! font_files {
    ($($file:literal),* $(,)?) => {
        &[$(include_bytes!(concat!("../assets/fonts/", $file)) as &[u8]),*]
    };
}

const FILES: &[&[u8]] = font_files![
    "Inter-Regular.ttf",
    "Inter-Medium.ttf",
    "Inter-SemiBold.ttf",
    "Inter-Bold.ttf",
    "Geist-400.ttf",
    "Geist-500.ttf",
    "Geist-600.ttf",
    "Geist-700.ttf",
    "IBMPlexSans-400.ttf",
    "IBMPlexSans-500.ttf",
    "IBMPlexSans-600.ttf",
    "IBMPlexSans-700.ttf",
    "Manrope-400.ttf",
    "Manrope-500.ttf",
    "Manrope-600.ttf",
    "Manrope-700.ttf",
    "DMSans-400.ttf",
    "DMSans-500.ttf",
    "DMSans-600.ttf",
    "DMSans-700.ttf",
    "Figtree-400.ttf",
    "Figtree-500.ttf",
    "Figtree-600.ttf",
    "Figtree-700.ttf",
    "InstrumentSans-400.ttf",
    "InstrumentSans-500.ttf",
    "InstrumentSans-600.ttf",
    "InstrumentSans-700.ttf",
    "JetBrainsMono-Regular.ttf",
    "JetBrainsMono-Medium.ttf",
    "JetBrainsMono-Bold.ttf",
    "JetBrainsMono-Italic.ttf",
    "GeistMono-400.ttf",
    "GeistMono-500.ttf",
    "GeistMono-700.ttf",
    "GeistMono-400Italic.ttf",
    "IBMPlexMono-400.ttf",
    "IBMPlexMono-500.ttf",
    "IBMPlexMono-700.ttf",
    "IBMPlexMono-400Italic.ttf",
    "FiraCode-400.ttf",
    "FiraCode-500.ttf",
    "FiraCode-700.ttf",
    "SourceCodePro-400.ttf",
    "SourceCodePro-500.ttf",
    "SourceCodePro-700.ttf",
    "SourceCodePro-400Italic.ttf",
    "DMMono-400.ttf",
    "DMMono-500.ttf",
    "DMMono-400Italic.ttf",
    "MartianMono-400.ttf",
    "MartianMono-500.ttf",
    "MartianMono-700.ttf",
];

/// Register every bundled font with the text system.
pub fn load(cx: &mut App) -> anyhow::Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = FILES.iter().map(|f| Cow::Borrowed(*f)).collect();
    cx.text_system().add_fonts(fonts)
}

#[cfg(test)]
mod tests {
    use super::{CODE_FONTS, UI_FONTS, code, ui};

    #[test]
    fn eight_of_each_and_fallbacks() {
        assert_eq!(UI_FONTS.len(), 8);
        assert_eq!(CODE_FONTS.len(), 8);
        assert_eq!(ui("nope").id, "inter");
        assert_eq!(code("nope").id, "jetbrains");
        assert_eq!(ui("sf").family, ".SystemUIFont");
    }
}
