//! A face: colours plus text attributes.

use crate::color::{Color, ColorDepth};
use crossterm::style::{Attribute, Attributes as CtAttributes, ContentStyle};

/// Text attributes. Each is tri-state: unset attributes are inherited rather
/// than forced off, which is what lets `error` inherit from `default` and only
/// add an underline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attributes {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// A wavy underline — what a spell-checker puts under a misspelling
    /// and an editor under an error — which needs no `underline` to go
    /// with it. A terminal that cannot draw one draws a plain underline.
    pub undercurl: Option<bool>,
    pub reverse: Option<bool>,
    pub dim: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl Attributes {
    /// All attributes explicitly off, as the `default` face has them.
    pub fn none() -> Attributes {
        Attributes {
            bold: Some(false),
            italic: Some(false),
            underline: Some(false),
            undercurl: Some(false),
            reverse: Some(false),
            dim: Some(false),
            strikethrough: Some(false),
        }
    }

    pub fn is_unset(&self) -> bool {
        *self == Attributes::default()
    }

    /// Fills unset attributes from `parent`.
    pub fn inherit_from(&mut self, parent: &Attributes) {
        macro_rules! fill {
            ($($f:ident),*) => {$( if self.$f.is_none() { self.$f = parent.$f; } )*};
        }
        fill!(
            bold,
            italic,
            underline,
            undercurl,
            reverse,
            dim,
            strikethrough
        );
    }

    /// Overlays `other`'s set attributes onto this one.
    pub fn overlay(&mut self, other: &Attributes) {
        macro_rules! take {
            ($($f:ident),*) => {$( if other.$f.is_some() { self.$f = other.$f; } )*};
        }
        take!(
            bold,
            italic,
            underline,
            undercurl,
            reverse,
            dim,
            strikethrough
        );
    }

    /// True when there is a line of some kind under the text.
    pub fn underlined(&self) -> bool {
        self.underline == Some(true) || self.undercurl == Some(true)
    }

    /// `curly` says whether the terminal draws a wavy underline; where it
    /// does not, the wave is a plain underline rather than nothing.
    fn to_crossterm(self, curly: bool) -> CtAttributes {
        let mut out = CtAttributes::none();
        if self.bold == Some(true) {
            out.set(Attribute::Bold);
        }
        if self.italic == Some(true) {
            out.set(Attribute::Italic);
        }
        match (self.undercurl == Some(true) && curly, self.underlined()) {
            (true, _) => out.set(Attribute::Undercurled),
            (false, true) => out.set(Attribute::Underlined),
            (false, false) => {}
        }
        if self.reverse == Some(true) {
            out.set(Attribute::Reverse);
        }
        if self.dim == Some(true) {
            out.set(Attribute::Dim);
        }
        if self.strikethrough == Some(true) {
            out.set(Attribute::CrossedOut);
        }
        out
    }
}

/// A resolved face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Face {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub attributes: Attributes,
}

impl Face {
    pub fn new() -> Face {
        Face::default()
    }

    /// A face with just a foreground colour, the common case in a theme.
    pub fn fg(color: Color) -> Face {
        Face {
            foreground: Some(color),
            ..Default::default()
        }
    }

    /// A face with just a background colour.
    pub fn bg(color: Color) -> Face {
        Face {
            background: Some(color),
            ..Default::default()
        }
    }

    pub fn with_fg(mut self, color: Color) -> Face {
        self.foreground = Some(color);
        self
    }

    pub fn with_bg(mut self, color: Color) -> Face {
        self.background = Some(color);
        self
    }

    pub fn bold(mut self) -> Face {
        self.attributes.bold = Some(true);
        self
    }

    pub fn italic(mut self) -> Face {
        self.attributes.italic = Some(true);
        self
    }

    pub fn underline(mut self) -> Face {
        self.attributes.underline = Some(true);
        self
    }

    /// A wavy underline, on its own: it needs no `underline` beside it.
    pub fn undercurl(mut self) -> Face {
        self.attributes.undercurl = Some(true);
        self
    }

    pub fn reverse(mut self) -> Face {
        self.attributes.reverse = Some(true);
        self
    }

    pub fn dim(mut self) -> Face {
        self.attributes.dim = Some(true);
        self
    }

    pub fn strikethrough(mut self) -> Face {
        self.attributes.strikethrough = Some(true);
        self
    }

