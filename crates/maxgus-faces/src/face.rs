//! A face: colours plus text attributes.

use crate::color::{Color, ColorDepth};
use crossterm::style::{Attribute, ContentStyle, Attributes as CtAttributes};

/// Text attributes. Each is tri-state: unset attributes are inherited rather
/// than forced off, which is what lets `error` inherit from `default` and only
/// add an underline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attributes {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
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
        fill!(bold, italic, underline, reverse, dim, strikethrough);
    }

    /// Overlays `other`'s set attributes onto this one.
    pub fn overlay(&mut self, other: &Attributes) {
        macro_rules! take {
            ($($f:ident),*) => {$( if other.$f.is_some() { self.$f = other.$f; } )*};
        }
        take!(bold, italic, underline, reverse, dim, strikethrough);
    }

    fn to_crossterm(self) -> CtAttributes {
        let mut out = CtAttributes::none();
        if self.bold == Some(true) {
            out.set(Attribute::Bold);
        }
        if self.italic == Some(true) {
            out.set(Attribute::Italic);
        }
        if self.underline == Some(true) {
            out.set(Attribute::Underlined);
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
        Face { foreground: Some(color), ..Default::default() }
    }

    /// A face with just a background colour.
    pub fn bg(color: Color) -> Face {
        Face { background: Some(color), ..Default::default() }
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

    /// The crossterm style used to draw text in this face.
    pub fn to_style(&self, depth: ColorDepth) -> ContentStyle {
        let face = self.degrade(depth);
        ContentStyle {
            foreground_color: face.foreground.map(Into::into),
            background_color: face.background.map(Into::into),
            underline_color: None,
            attributes: face.attributes.to_crossterm(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_compose() {
        let f = Face::fg(Color::Indexed(1)).with_bg(Color::Indexed(0)).bold().underline();
        assert_eq!(f.foreground, Some(Color::Indexed(1)));
        assert_eq!(f.background, Some(Color::Indexed(0)));
        assert_eq!(f.attributes.bold, Some(true));
        assert_eq!(f.attributes.underline, Some(true));
        assert_eq!(f.attributes.italic, None, "unset, not off");
    }

    #[test]
    fn an_empty_face_is_recognised() {
        assert!(Face::new().is_empty());
        assert!(!Face::fg(Color::Default).is_empty(), "an explicit default is still set");
        assert!(!Face::new().bold().is_empty());
    }

    #[test]
    fn inheritance_fills_only_unset_fields() {
        let parent = Face::fg(Color::Rgb(1, 1, 1)).with_bg(Color::Rgb(2, 2, 2)).bold();
        let mut child = Face::new().italic();
        child.inherit_from(&parent);
        assert_eq!(child.foreground, Some(Color::Rgb(1, 1, 1)));
        assert_eq!(child.background, Some(Color::Rgb(2, 2, 2)));
        assert_eq!(child.attributes.bold, Some(true), "taken from the parent");
        assert_eq!(child.attributes.italic, Some(true), "the child's own value survives");
    }

    #[test]
    fn inheritance_does_not_override_an_explicit_value() {
        let parent = Face::fg(Color::Indexed(1)).bold();
        let mut child = Face::fg(Color::Indexed(2));
        child.attributes.bold = Some(false);
        child.inherit_from(&parent);
        assert_eq!(child.foreground, Some(Color::Indexed(2)));
        assert_eq!(child.attributes.bold, Some(false), "explicitly off stays off");
    }

    #[test]
    fn overlay_lets_the_upper_face_win() {
        let base = Face::fg(Color::Indexed(7)).with_bg(Color::Indexed(0)).bold();
        let region = Face::bg(Color::Indexed(8));
        let merged = base.merged(&region);
        assert_eq!(merged.foreground, Some(Color::Indexed(7)), "syntax colour shows through");
        assert_eq!(merged.background, Some(Color::Indexed(8)), "region background wins");
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
        assert!(!f.to_style(ColorDepth::TrueColor).attributes.has(Attribute::Bold));
    }

    #[test]
    fn styles_carry_the_degraded_colour() {
        use crossterm::style::Color as Ct;
        let f = Face::fg(Color::Rgb(255, 0, 0));
        let s = f.to_style(ColorDepth::Ansi16);
        assert_eq!(s.foreground_color, Some(Ct::AnsiValue(9)));
    }
}
