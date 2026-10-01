//! Colors, and the fonts the settings chose (see `fonts.rs`).
//!
//! The palette becomes a kit `ThemeConfig`, so the kit derives the shades we
//! do not set.

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::*;
use serde_json::{Value, json};

/// The interface font family from the settings, as the theme holds it.
pub fn ui_font(cx: &App) -> SharedString {
    Theme::global(cx).font_family.clone()
}

/// The code font family from the settings, as the theme holds it.
pub fn mono_font(cx: &App) -> SharedString {
    Theme::global(cx).mono_font_family.clone()
}

/// Colors are `0xRRGGBB`, except `hover` and `highlight`: they are
/// `0xRRGGBBAA` tints, so they show the same on the sidebar, the panes and
/// popovers.
pub struct Palette {
    pub light: bool,
    pub bg: u32,
    pub sidebar: u32,
    pub elevated: u32,
    pub border: u32,
    pub input: u32,
    /// Rows under the pointer.
    pub hover: u32,
    /// The current item of menus and the command palette. Ghost buttons
    /// under the pointer use half of it in dark mode.
    pub highlight: u32,
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

// The colors of gibbons: the black fur of a siamang for the dark grounds,
// the cream face ring of a lar gibbon for text, and the gold of a female
// golden-cheeked gibbon for the accent. The grays lean warm to match. Light
// mode darkens the gold so text in it stays readable on white.

pub const DARK: Palette = Palette {
    light: false,
    bg: 0x171512,
    sidebar: 0x0F0E0C,
    elevated: 0x221F1B,
    border: 0x35302A,
    input: 0x4A443C,
    hover: 0xFFF1DC17,
    highlight: 0xFFF1DC2E,
    selection: 0x45351B,
    fg: 0xF2EDE4,
    muted_fg: 0xA69D90,
    accent: 0xF2AE3D,
    accent_fg: 0x1C1305,
    red: 0xF26D6D,
    green: 0x5CCB8A,
    yellow: 0xE9CD5B,
    blue: 0x62B6F2,
    magenta: 0xE66CA0,
    cyan: 0x3FC9C9,
    syntax: Syntax {
        keyword: 0xB69CFF,
        function: 0x7CB7FF,
        type_: 0x4FD6BE,
        string: 0xA6DA95,
        number: 0xF5A97F,
        comment: 0x7D756A,
        property: 0x8BD5F7,
        punct: 0x9C958A,
        tag: 0xF38BA8,
        attribute: 0xE8C77A,
    },
};

pub const LIGHT: Palette = Palette {
    light: true,
    bg: 0xFFFFFF,
    sidebar: 0xF5F0E7,
    elevated: 0xFFFFFF,
    border: 0xDDD5C8,
    input: 0xC4BAAA,
    hover: 0x5C452117,
    highlight: 0x5C45211C,
    selection: 0xF8DDA8,
    fg: 0x1E1A14,
    muted_fg: 0x6B6255,
    accent: 0xA05E00,
    accent_fg: 0xFFFFFF,
    red: 0xC8323F,
    green: 0x16804A,
    yellow: 0x8A6C00,
    blue: 0x1F6FBF,
    magenta: 0xC23C7E,
    cyan: 0x118C8C,
    syntax: Syntax {
        keyword: 0x8250DF,
        function: 0x0B5FC7,
        type_: 0x0B7A6E,
        string: 0x2B7A33,
        number: 0xB35900,
        comment: 0x857B6E,
        property: 0x0A6C9E,
        punct: 0x655D52,
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

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn hexa(c: u32, a: u8) -> String {
    format!("#{c:06x}{a:02x}")
}

/// A `0xRRGGBBAA` color.
fn rgba(c: u32) -> String {
    format!("#{c:08x}")
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
        ("secondary.background", rgba(p.hover)),
        ("secondary.foreground", hex(p.fg)),
        ("accent.background", rgba(p.highlight)),
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
        ("list.hover.background", rgba(p.hover)),
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
        theme.font_family = crate::fonts::ui(&s.ui_font).family.into();
        theme.font_size = px(s.ui_size);
        theme.mono_font_family = crate::fonts::code(&s.code_font).family.into();
        theme.mono_font_size = px(s.code_size);
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.shadow = true;
        let sidebar: Hsla = rgb(p.sidebar).into();
        theme.colors.sidebar = sidebar;
        theme.tokens.sidebar = sidebar.into();
        // The kit caps the alpha of list.active at 0.2, and selected rows
        // then barely show.
        let selection: Hsla = rgb(p.selection).into();
        theme.colors.list_active = selection;
        theme.tokens.list_active = selection.into();
        theme.notification.placement = Anchor::BottomRight;
        theme.notification.margins.bottom = px(36.);
        theme.notification.margins.right = px(12.);
    }
    Theme::sync_base(cx);
}
