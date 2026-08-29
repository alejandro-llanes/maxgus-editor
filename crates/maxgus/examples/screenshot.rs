//! Draws the screenshots the README uses.
//!
//! ```console
//! $ cargo run --example screenshot
//! ```
//!
//! The images are not mock-ups. Each one is the editor's own redisplay run
//! over a real `Editor` — the same `draw` the terminal calls — and every cell
//! is written out in the colour its face resolved to under the theme being
//! shown. Change a theme and rerunning this updates the pictures.
//!
//! Nerd Font glyphs are switched off here alone. They live in the private use
//! area, so a browser without such a font installed would draw a row of empty
//! boxes; the plain-text fallbacks (`**`, `--`, `%%`) are what the editor uses
//! on a terminal without one, and they render anywhere.

use maxgus_core::{Dispatcher, Editor, TaskResult};
use maxgus_faces::{Color, Face, Theme};
use maxgus_tree::{GitStatus, NodeKind, VisibleNode};
use maxgus_tui::{Rect, Size, Surface};

/// Cell metrics. Each run of text is drawn with an explicit `textLength`, so
/// the columns line up whatever monospace font the viewer happens to have.
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
const FONT_SIZE: f32 = 14.0;
const PAD: f32 = 14.0;
const FONTS: &str = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, \
                     'DejaVu Sans Mono', monospace";

const COLUMNS: u16 = 118;
const ROWS: u16 = 30;

/// What the screenshots show: this project's own fuzzy matcher.
const SOURCE: &str = r#"//! Fuzzy matching for the completion prompts.
//!
//! A query matches a candidate when its characters appear in order, not
//! necessarily together: `sbf` finds `save-buffer`. Scoring is what makes
//! that useful rather than merely permissive.

/// How well `query` matches `candidate`, or `None` if it does not at all.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    // An all-lowercase query ignores case; one capital makes it matter.
    let fold = !query.chars().any(char::is_uppercase);
    let text: Vec<char> = candidate.chars().collect();
    let mut score = 0;
    let mut previous: Option<usize> = None;

    for &wanted in &query.chars().collect::<Vec<_>>() {
        let found = text.iter().position(|&t| same(t, wanted, fold))?;
        if is_boundary(&text, found) {
            score += 16;
        }
        match previous {
            // Letters that follow one another are the strongest signal.
            Some(p) if found == p + 1 => score += 20,
            Some(p) => score -= ((found - p - 1) as i32).min(12),
            None => score -= (found as i32).min(12),
        }
        previous = Some(found);
    }
    Some(score - (text.len() as i32) / 8)
}
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    std::fs::create_dir_all(&out)?;

    // The built-in, then every theme that ships as a file, so the gallery is
    // exactly what `docs/themes` offers.
    let mut themes: Vec<(String, Theme)> =
        vec![("maxgus-dark".into(), maxgus_faces::defaults::builtin("maxgus-dark").unwrap())];
    let mut files: Vec<_> = std::fs::read_dir(out.join("../themes"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "kdl"))
        .collect();
    files.sort();
    for path in files {
        let source = std::fs::read_to_string(&path)?;
        let spec = maxgus_config::Config::parse(&source)?.themes.remove(0);
        let mut theme = maxgus_faces::defaults::builtin(spec.base.as_deref().unwrap()).unwrap();
        theme.apply_spec(&spec)?;
        themes.push((spec.name.clone(), theme));
    }

    // `--text` prints the scene as plain characters, which is how to check
    // the layout without opening an image.
    if std::env::args().any(|a| a == "--text") {
        let editor = scene(themes[0].1.clone(), std::env::args().any(|a| a == "--popup"));
        let mut surface = Surface::new(Size::new(COLUMNS, ROWS));
        maxgus_core::draw(&editor, &mut surface);
        for line in surface.to_lines() {
            println!("{}", line.trim_end());
        }
        return Ok(());
    }

    for (name, theme) in &themes {
        let editor = scene(theme.clone(), false);
        write(&out.join(format!("{name}.svg")), &render(&editor))?;
        println!("docs/screenshots/{name}.svg");
    }

    // One more of the command popup, over the theme the editor starts in.
    let editor = scene(themes[0].1.clone(), true);
    write(&out.join("command-popup.svg"), &render(&editor))?;
    println!("docs/screenshots/command-popup.svg");
    Ok(())
}

