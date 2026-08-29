//! Colours and terminal colour degradation.

/// How many colours the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    /// 24-bit direct colour.
    #[default]
    TrueColor,
    /// The xterm 256-colour palette.
    Ansi256,
    /// The original sixteen colours.
    Ansi16,
}

impl ColorDepth {
    /// Reads the depth from `COLORTERM` and `TERM`, the way terminal programs
    /// conventionally detect it.
    pub fn from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
        if matches!(colorterm, Some("truecolor") | Some("24bit")) {
            return ColorDepth::TrueColor;
        }
        match term {
            Some(t) if t.contains("256color") || t.contains("direct") => ColorDepth::Ansi256,
            Some(t) if t.contains("kitty") || t.contains("alacritty") || t.contains("wezterm") => {
                ColorDepth::TrueColor
            }
            _ => ColorDepth::Ansi16,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ColorError {
    #[error("`{0}` is not a colour: expected #rgb, #rrggbb, a palette index, or a colour name")]
    Unrecognised(String),
    #[error("palette index {0} is out of range (0-255)")]
    IndexOutOfRange(u32),
}

/// A colour as written in a theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    /// The terminal's own foreground or background.
    #[default]
    Default,
    /// An index into the terminal palette.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The sixteen ANSI colour names, in palette order.
const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

impl Color {
    /// Parses `#rgb`, `#rrggbb`, a bare palette index, or a colour name.
    pub fn parse(text: &str) -> Result<Color, ColorError> {
        let t = text.trim();
        if t.eq_ignore_ascii_case("default") || t.eq_ignore_ascii_case("none") {
            return Ok(Color::Default);
        }
        if let Some(hex) = t.strip_prefix('#') {
            return Self::parse_hex(hex).ok_or_else(|| ColorError::Unrecognised(text.to_string()));
        }
        if let Ok(n) = t.parse::<u32>() {
            return u8::try_from(n)
                .map(Color::Indexed)
                .map_err(|_| ColorError::IndexOutOfRange(n));
        }
        // `grey`/`gray` are the usual aliases for bright black.
        let normalised = t.to_ascii_lowercase().replace('_', "-");
        let normalised = match normalised.as_str() {
            "grey" | "gray" => "bright-black".to_string(),
            // Accept the `brightblue` spelling as well as `bright-blue`.
            other => other.strip_prefix("bright").map_or_else(
                || other.to_string(),
                |rest| format!("bright-{}", rest.trim_start_matches('-')),
            ),
        };
        ANSI_NAMES
            .iter()
            .position(|n| *n == normalised)
            .map(|i| Color::Indexed(i as u8))
            .ok_or_else(|| ColorError::Unrecognised(text.to_string()))
    }

    fn parse_hex(hex: &str) -> Option<Color> {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        match digits.len() {
            // `#rgb` expands each digit, so `#f0a` is `#ff00aa`.
            3 => Some(Color::Rgb(digits[0] * 17, digits[1] * 17, digits[2] * 17)),
            6 => Some(Color::Rgb(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            )),
            _ => None,
        }
    }

    /// Renders the colour the way a theme would write it.
    pub fn notation(&self) -> String {
        match self {
            Color::Default => "default".into(),
            Color::Indexed(i) if (*i as usize) < ANSI_NAMES.len() => ANSI_NAMES[*i as usize].into(),
            Color::Indexed(i) => i.to_string(),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }

    /// The RGB triple this colour shows as, resolving palette entries through
    /// the standard xterm palette.
    pub fn to_rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            Color::Default => None,
            Color::Rgb(r, g, b) => Some((r, g, b)),
            Color::Indexed(i) => Some(xterm_palette_rgb(i)),
        }
    }

    /// Reduces the colour to what `depth` can display.
    pub fn degrade(self, depth: ColorDepth) -> Color {
        match (self, depth) {
            (Color::Default, _) | (_, ColorDepth::TrueColor) => self,
            (Color::Rgb(r, g, b), ColorDepth::Ansi256) => Color::Indexed(rgb_to_ansi256(r, g, b)),
            (Color::Rgb(r, g, b), ColorDepth::Ansi16) => Color::Indexed(rgb_to_ansi16(r, g, b)),
            (Color::Indexed(i), ColorDepth::Ansi16) if i >= 16 => {
                let (r, g, b) = xterm_palette_rgb(i);
                Color::Indexed(rgb_to_ansi16(r, g, b))
            }
            (Color::Indexed(_), _) => self,
        }
    }

    /// Relative luminance, used to decide whether a background is dark.
    pub fn luminance(self) -> f32 {
        let Some((r, g, b)) = self.to_rgb() else {
            return 0.0;
        };
        // Rec. 601 luma, good enough for a light/dark decision.
        (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
    }

    pub fn is_dark(self) -> bool {
        self.luminance() < 0.5
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.notation())
    }
}

impl std::str::FromStr for Color {
    type Err = ColorError;
    fn from_str(s: &str) -> Result<Color, ColorError> {
        Color::parse(s)
    }
}

impl From<Color> for crossterm::style::Color {
    fn from(c: Color) -> crossterm::style::Color {
        use crossterm::style::Color as Ct;
        match c {
            Color::Default => Ct::Reset,
            Color::Indexed(i) => Ct::AnsiValue(i),
            Color::Rgb(r, g, b) => Ct::Rgb { r, g, b },
        }
    }
}

/// The standard sixteen ANSI colours as xterm renders them.
const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The RGB value xterm assigns to palette index `i`.
pub fn xterm_palette_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI16_RGB[i as usize],
        // The 6x6x6 colour cube.
        16..=231 => {
            let i = i as u32 - 16;
            let level = |v: u32| if v == 0 { 0u8 } else { (55 + v * 40) as u8 };
            (level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        // The 24-step greyscale ramp.
        232..=255 => {
            let v = 8 + (i as u32 - 232) * 10;
            (v as u8, v as u8, v as u8)
        }
    }
}

/// Nearest xterm-256 palette entry for an RGB triple.
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    // Greys that fall between cube levels are better served by the ramp.
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + ((r as u32 - 8) / 10) as u8;
    }
    let level = |v: u8| -> u32 {
        // Cube levels are 0, 95, 135, 175, 215, 255.
        const LEVELS: [u32; 6] = [0, 95, 135, 175, 215, 255];
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| l.abs_diff(v as u32))
            .map(|(i, _)| i as u32)
            .expect("LEVELS is non-empty")
    };
    (16 + 36 * level(r) + 6 * level(g) + level(b)) as u8
}

