//! TUI theme colors.

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
}

impl Theme {
    pub fn from_name(name: ThemeName) -> Theme {
        match name {
            ThemeName::Dark => Theme {
                name,
                bg: Color::Rgb(10, 11, 15),
                fg: Color::Rgb(226, 228, 236),
                dim: Color::Rgb(126, 131, 146),
                accent: Color::Rgb(97, 175, 239),
                border: Color::Rgb(56, 60, 74),
                warn: Color::Rgb(232, 166, 82),
                good: Color::Rgb(110, 190, 130),
                panel_bg: Color::Rgb(15, 17, 23),
                header_bg: Color::Rgb(23, 26, 34),
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
            },
        }
    }

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