/// The editor as the screenshots find it: a file open beside the tree, parsed,
/// with a branch on the mode line.
fn scene(theme: Theme, popup: bool) -> Editor {
    let settings = maxgus_config::Settings {
        line_numbers: true,
        // See the note at the top of the file.
        nerd_font_icons: false,
        ..Default::default()
    };

    let mut editor = Editor::new(settings, theme, Rect::new(0, 0, COLUMNS, ROWS));
    let registry = maxgus_core::standard_registry();
    editor.command_names = registry.interactive_names();
    editor.command_docs = registry.iter().map(|c| (c.name.to_string(), c.doc.to_string())).collect();
    let mut dispatcher = Dispatcher::new(registry);

    let buffer = editor.buffers.visit_file("/maxgus/crates/maxgus-core/src/fuzzy.rs", SOURCE);
    editor.switch_to_buffer(buffer).unwrap();
    editor.with_current_buffer(|b| b.set_point(0));
    editor.git_branch = Some("main".to_string());

    // The tree, opened with the key that opens it and filled from a snapshot
    // the way a real directory walk would fill it.
    dispatcher.handle_keys(&mut editor, "C-x");
    dispatcher.handle_keys(&mut editor, "t");
    dispatcher.handle_keys(&mut editor, "t");
    editor
        .apply_task_result(TaskResult::TreeUpdated {
            nodes: tree(),
            // The tree cursor on the file that is open, which is where follow
            // mode would have put it.
            select: Some("/maxgus/crates/maxgus-core/src/fuzzy.rs".into()),
            show_hidden: false,
        })
        .unwrap();

    // Real tree-sitter output, so the colours are the ones a session sees.
    if let Ok(mut highlighter) = maxgus_syntax::Highlighter::new("rust")
        && highlighter.parse(SOURCE).is_ok()
    {
        let revision = editor.buffers.get(buffer).unwrap().revision();
        editor
            .apply_task_result(TaskResult::Reparsed {
                buffer,
                revision,
                range: 0..SOURCE.len(),
                highlights: highlighter.highlights(SOURCE),
            })
            .unwrap();
    }

    // Back to the text, with point somewhere that reads as a working session.
    // The window carries the point that redisplay draws, so it is the one to
    // move; the buffer follows on the next command.
    while Some(editor.windows.current_id()) == editor.tree_window {
        dispatcher.handle_keys(&mut editor, "C-x");
        dispatcher.handle_keys(&mut editor, "o");
    }
    let at = SOURCE.find("== p + 1").unwrap_or(0) + 8;
    editor.windows.current_mut().point = at;
    editor.with_current_buffer(move |b| b.set_point(at));
    editor.follow_point();

    if popup {
        dispatcher.handle_keys(&mut editor, "M-x");
        for key in ["s", "b", "f"] {
            dispatcher.handle_keys(&mut editor, key);
        }
    }
    editor.tasks.drain();
    editor
}

fn tree() -> Vec<VisibleNode> {
    let node = |path: &str, name: &str, dir: bool, depth: usize, git: Option<GitStatus>| {
        VisibleNode {
            path: path.into(),
            name: name.into(),
            kind: if dir { NodeKind::Directory } else { NodeKind::File },
            depth,
            expanded: dir,
            expandable: dir,
            git,
            is_root: depth == 0,
        }
    };
    vec![
        node("/maxgus", "maxgus", true, 0, None),
        node("/maxgus/crates", "crates", true, 1, None),
        node("/maxgus/crates/maxgus-core", "maxgus-core", true, 2, None),
        node("/maxgus/crates/maxgus-core/src", "src", true, 3, None),
        node("/maxgus/crates/maxgus-core/src/editor.rs", "editor.rs", false, 4, None),
        node("/maxgus/crates/maxgus-core/src/fuzzy.rs", "fuzzy.rs", false, 4, Some(GitStatus::Added)),
        node("/maxgus/crates/maxgus-core/src/render.rs", "render.rs", false, 4, Some(GitStatus::Modified)),
        node("/maxgus/crates/maxgus-faces", "maxgus-faces", true, 2, None),
        node("/maxgus/crates/maxgus-syntax", "maxgus-syntax", true, 2, None),
        node("/maxgus/crates/maxgus-tree", "maxgus-tree", true, 2, None),
        node("/maxgus/docs", "docs", true, 1, None),
        node("/maxgus/docs/themes", "themes", true, 2, None),
        node("/maxgus/docs/themes/dracula.kdl", "dracula.kdl", false, 3, None),
        node("/maxgus/docs/themes/gruvbox.kdl", "gruvbox.kdl", false, 3, None),
        node("/maxgus/docs/themes/nord.kdl", "nord.kdl", false, 3, None),
        node("/maxgus/Cargo.toml", "Cargo.toml", false, 1, None),
        node("/maxgus/README.md", "README.md", false, 1, Some(GitStatus::Modified)),
    ]
}

// ---- the surface, as SVG -----------------------------------------------

