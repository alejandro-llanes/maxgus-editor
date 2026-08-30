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
    let mut themes: Vec<(String, Theme)> = vec![(
        "maxgus-dark".into(),
        maxgus_faces::defaults::builtin("maxgus-dark").unwrap(),
    )];
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
        let editor = scene(
            themes[0].1.clone(),
            std::env::args().any(|a| a == "--popup"),
        );
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

    let editor = terminal_scene(themes[0].1.clone());
    write(&out.join("terminal.svg"), &render(&editor))?;
    println!("docs/screenshots/terminal.svg");

    let editor = git_scene(themes[0].1.clone());
    write(&out.join("magit.svg"), &render(&editor))?;
    println!("docs/screenshots/magit.svg");

    // The same status view with the top-level menu up.
    let mut editor = git_scene(themes[0].1.clone());
    let registry = maxgus_core::standard_registry();
    let mut dispatcher = Dispatcher::new(registry);
    dispatcher.handle_keys(&mut editor, "?");
    write(&out.join("magit-menu.svg"), &render(&editor))?;
    println!("docs/screenshots/magit-menu.svg");

    // `C-c` held long enough for the panel to say what can follow it.
    let mut editor = scene(themes[0].1.clone(), false);
    editor.which_key = Some("C-c".into());
    editor.pending_keys = Some("C-c".into());
    write(&out.join("which-key.svg"), &render(&editor))?;
    println!("docs/screenshots/which-key.svg");

    // What the language server knows about the symbol the cursor is on.
    let mut editor = scene(themes[0].1.clone(), false);
    let line = {
        let point = editor.windows.current().point;
        editor.current_buffer().line_of(point)
    };
    // The markdown a language server really sends, once the client asks
    // for it: a heading, a rule, the parameters, the prose, the signature.
    editor.doc = Some(maxgus_core::Doc {
        text: "### `score`\n\n---\n→ `Option<i32>`\n\nParameters:\n\n\
               - `query: &str`\n- `candidate: &str`\n\n\
               How well **query** matches `candidate`, or `None` when it \
               does not match at all.\n\n---\n```rust\n\
               pub fn score(query: &str, candidate: &str) -> Option<i32>\n```"
            .into(),
        line,
        window: editor.windows.current_id(),
    });
    write(&out.join("lsp-doc.svg"), &render(&editor))?;
    println!("docs/screenshots/lsp-doc.svg");

    // The light that says where the cursor just landed.
    let mut editor = scene(themes[0].1.clone(), false);
    editor.settings.beacon = true;
    editor.settings.beacon_size = 34;
    let (window, offset) = (editor.windows.current_id(), editor.windows.current().point);
    editor.beacon = Some(maxgus_core::beacon::Beacon::new(window, offset));
    write(&out.join("beacon.svg"), &render(&editor))?;
    println!("docs/screenshots/beacon.svg");
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
    editor.command_docs = registry
        .iter()
        .map(|c| (c.name.to_string(), c.doc.to_string()))
        .collect();
    let mut dispatcher = Dispatcher::new(registry);

    let buffer = editor
        .buffers
        .visit_file("/maxgus/crates/maxgus-core/src/fuzzy.rs", SOURCE);
    editor.switch_to_buffer(buffer).unwrap();
    editor.with_current_buffer(|b| b.set_point(0));
    editor.git_branch = Some("main".to_string());

    // The server first: which windows the panel has is decided when it opens.
    editor
        .apply_task_result(TaskResult::LanguageServerStarted {
            language: "rust".into(),
            encoding: maxgus_lsp::PositionEncoding::Utf16,
        })
        .unwrap();

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

    // The outline the server would answer with. The JSON is the shape
    // `rust-analyzer` sends, read by the same parser a real reply goes
    // through — the panel is drawn from real data, not from a fixture shaped
    // to look good.
    editor.apply_lsp_response(TaskResult::LspResponse {
        language: "rust".into(),
        uri: "file:///maxgus/crates/maxgus-core/src/fuzzy.rs".into(),
        query: maxgus_core::LspQuery::DocumentSymbols { for_panel: true },
        result: outline(),
    });

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

