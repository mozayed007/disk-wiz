//! TUI theme colors.
//!
//! The default `auto` theme uses the terminal's own palette (reset background,
//! ANSI accent colors) so the interface inherits whatever scheme the user
//! runs. `dark` and `light` are explicit RGB themes for people who want the
//! chrome to have a fixed look.

use ratatui::style::Color;

use crate::color::Rgb;
use crate::config::ThemeName;

pub fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// Resolved colors for the interface chrome (not treemap cells).
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: ThemeName,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub warn: Color,
    pub good: Color,
    pub panel_bg: Color,
    pub header_bg: Color,
    /// Foreground for text on top of `accent` (chips, active tabs).
    pub chip_fg: Color,
}

impl Theme {
    pub fn from_name(name: ThemeName) -> Theme {
        match name {
            ThemeName::Auto => Theme {
                name,
                bg: Color::Reset,
                fg: Color::Reset,
                dim: Color::DarkGray,
                accent: Color::LightBlue,
                border: Color::DarkGray,
                warn: Color::LightYellow,
                good: Color::LightGreen,
                panel_bg: Color::Reset,
                header_bg: Color::Reset,
                chip_fg: Color::Black,
            },
            ThemeName::Dark => Theme {
                name,
                bg: Color::Rgb(10, 11, 15),
                fg: Color::Rgb(226, 228, 236),
                dim: Color::Rgb(126, 131, 146),
                accent: Color::Rgb(97, 175, 239),
                border: Color::Rgb(52, 56, 70),
                warn: Color::Rgb(232, 166, 82),
                good: Color::Rgb(110, 190, 130),
                panel_bg: Color::Rgb(15, 17, 23),
                header_bg: Color::Rgb(23, 26, 34),
                chip_fg: Color::Rgb(10, 11, 15),
            },
            ThemeName::Light => Theme {
                name,
                bg: Color::Rgb(249, 249, 252),
                fg: Color::Rgb(28, 30, 38),
                dim: Color::Rgb(110, 114, 126),
                accent: Color::Rgb(30, 96, 200),
                border: Color::Rgb(198, 200, 210),
                warn: Color::Rgb(190, 110, 20),
                good: Color::Rgb(30, 130, 70),
                panel_bg: Color::Rgb(240, 241, 246),
                header_bg: Color::Rgb(232, 234, 240),
                chip_fg: Color::Rgb(249, 249, 252),
            },
        }
    }

    /// True only for the explicit light theme; `auto` assumes a dark terminal
    /// when deciding how to invert sequential ramps.
    pub fn is_light(&self) -> bool {
        self.name == ThemeName::Light
    }

    /// Brighten (or darken, for light themes) an RGB color.
    pub fn emphasize(&self, c: Rgb, amount: f64) -> Color {
        let f = |x: u8| {
            let v = x as f64;
            let out = if self.is_light() {
                v * (1.0 - amount)
            } else {
                v + (255.0 - v) * amount
            };
            out.round().clamp(0.0, 255.0) as u8
        };
        Color::Rgb(f(c.0), f(c.1), f(c.2))
    }
}