/// Nearest of the sixteen ANSI colours for an RGB triple.
pub fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> u8 {
    ANSI16_RGB
        .iter()
        .enumerate()
        .min_by_key(|(_, (cr, cg, cb))| {
            // Squared distance in RGB space; adequate for a 16-colour fallback.
            let d = |a: u8, b: u8| {
                let d = a as i32 - b as i32;
                d * d
            };
            d(*cr, r) + d(*cg, g) + d(*cb, b)
        })
        .map(|(i, _)| i as u8)
        .expect("ANSI16_RGB is non-empty")
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn six_digit_hex_parses() {
        assert_eq!(
            Color::parse("#1d1f21").unwrap(),
            Color::Rgb(0x1d, 0x1f, 0x21)
        );
        assert_eq!(Color::parse("#FFFFFF").unwrap(), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn three_digit_hex_expands_each_digit() {
        assert_eq!(Color::parse("#f0a").unwrap(), Color::Rgb(0xff, 0x00, 0xaa));
        assert_eq!(Color::parse("#fff").unwrap(), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn palette_indices_parse() {
        assert_eq!(Color::parse("0").unwrap(), Color::Indexed(0));
        assert_eq!(Color::parse("255").unwrap(), Color::Indexed(255));
        assert_eq!(Color::parse("256"), Err(ColorError::IndexOutOfRange(256)));
    }

    #[test]
    fn colour_names_parse_in_their_usual_spellings() {
        assert_eq!(Color::parse("red").unwrap(), Color::Indexed(1));
        assert_eq!(Color::parse("BLUE").unwrap(), Color::Indexed(4));
        assert_eq!(Color::parse("bright-blue").unwrap(), Color::Indexed(12));
        assert_eq!(Color::parse("brightblue").unwrap(), Color::Indexed(12));
        assert_eq!(Color::parse("bright_blue").unwrap(), Color::Indexed(12));
        assert_eq!(Color::parse("grey").unwrap(), Color::Indexed(8));
        assert_eq!(Color::parse("gray").unwrap(), Color::Indexed(8));
    }

    #[test]
    fn default_means_the_terminals_own_colour() {
        assert_eq!(Color::parse("default").unwrap(), Color::Default);
        assert_eq!(Color::parse("none").unwrap(), Color::Default);
        assert_eq!(Color::Default.to_rgb(), None);
    }

    #[test]
    fn malformed_colours_are_rejected() {
        assert!(matches!(
            Color::parse("#12345"),
            Err(ColorError::Unrecognised(_))
        ));
        assert!(matches!(
            Color::parse("#gggggg"),
            Err(ColorError::Unrecognised(_))
        ));
        assert!(matches!(
            Color::parse("chartreuse"),
            Err(ColorError::Unrecognised(_))
        ));
        assert!(matches!(Color::parse(""), Err(ColorError::Unrecognised(_))));
    }

    #[test]
    fn colours_round_trip_through_their_notation() {
        for text in ["#1d1f21", "red", "bright-white", "200", "default"] {
            let c = Color::parse(text).unwrap();
            assert_eq!(c.notation(), text, "`{text}` should round-trip");
            assert_eq!(Color::parse(&c.notation()).unwrap(), c);
        }
    }

    #[test]
    fn the_xterm_palette_matches_known_entries() {
        assert_eq!(xterm_palette_rgb(0), (0, 0, 0));
        assert_eq!(xterm_palette_rgb(16), (0, 0, 0), "cube origin");
        assert_eq!(xterm_palette_rgb(21), (0, 0, 255), "cube blue");
        assert_eq!(xterm_palette_rgb(231), (255, 255, 255), "cube white");
        assert_eq!(xterm_palette_rgb(232), (8, 8, 8), "greyscale start");
        assert_eq!(xterm_palette_rgb(255), (238, 238, 238), "greyscale end");
    }

    #[test]
    fn rgb_maps_onto_the_256_colour_cube() {
        assert_eq!(rgb_to_ansi256(255, 0, 0), 196);
        assert_eq!(rgb_to_ansi256(0, 255, 0), 46);
        assert_eq!(rgb_to_ansi256(0, 0, 255), 21);
    }

    #[test]
    fn greys_use_the_greyscale_ramp() {
        let c = rgb_to_ansi256(128, 128, 128);
        assert!(
            (232..=255).contains(&c),
            "expected a greyscale entry, got {c}"
        );
        assert_eq!(
            rgb_to_ansi256(0, 0, 0),
            16,
            "pure black uses the cube origin"
        );
    }

    #[test]
    fn rgb_maps_onto_the_sixteen_colour_palette() {
        assert_eq!(rgb_to_ansi16(0, 0, 0), 0);
        assert_eq!(rgb_to_ansi16(255, 255, 255), 15);
        assert_eq!(rgb_to_ansi16(250, 10, 10), 9, "bright red");
    }

    #[test]
    fn degrading_reduces_only_what_the_depth_requires() {
        let c = Color::Rgb(0xb2, 0x94, 0xbb);
        assert_eq!(c.degrade(ColorDepth::TrueColor), c);
        assert!(matches!(c.degrade(ColorDepth::Ansi256), Color::Indexed(_)));
        let sixteen = c.degrade(ColorDepth::Ansi16);
        assert!(matches!(sixteen, Color::Indexed(i) if i < 16));
    }

    #[test]
    fn degrading_default_never_picks_a_colour() {
        assert_eq!(Color::Default.degrade(ColorDepth::Ansi16), Color::Default);
    }

    #[test]
    fn high_palette_indices_degrade_to_the_base_sixteen() {
        assert!(
            matches!(Color::Indexed(196).degrade(ColorDepth::Ansi16), Color::Indexed(i) if i < 16)
        );
        assert_eq!(
            Color::Indexed(3).degrade(ColorDepth::Ansi16),
            Color::Indexed(3),
            "already low"
        );
        assert_eq!(
            Color::Indexed(196).degrade(ColorDepth::Ansi256),
            Color::Indexed(196)
        );
    }

    #[test]
    fn luminance_separates_light_from_dark() {
        assert!(Color::Rgb(0x1d, 0x1f, 0x21).is_dark());
        assert!(!Color::Rgb(0xff, 0xff, 0xff).is_dark());
        assert!(Color::Rgb(0, 0, 0).luminance() < Color::Rgb(255, 255, 255).luminance());
    }

    #[test]
    fn colour_depth_is_detected_from_the_environment() {
        assert_eq!(
            ColorDepth::from_env(Some("truecolor"), None),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_env(Some("24bit"), Some("dumb")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_env(None, Some("xterm-256color")),
            ColorDepth::Ansi256
        );
        assert_eq!(
            ColorDepth::from_env(None, Some("xterm-kitty")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_env(None, Some("vt100")),
            ColorDepth::Ansi16
        );
        assert_eq!(ColorDepth::from_env(None, None), ColorDepth::Ansi16);
    }

    #[test]
    fn colours_convert_to_crossterm() {
        use crossterm::style::Color as Ct;
        assert_eq!(Ct::from(Color::Default), Ct::Reset);
        assert_eq!(Ct::from(Color::Indexed(9)), Ct::AnsiValue(9));
        assert_eq!(Ct::from(Color::Rgb(1, 2, 3)), Ct::Rgb { r: 1, g: 2, b: 3 });
    }
}
