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

/// Colors to choose from in Settings, for both appearances.
pub struct Scheme {
    /// Stored in the settings.
    pub id: &'static str,
    /// Shown in Settings.
    pub name: &'static str,
    pub note: &'static str,
    pub dark: Palette,
    pub light: Palette,
    /// The color of `graph::COLORS` that the first lane takes, the one
    /// nearest to the accent.
    pub lane: usize,
}

pub const SCHEMES: &[Scheme] = &[
    Scheme {
        id: "gibbon",
        name: "Gibbon",
        note: "golden fur",
        dark: GIBBON_DARK,
        light: GIBBON_LIGHT,
        lane: 0,
    },
    Scheme {
        id: "indigo",
        name: "Indigo",
        note: "cool blue",
        dark: INDIGO_DARK,
        light: INDIGO_LIGHT,
        lane: 2,
    },
    Scheme {
        id: "canopy",
        name: "Canopy",
        note: "leaf green",
        dark: CANOPY_DARK,
        light: CANOPY_LIGHT,
        lane: 6,
    },
    Scheme {
        id: "lagoon",
        name: "Lagoon",
        note: "deep teal",
        dark: LAGOON_DARK,
        light: LAGOON_LIGHT,
        lane: 1,
    },
    Scheme {
        id: "orchid",
        name: "Orchid",
        note: "soft violet",
        dark: ORCHID_DARK,
        light: ORCHID_LIGHT,
        lane: 5,
    },
];

pub const DEFAULT: &str = "gibbon";

pub fn scheme(id: &str) -> &'static Scheme {
    SCHEMES
        .iter()
        .find(|s| s.id == id)
        .or_else(|| SCHEMES.iter().find(|s| s.id == DEFAULT))
        .expect("default scheme")
}

/// The scheme from the settings.
pub fn current(cx: &App) -> &'static Scheme {
    scheme(&crate::settings::get(cx).theme)
}

// Gibbon, the default, takes the colors of gibbons: the black fur of a
// siamang for the dark grounds, the cream face ring of a lar gibbon for
// text, and the gold of a female golden-cheeked gibbon for the accent. The
// grays lean warm to match. Light mode darkens the gold so text in it stays
// readable on white.
//
// The other schemes keep the luminance of each Gibbon color and change only
// the hue, so every scheme has the contrast of Gibbon. They take the status
// and code colors of Gibbon.

const GIBBON_DARK: Palette = Palette {
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

const GIBBON_LIGHT: Palette = Palette {
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

const INDIGO_DARK: Palette = Palette {
    bg: 0x141519,
    sidebar: 0x0D0E10,
    elevated: 0x1E1F24,
    border: 0x2F3137,
    input: 0x42444E,
    hover: 0xF0F2FF17,
    highlight: 0xF0F2FF2E,
    selection: 0x313650,
    fg: 0xEBEDF7,
    muted_fg: 0x9A9DAC,
    accent: 0xA9B7FE,
    accent_fg: 0x111322,
    syntax: Syntax {
        comment: 0x737582,
        punct: 0x9396A1,
        ..GIBBON_DARK.syntax
    },
    ..GIBBON_DARK
};

const INDIGO_LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    sidebar: 0xEEF0FA,
    elevated: 0xFFFFFF,
    border: 0xD2D5E4,
    input: 0xB6BBCC,
    hover: 0x40476B17,
    highlight: 0x40476B1C,
    selection: 0xD8DFFF,
    fg: 0x191B21,
    muted_fg: 0x5F6271,
    accent: 0x5B68B6,
    accent_fg: 0xFFFFFF,
    syntax: Syntax {
        comment: 0x787C8B,
        punct: 0x5B5D6A,
        ..GIBBON_LIGHT.syntax
    },
    ..GIBBON_LIGHT
};

const CANOPY_DARK: Palette = Palette {
    bg: 0x141512,
    sidebar: 0x0D0F0D,
    elevated: 0x1E201C,
    border: 0x2E322B,
    input: 0x42473E,
    hover: 0xEBF7E117,
    highlight: 0xEBF7E12E,
    selection: 0x2F3C22,
    fg: 0xEAEFE6,
    muted_fg: 0x99A093,
    accent: 0x98CA5F,
    accent_fg: 0x0F1708,
    syntax: Syntax {
        comment: 0x72786D,
        punct: 0x91988C,
        ..GIBBON_DARK.syntax
    },
    ..GIBBON_DARK
};

const CANOPY_LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    sidebar: 0xEEF2E9,
    elevated: 0xFFFFFF,
    border: 0xD1D8CB,
    input: 0xB5BEAE,
    hover: 0x3D4E2B17,
    highlight: 0x3D4E2B1C,
    selection: 0xCEE9B5,
    fg: 0x181B15,
    muted_fg: 0x5E6658,
    accent: 0x537921,
    accent_fg: 0xFFFFFF,
    syntax: Syntax {
        comment: 0x777F70,
        punct: 0x5A6055,
        ..GIBBON_LIGHT.syntax
    },
    ..GIBBON_LIGHT
};

