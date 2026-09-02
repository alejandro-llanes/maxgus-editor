//! Themes: named face tables, resolved from config specs.

use crate::{
    color::{Color, ColorDepth, ColorError},
    face::Face,
    names,
};
use maxgus_config::spec::{FaceSpec, ThemeSpec};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ThemeError {
    #[error("face `{face}`: {source}")]
    BadColor {
        face: String,
        #[source]
        source: ColorError,
    },
    #[error("face `{0}` inherits from itself")]
    InheritanceCycle(String),
    #[error("no theme named `{0}`")]
    Unknown(String),
}

/// A resolved theme: face name to face, plus the inheritance graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    name: String,
    faces: HashMap<String, Face>,
    inherits: HashMap<String, String>,
}

impl Theme {
    /// A theme with only a `default` face.
    pub fn new(name: impl Into<String>) -> Theme {
        let mut faces = HashMap::new();
        faces.insert(names::DEFAULT.to_string(), Face::default());
        Theme {
            name: name.into(),
            faces,
            inherits: HashMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Defines a face outright.
    pub fn set(&mut self, face_name: impl Into<String>, face: Face) {
        self.faces.insert(face_name.into(), face);
    }

    /// Declares that `child` inherits from `parent`.
    pub fn set_inherit(&mut self, child: impl Into<String>, parent: impl Into<String>) {
        self.inherits.insert(child.into(), parent.into());
    }

    /// The face as written, without inheritance applied.
    pub fn raw(&self, face_name: &str) -> Option<&Face> {
        self.faces.get(face_name)
    }

    /// The `default` face, which every lookup falls back to.
    pub fn default_face(&self) -> Face {
        self.faces.get(names::DEFAULT).copied().unwrap_or_default()
    }

    /// Resolves `face_name` through its inheritance chain and then through
    /// `default`, so the result always has concrete colours when the theme
    /// defines them.
    ///
    /// An unknown face resolves to `default` rather than failing: a theme is
    /// not required to mention every face, and a grammar may produce captures
    /// the theme has never heard of.
    pub fn resolve(&self, face_name: &str) -> Face {
        self.resolve_inner(face_name, true)
    }

    /// The face as an *overlay*: inheritance is applied, but the `default`
    /// face is not filled in behind it.
    ///
    /// This is the difference between drawing text in a face and drawing a
    /// face on top of other text. `region` sets only a background; resolved
    /// normally it also carries `default`'s foreground, which would wipe out
    /// the syntax colour underneath it. As an overlay it contributes only the
    /// background, and the colour beneath shows through.
    pub fn resolve_overlay(&self, face_name: &str) -> Face {
        self.resolve_inner(face_name, false)
    }

    fn resolve_inner(&self, face_name: &str, fill_default: bool) -> Face {
        let mut face = self.faces.get(face_name).copied().unwrap_or_default();
        let mut seen = vec![face_name.to_string()];
        let mut current = face_name.to_string();
        // Walk the inheritance chain, filling gaps as we go.
        while let Some(parent) = self.inherits.get(&current) {
            if seen.contains(parent) {
                break; // A cycle; `validate` reports it, resolution just stops.
            }
            if let Some(pf) = self.faces.get(parent) {
                face.inherit_from(pf);
            }
            seen.push(parent.clone());
            current = parent.clone();
        }
        if fill_default && face_name != names::DEFAULT {
            face.inherit_from(&self.default_face());
        }
        face
    }

    /// Resolves a face and reduces it to what `depth` can display.
    pub fn resolve_for(&self, face_name: &str, depth: ColorDepth) -> Face {
        self.resolve(face_name).degrade(depth)
    }

    /// The face for a tree-sitter capture, falling back to `default` when the
    /// capture maps to nothing.
    pub fn face_for_capture(&self, capture: &str) -> Face {
        match names::face_for_capture(capture) {
            Some(face) => self.resolve(face),
            None => self.default_face(),
        }
    }

    /// True when the theme's `default` background is dark. Used to pick
    /// sensible fallbacks for faces the theme leaves out.
    pub fn is_dark(&self) -> bool {
        self.default_face().background.is_some_and(Color::is_dark)
    }

    /// Face names defined by this theme, sorted.
    pub fn face_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.faces.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Reports inheritance cycles. Colour errors are caught at parse time.
    pub fn validate(&self) -> Result<(), ThemeError> {
        for start in self.inherits.keys() {
            let mut seen = vec![start.clone()];
            let mut current = start.clone();
            while let Some(parent) = self.inherits.get(&current) {
                if seen.contains(parent) {
                    return Err(ThemeError::InheritanceCycle(start.clone()));
                }
                seen.push(parent.clone());
                current = parent.clone();
            }
        }
        Ok(())
    }

    /// Layers `other` over this theme: same-named faces are overlaid, new ones
    /// are added. This is how a user's `theme` block customises a built-in.
    pub fn overlay(&mut self, other: &Theme) {
        for (name, face) in &other.faces {
            self.faces.entry(name.clone()).or_default().overlay(face);
        }
        for (child, parent) in &other.inherits {
            self.inherits.insert(child.clone(), parent.clone());
        }
    }

    /// Builds a theme from a parsed config block.
    pub fn from_spec(spec: &ThemeSpec) -> Result<Theme, ThemeError> {
        let mut theme = Theme::new(&spec.name);
        for face_spec in &spec.faces {
            let (face, inherit) = face_from_spec(face_spec)?;
            // A `face "default"` block replaces the empty starting default.
            match theme.faces.get_mut(&face_spec.name) {
                Some(existing) => existing.overlay(&face),
                None => {
                    theme.faces.insert(face_spec.name.clone(), face);
                }
            }
            if let Some(parent) = inherit {
                theme.inherits.insert(face_spec.name.clone(), parent);
            }
        }
        theme.validate()?;
        Ok(theme)
    }

    /// Applies a config block on top of an existing theme.
    pub fn apply_spec(&mut self, spec: &ThemeSpec) -> Result<(), ThemeError> {
        let overlay = Theme::from_spec(spec)?;
        // `from_spec` seeds an empty `default`; do not let it blank ours.
        let mut overlay = overlay;
        if spec.face(names::DEFAULT).is_none() {
            overlay.faces.remove(names::DEFAULT);
        }
        self.overlay(&overlay);
        self.validate()
    }
}

/// Converts one face spec, returning the face and its `inherit` target.
fn face_from_spec(spec: &FaceSpec) -> Result<(Face, Option<String>), ThemeError> {
    let color = |text: &Option<String>| -> Result<Option<Color>, ThemeError> {
        text.as_deref()
            .map(|t| {
                Color::parse(t).map_err(|e| ThemeError::BadColor {
                    face: spec.name.clone(),
                    source: e,
                })
            })
            .transpose()
    };
    let face = Face {
        foreground: color(&spec.foreground)?,
        background: color(&spec.background)?,
        attributes: crate::face::Attributes {
            bold: spec.bold,
            italic: spec.italic,
            underline: spec.underline,
            undercurl: spec.undercurl,
            reverse: spec.reverse,
            dim: spec.dim,
            strikethrough: spec.strikethrough,
        },
    };
    Ok((face, spec.inherit.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kdl: &str) -> ThemeSpec {
        let config = maxgus_config::Config::parse(kdl).expect("valid kdl");
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        config.themes.into_iter().next().expect("one theme")
    }

    #[test]
    fn a_new_theme_has_only_a_default_face() {
        let t = Theme::new("empty");
        assert_eq!(t.name(), "empty");
        assert_eq!(t.face_names(), vec!["default"]);
        assert!(t.resolve("default").is_empty());
    }

    #[test]
    fn faces_resolve_to_what_the_theme_set() {
        let mut t = Theme::new("t");
        t.set("font-lock-keyword", Face::fg(Color::Indexed(5)).bold());
        let f = t.resolve("font-lock-keyword");
        assert_eq!(f.foreground, Some(Color::Indexed(5)));
        assert_eq!(f.attributes.bold, Some(true));
    }

    #[test]
    fn unset_fields_fall_through_to_default() {
        let mut t = Theme::new("t");
        t.set(
            "default",
            Face::fg(Color::Indexed(7)).with_bg(Color::Indexed(0)),
        );
        t.set("region", Face::bg(Color::Indexed(8)));
        let f = t.resolve("region");
        assert_eq!(f.background, Some(Color::Indexed(8)), "its own background");
        assert_eq!(f.foreground, Some(Color::Indexed(7)), "default foreground");
    }

    #[test]
    fn an_unknown_face_resolves_to_default() {
        let mut t = Theme::new("t");
        t.set("default", Face::fg(Color::Indexed(7)));
        assert_eq!(
            t.resolve("never-heard-of-it").foreground,
            Some(Color::Indexed(7))
        );
        assert!(t.raw("never-heard-of-it").is_none());
    }

    #[test]
    fn inheritance_chains_are_walked() {
        let mut t = Theme::new("t");
        t.set("default", Face::fg(Color::Indexed(7)));
        t.set("base", Face::fg(Color::Indexed(1)).bold());
        t.set("middle", Face::new().italic());
        t.set_inherit("middle", "base");
        t.set("leaf", Face::new().underline());
        t.set_inherit("leaf", "middle");

        let f = t.resolve("leaf");
        assert_eq!(f.attributes.underline, Some(true), "its own");
        assert_eq!(f.attributes.italic, Some(true), "from middle");
        assert_eq!(f.attributes.bold, Some(true), "from base");
        assert_eq!(
            f.foreground,
            Some(Color::Indexed(1)),
            "from base, not default"
        );
    }

    #[test]
    fn an_inheritance_cycle_is_detected_and_resolution_still_terminates() {
        let mut t = Theme::new("t");
        t.set("a", Face::new().bold());
        t.set("b", Face::new().italic());
        t.set_inherit("a", "b");
        t.set_inherit("b", "a");
        assert!(matches!(t.validate(), Err(ThemeError::InheritanceCycle(_))));
        let f = t.resolve("a");
        assert_eq!(f.attributes.bold, Some(true));
        assert_eq!(f.attributes.italic, Some(true));
    }

    #[test]
    fn a_theme_builds_from_a_config_spec() {
        let t = Theme::from_spec(&spec(
            r##"
            theme "test" {
                face "default" fg="#c5c8c6" bg="#1d1f21"
                face "font-lock-keyword" fg="#b294bb" bold=#true
                face "error" inherit="font-lock-keyword" underline=#true
            }
            "##,
        ))
        .unwrap();
        assert_eq!(t.name(), "test");
        assert_eq!(
            t.resolve("default").foreground,
            Some(Color::Rgb(0xc5, 0xc8, 0xc6))
        );
        let err = t.resolve("error");
        assert_eq!(err.attributes.underline, Some(true));
        assert_eq!(err.attributes.bold, Some(true), "inherited");
        assert_eq!(
            err.foreground,
            Some(Color::Rgb(0xb2, 0x94, 0xbb)),
            "inherited"
        );
    }

    #[test]
    fn a_bad_colour_in_a_spec_is_an_error() {
        let err = Theme::from_spec(&spec(r##"theme "t" { face "default" fg="chartreuse" }"##))
            .unwrap_err();
        assert!(matches!(err, ThemeError::BadColor { .. }));
        assert!(err.to_string().contains("default"));
    }

    #[test]
    fn a_spec_cycle_is_rejected_at_build_time() {
        let s = spec(
            r##"
            theme "t" {
                face "a" inherit="b" bold=#true
                face "b" inherit="a" italic=#true
            }
            "##,
        );
        assert!(matches!(
            Theme::from_spec(&s),
            Err(ThemeError::InheritanceCycle(_))
        ));
    }

    #[test]
    fn applying_a_spec_customises_without_erasing_the_rest() {
        let mut t = Theme::new("base");
        t.set(
            "default",
            Face::fg(Color::Indexed(7)).with_bg(Color::Indexed(0)),
        );
        t.set("font-lock-string", Face::fg(Color::Indexed(2)));
        t.set("font-lock-keyword", Face::fg(Color::Indexed(4)));

        t.apply_spec(&spec(
            r##"theme "base" { face "font-lock-keyword" fg="#ff00ff" }"##,
        ))
        .unwrap();

        assert_eq!(
            t.resolve("font-lock-keyword").foreground,
            Some(Color::Rgb(255, 0, 255))
        );
        assert_eq!(
            t.resolve("font-lock-string").foreground,
            Some(Color::Indexed(2)),
            "untouched"
        );
        assert_eq!(
            t.resolve("default").background,
            Some(Color::Indexed(0)),
            "default survives"
        );
    }

    #[test]
    fn applying_a_spec_that_sets_default_does_override_it() {
        let mut t = Theme::new("base");
        t.set(
            "default",
            Face::fg(Color::Indexed(7)).with_bg(Color::Indexed(0)),
        );
        t.apply_spec(&spec(r##"theme "base" { face "default" bg="#101010" }"##))
            .unwrap();
        assert_eq!(
            t.resolve("default").background,
            Some(Color::Rgb(0x10, 0x10, 0x10))
        );
        assert_eq!(
            t.resolve("default").foreground,
            Some(Color::Indexed(7)),
            "fg untouched"
        );
    }

    #[test]
    fn overlaying_merges_face_tables() {
        let mut a = Theme::new("a");
        a.set("x", Face::fg(Color::Indexed(1)).bold());
        let mut b = Theme::new("b");
        b.set("x", Face::fg(Color::Indexed(2)));
        b.set("y", Face::fg(Color::Indexed(3)));
        a.overlay(&b);
        assert_eq!(a.resolve("x").foreground, Some(Color::Indexed(2)));
        assert_eq!(a.resolve("x").attributes.bold, Some(true));
        assert_eq!(a.resolve("y").foreground, Some(Color::Indexed(3)));
    }

    #[test]
    fn tree_sitter_captures_resolve_through_the_face_table() {
        let mut t = Theme::new("t");
        t.set("font-lock-keyword", Face::fg(Color::Indexed(5)));
        assert_eq!(
            t.face_for_capture("keyword.function").foreground,
            Some(Color::Indexed(5))
        );
        assert_eq!(t.face_for_capture("nonsense"), t.default_face());
    }

    #[test]
    fn darkness_is_read_from_the_default_background() {
        let mut t = Theme::new("t");
        t.set("default", Face::bg(Color::Rgb(0x1d, 0x1f, 0x21)));
        assert!(t.is_dark());
        t.set("default", Face::bg(Color::Rgb(0xff, 0xff, 0xff)));
        assert!(!t.is_dark());
        assert!(
            !Theme::new("no-bg").is_dark(),
            "no background means not dark"
        );
    }

    #[test]
    fn an_overlay_face_contributes_only_what_it_sets() {
        let mut t = Theme::new("t");
        t.set(
            "default",
            Face::fg(Color::Indexed(7)).with_bg(Color::Indexed(0)),
        );
        t.set("region", Face::bg(Color::Indexed(8)));

        // Drawn as text, `region` picks up the default foreground.
        assert_eq!(t.resolve("region").foreground, Some(Color::Indexed(7)));
        // Drawn as an overlay, it contributes only its background, so
        // whatever is underneath keeps its colour.
        let overlay = t.resolve_overlay("region");
        assert_eq!(overlay.foreground, None);
        assert_eq!(overlay.background, Some(Color::Indexed(8)));

        let mut syntax = Face::fg(Color::Indexed(5));
        syntax.overlay(&overlay);
        assert_eq!(
            syntax.foreground,
            Some(Color::Indexed(5)),
            "the syntax colour survives"
        );
        assert_eq!(syntax.background, Some(Color::Indexed(8)));
    }

    #[test]
    fn an_overlay_still_follows_the_inheritance_chain() {
        let mut t = Theme::new("t");
        t.set("default", Face::fg(Color::Indexed(7)));
        t.set("base", Face::new().bold());
        t.set("derived", Face::bg(Color::Indexed(3)));
        t.set_inherit("derived", "base");
        let overlay = t.resolve_overlay("derived");
        assert_eq!(overlay.attributes.bold, Some(true), "inherited");
        assert_eq!(overlay.foreground, None, "but not filled from default");
    }

    #[test]
    fn resolving_for_a_depth_degrades_the_colours() {
        let mut t = Theme::new("t");
        t.set("x", Face::fg(Color::Rgb(255, 0, 0)));
        assert_eq!(
            t.resolve_for("x", ColorDepth::Ansi16).foreground,
            Some(Color::Indexed(9))
        );
    }
}