/// Draws the editor and writes the cells out as SVG.
fn render(editor: &Editor) -> String {
    let mut surface = Surface::new(Size::new(COLUMNS, ROWS));
    maxgus_core::draw(editor, &mut surface);

    let default = editor.theme.resolve("default");
    let page_bg = solid(default.background, "#1d2021");
    let page_fg = solid(default.foreground, "#d5c4a1");
    let width = COLUMNS as f32 * CELL_W + PAD * 2.0;
    let height = ROWS as f32 * CELL_H + PAD * 2.0;

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"{FONTS}\" \
         font-size=\"{FONT_SIZE}\" role=\"img\" aria-label=\"maxgus\">\n\
         <rect width=\"{width:.0}\" height=\"{height:.0}\" rx=\"10\" fill=\"{page_bg}\"/>\n"
    );

    // Backgrounds first, as one rectangle per run, then the text over them.
    // Painting per cell would quadruple the file for no visible difference.
    for y in 0..ROWS {
        for (start, len, face) in runs(&surface, y, |f| (f.background, f.attributes.reverse)) {
            let fill = match face.attributes.reverse.unwrap_or(false) {
                true => solid(face.foreground, &page_fg),
                false => match face.background {
                    Some(color) if hex(color).is_some() => hex(color).unwrap(),
                    _ => continue,
                },
            };
            if fill == page_bg {
                continue;
            }
            svg += &format!(
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{CELL_H}\" fill=\"{fill}\"/>\n",
                PAD + start as f32 * CELL_W,
                PAD + y as f32 * CELL_H,
                len as f32 * CELL_W,
            );
        }
    }

    for y in 0..ROWS {
        for (start, len, face) in runs(&surface, y, |f| (f.foreground, f.attributes)) {
            let text: String = (start..start + len)
                .filter_map(|x| surface.get(x, y))
                .filter(|cell| !cell.continuation)
                .map(|cell| cell.ch)
                .collect();
            if text.trim().is_empty() {
                continue;
            }
            let reverse = face.attributes.reverse.unwrap_or(false);
            let fill = match reverse {
                true => solid(face.background, &page_bg),
                false => solid(face.foreground, &page_fg),
            };
            let mut style = String::new();
            if face.attributes.bold.unwrap_or(false) {
                style += " font-weight=\"bold\"";
            }
            if face.attributes.italic.unwrap_or(false) {
                style += " font-style=\"italic\"";
            }
            if face.attributes.underline.unwrap_or(false) {
                style += " text-decoration=\"underline\"";
            }
            if face.attributes.dim.unwrap_or(false) {
                style += " opacity=\"0.65\"";
            }
            svg += &format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"{fill}\"{style} textLength=\"{:.1}\" \
                 lengthAdjust=\"spacing\" xml:space=\"preserve\">{}</text>\n",
                PAD + start as f32 * CELL_W,
                PAD + y as f32 * CELL_H + CELL_H * 0.74,
                len as f32 * CELL_W,
                escape(&text),
            );
        }
    }
    svg + "</svg>\n"
}

/// Splits row `y` into runs of cells whose `key` is the same.
fn runs<K: PartialEq>(
    surface: &Surface,
    y: u16,
    key: impl Fn(&Face) -> K,
) -> Vec<(u16, u16, Face)> {
    let mut out: Vec<(u16, u16, Face)> = Vec::new();
    for x in 0..surface.width() {
        let Some(cell) = surface.get(x, y) else { continue };
        match out.last_mut() {
            Some((_, len, face)) if key(face) == key(&cell.face) => *len += 1,
            _ => out.push((x, 1, cell.face)),
        }
    }
    out
}

/// A colour as `#rrggbb`, or `None` when it is the terminal's own.
fn hex(color: Color) -> Option<String> {
    match color {
        Color::Default => None,
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        // The xterm palette: sixteen named, a 6×6×6 cube, then a grey ramp.
        Color::Indexed(i) => Some(match i {
            0..=15 => {
                const BASE: [(u8, u8, u8); 16] = [
                    (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0),
                    (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
                    (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0),
                    (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
                ];
                let (r, g, b) = BASE[i as usize];
                format!("#{r:02x}{g:02x}{b:02x}")
            }
            16..=231 => {
                let step = |v: u8| if v == 0 { 0u8 } else { 55 + v * 40 };
                let n = i - 16;
                let (r, g, b) = (step(n / 36), step((n % 36) / 6), step(n % 6));
                format!("#{r:02x}{g:02x}{b:02x}")
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                format!("#{v:02x}{v:02x}{v:02x}")
            }
        }),
    }
}

/// A colour that is definitely something, falling back to the page's own.
fn solid(color: Option<Color>, fallback: &str) -> String {
    color.and_then(hex).unwrap_or_else(|| fallback.to_string())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn write(path: &std::path::Path, svg: &str) -> std::io::Result<()> {
    std::fs::write(path, svg)
}