const LAGOON_DARK: Palette = Palette {
    bg: 0x121616,
    sidebar: 0x0C0F0E,
    elevated: 0x1B2020,
    border: 0x293232,
    input: 0x3C4846,
    hover: 0xDBF9F617,
    highlight: 0xDBF9F62E,
    selection: 0x143D3B,
    fg: 0xE4F0EF,
    muted_fg: 0x8FA2A1,
    accent: 0x04D1C9,
    accent_fg: 0x021816,
    syntax: Syntax {
        comment: 0x697978,
        punct: 0x899998,
        ..GIBBON_DARK.syntax
    },
    ..GIBBON_DARK
};

const LAGOON_LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    sidebar: 0xE7F3F2,
    elevated: 0xFFFFFF,
    border: 0xC7DAD8,
    input: 0xA9BFBD,
    hover: 0x14514E17,
    highlight: 0x14514E1C,
    selection: 0xA2EDE8,
    fg: 0x141C1C,
    muted_fg: 0x546765,
    accent: 0x017A76,
    accent_fg: 0xFFFFFF,
    syntax: Syntax {
        comment: 0x6D807F,
        punct: 0x516160,
        ..GIBBON_LIGHT.syntax
    },
    ..GIBBON_LIGHT
};

const ORCHID_DARK: Palette = Palette {
    bg: 0x171517,
    sidebar: 0x0F0D0F,
    elevated: 0x221E23,
    border: 0x343036,
    input: 0x49434A,
    hover: 0xFBEFFF17,
    highlight: 0xFBEFFF2E,
    selection: 0x433249,
    fg: 0xF2ECF4,
    muted_fg: 0xA49BA8,
    accent: 0xE7A1FD,
    accent_fg: 0x1B111E,
    syntax: Syntax {
        comment: 0x7C737E,
        punct: 0x9B939E,
        ..GIBBON_DARK.syntax
    },
    ..GIBBON_DARK
};

const ORCHID_LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    sidebar: 0xF5EEF7,
    elevated: 0xFFFFFF,
    border: 0xDCD3DF,
    input: 0xC3B8C7,
    hover: 0x59406117,
    highlight: 0x5940611C,
    selection: 0xF5D6FE,
    fg: 0x1E191F,
    muted_fg: 0x6A606D,
    accent: 0x8F59A0,
    accent_fg: 0xFFFFFF,
    syntax: Syntax {
        comment: 0x837987,
        punct: 0x645B66,
        ..GIBBON_LIGHT.syntax
    },
    ..GIBBON_LIGHT
};

pub fn palette(cx: &App) -> &'static Palette {
    use crate::settings::Appearance;
    let scheme = current(cx);
    let light = match crate::settings::get(cx).appearance {
        Appearance::Light => true,
        Appearance::Dark => false,
        Appearance::System => matches!(
            cx.window_appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    };
    if light { &scheme.light } else { &scheme.dark }
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

#[cfg(test)]
mod tests {
    // Not `super::*`: the GPUI glob would shadow the built-in #[test].
    use super::{DEFAULT, Palette, SCHEMES};

    fn luminance(c: u32) -> f64 {
        let channel = |shift: u32| {
            let v = ((c >> shift) & 0xFF) as f64 / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    /// The WCAG contrast ratio of two `0xRRGGBB` colors.
    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn scheme_ids_are_unique_and_include_the_default() {
        for (i, s) in SCHEMES.iter().enumerate() {
            assert!(SCHEMES[..i].iter().all(|o| o.id != s.id), "{}", s.id);
        }
        assert!(SCHEMES.iter().any(|s| s.id == DEFAULT));
    }

    #[test]
    fn every_scheme_keeps_the_contrast() {
        let check = |name: &str, p: &Palette| {
            let pairs = [
                ("fg on bg", p.fg, p.bg, 12.),
                ("fg on sidebar", p.fg, p.sidebar, 12.),
                ("fg on selection", p.fg, p.selection, 7.),
                ("muted on bg", p.muted_fg, p.bg, 4.5),
                ("muted on sidebar", p.muted_fg, p.sidebar, 4.5),
                ("accent on bg", p.accent, p.bg, 4.5),
                ("text on accent", p.accent_fg, p.accent, 4.5),
            ];
            for (what, a, b, min) in pairs {
                let ratio = contrast(a, b);
                assert!(ratio >= min, "{name}: {what} is {ratio:.2}, below {min}");
            }
        };
        for s in SCHEMES {
            check(&format!("{} dark", s.id), &s.dark);
            check(&format!("{} light", s.id), &s.light);
        }
    }
}
