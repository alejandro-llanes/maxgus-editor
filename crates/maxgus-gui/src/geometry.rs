//! The window's size, remembered between runs.
//!
//! A window that opens at the same size every time is a window that has to
//! be dragged out every time. The size the window was closed at is kept
//! under the state directory, beside the sessions, and is where the next
//! one opens, and whether it filled the screen, which is where the next one
//! goes too; nothing else about the window is — position is the
//! compositor's to decide, and most will not take a request for it.

use std::path::{Path, PathBuf};

/// The window's inner size, in logical pixels: the same on any display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    /// Whether the window filled the screen. The size is the one it had
    /// as a window, which is what it comes back to.
    pub fullscreen: bool,
}

impl Geometry {
    /// What a window opens at when nothing is remembered.
    pub const DEFAULT: Geometry = Geometry {
        width: 1100.0,
        height: 720.0,
        fullscreen: false,
    };

    /// Reads a size back out of what `serialize` wrote. Anything that is
    /// not two sensible numbers is treated as nothing remembered; the word
    /// `fullscreen` after them is what it says.
    pub fn parse(text: &str) -> Option<Geometry> {
        let mut fields = text.split_whitespace();
        if fields.next()? != "window" {
            return None;
        }
        let width: f64 = fields.next()?.parse().ok()?;
        let height: f64 = fields.next()?.parse().ok()?;
        let sensible = |n: f64| n.is_finite() && (100.0..=65535.0).contains(&n);
        let fullscreen = fields.next() == Some("fullscreen");
        match sensible(width) && sensible(height) {
            true => Some(Geometry {
                width,
                height,
                fullscreen,
            }),
            false => None,
        }
    }

    pub fn serialize(&self) -> String {
        let fullscreen = match self.fullscreen {
            true => " fullscreen",
            false => "",
        };
        format!("window {:.0} {:.0}{fullscreen}\n", self.width, self.height)
    }
}

/// Where the size is kept.
pub fn path_for(state_dir: &Path) -> PathBuf {
    state_dir.join("window.kdl")
}

pub fn read(path: &Path) -> Option<Geometry> {
    Geometry::parse(&std::fs::read_to_string(path).ok()?)
}

/// Writes the size, making the directory if this is the first run.
pub fn write(path: &Path, geometry: Geometry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, geometry.serialize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_survives_the_round_trip() {
        let geometry = Geometry {
            width: 1280.0,
            height: 800.0,
            fullscreen: false,
        };
        assert_eq!(Geometry::parse(&geometry.serialize()), Some(geometry));
        let filled = Geometry {
            fullscreen: true,
            ..geometry
        };
        assert_eq!(filled.serialize(), "window 1280 800 fullscreen\n");
        assert_eq!(Geometry::parse(&filled.serialize()), Some(filled));
        // What an older maxgus wrote is a window, as it was then.
        assert_eq!(Geometry::parse("window 1280 800\n"), Some(geometry));
    }

    #[test]
    fn nonsense_is_nothing_remembered() {
        assert_eq!(Geometry::parse(""), None);
        assert_eq!(Geometry::parse("window 12"), None);
        assert_eq!(Geometry::parse("window abc 700"), None);
        assert_eq!(
            Geometry::parse("window 1 1"),
            None,
            "a window nobody can see"
        );
        assert_eq!(Geometry::parse("session 1100 720"), None);
    }

    #[test]
    fn the_file_is_written_and_read_back() {
        let dir = std::env::temp_dir().join(format!("maxgus-geometry-{}", std::process::id()));
        let path = path_for(&dir.join("state"));
        assert_eq!(read(&path), None, "nothing written yet");
        let geometry = Geometry {
            width: 900.0,
            height: 600.0,
            fullscreen: true,
        };
        write(&path, geometry).expect("written");
        assert_eq!(read(&path), Some(geometry));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