    /// True when the face specifies nothing and would render as `default`.
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none() && self.background.is_none() && self.attributes.is_unset()
    }

    /// Fills unset fields from `parent`, as Emacs' `:inherit` does.
    pub fn inherit_from(&mut self, parent: &Face) {
        if self.foreground.is_none() {
            self.foreground = parent.foreground;
        }
        if self.background.is_none() {
            self.background = parent.background;
        }
        self.attributes.inherit_from(&parent.attributes);
    }

    /// Draws `other` on top of this face; `other`'s set fields win. This is how
    /// overlapping display properties combine — a region highlight over syntax
    /// colouring, for instance.
    pub fn overlay(&mut self, other: &Face) {
        if other.foreground.is_some() {
            self.foreground = other.foreground;
        }
        if other.background.is_some() {
            self.background = other.background;
        }
        self.attributes.overlay(&other.attributes);
    }

    /// A copy of this face with `other` overlaid.
    pub fn merged(&self, other: &Face) -> Face {
        let mut out = *self;
        out.overlay(other);
        out
    }

    /// Reduces both colours to what `depth` can display.
    pub fn degrade(&self, depth: ColorDepth) -> Face {
        Face {
            foreground: self.foreground.map(|c| c.degrade(depth)),
            background: self.background.map(|c| c.degrade(depth)),
            attributes: self.attributes,
        }
    }

    /// The crossterm style used to draw text in this face, for a terminal
    /// that draws no wavy underline.
    pub fn to_style(&self, depth: ColorDepth) -> ContentStyle {
        self.to_style_with(depth, false)
    }

    /// The crossterm style used to draw text in this face; `curly` says
    /// whether the terminal draws a wavy underline, and a face's undercurl
    /// is a plain underline where it does not.
    pub fn to_style_with(&self, depth: ColorDepth, curly: bool) -> ContentStyle {
        let face = self.degrade(depth);
        ContentStyle {
            foreground_color: face.foreground.map(Into::into),
            background_color: face.background.map(Into::into),
            underline_color: None,
            attributes: face.attributes.to_crossterm(curly),
        }
    }
}

