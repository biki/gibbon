//! Fonts and colors. Inter for the UI, JetBrains Mono for code; both are
//! embedded (SIL OFL 1.1, licenses in `assets/fonts/`).
//!
//! The palette becomes a kit `ThemeConfig`, so the kit derives the shades we
//! do not set.

use std::borrow::Cow;
use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::*;
use serde_json::{Value, json};

pub const UI_FONT: &str = "Inter";
pub const MONO_FONT: &str = "JetBrains Mono";

pub struct Palette {
    pub light: bool,
    pub bg: u32,
    pub sidebar: u32,
    pub elevated: u32,
    pub border: u32,
    pub input: u32,
    pub hover: u32,
    pub selection: u32,
    pub fg: u32,
    pub muted_fg: u32,
    pub accent: u32,
    pub accent_fg: u32,
    pub red: u32,
    pub green: u32,
    pub yellow: u32,
    pub blue: u32,
    pub magenta: u32,
    pub cyan: u32,
    pub syntax: Syntax,
}

/// Code colors for diffs.
pub struct Syntax {
    pub keyword: u32,
    pub function: u32,
    pub type_: u32,
    pub string: u32,
    pub number: u32,
    pub comment: u32,
    pub property: u32,
    pub punct: u32,
    pub tag: u32,
    pub attribute: u32,
}

pub const DARK: Palette = Palette {
    light: false,
    bg: 0x131418,
    sidebar: 0x0F1013,
    elevated: 0x1C1D22,
    border: 0x24262C,
    input: 0x2C2E36,
    hover: 0x1B1C21,
    selection: 0x262A45,
    fg: 0xE4E5EA,
    muted_fg: 0x8A8C97,
    accent: 0x7C8CFF,
    accent_fg: 0xFFFFFF,
    red: 0xF0616D,
    green: 0x3FC98A,
    yellow: 0xE8B64A,
    blue: 0x5BB8F5,
    magenta: 0xE66CA0,
    cyan: 0x3FC9C9,
    syntax: Syntax {
        keyword: 0xB69CFF,
        function: 0x7CB7FF,
        type_: 0x4FD6BE,
        string: 0xA6DA95,
        number: 0xF5A97F,
        comment: 0x6C707D,
        property: 0x8BD5F7,
        punct: 0x9499A8,
        tag: 0xF38BA8,
        attribute: 0xE8C77A,
    },
};

pub const LIGHT: Palette = Palette {
    light: true,
    bg: 0xFFFFFF,
    sidebar: 0xF6F6F8,
    elevated: 0xFFFFFF,
    border: 0xE4E4E9,
    input: 0xD8D8DF,
    hover: 0xF1F1F4,
    selection: 0xE3E7FF,
    fg: 0x1B1C21,
    muted_fg: 0x6A6D79,
    accent: 0x5566F0,
    accent_fg: 0xFFFFFF,
    red: 0xD63A4A,
    green: 0x16965C,
    yellow: 0xB27A0C,
    blue: 0x1F7FD1,
    magenta: 0xC23C7E,
    cyan: 0x118C8C,
    syntax: Syntax {
        keyword: 0x8250DF,
        function: 0x0B5FC7,
        type_: 0x0B7A6E,
        string: 0x2B7A33,
        number: 0xB35900,
        comment: 0x7B7F8C,
        property: 0x0A6C9E,
        punct: 0x5F6370,
        tag: 0xB42B55,
        attribute: 0x8A5A00,
    },
};

pub fn palette(cx: &App) -> &'static Palette {
    use crate::settings::Appearance;
    match crate::settings::get(cx).appearance {
        Appearance::Light => &LIGHT,
        Appearance::Dark => &DARK,
        Appearance::System => match cx.window_appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => &LIGHT,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => &DARK,
        },
    }
}

pub fn load_fonts(cx: &mut App) -> anyhow::Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf")),
    ];
    cx.text_system().add_fonts(fonts)?;
    Ok(())
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn hexa(c: u32, a: u8) -> String {
    format!("#{c:06x}{a:02x}")
}