/// The git status view, over a repository with something to look at.
///
/// The snapshot is fed through the same parsers a real `git status` and
/// `git diff` go through, so what is drawn is what the editor would draw.
fn git_scene(theme: Theme) -> Editor {
    let settings = maxgus_config::Settings {
        nerd_font_icons: false,
        ..Default::default()
    };
    let mut editor = Editor::new(settings, theme, Rect::new(0, 0, COLUMNS, ROWS));
    let registry = maxgus_core::standard_registry();
    editor.command_names = registry.interactive_names();
    let mut dispatcher = Dispatcher::new(registry);

    let buffer = editor.buffers.visit_file("/maxgus/src/fuzzy.rs", SOURCE);
    editor.switch_to_buffer(buffer).unwrap();
    dispatcher.handle_keys(&mut editor, "C-x");
    dispatcher.handle_keys(&mut editor, "g");

    // Raw strings: a diff is full of quotes and backslashes, and escaping
    // them here would make the fixture unreadable.
    let unstaged = r#"diff --git a/crates/maxgus-core/src/render.rs b/crates/maxgus-core/src/render.rs
index 83db48f..bf269f4 100644
--- a/crates/maxgus-core/src/render.rs
+++ b/crates/maxgus-core/src/render.rs
@@ -412,7 +412,9 @@ fn draw_git_row
         Row::Section(section) => {
             let folded = editor.git.is_collapsed(*section);
-            let arrow = if folded { '>' } else { 'v' };
+            // A triangle rather than a caret: it reads as a fold at a
+            // glance, which a punctuation mark does not.
+            let arrow = if folded { TRIANGLE_RIGHT } else { TRIANGLE_DOWN };
             surface.set_char(x, area.y, arrow, face(SHADOW));
         }
"#;
    let staged = r#"diff --git a/README.md b/README.md
index aaa..bbb 100644
--- a/README.md
+++ b/README.md
@@ -1,3 +1,3 @@
-**1531 tests.**
+**1597 tests.**
 Unit tests beside the code.
"#;
    let snapshot = maxgus_core::task::GitSnapshot {
        root: "/maxgus".into(),
        status: maxgus_git::status::parse(
            concat!(
                "# branch.oid 5958f5e13418d8b5\0",
                "# branch.head main\0",
                "# branch.upstream origin/main\0",
                "# branch.ab +2 -0\0",
                "1 .M N... 100644 100644 100644 a b crates/maxgus-core/src/render.rs\0",
                "1 M. N... 100644 100644 100644 a b README.md\0",
                "? docs/screenshots/magit.svg\0",
            )
            .as_bytes(),
        ),
        unstaged: maxgus_git::diff::parse(unstaged),
        staged: maxgus_git::diff::parse(staged),
        stashes: maxgus_git::log::parse_stashes("stash@{0}\u{1f}WIP on main: the terminal\u{1e}\n"),
        unpushed: maxgus_git::log::parse_log(
            "h1\u{1f}a1b2c3d\u{1f}Alejandro\u{1f}an hour ago\u{1f}\u{1f}Draw the status view\u{1e}\n\
             h2\u{1f}e4f5a6b\u{1f}Alejandro\u{1f}2 hours ago\u{1f}\u{1f}Read a diff into hunks\u{1e}\n",
        ),
        unpulled: Vec::new(),
        recent: maxgus_git::log::parse_log(
            "h3\u{1f}5958f5e\u{1f}Alejandro\u{1f}a day ago\u{1f}HEAD -> main, origin/main, tag: v0.1.0\u{1f}Count the tests correctly\u{1e}\n",
        ),
        head_subject: "Count the tests correctly".into(),
        branches: vec!["main".into()],
        references: vec![maxgus_git::Reference {
            name: "main".into(),
            kind: maxgus_git::RefKind::Local,
        }],
    };
    editor
        .apply_task_result(TaskResult::GitRefreshed(Box::new(snapshot)))
        .unwrap();
    // The unstaged file opened, so the hunk and its lines are on show.
    editor.git.toggle_file(
        maxgus_core::git::Section::Unstaged,
        "crates/maxgus-core/src/render.rs",
    );
    editor.render_git_buffer();
    editor.move_git_cursor_to_line(
        editor
            .git
            .line_of(&maxgus_core::git::Row::Hunk {
                section: maxgus_core::git::Section::Unstaged,
                file: 0,
                hunk: 0,
            })
            .unwrap_or(0),
    );
    editor.tasks.drain();
    editor
}