/// Whether the terminal draws a wavy underline when asked for one with
/// `SGR 4:3`, judged from its environment the way its colours are:
/// `var` looks a variable up. A terminal that does not understand the
/// request may show nothing at all under the text, so the answer is yes
/// only for those known to — and for tmux, which turns it into whatever
/// the terminal outside it can do.
pub fn terminal_draws_undercurl(var: impl Fn(&str) -> Option<String>) -> bool {
    let term = var("TERM").unwrap_or_default();
    let program = var("TERM_PROGRAM").unwrap_or_default();
    let curly_terms = [
        "kitty",
        "foot",
        "wezterm",
        "alacritty",
        "contour",
        "ghostty",
        "tmux",
    ];
    if curly_terms.iter().any(|name| term.contains(name)) {
        return true;
    }
    let curly_programs = ["WezTerm", "iTerm.app", "ghostty", "vscode", "Hyper", "rio"];
    if curly_programs.iter().any(|name| program == *name) {
        return true;
    }
    // VTE (GNOME's, and most of the desktop's) from 0.52; Konsole from
    // 22.04; Windows Terminal from 1.13, which is any of them still in use.
    let version = |name: &str| var(name).and_then(|v| v.parse::<u64>().ok());
    version("VTE_VERSION").is_some_and(|v| v >= 5200)
        || version("KONSOLE_VERSION").is_some_and(|v| v >= 220400)
        || var("WT_SESSION").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_compose() {
        let f = Face::fg(Color::Indexed(1))
            .with_bg(Color::Indexed(0))
            .bold()
            .underline();
        assert_eq!(f.foreground, Some(Color::Indexed(1)));
        assert_eq!(f.background, Some(Color::Indexed(0)));
        assert_eq!(f.attributes.bold, Some(true));
        assert_eq!(f.attributes.underline, Some(true));
        assert_eq!(f.attributes.italic, None, "unset, not off");
    }

    #[test]
    fn an_undercurl_is_a_wave_where_the_terminal_draws_one_and_a_line_elsewhere() {
        let face = Face::fg(Color::Indexed(1)).undercurl();
        assert!(face.attributes.underlined(), "a wave is an underline");
        assert_eq!(face.attributes.underline, None, "but not the plain kind");
        let curly = face.to_style_with(ColorDepth::Ansi16, true).attributes;
        assert!(curly.has(Attribute::Undercurled));
        assert!(!curly.has(Attribute::Underlined));
        let plain = face.to_style(ColorDepth::Ansi16).attributes;
        assert!(plain.has(Attribute::Underlined));
        assert!(!plain.has(Attribute::Undercurled));
        // Both asked for: the wave, where it can be had.
        let both = Face::new().underline().undercurl();
        assert!(
            both.to_style_with(ColorDepth::Ansi16, true)
                .attributes
                .has(Attribute::Undercurled)
        );
        // Inherited and overlaid like the rest.
        let mut child = Face::fg(Color::Indexed(2));
        child.inherit_from(&face);
        assert_eq!(child.attributes.undercurl, Some(true));
        let mut off = face;
        off.overlay(&Face {
            attributes: Attributes {
                undercurl: Some(false),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(!off.attributes.underlined());
    }

    #[test]
    fn the_terminals_that_curl_are_known_by_name_or_version() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert!(terminal_draws_undercurl(env(&[("TERM", "xterm-kitty")])));
        assert!(terminal_draws_undercurl(env(&[("TERM", "foot-extra")])));
        assert!(terminal_draws_undercurl(env(&[("TERM", "tmux-256color")])));
        assert!(terminal_draws_undercurl(env(&[
            ("TERM", "xterm-256color"),
            ("TERM_PROGRAM", "WezTerm")
        ])));
        assert!(terminal_draws_undercurl(env(&[
            ("TERM", "xterm-256color"),
            ("VTE_VERSION", "7602")
        ])));
        assert!(terminal_draws_undercurl(env(&[(
            "KONSOLE_VERSION",
            "230800"
        )])));
        assert!(terminal_draws_undercurl(env(&[("WT_SESSION", "x")])));
        assert!(!terminal_draws_undercurl(env(&[(
            "TERM",
            "xterm-256color"
        )])));
        assert!(!terminal_draws_undercurl(env(&[("TERM", "screen")])));
        assert!(!terminal_draws_undercurl(env(&[("VTE_VERSION", "4800")])));
        assert!(!terminal_draws_undercurl(env(&[])));
    }

    #[test]
    fn an_empty_face_is_recognised() {
        assert!(Face::new().is_empty());
        assert!(
            !Face::fg(Color::Default).is_empty(),
            "an explicit default is still set"
        );
        assert!(!Face::new().bold().is_empty());
    }

    #[test]
    fn inheritance_fills_only_unset_fields() {
        let parent = Face::fg(Color::Rgb(1, 1, 1))
            .with_bg(Color::Rgb(2, 2, 2))
            .bold();
        let mut child = Face::new().italic();
        child.inherit_from(&parent);
        assert_eq!(child.foreground, Some(Color::Rgb(1, 1, 1)));
        assert_eq!(child.background, Some(Color::Rgb(2, 2, 2)));
        assert_eq!(child.attributes.bold, Some(true), "taken from the parent");
        assert_eq!(
            child.attributes.italic,
            Some(true),
            "the child's own value survives"
        );
    }

    #[test]
    fn inheritance_does_not_override_an_explicit_value() {
        let parent = Face::fg(Color::Indexed(1)).bold();
        let mut child = Face::fg(Color::Indexed(2));
        child.attributes.bold = Some(false);
        child.inherit_from(&parent);
        assert_eq!(child.foreground, Some(Color::Indexed(2)));
        assert_eq!(
            child.attributes.bold,
            Some(false),
            "explicitly off stays off"
        );
    }

    #[test]
    fn overlay_lets_the_upper_face_win() {
        let base = Face::fg(Color::Indexed(7))
            .with_bg(Color::Indexed(0))
            .bold();
        let region = Face::bg(Color::Indexed(8));
        let merged = base.merged(&region);
        assert_eq!(
            merged.foreground,
            Some(Color::Indexed(7)),
            "syntax colour shows through"
        );
        assert_eq!(
            merged.background,
            Some(Color::Indexed(8)),
            "region background wins"
        );
        assert_eq!(merged.attributes.bold, Some(true));
    }

    #[test]
    fn overlay_leaves_the_base_untouched() {
        let base = Face::fg(Color::Indexed(7));
        let _ = base.merged(&Face::fg(Color::Indexed(1)));
        assert_eq!(base.foreground, Some(Color::Indexed(7)));
    }

    #[test]
    fn attributes_none_is_explicitly_off_everywhere() {
        let a = Attributes::none();
        assert_eq!(a.bold, Some(false));
        assert!(!a.is_unset());
        assert!(Attributes::default().is_unset());
    }

    #[test]
    fn degrading_a_face_reduces_both_colours() {
        let f = Face::fg(Color::Rgb(0xb2, 0x94, 0xbb)).with_bg(Color::Rgb(0x1d, 0x1f, 0x21));
        let d = f.degrade(ColorDepth::Ansi16);
        assert!(matches!(d.foreground, Some(Color::Indexed(i)) if i < 16));
        assert!(matches!(d.background, Some(Color::Indexed(i)) if i < 16));
    }

    #[test]
    fn a_face_converts_to_a_crossterm_style() {
        use crossterm::style::{Attribute, Color as Ct};
        let f = Face::fg(Color::Rgb(1, 2, 3)).bold().italic();
        let s = f.to_style(ColorDepth::TrueColor);
        assert_eq!(s.foreground_color, Some(Ct::Rgb { r: 1, g: 2, b: 3 }));
        assert_eq!(s.background_color, None);
        assert!(s.attributes.has(Attribute::Bold));
        assert!(s.attributes.has(Attribute::Italic));
        assert!(!s.attributes.has(Attribute::Underlined));
    }

    #[test]
    fn attributes_explicitly_off_are_not_emitted() {
        use crossterm::style::Attribute;
        let mut f = Face::new();
        f.attributes.bold = Some(false);
        assert!(
            !f.to_style(ColorDepth::TrueColor)
                .attributes
                .has(Attribute::Bold)
        );
    }

    #[test]
    fn styles_carry_the_degraded_colour() {
        use crossterm::style::Color as Ct;
        let f = Face::fg(Color::Rgb(255, 0, 0));
        let s = f.to_style(ColorDepth::Ansi16);
        assert_eq!(s.foreground_color, Some(Ct::AnsiValue(9)));
    }
}
