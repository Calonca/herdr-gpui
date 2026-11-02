//! Herdr UI's theme, as the WASM protocol states it to a module and as the
//! renderer paints its components.

use crate::config::{Theme, mix};
use gpui::{Hsla, Rgba, rgb};

/// The colors the renderer paints with, from the window's theme.
pub(crate) struct Colors {
    pub bg: Hsla,
    pub fg: Hsla,
    /// Secondary text.
    pub muted: Hsla,
    /// Hints and small print.
    pub dim: Hsla,
    pub accent: Hsla,
    /// Text on an accent fill.
    pub accent_fg: Hsla,
    pub border: Hsla,
    pub surface: Hsla,
    pub popover: Hsla,
    /// A raised control: a secondary button, a pill, a key cap.
    pub raised: Hsla,
    /// A field's border.
    pub input: Hsla,
    /// A soft selection.
    pub selected: Hsla,
    pub hover: Hsla,
    /// A primary button's fill and text.
    pub solid: Hsla,
    pub solid_fg: Hsla,
    /// The theme's first eight ANSI colors, as marks on its chrome.
    pub ansi: [Hsla; 8],
}

impl Colors {
    pub(crate) fn of(theme: &Theme) -> Self {
        let primary = theme.primary();
        Self {
            bg: rgb(theme.background).into(),
            fg: rgb(theme.foreground).into(),
            muted: rgb(theme.subtext()).into(),
            dim: rgb(theme.muted).into(),
            accent: rgb(primary).into(),
            accent_fg: rgb(theme.text_on(primary)).into(),
            border: rgb(theme.active).into(),
            surface: rgb(theme.surface).into(),
            popover: rgb(theme.surface).into(),
            raised: rgb(theme.active).into(),
            input: rgb(mix(theme.active, theme.foreground, 15)).into(),
            selected: rgb(theme.primary_wash()).into(),
            hover: rgb(theme.active).into(),
            solid: rgb(primary).into(),
            solid_fg: rgb(theme.text_on(primary)).into(),
            ansi: std::array::from_fn(|index| rgb(theme.ink(theme.palette[index])).into()),
        }
    }
}

/// `0xRRGGBBAA`, the protocol's color, as GPUI's.
pub(crate) fn hsla(color: u32) -> Hsla {
    let channel = |shift: u32| ((color >> shift) & 0xff) as f32 / 255.;
    Rgba {
        r: channel(24),
        g: channel(16),
        b: channel(8),
        a: channel(0),
    }
    .into()
}

/// `0xRRGGBB`, opaque, as the protocol's `0xRRGGBBAA`.
fn opaque(color: u32) -> u32 {
    (color & 0xff_ff_ff) << 8 | 0xff
}

/// The theme a module is told, so what it draws itself (a canvas, its own
/// colors) matches. `name` is the configured theme's.
pub(crate) fn protocol(theme: &Theme, name: &str, mono: &str) -> wasm_mods_protocol::Theme {
    wasm_mods_protocol::Theme {
        name: name.to_owned(),
        space: opaque(theme.background),
        surface: opaque(theme.surface),
        text: opaque(theme.foreground),
        subtext: opaque(theme.subtext()),
        muted: opaque(theme.muted),
        accent: opaque(theme.primary()),
        border: opaque(theme.active),
        ansi: theme.palette[..16].iter().copied().map(opaque).collect(),
        density: Default::default(),
        fonts: wasm_mods_protocol::Fonts {
            ui: String::new(),
            mono: mono.to_owned(),
        },
    }
}
