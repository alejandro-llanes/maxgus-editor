//! A picture where a buffer would be.
//!
//! Visiting a PNG does not fill a buffer with its bytes: the executor decodes
//! it and the buffer holds a caption — the file's dimensions and size —
//! while the picture itself lives beside the buffer, for a frontend that
//! can draw it. The GUI paints it into the window under the caption; a
//! terminal, which cannot, shows the caption and says so.

use std::path::Path;
use std::sync::Arc;

/// A decoded picture and what is known about the file it came from.
#[derive(Clone, PartialEq)]
pub struct Picture {
    /// The picture's size as it is on disk, for the caption.
    pub width: u32,
    pub height: u32,
    /// The format's name, as in "PNG".
    pub format: String,
    /// The file's size in bytes.
    pub bytes: u64,
    /// The pixels, which may be fewer than `width` by `height`: a
    /// photograph straight off a camera is cut down to something a window
    /// could show, since no window is that big.
    pub pixels: Pixels,
}

/// Pixels, eight bits each of red, green, blue and alpha, row by row.
#[derive(Clone, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Picture({}×{} {}, {} bytes, held at {}×{})",
            self.width, self.height, self.format, self.bytes, self.pixels.width, self.pixels.height
        )
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({}×{})", self.width, self.height)
    }
}

impl Picture {
    /// The line the buffer holds in the picture's stead.
    pub fn caption(&self, path: &Path) -> String {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        format!(
            "{name}: {}×{} {}, {}",
            self.width,
            self.height,
            self.format,
            size(self.bytes)
        )
    }
}

/// A file's size the way a listing says it.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// The formats the executor decodes, by extension.
pub const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Whether `path` names a picture, going by its extension.
pub fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// The picture `line` refers to at `column`, if it refers to one: a
/// markdown `![alt](path)`, an HTML `<img src="path">`, or a bare path with
/// a picture's extension. The reference has to be under point, or the
/// nearest one on the line when point is between them.
pub fn reference_at(line: &str, column: usize) -> Option<String> {
    let mut found: Vec<(usize, usize, String)> = Vec::new();
    // `![alt](path "title")`: the path runs to the first space or the
    // closing bracket.
    let mut from = 0;
    while let Some(at) = line[from..].find("](") {
        let start = from + at + 2;
        let end = line[start..]
            .find(')')
            .map(|n| start + n)
            .unwrap_or(line.len());
        let target = line[start..end]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let opened = line[..from + at].rfind("![").unwrap_or(from + at);
        if !target.is_empty() && is_picture(Path::new(&target)) {
            found.push((opened, end + 1, target));
        }
        from = end.min(line.len() - 1) + 1;
        if from >= line.len() {
            break;
        }
    }
    // `<img src="path">`.
    let mut from = 0;
    while let Some(at) = line[from..].find("src=") {
        let start = from + at + 4;
        let quote = line[start..].chars().next();
        let (start, closing) = match quote {
            Some(q @ ('"' | '\'')) => (start + 1, Some(q)),
            _ => (start, None),
        };
        let end = line[start..]
            .find(|c: char| closing.map_or(c.is_whitespace() || c == '>', |q| c == q))
            .map(|n| start + n)
            .unwrap_or(line.len());
        let target = line[start..end].to_string();
        if is_picture(Path::new(&target)) {
            let opened = line[..from + at].rfind('<').unwrap_or(from + at);
            found.push((opened, end + 1, target));
        }
        from = end;
        if from >= line.len() {
            break;
        }
    }
    // A bare path.
    if found.is_empty() {
        let mut start = 0;
        for word in line.split(|c: char| c.is_whitespace() || "()<>\"'`,;".contains(c)) {
            let at = line[start..].find(word).map(|n| start + n).unwrap_or(start);
            if !word.is_empty() && is_picture(Path::new(word)) {
                found.push((at, at + word.len(), word.to_string()));
            }
            start = at + word.len();
        }
    }
    // The one under point, or the nearest.
    found
        .into_iter()
        .min_by_key(|(start, end, _)| match column {
            c if c < *start => *start - c,
            c if c >= *end => c - *end + 1,
            _ => 0,
        })
        .map(|(_, _, target)| target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_says_what_the_picture_is_without_being_it() {
        let picture = Picture {
            width: 1920,
            height: 1080,
            format: "PNG".into(),
            bytes: 2_621_440,
            pixels: Pixels {
                width: 960,
                height: 540,
                rgba: Arc::from(vec![0u8; 4]),
            },
        };
        assert_eq!(
            picture.caption(Path::new("/tmp/shots/desk.png")),
            "desk.png: 1920×1080 PNG, 2.5 MB"
        );
        assert_eq!(size(512), "512 B");
        assert_eq!(size(10 * 1024), "10.0 KB");
        assert!(is_picture(Path::new("a.JPG")));
        assert!(!is_picture(Path::new("a.rs")));
        assert!(!is_picture(Path::new("png")));
    }

    #[test]
    fn the_reference_under_point_is_found_in_markdown_html_and_bare() {
        let md = "See ![the desk](shots/desk.png \"title\") and ![logo](logo.svg) and ![b](b.jpg).";
        assert_eq!(reference_at(md, 8).as_deref(), Some("shots/desk.png"));
        assert_eq!(reference_at(md, 0).as_deref(), Some("shots/desk.png"));
        assert_eq!(reference_at(md, md.len() - 1).as_deref(), Some("b.jpg"));
        // Between the two, the nearer one; the svg is not a picture we draw.
        assert_eq!(reference_at(md, 50).as_deref(), Some("shots/desk.png"));
        let html = "<p><img src='x/y.webp' width=3></p>";
        assert_eq!(reference_at(html, 10).as_deref(), Some("x/y.webp"));
        let bare = "look at /tmp/a.png, then b.gif";
        assert_eq!(reference_at(bare, 12).as_deref(), Some("/tmp/a.png"));
        assert_eq!(reference_at(bare, 25).as_deref(), Some("b.gif"));
        assert_eq!(reference_at("nothing here", 3), None);
    }
}