fn config_json(p: &Palette) -> Value {
    let colors: serde_json::Map<String, Value> = [
        ("background", hex(p.bg)),
        ("foreground", hex(p.fg)),
        ("border", hex(p.border)),
        ("input.border", hex(p.input)),
        ("muted.background", hex(p.elevated)),
        ("muted.foreground", hex(p.muted_fg)),
        ("primary.background", hex(p.accent)),
        ("primary.foreground", hex(p.accent_fg)),
        ("secondary.background", hex(p.hover)),
        ("secondary.foreground", hex(p.fg)),
        ("accent.background", hex(p.hover)),
        ("accent.foreground", hex(p.fg)),
        ("popover.background", hex(p.elevated)),
        ("popover.foreground", hex(p.fg)),
        ("title_bar.background", hex(p.sidebar)),
        ("title_bar.border", hex(p.border)),
        ("status_bar.background", hex(p.sidebar)),
        ("status_bar.border", hex(p.border)),
        ("sidebar.background", hex(p.sidebar)),
        ("sidebar.border", hex(p.border)),
        ("sidebar.foreground", hex(p.fg)),
        ("sidebar.accent.background", hex(p.selection)),
        ("sidebar.accent.foreground", hex(p.fg)),
        ("list.background", hex(p.bg)),
        ("list.hover.background", hex(p.hover)),
        ("list.active.background", hex(p.selection)),
        ("list.active.border", hexa(p.accent, 0x00)),
        ("selection.background", hexa(p.accent, 0x55)),
        ("caret", hex(p.accent)),
        ("ring", hex(p.accent)),
        ("link", hex(p.accent)),
        ("scrollbar.thumb.background", hexa(p.muted_fg, 0x66)),
        ("window.border", hex(p.border)),
        ("base.red", hex(p.red)),
        ("base.green", hex(p.green)),
        ("base.yellow", hex(p.yellow)),
        ("base.blue", hex(p.blue)),
        ("base.magenta", hex(p.magenta)),
        ("base.cyan", hex(p.cyan)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v)))
    .collect();
    let y = &p.syntax;
    let c = |v: u32| json!({ "color": hex(v) });
    let syntax = json!({
        "keyword": c(y.keyword),
        "function": c(y.function),
        "constructor": c(y.type_),
        "type": c(y.type_),
        "enum": c(y.type_),
        "variant": c(y.type_),
        "string": c(y.string),
        "string.escape": c(y.number),
        "string.regex": c(y.string),
        "string.special": c(y.string),
        "string.special.symbol": c(y.number),
        "number": c(y.number),
        "boolean": c(y.number),
        "constant": c(y.number),
        "comment": { "color": hex(y.comment), "font_style": "italic" },
        "comment.doc": { "color": hex(y.comment), "font_style": "italic" },
        "property": c(y.property),
        "label": c(y.property),
        "variable.special": c(y.keyword),
        "operator": c(y.punct),
        "punctuation": c(y.punct),
        "punctuation.bracket": c(y.punct),
        "punctuation.delimiter": c(y.punct),
        "punctuation.special": c(y.keyword),
        "tag": c(y.tag),
        "attribute": c(y.attribute),
        "preproc": c(y.keyword),
        "embedded": c(p.fg),
        "title": { "color": hex(y.function), "font_weight": 700 },
        "emphasis": { "font_style": "italic" },
        "emphasis.strong": { "font_weight": 700 },
        "link_text": c(y.function),
        "link_uri": c(y.property),
        "text.literal": c(y.string),
    });
    json!({
        "name": if p.light { "Gibbon Light" } else { "Gibbon Dark" },
        "mode": if p.light { "light" } else { "dark" },
        "colors": colors,
        "highlight": {
            "editor.background": hex(p.bg),
            "editor.foreground": hex(p.fg),
            "syntax": syntax,
        },
    })
}

/// Apply the palette that matches the system appearance.
pub fn apply(cx: &mut App) {
    let p = palette(cx);
    let config = match serde_json::from_value::<ThemeConfig>(config_json(p)) {
        Ok(c) => Rc::new(c),
        Err(e) => {
            eprintln!("theme failed to parse: {e}");
            return;
        }
    };
    let mode = if p.light {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    if !cx.has_global::<Theme>() {
        Theme::change(mode, None, cx);
    }
    {
        let theme = Theme::global_mut(cx);
        if p.light {
            theme.light_theme = config;
        } else {
            theme.dark_theme = config;
        }
    }
    Theme::change(mode, None, cx);
    {
        let s = crate::settings::get(cx).clone();
        let theme = Theme::global_mut(cx);
        theme.font_family = UI_FONT.into();
        theme.font_size = px(s.ui_size);
        theme.mono_font_family = MONO_FONT.into();
        theme.mono_font_size = px(s.code_size);
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.shadow = true;
        let sidebar: Hsla = rgb(p.sidebar).into();
        theme.colors.sidebar = sidebar;
        theme.tokens.sidebar = sidebar.into();
        theme.notification.placement = Anchor::BottomRight;
        theme.notification.margins.bottom = px(36.);
        theme.notification.margins.right = px(12.);
    }
    Theme::sync_base(cx);
}