/// The same session with the terminal panel open and two tabs in it.
///
/// The output below is fed through the real emulator, so the colours and the
/// layout are what a shell writing those bytes would actually produce.
fn terminal_scene(theme: Theme) -> Editor {
    let mut editor = scene(theme, false);
    let registry = maxgus_core::standard_registry();
    let mut dispatcher = Dispatcher::new(registry);
    dispatcher.handle_keys(&mut editor, "C-x");
    dispatcher.handle_keys(&mut editor, "t");
    dispatcher.handle_keys(&mut editor, "v");

    let first = editor.terminals.current().expect("a terminal").id;
    feed(
        &mut editor,
        first,
        "\u{1b}[1;32m~/maxgus\u{1b}[0m $ cargo test --workspace\r\n\
         \u{1b}[1;32m   Compiling\u{1b}[0m maxgus-term v0.1.0\r\n\
         \u{1b}[1;32m    Finished\u{1b}[0m `test` profile in 4.11s\r\n\
         \u{1b}[1;32m     Running\u{1b}[0m unittests src/lib.rs\r\n\r\n\
         test result: \u{1b}[1;32mok\u{1b}[0m. 1531 passed; 0 failed; 2 ignored\r\n\r\n\
         \u{1b}[1;32m~/maxgus\u{1b}[0m $ \u{1b}]0;cargo test\u{7}",
    );
    // A second tab, so the bar has something to show.
    dispatcher.handle_keys(&mut editor, "C-c");
    dispatcher.handle_keys(&mut editor, "t");
    let second = editor.terminals.current().expect("a terminal").id;
    feed(
        &mut editor,
        second,
        "\u{1b}]0;htop\u{7}\u{1b}[1;34m  PID USER      CPU%  MEM%  COMMAND\u{1b}[0m\r\n 4711 alejandro  2.1   0.4  maxgus\r\n",
    );
    editor.terminals.select(0);
    editor.tasks.drain();
    editor
}

fn feed(editor: &mut Editor, terminal: maxgus_core::task::TerminalId, bytes: &str) {
    editor
        .apply_task_result(TaskResult::TerminalOutput {
            terminal,
            bytes: bytes.as_bytes().to_vec(),
        })
        .unwrap();
}

/// What a server answers with for the file being shown.
fn outline() -> serde_json::Value {
    serde_json::json!([
        {"name": "score", "kind": 12, "detail": "fn(&str, &str) -> Option<i32>",
         "selectionRange": {"start": {"line": 7, "character": 7}}},
        {"name": "Match", "kind": 23,
         "selectionRange": {"start": {"line": 20, "character": 11}},
         "children": [
            {"name": "score", "kind": 8,
             "selectionRange": {"start": {"line": 21, "character": 4}}},
            {"name": "at", "kind": 8,
             "selectionRange": {"start": {"line": 22, "character": 4}}}
         ]},
        {"name": "matches", "kind": 12, "detail": "fn(&str) -> Vec<String>",
         "selectionRange": {"start": {"line": 30, "character": 7}}},
        {"name": "same", "kind": 12,
         "selectionRange": {"start": {"line": 40, "character": 3}}}
    ])
}

fn tree() -> Vec<VisibleNode> {
    let node =
        |path: &str, name: &str, dir: bool, depth: usize, git: Option<GitStatus>| VisibleNode {
            path: path.into(),
            name: name.into(),
            kind: if dir {
                NodeKind::Directory
            } else {
                NodeKind::File
            },
            depth,
            expanded: dir,
            expandable: dir,
            git,
            is_root: depth == 0,
        };
    vec![
        node("/maxgus", "maxgus", true, 0, None),
        node("/maxgus/crates", "crates", true, 1, None),
        node("/maxgus/crates/maxgus-core", "maxgus-core", true, 2, None),
        node("/maxgus/crates/maxgus-core/src", "src", true, 3, None),
        node(
            "/maxgus/crates/maxgus-core/src/editor.rs",
            "editor.rs",
            false,
            4,
            None,
        ),
        node(
            "/maxgus/crates/maxgus-core/src/fuzzy.rs",
            "fuzzy.rs",
            false,
            4,
            Some(GitStatus::Added),
        ),
        node(
            "/maxgus/crates/maxgus-core/src/render.rs",
            "render.rs",
            false,
            4,
            Some(GitStatus::Modified),
        ),
        node("/maxgus/crates/maxgus-faces", "maxgus-faces", true, 2, None),
        node(
            "/maxgus/crates/maxgus-syntax",
            "maxgus-syntax",
            true,
            2,
            None,
        ),
        node("/maxgus/crates/maxgus-tree", "maxgus-tree", true, 2, None),
        node("/maxgus/docs", "docs", true, 1, None),
        node("/maxgus/docs/themes", "themes", true, 2, None),
        node(
            "/maxgus/docs/themes/dracula.kdl",
            "dracula.kdl",
            false,
            3,
            None,
        ),
        node(
            "/maxgus/docs/themes/gruvbox.kdl",
            "gruvbox.kdl",
            false,
            3,
            None,
        ),
        node("/maxgus/docs/themes/nord.kdl", "nord.kdl", false, 3, None),
        node("/maxgus/Cargo.toml", "Cargo.toml", false, 1, None),
        node(
            "/maxgus/README.md",
            "README.md",
            false,
            1,
            Some(GitStatus::Modified),
        ),
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
        let Some(cell) = surface.get(x, y) else {
            continue;
        };
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
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write(path: &std::path::Path, svg: &str) -> std::io::Result<()> {
    std::fs::write(path, svg)
}
