//! Two palettes and two frames, both switchable at runtime.
//!
//! The layout underneath is identical either way — this only decides which
//! borders are drawn and what colour things are — so it stays a preference
//! rather than a rebuild.

use ratatui::style::Color;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    /// Siddran's warm identity.
    Ember,
    /// yazi's blue/violet.
    Yazi,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// A border with curved corners, the way yazi draws it.
    Rounded,
    /// The same border with square corners.
    Square,
    /// No border at all — panes flush to the edges.
    Flat,
}

impl Frame {
    pub fn parse(s: &str) -> Frame {
        match s {
            "flat" => Frame::Flat,
            "square" | "boxed" => Frame::Square,
            _ => Frame::Rounded,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Frame::Rounded => "rounded",
            Frame::Square => "square",
            Frame::Flat => "flat",
        }
    }

    pub fn boxed(self) -> bool {
        self != Frame::Flat
    }

    pub fn border_type(self) -> ratatui::widgets::BorderType {
        use ratatui::widgets::BorderType;
        match self {
            Frame::Rounded => BorderType::Rounded,
            _ => BorderType::Plain,
        }
    }
}

#[allow(dead_code)]
pub struct Theme {
    /// Unused as a fill — kept so a future opaque theme has somewhere to put a
    /// background. Deliberately `Color::Reset` in both palettes: the terminal's own
    /// background (and its transparency, blur, or image) shows through, the way
    /// yazi behaves. Painting a solid colour here is what made the app look like
    /// a box pasted over the terminal rather than part of it.
    pub bg: Color,
    pub panel: Color,
    pub line: Color,
    pub fg: Color,
    pub dim: Color,
    pub dimmer: Color,
    pub accent: Color,
    pub flame: Color,
    pub teal: Color,
    pub violet: Color,
    pub sel: Color,
    pub on_accent: Color,
    pub frame: Frame,
}

impl Theme {
    pub fn new(p: Palette, frame: Frame) -> Theme {
        match p {
            Palette::Ember => Theme {
                bg: Color::Reset,
                panel: Color::Reset,
                line: Color::Rgb(0x4a, 0x40, 0x32),
                fg: Color::Rgb(0xdd, 0xd3, 0xc4),
                dim: Color::Rgb(0x8a, 0x7f, 0x70),
                dimmer: Color::Rgb(0x5d, 0x55, 0x48),
                accent: Color::Rgb(0xf0, 0xa3, 0x5e),
                flame: Color::Rgb(0xe2, 0x6a, 0x5a),
                teal: Color::Rgb(0x5f, 0xb8, 0xa8),
                violet: Color::Rgb(0x9c, 0x7a, 0xd8),
                sel: Color::Rgb(0x31, 0x28, 0x1c),
                on_accent: Color::Rgb(0x1a, 0x14, 0x09),
                frame,
            },
            Palette::Yazi => Theme {
                bg: Color::Reset,
                panel: Color::Reset,
                line: Color::Rgb(0x54, 0x5c, 0x7e),
                fg: Color::Rgb(0xc0, 0xca, 0xf5),
                dim: Color::Rgb(0x7f, 0x86, 0xab),
                dimmer: Color::Rgb(0x56, 0x5f, 0x89),
                accent: Color::Rgb(0x7a, 0xa2, 0xf7),
                flame: Color::Rgb(0xbb, 0x9a, 0xf7),
                teal: Color::Rgb(0x7d, 0xcf, 0xff),
                violet: Color::Rgb(0x9e, 0xce, 0x6a),
                sel: Color::Rgb(0x28, 0x34, 0x57),
                on_accent: Color::Rgb(0x1a, 0x1b, 0x26),
                frame,
            },
        }
    }
}
