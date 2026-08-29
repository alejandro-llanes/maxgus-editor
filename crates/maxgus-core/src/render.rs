//! Drawing the editor into a surface.
//!
//! Redisplay is a pure function of editor state: given an [`Editor`] and a
//! [`Surface`], it paints and returns. Nothing here reads input, touches the
//! terminal or mutates the editor, which is what lets a whole screen be
//! rendered in a test and asserted on line by line.
//!
//! Faces are composed in layers, lowest first: the default face, then syntax
//! highlighting, then the region, then search matches, then diagnostics. Each
//! layer only sets what it means to change, so a search match keeps its
//! syntax colour and takes only the highlight background.

use crate::{editor::Editor, window::Window};
use maxgus_faces::{Face, Theme};
use maxgus_text::{Buffer, Range};
use maxgus_tui::{Rect, Surface};

/// Paints the whole frame.
pub fn draw(editor: &Editor, surface: &mut Surface) {
    let theme = &editor.theme;
    let default = theme.resolve("default");
    surface.clear(default);

    let frame = surface.area();
    if frame.height == 0 {
        return;
    }
    // The last row is the echo area; the windows share what is left, which is
    // the same split `Editor::set_frame` lays them out into.
    let (body, echo) = frame.split_bottom(1);
    for window in editor.windows.iter() {
        if let Some(area) = window.rect.intersect(&body) {
            draw_window(editor, surface, window, area);
        }
    }
    // The popup goes over the top of the windows rather than resizing them, so
    // opening the list does not reflow what is being edited. It carries the
    // prompt with it, and the echo area stays out of the way while it is up.
    #[cfg(feature = "git")]
    if let Some(active) = editor.transient.as_ref() {
        draw_transient(editor, surface, frame, active);
    }
    match completion_popup(editor, frame) {
        Some(area) => {
            draw_completion_popup(editor, surface, area);
            surface.clear_rect(echo, default);
        }
        None => draw_echo_area(editor, surface, echo),
    }
}

/// Paints one window: its contents and its mode line.
/// Paints a window belonging to a subsystem — a magit view, the terminal —
/// and says whether it did.
///
/// Kept apart from `draw_window` so a build without those subsystems has one
/// place that knows they are absent, rather than a `cfg` in the middle of a
/// chain of `else if`s.
fn draw_subsystem_window(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    area: Rect,
    name: &str,
) -> bool {
    #[cfg(feature = "git")]
    {
        if name == crate::commands::git::STATUS_BUFFER_NAME {
            draw_git_status(editor, surface, window, area);
            return true;
        }
        if let Some(view) = editor.git_diffs.get(name) {
            draw_git_diff(editor, surface, window, area, view);
            return true;
        }
        if let Some(view) = editor.git_lists.get(name) {
            draw_git_list(editor, surface, window, area, view);
            return true;
        }
    }
    #[cfg(feature = "terminal")]
    if Some(window.id) == editor.terminal_window {
        draw_terminal(editor, surface, area);
        return true;
    }
    let _ = (editor, surface, window, area, name);
    false
}

fn draw_window(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let Some(buffer) = editor.buffers.get(window.buffer) else {
        return;
    };
    let selected = window.id == editor.windows.current_id();
    let (text_area, mode_line_area) = area.split_bottom(1);
    // The tree is drawn from its own snapshot rather than as buffer text, so
    // each node can carry the face its kind and git status call for.
    let name = editor
        .buffers
        .get(window.buffer)
        .map(|b| b.name().to_string())
        .unwrap_or_default();
    if draw_subsystem_window(editor, surface, window, text_area, &name) {
        // Drawn by whichever subsystem owns it.
    } else if name == crate::commands::tree::TREE_BUFFER_NAME {
        draw_tree(editor, surface, window, text_area);
    } else if name == crate::commands::tree::SYMBOLS_BUFFER_NAME {
        draw_symbols(editor, surface, window, text_area);
    } else if name == crate::commands::tree::BUFFERS_BUFFER_NAME {
        draw_buffer_list(editor, surface, window, text_area);
    } else {
        draw_text(editor, surface, window, buffer, text_area);
    }
    draw_mode_line(editor, surface, window, mode_line_area, selected);
}

#[cfg(feature = "git")]
/// Paints a diff or revision buffer: a title, what is above the diff, then
/// the files with their hunks.
fn draw_git_diff(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    area: Rect,
    view: &crate::git::DiffView,
) {
    let theme = &editor.theme;
    surface.clear_rect(area, theme.resolve("default"));
    let cursor = editor
        .buffers
        .get(window.buffer)
        .map(|buffer| buffer.line_of(window.point))
        .unwrap_or(0);

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(entry) = view.row(line) else { break };
        let y = area.y + row;
        let selected = line == cursor;
        let here = Rect::new(area.x, y, area.width, 1);
        if selected {
            surface.clear_rect(here, theme.resolve("magit-section-highlight"));
        }
        let face = |name: &str| {
            let mut face = theme.resolve(name);
            if selected {
                face.overlay(&theme.resolve_overlay("magit-section-highlight"));
            }
            face
        };
        use crate::git::DiffRow;
        match entry {
            DiffRow::Title => {
                let (added, removed) = view.counts();
                let x = surface.set_string(
                    area.x,
                    y,
                    &view.title,
                    face("magit-section-heading"),
                    area.width,
                );
                let summary = format!("  +{added} \u{2212}{removed}");
                surface.set_string(
                    x,
                    y,
                    &summary,
                    face("shadow"),
                    area.right().saturating_sub(x),
                );
            }
            DiffRow::Preamble(index) => {
                if let Some((text, name)) = view.preamble.get(*index) {
                    surface.set_string(area.x, y, text, face(name), area.width);
                }
            }
            DiffRow::Blank => {}
            DiffRow::Empty => {
                surface.set_string(area.x, y, "No changes", face("shadow"), area.width);
            }
            DiffRow::File(index) => {
                let Some(file) = view.files.get(*index) else {
                    continue;
                };
                let folded = view.is_collapsed(&file.path);
                let mut x = area.x;
                x += surface.set_char(
                    x,
                    y,
                    if folded { '\u{25b8}' } else { '\u{25be}' },
                    face("shadow"),
                ) + 1;
                x = surface.set_string(
                    x,
                    y,
                    &file.path,
                    face("magit-diff-file-heading"),
                    area.right().saturating_sub(x),
                );
                let (added, removed) = file.counts();
                let label = format!("+{added} \u{2212}{removed}");
                let at = area
                    .right()
                    .saturating_sub(label.chars().count() as u16 + 1);
                if at > x {
                    surface.set_string(at, y, &label, face("shadow"), area.width);
                }
            }
            DiffRow::Hunk(file, hunk) => {
                let text = view
                    .files
                    .get(*file)
                    .and_then(|file| file.hunks.get(*hunk))
                    .map(|hunk| hunk.header.clone())
                    .unwrap_or_default();
                let band = face("magit-diff-hunk-heading");
                surface.clear_rect(here, band);
                surface.set_string(area.x + 2, y, &text, band, area.width);
            }
            DiffRow::Line(file, hunk, line) => {
                let Some(line) = view
                    .files
                    .get(*file)
                    .and_then(|file| file.hunks.get(*hunk))
                    .and_then(|hunk| hunk.lines.get(*line))
                else {
                    continue;
                };
                let name = match line.kind {
                    maxgus_git::LineKind::Added => "magit-diff-added",
                    maxgus_git::LineKind::Removed => "magit-diff-removed",
                    _ => "magit-diff-context",
                };
                let painted = face(name);
                if line.kind != maxgus_git::LineKind::Context {
                    surface.clear_rect(here, painted);
                }
                surface.set_string(area.x + 2, y, &line.to_patch_line(), painted, area.width);
            }
        }
    }
}

#[cfg(feature = "git")]
/// Paints a log, the references, or what git has been asked to do.
fn draw_git_list(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    area: Rect,
    view: &crate::git::ListView,
) {
    let theme = &editor.theme;
    surface.clear_rect(area, theme.resolve("default"));
    let cursor = editor
        .buffers
        .get(window.buffer)
        .map(|buffer| buffer.line_of(window.point))
        .unwrap_or(0);

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(entry) = view.lines.get(line) else {
            break;
        };
        let y = area.y + row;
        let selected = line == cursor;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("magit-section-highlight"),
            );
        }
        let mut x = area.x;
        for (text, name) in &entry.spans {
            let mut face = theme.resolve(name);
            if selected {
                face.overlay(&theme.resolve_overlay("magit-section-highlight"));
            }
            x = surface.set_string(x, y, text, face, area.right().saturating_sub(x));
            if x >= area.right() {
                break;
            }
        }
    }
}

#[cfg(feature = "git")]
/// The text of one diff-buffer row, which is what point moves through.
pub fn git_diff_row_text(view: &crate::git::DiffView, row: &crate::git::DiffRow) -> String {
    use crate::git::DiffRow;
    match row {
        DiffRow::Title => view.title.clone(),
        DiffRow::Preamble(index) => view
            .preamble
            .get(*index)
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        DiffRow::Blank => String::new(),
        DiffRow::Empty => "No changes".to_string(),
        DiffRow::File(index) => view
            .files
            .get(*index)
            .map(|file| {
                let (added, removed) = file.counts();
                format!("{}  +{added} -{removed}", file.path)
            })
            .unwrap_or_default(),
        DiffRow::Hunk(file, hunk) => view
            .files
            .get(*file)
            .and_then(|file| file.hunks.get(*hunk))
            .map(|hunk| hunk.header.clone())
            .unwrap_or_default(),
        DiffRow::Line(file, hunk, line) => view
            .files
            .get(*file)
            .and_then(|file| file.hunks.get(*hunk))
            .and_then(|hunk| hunk.lines.get(*line))
            .map(|line| line.to_patch_line())
            .unwrap_or_default(),
    }
}

#[cfg(feature = "git")]
/// Paints the git status view.
///
/// Drawn from the row list rather than from the buffer's text so that every
/// kind of row can carry its own faces — a hunk heading its band, an added
/// line its green, a branch name its colour — while point still moves through
/// ordinary buffer lines.
fn draw_git_status(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let theme = &editor.theme;
    surface.clear_rect(area, theme.resolve("default"));
    let cursor_line = editor.git_cursor_line();

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(entry) = editor.git.row(line) else {
            break;
        };
        let y = area.y + row;
        let selected = line == cursor_line;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("magit-section-highlight"),
            );
        }
        let face = |name: &str| {
            let mut face = theme.resolve(name);
            if selected {
                face.overlay(&theme.resolve_overlay("magit-section-highlight"));
            }
            face
        };
        draw_git_row(
            editor,
            surface,
            entry,
            Rect::new(area.x, y, area.width, 1),
            &face,
        );
    }
}

#[cfg(feature = "git")]
fn draw_git_row(
    editor: &Editor,
    surface: &mut Surface,
    row: &crate::git::Row,
    area: Rect,
    face: &dyn Fn(&str) -> Face,
) {
    use crate::git::Row;
    let right = area.right();
    match row {
        Row::Blank => {}
        Row::Header(head) => {
            let mut x = surface.set_string(area.x, area.y, &head.label, face("shadow"), area.width);
            x = surface.set_string(
                area.x + 9.min(area.width),
                area.y,
                &head.reference,
                face("magit-branch-local"),
                right.saturating_sub(x),
            );
            surface.set_string(
                x + 1,
                area.y,
                &head.subject,
                face("default"),
                right.saturating_sub(x),
            );
        }
        Row::Section(section) => {
            let folded = editor.git.is_collapsed(*section);
            let mut x = area.x;
            x += surface.set_char(
                x,
                area.y,
                if folded { '\u{25b8}' } else { '\u{25be}' },
                face("shadow"),
            );
            x = surface.set_string(
                x + 1,
                area.y,
                section.title(),
                face("magit-section-heading"),
                right.saturating_sub(x),
            );
            let count = format!(" ({})", editor.git.count(*section));
            surface.set_string(x, area.y, &count, face("shadow"), right.saturating_sub(x));
        }
        Row::Empty(_) => {
            surface.set_string(
                area.x,
                area.y,
                "Nothing to commit, the working tree is clean",
                face("success"),
                area.width,
            );
        }
        Row::File { section, file } => {
            let path = editor
                .git
                .paths(*section)
                .get(*file)
                .cloned()
                .unwrap_or_default();
            let expanded = editor.git.is_file_expanded(*section, &path);
            let mut x = area.x + 2;
            if section.is_files() && !editor.git.files(*section).is_empty() {
                x += surface.set_char(
                    x,
                    area.y,
                    if expanded { '\u{25be}' } else { '\u{25b8}' },
                    face("shadow"),
                );
                x += 1;
            } else {
                x += 2;
            }
            // The word for what happened, in the colour of what happened.
            if let Some(word) = git_word(editor, *section, &path) {
                let name = match word {
                    "deleted" => "magit-diff-removed",
                    "new file" => "magit-diff-added",
                    _ => "shadow",
                };
                x = surface.set_string(
                    x,
                    area.y,
                    &format!("{word:<11}"),
                    face(name),
                    right.saturating_sub(x),
                );
            }
            x = surface.set_string(
                x,
                area.y,
                &path,
                face("magit-diff-file-heading"),
                right.saturating_sub(x),
            );
            // The size of the change, pushed to the right edge where a column
            // of them reads as a column.
            if let Some(diff) = editor.git.files(*section).get(*file) {
                let (added, removed) = diff.counts();
                let label = format!("+{added} \u{2212}{removed}");
                let at = right.saturating_sub(label.chars().count() as u16 + 1);
                if at > x {
                    let mut x = at;
                    x = surface.set_string(
                        x,
                        area.y,
                        &format!("+{added}"),
                        face("magit-diff-added"),
                        right.saturating_sub(x),
                    );
                    surface.set_string(
                        x + 1,
                        area.y,
                        &format!("\u{2212}{removed}"),
                        face("magit-diff-removed"),
                        right.saturating_sub(x),
                    );
                }
            }
        }
        Row::Hunk {
            section,
            file,
            hunk,
        } => {
            let text = editor
                .git
                .files(*section)
                .get(*file)
                .and_then(|diff| diff.hunks.get(*hunk))
                .map(|hunk| hunk.header.clone())
                .unwrap_or_default();
            // A band across the whole width, which is what makes a long diff
            // readable: the eye finds the next hunk without counting.
            let band = face("magit-diff-hunk-heading");
            surface.clear_rect(area, band);
            surface.set_string(area.x + 4, area.y, &text, band, area.width);
        }
        Row::Line {
            section,
            file,
            hunk,
            line,
        } => {
            let Some(diff) = editor.git.files(*section).get(*file) else {
                return;
            };
            let Some(line) = diff.hunks.get(*hunk).and_then(|hunk| hunk.lines.get(*line)) else {
                return;
            };
            let name = match line.kind {
                maxgus_git::LineKind::Added => "magit-diff-added",
                maxgus_git::LineKind::Removed => "magit-diff-removed",
                _ => "magit-diff-context",
            };
            let painted = face(name);
            // The band runs the full width so a row of added lines reads as a
            // block rather than as ragged text.
            if line.kind != maxgus_git::LineKind::Context {
                surface.clear_rect(area, painted);
            }
            surface.set_string(
                area.x + 4,
                area.y,
                &line.to_patch_line(),
                painted,
                area.width,
            );
        }
        Row::Stash(index) => {
            let Some(stash) = editor.git.stashes.get(*index) else {
                return;
            };
            let mut x = surface.set_string(
                area.x + 2,
                area.y,
                &stash.name,
                face("magit-hash"),
                area.width,
            );
            surface.set_string(
                x + 1,
                area.y,
                &stash.subject,
                face("default"),
                right.saturating_sub(x),
            );
            x = x.max(area.x);
            let _ = x;
        }
        Row::Commit { section, commit } => {
            let Some(commit) = editor.git.commits(*section).get(*commit) else {
                return;
            };
            let mut x = surface.set_string(
                area.x + 2,
                area.y,
                &commit.short,
                face("magit-hash"),
                area.width,
            );
            x += 1;
            // Branch and tag names as chips, coloured by what they are.
            for reference in &commit.refs {
                let name = if reference.starts_with("tag: ") {
                    "magit-tag"
                } else if reference.contains('/') {
                    "magit-branch-remote"
                } else {
                    "magit-branch-local"
                };
                let text = format!("{} ", reference.trim_start_matches("tag: "));
                x = surface.set_string(x, area.y, &text, face(name), right.saturating_sub(x));
            }
            surface.set_string(
                x,
                area.y,
                &commit.subject,
                face("default"),
                right.saturating_sub(x),
            );
        }
    }
}

#[cfg(feature = "git")]
/// The word describing what happened to a file, from the status.
fn git_word(editor: &Editor, section: crate::git::Section, path: &str) -> Option<&'static str> {
    use crate::git::Section;
    let entry = editor
        .git
        .status
        .entries
        .iter()
        .find(|e| e.path.to_string_lossy() == path)?;
    Some(match section {
        Section::Untracked => "new file",
        Section::Unmerged => "unmerged",
        Section::Staged => entry.index.label(),
        _ => entry.worktree.label(),
    })
}

#[cfg(feature = "terminal")]
/// Paints the terminal panel: a bar of tabs, then the screen of the one
/// showing.
///
/// The cells come from the emulator with the colours the program asked for,
/// so nothing here decides what anything looks like except where a cell is
/// unpainted, which falls back to the `terminal` face.
fn draw_terminal(editor: &Editor, surface: &mut Surface, area: Rect) {
    if area.height == 0 {
        return;
    }
    let theme = &editor.theme;
    let (bar, screen) = area.split_top(1);
    draw_terminal_tabs(editor, surface, bar);

    let base = theme.resolve("terminal");
    surface.clear_rect(screen, base);
    let Some(terminal) = editor.terminals.current() else {
        surface.set_string(
            screen.x + 1,
            screen.y,
            "No terminal",
            theme.resolve("shadow"),
            screen.width,
        );
        return;
    };

    let grid = terminal.emulator.grid();
    let top = terminal.top_line();
    let lines: Vec<&maxgus_term::Line> = grid.all_lines().collect();
    let region = theme.resolve_overlay("region");

    for row in 0..screen.height {
        let Some(line) = lines.get(top + row as usize) else {
            break;
        };
        let y = screen.y + row;
        for (column, cell) in line.cells.iter().enumerate() {
            let x = screen.x + column as u16;
            if x >= screen.right() || cell.wide_continuation {
                continue;
            }
            // The program's colours over the terminal's own, so a cell that
            // asked for nothing takes the theme rather than black on black.
            let mut face = cell.face;
            face.inherit_from(&base);
            let absolute = top + row as usize;
            if terminal
                .selection
                .is_some_and(|s| s.contains(absolute, column))
            {
                face.overlay(&region);
            }
            surface.set_char(x, y, cell.ch, face);
        }
    }
}

#[cfg(feature = "terminal")]
/// The bar of tabs across the top of the panel.
fn draw_terminal_tabs(editor: &Editor, surface: &mut Surface, area: Rect) {
    let theme = &editor.theme;
    let plain = theme.resolve("terminal-tab");
    surface.clear_rect(area, plain);

    let mut x = area.x;
    for (index, terminal) in editor.terminals.iter().enumerate() {
        if x >= area.right() {
            break;
        }
        let selected = index == editor.terminals.current_index();
        let face = match (selected, terminal.exited.is_some()) {
            (_, true) => theme.resolve("terminal-exited"),
            (true, _) => theme.resolve("terminal-tab-selected"),
            _ => plain,
        };
        // The number is what `C-c 1` and friends refer to, so it is shown.
        let label = format!(" {} {} ", index + 1, terminal.label());
        x = surface.set_string(x, area.y, &label, face, area.right().saturating_sub(x));
        x = surface.set_string(x, area.y, "\u{2502}", plain, area.right().saturating_sub(x));
    }
    // What mode the keys are in, said where the eye already is.
    let note = match editor.terminals.current() {
        Some(terminal) if terminal.in_copy_mode() => "  READING  C-g to type  ",
        _ => "",
    };
    if !note.is_empty() {
        let at = area.right().saturating_sub(note.chars().count() as u16);
        if at > x {
            surface.set_string(
                at,
                area.y,
                note,
                theme.resolve("terminal-tab-selected"),
                area.width,
            );
        }
    }
}

/// Paints the file tree: one node per row, each in the face its kind asks for.
fn draw_tree(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let theme = &editor.theme;
    let cursor = editor.tree_cursor_line();
    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(node) = editor.tree.get(line) else {
            break;
        };
        let y = area.y + row;
        let selected = line == cursor;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("tree-selected"),
            );
        }
        let face = section_face(theme, selected);
        draw_tree_row(editor, surface, node, area, y, &face);
    }
}

/// Paints the symbol outline: one symbol per row, folded ones hidden.
fn draw_symbols(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let theme = &editor.theme;
    let icons = editor.settings.nerd_font_icons;
    let visible = editor.panel.visible_symbols();
    let cursor = editor
        .buffers
        .find_by_name(crate::commands::tree::SYMBOLS_BUFFER_NAME)
        .map(|id| editor.line_in(id))
        .unwrap_or(0);

    if visible.is_empty() {
        let note = if editor.panel.symbols_pending {
            "Reading…"
        } else {
            "No symbols"
        };
        surface.set_string(
            area.x + 1,
            area.y,
            note,
            theme.resolve("panel-note"),
            area.width,
        );
        return;
    }
    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(index) = visible.get(line) else {
            break;
        };
        let Some(symbol) = editor.panel.symbols.get(*index) else {
            break;
        };
        let y = area.y + row;
        let selected = line == cursor;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("tree-selected"),
            );
        }
        let face = section_face(theme, selected);
        draw_symbol_row(surface, symbol, area, y, icons, &face);
    }
}

/// Paints the list of open buffers.
fn draw_buffer_list(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let theme = &editor.theme;
    let icons = editor.settings.nerd_font_icons;
    let listed = editor.panel_buffers();
    let cursor = editor
        .buffers
        .find_by_name(crate::commands::tree::BUFFERS_BUFFER_NAME)
        .map(|id| editor.line_in(id))
        .unwrap_or(0);

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some((id, _)) = listed.get(line) else {
            break;
        };
        let y = area.y + row;
        let selected = line == cursor;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("tree-selected"),
            );
        }
        let face = section_face(theme, selected);
        draw_buffer_row(editor, surface, *id, area, y, icons, &face);
    }
}

/// The face lookup a panel row uses: its own, with the cursor row's highlight
/// laid over it when this is the row point is on.
fn section_face(theme: &Theme, selected: bool) -> impl Fn(&str) -> Face + '_ {
    move |name: &str| {
        let mut face = theme.resolve(name);
        if selected {
            face.overlay(&theme.resolve_overlay("tree-selected"));
        }
        face
    }
}

fn draw_tree_row(
    editor: &Editor,
    surface: &mut Surface,
    node: &maxgus_tree::VisibleNode,
    area: Rect,
    y: u16,
    face: &dyn Fn(&str) -> Face,
) {
    let mut x = area.x + (node.depth as u16 * 2).min(area.width);
    // The arrow marks what can be opened.
    x = surface.set_string(x, y, node.arrow(), face("tree-arrow"), area.right() - x);
    // The glyph says what kind of thing it is at a glance, in the face of
    // the node itself so a directory's icon reads as a directory.
    if editor.settings.nerd_font_icons {
        let icon = format!("{} ", tree_glyph(node));
        x = surface.set_string(
            x,
            y,
            &icon,
            face(node.face()),
            area.right().saturating_sub(x),
        );
    }
    x = surface.set_string(
        x,
        y,
        &node.name,
        face(node.face()),
        area.right().saturating_sub(x),
    );

    // The git indicator sits at the right edge, where a column of them
    // reads as a column.
    if let Some(status) = node.git {
        let at = area.right().saturating_sub(2).max(x + 1);
        if at < area.right() {
            surface.set_char(at, y, status.indicator(), face(status.face()));
        }
    }
}

/// One symbol of the outline: its arrow, its glyph, its name, then whatever
/// the server said about it, dimmed and pushed right.
fn draw_symbol_row(
    surface: &mut Surface,
    symbol: &crate::panel::Symbol,
    area: Rect,
    y: u16,
    icons: bool,
    face: &dyn Fn(&str) -> Face,
) {
    let indent = ((symbol.depth as u16 + 1) * 2).min(area.width);
    let mut x = area.x + indent;
    x = surface.set_string(x, y, symbol.arrow(), face("tree-arrow"), area.right() - x);
    if icons {
        let icon = format!("{} ", crate::icons::for_symbol(symbol.kind));
        x = surface.set_string(
            x,
            y,
            &icon,
            face(symbol.face()),
            area.right().saturating_sub(x),
        );
    }
    x = surface.set_string(
        x,
        y,
        &symbol.name,
        face(symbol.face()),
        area.right().saturating_sub(x),
    );

    // The kind, when there is room for it, so `fn` and `struct` are visible
    // without the glyph having to carry the whole meaning.
    let kind = symbol.kind_name();
    if !kind.is_empty() {
        let at = area.right().saturating_sub(kind.chars().count() as u16 + 1);
        if at > x + 1 {
            surface.set_string(at, y, kind, face("symbol-detail"), area.width);
        }
    }
}

/// One open buffer, marked when it is the one being edited or is unsaved.
fn draw_buffer_row(
    editor: &Editor,
    surface: &mut Surface,
    id: maxgus_text::BufferId,
    area: Rect,
    y: u16,
    icons: bool,
    face: &dyn Fn(&str) -> Face,
) {
    let Some(buffer) = editor.buffers.get(id) else {
        return;
    };
    let current = editor
        .windows
        .iter()
        .find(|w| !editor.panel_windows.contains(&w.id))
        .is_some_and(|w| w.buffer == id);
    let name_face = if current {
        "panel-current-buffer"
    } else {
        "tree-file"
    };

    let mut x = area.x + 2;
    // A bar down the left of the buffer being edited, which reads faster
    // than a colour difference alone.
    x = surface.set_string(
        x,
        y,
        if current { "\u{2502} " } else { "  " },
        face(name_face),
        area.width,
    );
    if icons {
        let glyph = match buffer.path() {
            Some(path) => crate::icons::for_file(path),
            None => crate::icons::for_language(buffer.language().unwrap_or_default()),
        };
        x = surface.set_string(
            x,
            y,
            &format!("{glyph} "),
            face(name_face),
            area.right().saturating_sub(x),
        );
    }
    x = surface.set_string(
        x,
        y,
        buffer.name(),
        face(name_face),
        area.right().saturating_sub(x),
    );

    if buffer.is_modified() {
        let at = area.right().saturating_sub(2).max(x + 1);
        if at < area.right() {
            surface.set_char(at, y, '\u{2022}', face("error"));
        }
    }
}

/// The glyph for one row of the tree.
///
/// A directory says whether it is open, because that is the one thing the
/// arrow beside it already says and the two reading differently would be
/// worse than either alone.
fn tree_glyph(node: &maxgus_tree::VisibleNode) -> char {
    match node.kind {
        maxgus_tree::NodeKind::Directory => match node.expanded {
            true => crate::icons::DIRECTORY_OPEN,
            false => crate::icons::DIRECTORY,
        },
        maxgus_tree::NodeKind::Symlink => crate::icons::SYMLINK,
        maxgus_tree::NodeKind::File => crate::icons::for_file(&node.path),
    }
}

/// The width the line-number column takes, including its trailing space.
///
/// Shared with `Editor::cursor_position`, which has to move the cursor over by
/// the same amount the text is moved over — otherwise it sits in the gutter,
/// three columns adrift of the character it is on.
pub(crate) fn line_number_width(editor: &Editor, buffer: &Buffer) -> u16 {
    if !editor.settings.line_numbers {
        return 0;
    }
    // Enough digits for the last line, plus a separating space.
    let digits = buffer.len_lines().max(1).to_string().len();
    (digits + 1) as u16
}

/// Paints a window's buffer text.
fn draw_text(editor: &Editor, surface: &mut Surface, window: &Window, buffer: &Buffer, area: Rect) {
    let theme = &editor.theme;
    let gutter = line_number_width(editor, buffer);
    let point_line = buffer.line_of(window.point.min(buffer.len_chars()));
    // The extra cursors, so it is obvious where typing will go.
    let extra_cursors: Vec<usize> = match window.id == editor.windows.current_id() {
        true => editor.cursors.offsets().to_vec(),
        false => Vec::new(),
    };
    // Diagnostics are resolved once for the whole window. Doing it per line
    // would repeat the work for every row on screen.
    #[cfg(feature = "lsp")]
    let diagnostics = resolve_diagnostics(editor, buffer);
    #[cfg(not(feature = "lsp"))]
    let diagnostics: Vec<(Range, &'static str)> = Vec::new();
    // The other matches of a running search, and the delimiter matching the
    // one under point. Both are resolved once for the window, like the
    // diagnostics: computing them per line would repeat the work per row.
    let first_line = window.top_line;
    let last_line = (window.top_line + area.height as usize).min(buffer.len_lines());
    let matches = resolve_search_matches(editor, buffer, first_line, last_line);
    let paren = matching_delimiter(editor, buffer, window);

    // The fill column, marked before the text so the text draws over it.
    if editor.settings.fill_column_indicator {
        draw_fill_column(editor, surface, window, area, gutter);
    }

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let y = area.y + row;
        if line >= buffer.len_lines() {
            // Past the end of the buffer: Emacs draws nothing, not tildes.
            continue;
        }
        if gutter > 0 {
            draw_line_number(surface, theme, line, point_line, area.x, y, gutter);
        }
        draw_line(
            editor,
            surface,
            window,
            buffer,
            line,
            &LineArea { area, gutter },
            &Overlays {
                diagnostics: &diagnostics,
                matches: &matches,
                paren,
            },
        );
    }

    // The extra cursors, painted over the text once it is drawn. A block
    // where the terminal cannot put a second hardware cursor, which is the
    // only way to show where typing will also go.
    let face = theme.resolve("cursor");
    for cursor in &extra_cursors {
        let Some((x, y)) = cell_of(*cursor, buffer, window, &LineArea { area, gutter }) else {
            continue;
        };
        let mut cell = surface.get(x, y).copied().unwrap_or_default();
        cell.face = face;
        surface.set(x, y, cell);
    }
}

/// The screen cell an offset is drawn in, or `None` when it is not on screen.
fn cell_of(offset: usize, buffer: &Buffer, window: &Window, area: &LineArea) -> Option<(u16, u16)> {
    let line = buffer.line_of(offset.min(buffer.len_chars()));
    if line < window.top_line {
        return None;
    }
    let row = line - window.top_line;
    if row >= area.area.height as usize {
        return None;
    }
    let column = buffer
        .display_column(offset)
        .checked_sub(window.left_column)?;
    let x = area.area.x + area.gutter + column as u16;
    if x >= area.area.right() {
        return None;
    }
    Some((x, area.area.y + row as u16))
}

fn draw_line_number(
    surface: &mut Surface,
    theme: &Theme,
    line: usize,
    point_line: usize,
    x: u16,
    y: u16,
    width: u16,
) {
    let face = if line == point_line {
        theme.resolve("line-number-current-line")
    } else {
        theme.resolve("line-number")
    };
    // Right-aligned in the column, with the separating space after it.
    let text = format!("{:>width$} ", line + 1, width = (width - 1) as usize);
    surface.set_string(x, y, &text, face, width);
}

/// Paints one line of buffer text, honouring horizontal scroll and tabs.
/// Where one line of text goes: the window area and the gutter taken out of it.
struct LineArea {
    area: Rect,
    gutter: u16,
}

/// What is drawn over the buffer text, resolved once for the whole window.
struct Overlays<'a> {
    diagnostics: &'a [(Range, &'static str)],
    /// Every match of a running search that is on screen.
    matches: &'a [Range],
    /// The delimiter matching the one under point, and the one under point.
    paren: Option<(usize, usize)>,
}

fn draw_line(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    buffer: &Buffer,
    line: usize,
    place: &LineArea,
    overlays: &Overlays<'_>,
) {
    let LineArea { area, gutter } = *place;
    let start = buffer.line_start(line);
    let end = maxgus_text::Motion::line_end(buffer.rope(), start);
    let layers = Layers::new(editor, window, buffer, line, overlays);

    let left = area.x + gutter;
    let right = area.right();
    // Display column of the first character shown, for horizontal scrolling.
    let mut column = 0usize;
    let mut offset = start;

    while offset < end {
        let c = buffer.rope().char(offset);
        let width = buffer.char_display_width(c, column);
        let face = layers.face_at(offset, c);

        // Skip what horizontal scrolling has moved off the left edge.
        if column + width > window.left_column {
            let x = left + (column.saturating_sub(window.left_column) as u16);
            if x >= right {
                break;
            }
            match c {
                // A tab paints as blanks up to the next tab stop.
                '\t' => {
                    for i in 0..width {
                        let at = x + i as u16;
                        if at < right {
                            surface.set(at, area.y + line_row(window, line), cell(' ', face));
                        }
                    }
                }
                // Control characters show as `^X`, as Emacs draws them.
                c if (c as u32) < 0x20 => {
                    let caret = format!("^{}", (b'@' + c as u8) as char);
                    surface.set_string(x, area.y + line_row(window, line), &caret, face, right - x);
                }
                c => {
                    surface.set_char(x, area.y + line_row(window, line), c, face);
                }
            }
        }
        column += width;
        offset += 1;
    }

    // The region and search highlights extend across the newline, so a
    // selected line reads as selected all the way to the right edge.
    if let Some(face) = layers.eol_face() {
        let x = left + (column.saturating_sub(window.left_column) as u16);
        for at in x..right {
            surface.set(at, area.y + line_row(window, line), cell(' ', face));
        }
    }
}

/// The row within the window that `line` is drawn on.
fn line_row(window: &Window, line: usize) -> u16 {
    (line - window.top_line) as u16
}

fn cell(ch: char, face: Face) -> maxgus_tui::Cell {
    maxgus_tui::Cell::new(ch, face)
}

/// The face layers in effect for one line.
struct Layers<'a> {
    theme: &'a Theme,
    default: Face,
    /// Syntax spans overlapping this line, in byte offsets.
    #[cfg(feature = "syntax")]
    highlights: Vec<&'a maxgus_syntax::Highlight>,
    /// Read to turn a character offset into the byte offset the spans use.
    #[cfg(feature = "syntax")]
    rope: &'a ropey::Rope,
    region: Option<Range>,
    /// The search match point is on.
    current: Option<Range>,
    /// The other matches on this line.
    others: Vec<Range>,
    /// Delimiter positions to mark on this line.
    parens: Vec<usize>,
    diagnostics: Vec<(Range, &'static str)>,
    /// Trailing whitespace on this line, when the face is worth showing.
    trailing: Option<Range>,
    /// True when the region or a match runs past the end of this line.
    region_spans_eol: bool,
}

impl<'a> Layers<'a> {
    #[cfg_attr(not(feature = "syntax"), allow(unused_variables))]
    fn new(
        editor: &'a Editor,
        window: &Window,
        buffer: &'a Buffer,
        line: usize,
        overlays: &Overlays<'_>,
    ) -> Layers<'a> {
        let start = buffer.line_start(line);
        let end = maxgus_text::Motion::line_end(buffer.rope(), start);
        let line_range = Range::new(start, end);
        let rope = buffer.rope();

        #[cfg(feature = "syntax")]
        let highlights = {
            let line_start_byte = rope.char_to_byte(start);
            let line_end_byte = rope.char_to_byte(end);
            editor
                .highlights_for(window.buffer)
                .iter()
                .filter(|h| h.start < line_end_byte && line_start_byte < h.end)
                .collect()
        };

        // The region is only shown in the window whose buffer owns it.
        let region = buffer
            .region()
            .filter(|r| r.overlaps(&line_range) || r.is_empty());
        let region_spans_eol = region.is_some_and(|r| r.start <= end && r.end > end);

        // The match point is on is drawn differently from the others.
        let current = editor
            .isearch
            .as_ref()
            .and_then(|s| s.current)
            .filter(|m| m.overlaps(&line_range));
        let others: Vec<Range> = overlays
            .matches
            .iter()
            .filter(|m| m.overlaps(&line_range) && Some(**m) != current)
            .copied()
            .collect();
        let parens: Vec<usize> = overlays
            .paren
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .filter(|at| line_range.contains(*at))
            .collect();

        // Only the ones touching this line need carrying into the loop.
        let diagnostics: Vec<(Range, &'static str)> = overlays
            .diagnostics
            .iter()
            .filter(|(r, _)| r.overlaps(&line_range))
            .copied()
            .collect();
        let trailing = trailing_whitespace(buffer, line_range);

        Layers {
            theme: &editor.theme,
            default: editor.theme.resolve("default"),
            #[cfg(feature = "syntax")]
            highlights,
            #[cfg(feature = "syntax")]
            rope,
            region,
            current,
            others,
            parens,
            diagnostics,
            trailing,
            region_spans_eol,
        }
    }

    /// The composed face for the character at `offset`.
    fn face_at(&self, offset: usize, _c: char) -> Face {
        let mut face = self.default;

        // Syntax highlighting, looked up by byte offset: the spans come from
        // tree-sitter, which counts bytes.
        #[cfg(feature = "syntax")]
        if !self.highlights.is_empty() {
            let byte = self.rope.char_to_byte(offset);
            if let Some(span) = self
                .highlights
                .iter()
                .find(|h| h.start <= byte && byte < h.end)
            {
                face.overlay(&self.theme.resolve_overlay(span.face));
            }
        }
        if self.trailing.is_some_and(|r| r.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay("trailing-whitespace"));
        }
        if self.region.is_some_and(|r| r.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay("region"));
        }
        // Every match is marked, the one point is on more strongly.
        if self.others.iter().any(|m| m.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay("lazy-highlight"));
        }
        if self.current.is_some_and(|m| m.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay("isearch"));
        }
        if self.parens.contains(&offset) {
            face.overlay(&self.theme.resolve_overlay("match-paren"));
        }
        if let Some((_, name)) = self.diagnostics.iter().find(|(r, _)| r.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay(name));
        }
        face
    }

    /// The face to paint past the end of the line, when the region runs on.
    fn eol_face(&self) -> Option<Face> {
        self.region_spans_eol.then(|| {
            let mut face = self.default;
            face.overlay(&self.theme.resolve_overlay("region"));
            face
        })
    }
}

#[cfg(feature = "lsp")]
/// Every diagnostic for `buffer`, as character ranges with the face to use.
///
/// This is computed once per window. It used to be done per line, which meant
/// rendering the whole buffer to a string for each row on screen — fine on a
/// small file, ruinous on a large one.
fn resolve_diagnostics(editor: &Editor, buffer: &Buffer) -> Vec<(Range, &'static str)> {
    let Some(path) = buffer.path() else {
        return Vec::new();
    };
    let uri = maxgus_lsp::client::path_to_uri(path);
    let entries = editor.diagnostics.for_uri(&uri);
    if entries.is_empty() {
        return Vec::new();
    }
    let encoding = maxgus_lsp::PositionEncoding::Utf16;
    entries
        .iter()
        .map(|d| {
            let start = crate::position::offset_of_position(buffer, d.range.start, encoding);
            let end = crate::position::offset_of_position(buffer, d.range.end, encoding);
            // A zero-width diagnostic still marks the character it sits on.
            (Range::new(start.min(end), end.max(start + 1)), d.face())
        })
        .collect()
}

/// Marks the fill column down the height of the window.
fn draw_fill_column(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    area: Rect,
    gutter: u16,
) {
    let column = editor.settings.fill_column;
    // Nothing to mark if the column has scrolled off the left.
    let Some(offset) = column.checked_sub(window.left_column) else {
        return;
    };
    let x = area.x + gutter + offset as u16;
    if x >= area.right() {
        return;
    }
    let face = editor.theme.resolve("fill-column-indicator");
    for y in area.y..area.bottom() {
        surface.set(x, y, maxgus_tui::Cell::new('│', face));
    }
}

/// Every match of a running search that the window can show.
///
/// Only the visible text is searched: scanning the whole buffer on every frame
/// would cost the size of the file, and nothing off screen would be drawn.
fn resolve_search_matches(
    editor: &Editor,
    buffer: &Buffer,
    first_line: usize,
    last_line: usize,
) -> Vec<Range> {
    let Some(search) = editor.isearch.as_ref() else {
        return Vec::new();
    };
    if search.query.is_empty() || search.failing {
        return Vec::new();
    }
    let Ok(query) =
        maxgus_text::SearchQuery::new(&search.query, search.kind, editor.settings.case_fold_search)
    else {
        return Vec::new();
    };
    let start = buffer.line_start(first_line);
    let end = buffer.line_start(last_line);
    if end <= start {
        return Vec::new();
    }
    let visible = ropey::Rope::from_str(&buffer.slice(Range::new(start, end)));
    query
        .find_all(&visible)
        .into_iter()
        .map(|m| Range::new(start + m.range.start, start + m.range.end))
        .collect()
}

/// The delimiter under or before point and its partner, when they match.
///
/// This is `show-paren-mode`: seeing which bracket closes the one you are on
/// is most of what makes editing nested code bearable.
fn matching_delimiter(editor: &Editor, buffer: &Buffer, window: &Window) -> Option<(usize, usize)> {
    // Only the selected window marks a pair, so a split does not show two.
    if window.id != editor.windows.current_id() {
        return None;
    }
    let point = window.point.min(buffer.len_chars());
    let rope = buffer.rope();
    // Emacs looks at the character after point, then the one before it.
    for at in [point, point.checked_sub(1)?] {
        if at >= buffer.len_chars() {
            continue;
        }
        if let Some(partner) = maxgus_text::Motion::matching_delimiter(rope, at) {
            return Some((at, partner));
        }
    }
    None
}

/// The run of blanks at the end of `line`, if there is one.
fn trailing_whitespace(buffer: &Buffer, line: Range) -> Option<Range> {
    let mut start = line.end;
    while start > line.start {
        let c = buffer.rope().char(start - 1);
        if c != ' ' && c != '\t' {
            break;
        }
        start -= 1;
    }
    (start < line.end).then(|| Range::new(start, line.end))
}

/// Paints a window's mode line.
fn draw_mode_line(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    area: Rect,
    selected: bool,
) {
    if area.height == 0 {
        return;
    }
    let face = if selected {
        editor.theme.resolve("mode-line")
    } else {
        editor.theme.resolve("mode-line-inactive")
    };
    // The whole row is painted, so the mode line reads as a bar.
    surface.clear_rect(area, face);

    // Each segment keeps the bar's background and takes its own foreground,
    // so the row still reads as one bar. An unselected window gives all of
    // them the inactive face: colour is how the selected window is told
    // apart, and colouring both would take that away.
    let paint = |name: &str| match selected {
        true => {
            let mut own = editor.theme.resolve_overlay(name);
            own.background = face.background;
            let mut merged = face;
            merged.overlay(&own);
            merged
        }
        false => face,
    };

    let segments = editor.mode_line_segments(window.id);
    let (right, left): (Vec<_>, Vec<_>) = segments.into_iter().partition(|s| s.right);

    let mut x = area.x;
    for segment in &left {
        if x >= area.right() {
            break;
        }
        x = surface.set_string(
            x,
            area.y,
            &segment.text,
            paint(segment.face),
            area.right() - x,
        );
    }

    // The right-hand group is placed from the edge inwards, and dropped
    // rather than overlapped when the bar is too narrow for both: the file
    // being edited is what must survive a narrow window.
    let width: u16 = right.iter().map(|s| s.text.chars().count() as u16).sum();
    let mut at = area.right().saturating_sub(width);
    if at > x {
        for segment in &right {
            at = surface.set_string(
                at,
                area.y,
                &segment.text,
                paint(segment.face),
                area.right() - at,
            );
        }
    }
}

#[cfg(feature = "git")]
/// Paints the list of completions above the echo area.
///
/// Emacs opens a `*Completions*` window; on a terminal a few rows over the
/// bottom of the frame say the same thing without disturbing the layout. Until
/// this existed, `TAB` on an ambiguous prefix appeared to do nothing at all.
/// Paints the menu that is up, across the bottom of the frame.
///
/// Below rather than above, unlike the completion popup: a menu is read while
/// looking at what it will act on, and covering that is the one thing it must
/// not do.
fn draw_transient(
    editor: &Editor,
    surface: &mut Surface,
    frame: Rect,
    active: &crate::transient::Active,
) {
    let Some(transient) = active.current() else {
        return;
    };
    let theme = &editor.theme;

    let mut lines: Vec<Vec<(String, &'static str)>> = Vec::new();
    for group in transient.groups {
        lines.push(vec![(group.title.to_string(), "transient-heading")]);
        for item in group.items {
            let label_face = match item.action {
                crate::transient::Action::Switch(flag) if active.is_on(flag) => {
                    "transient-switch-on"
                }
                crate::transient::Action::Switch(_) => "transient-switch-off",
                _ => "default",
            };
            // A switch says whether it is on; a prefix says it opens another
            // menu. Both are read at a glance rather than from the label.
            let mark = match item.action {
                crate::transient::Action::Switch(flag) if active.is_on(flag) => " \u{2713}",
                crate::transient::Action::Switch(_) => "",
                crate::transient::Action::Prefix(_) => " \u{25b8}",
                crate::transient::Action::Command(_) => "",
            };
            lines.push(vec![
                (format!(" {:<5}", item.key), "transient-key"),
                (item.label.to_string(), label_face),
                (mark.to_string(), "transient-switch-on"),
            ]);
        }
    }

    // Two columns when there is room, so a long menu does not run off.
    let column = (frame.width / 2).max(24);
    let columns = ((frame.width / column).max(1) as usize).min(2);
    let rows = lines.len().div_ceil(columns);
    let height = (rows as u16 + 2).min(frame.height.saturating_sub(2));
    if height < 3 {
        return;
    }
    let area = Rect::new(
        frame.x,
        frame.bottom().saturating_sub(1 + height),
        frame.width,
        height,
    );
    surface.clear_rect(area, theme.resolve("default"));
    draw_border(surface, area, theme.resolve("completion-border"));

    let inner = area.inset(1);
    let title = format!(" {} ", transient.title);
    surface.set_string(
        area.x + 2,
        area.y,
        &title,
        theme.resolve("transient-heading"),
        area.width,
    );

    let rows = inner.height as usize;
    for (index, line) in lines.iter().enumerate() {
        let (row, offset) = (index % rows, (index / rows) as u16 * column);
        let y = inner.y + row as u16;
        if y >= inner.bottom() || inner.x + offset >= inner.right() {
            continue;
        }
        let mut x = inner.x + offset;
        for (text, name) in line {
            x = surface.set_string(
                x,
                y,
                text,
                theme.resolve(name),
                inner.right().saturating_sub(x),
            );
        }
    }
}

/// Where the completion popup goes, when a completing prompt is open.
///
/// The list is a box at the *top* of the frame rather than a few rows above
/// the echo area, in the manner of vertico: the prompt is the box's first
/// line and the candidates sit directly under it, so what was typed and what
/// it matched are read in one place instead of at opposite ends of the
/// screen. The cursor follows the prompt up here, which is why this is a
/// function of the editor alone — `Editor::cursor_position` asks it too, and
/// the two must agree to the column.
pub(crate) fn completion_popup(editor: &Editor, frame: Rect) -> Option<Rect> {
    if !editor
        .minibuffer
        .kind()
        .is_some_and(crate::MinibufferKind::completes)
    {
        return None;
    }
    // A prompt with nothing to complete over has nothing to put in a box, and
    // keeps to the echo area. A query matching none of a set still gets the
    // popup, so typing past the last match does not throw the prompt to the
    // other end of the screen.
    if editor.minibuffer.completion().total == 0 {
        return None;
    }
    // Two borders and the prompt line, then as many candidates as fit.
    let height = editor.completion_rows() as u16 + 3;
    // Three fifths of the frame rather than all of it: the buffer stays
    // readable beside the list, and a command name with its key and its one
    // line of documentation still fits — half a wide frame does not leave the
    // documentation column room to say anything. Narrow frames give what they
    // have; there is nothing to keep readable there anyway.
    let width = (frame.width * 3 / 5).max(48).min(frame.width);
    // Centred across the frame: the list is what the eye is on while a prompt
    // is open, and a box against the left edge reads as part of the window
    // under it rather than as something over the top of everything.
    let x = frame.x + (frame.width - width) / 2;
    // A frame too short to leave any of the buffer visible keeps the prompt
    // in the echo area, which needs no room at all.
    (frame.height > height && frame.width > 8).then(|| Rect::new(x, frame.y, width, height))
}

/// `3/812 ` — which candidate is highlighted, out of how many match.
///
/// The trailing space is part of it so that the drawing and the cursor agree
/// on where the prompt starts without either having to know about the other.
pub(crate) fn completion_count(editor: &Editor) -> String {
    let completion = editor.minibuffer.completion();
    let at = completion.selected.map_or(0, |index| index + 1);
    format!("{at}/{} ", completion.len())
}

/// Paints the completion popup: its frame, the prompt, and the candidates.
fn draw_completion_popup(editor: &Editor, surface: &mut Surface, area: Rect) {
    let theme = &editor.theme;
    let default = theme.resolve("default");
    surface.clear_rect(area, default);
    draw_border(surface, area, theme.resolve("completion-border"));

    let inner = area.inset(1);
    if inner.is_empty() {
        return;
    }
    let completion = editor.minibuffer.completion();

    // The prompt line: `3/812 M-x buf`.
    let mut x = surface.set_string(
        inner.x,
        inner.y,
        &completion_count(editor),
        theme.resolve("completion-count"),
        inner.width,
    );
    x = surface.set_string(
        x,
        inner.y,
        editor.minibuffer.prompt(),
        theme.resolve("minibuffer-prompt"),
        inner.right().saturating_sub(x),
    );
    surface.set_string(
        x,
        inner.y,
        editor.minibuffer.input(),
        default,
        inner.right().saturating_sub(x),
    );

    let rows = inner.height.saturating_sub(1) as usize;
    // The window into the list, which scrolls under the box.
    let top = completion.top.min(completion.len().saturating_sub(1));
    let shown = &completion.candidates[top..completion.len().min(top + rows)];
    let annotations: Vec<(String, String)> = shown.iter().map(|c| annotate(editor, c)).collect();
    // The columns are as wide as their widest entry, so a list of short names
    // does not push its documentation across the screen away from it.
    let names = column_width(shown.iter().map(String::as_str), inner.width / 2);
    let keys = column_width(annotations.iter().map(|(k, _)| k.as_str()), inner.width / 4);

    for (row, candidate) in shown.iter().enumerate() {
        let y = inner.y + 1 + row as u16;
        let chosen = completion.selected == Some(top + row);
        let face = if chosen {
            theme.resolve("completion-selected")
        } else {
            default
        };
        // The highlight runs the full width of the box, as in the mock-up:
        // a row's worth of colour is far easier to track with the arrow keys
        // than a word's worth.
        surface.clear_rect(Rect::new(inner.x, y, inner.width, 1), face);
        surface.set_string(inner.x, y, candidate, face, names);

        let (key, doc) = &annotations[row];
        let mut x = inner.x + names + 1;
        if keys > 0 {
            let key_face = if chosen {
                face
            } else {
                theme.resolve("completion-key")
            };
            surface.set_string(x, y, key, key_face, keys);
            x += keys + 1;
        }
        if x < inner.right() {
            let doc_face = if chosen {
                face
            } else {
                theme.resolve("completion-annotation")
            };
            surface.set_string(x, y, doc, doc_face, inner.right() - x);
        }
    }
}

/// The width of a column: its widest entry, capped, and zero when empty.
fn column_width<'a>(entries: impl Iterator<Item = &'a str>, most: u16) -> u16 {
    let widest = entries.map(|e| e.chars().count()).max().unwrap_or(0) as u16;
    widest.min(most)
}

/// The two annotation columns for one candidate.
///
/// `M-x` is the reason they exist: a command list is worth much more with the
/// key that runs each command and a line saying what it does. A buffer list
/// gets the file each name stands for, which is the same question asked of a
/// different set of names.
fn annotate(editor: &Editor, candidate: &str) -> (String, String) {
    match editor.minibuffer.kind() {
        Some(crate::MinibufferKind::Command) => {
            let key = editor
                .keymaps
                .where_is(candidate)
                .first()
                .map(|sequence| sequence.notation())
                .unwrap_or_default();
            let doc = editor
                .command_docs
                .iter()
                .find(|(name, _)| name == candidate)
                .and_then(|(_, doc)| doc.lines().next())
                .unwrap_or_default()
                .to_string();
            (key, doc)
        }
        Some(crate::MinibufferKind::Buffer) => {
            let path = editor
                .buffers
                .iter()
                .find(|buffer| buffer.name() == candidate)
                .and_then(Buffer::path)
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            (String::new(), path)
        }
        _ => (String::new(), String::new()),
    }
}

/// Draws a rounded box around `area`.
fn draw_border(surface: &mut Surface, area: Rect, face: Face) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let (left, right) = (area.x, area.right() - 1);
    let (top, bottom) = (area.y, area.bottom() - 1);
    for x in left..=right {
        surface.set_char(x, top, '─', face);
        surface.set_char(x, bottom, '─', face);
    }
    for y in top..=bottom {
        surface.set_char(left, y, '│', face);
        surface.set_char(right, y, '│', face);
    }
    surface.set_char(left, top, '╭', face);
    surface.set_char(right, top, '╮', face);
    surface.set_char(left, bottom, '╰', face);
    surface.set_char(right, bottom, '╯', face);
}

fn draw_echo_area(editor: &Editor, surface: &mut Surface, area: Rect) {
    if area.height == 0 {
        return;
    }
    let theme = &editor.theme;
    let face = match () {
        // A search that is finding nothing says so in its own face, which is
        // the difference between noticing and typing on obliviously.
        _ if editor.isearch.as_ref().is_some_and(|s| s.failing) => theme.resolve("isearch-fail"),
        _ if editor.minibuffer.is_active() => theme.resolve("minibuffer-prompt"),
        _ if editor.minibuffer.message_is_error() => theme.resolve("error"),
        _ => match message_tone(&editor.minibuffer.display()) {
            Some(name) => theme.resolve(name),
            None => theme.resolve("echo-area"),
        },
    };
    surface.clear_rect(area, theme.resolve("default"));

    let text = echo_text(editor);
    surface.set_string(area.x, area.y, &text, face, area.width);
}

/// The face a message deserves, judged by what it says.
///
/// Emacs has no separate channel for these; a command that saved something and
/// one that could not both call `message`. Recognising the few words the
/// editor itself uses is enough to colour them apart.
fn message_tone(message: &str) -> Option<&'static str> {
    const WARNINGS: [&str; 4] = ["unsaved", "read-only", "already", "cannot"];
    const SUCCESSES: [&str; 3] = ["Wrote ", "Saved", "Applied "];
    let lowered = message.to_lowercase();
    if SUCCESSES.iter().any(|word| message.starts_with(word)) {
        return Some("success");
    }
    if WARNINGS.iter().any(|word| lowered.contains(word)) {
        return Some("warning");
    }
    None
}

/// What the echo area should show, in priority order.
pub fn echo_text(editor: &Editor) -> String {
    // A search takes over the echo area entirely.
    if let Some(search) = editor.isearch.as_ref() {
        return search.prompt();
    }
    if editor.minibuffer.is_active() {
        return editor.minibuffer.display();
    }
    // A half-typed key sequence is echoed so the user can see where they are.
    if let Some(pending) = editor.pending_keys.as_ref() {
        return pending.clone();
    }
    editor.minibuffer.display()
}

#[cfg(test)]
mod tests {
    fn node(
        path: &str,
        name: &str,
        kind: maxgus_tree::NodeKind,
        depth: usize,
        expanded: bool,
    ) -> maxgus_tree::VisibleNode {
        maxgus_tree::VisibleNode {
            path: path.into(),
            name: name.into(),
            kind,
            depth,
            expanded,
            expandable: matches!(kind, maxgus_tree::NodeKind::Directory),
            git: None,
            is_root: depth == 0,
        }
    }

    use super::*;
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    #[cfg(feature = "syntax")]
    use maxgus_syntax::Highlight;
    use maxgus_tui::Size;

    /// An editor with a buffer of `text`, and a surface to draw it into.
    fn setup(text: &str, width: u16, height: u16) -> (Editor, Surface) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, width, height),
        );
        let id = editor.buffers.create_with_text("test", text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));
        (editor, Surface::new(Size::new(width, height)))
    }

    fn rendered(editor: &Editor, surface: &mut Surface) -> Vec<String> {
        draw(editor, surface);
        surface.to_lines()
    }

    /// The face of the cell at (x, y).
    fn face_at(surface: &Surface, x: u16, y: u16) -> Face {
        surface.get(x, y).expect("inside the surface").face
    }

    #[test]
    fn buffer_text_is_drawn_from_the_top() {
        let (e, mut s) = setup("one\ntwo\nthree\n", 20, 6);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "one                 ");
        assert_eq!(lines[1], "two                 ");
        assert_eq!(lines[2], "three               ");
    }

    #[test]
    fn the_last_row_is_the_echo_area_and_the_one_above_is_the_mode_line() {
        // Wide enough that the mode line has room to be padded.
        let (mut e, mut s) = setup("text", 60, 5);
        e.message("a message");
        let lines = rendered(&e, &mut s);
        assert!(lines[4].starts_with("a message"), "got `{}`", lines[4]);
        assert!(
            lines[3].contains("test"),
            "the mode line, got `{}`",
            lines[3]
        );
        // The bar is a painted row rather than one padded with dashes, so it
        // is the background that has to reach the edge.
        let bar = e.theme.resolve("mode-line").background;
        for x in 0..60u16 {
            assert_eq!(
                face_at(&s, x, 3).background,
                bar,
                "the bar stops at column {x}"
            );
        }
    }

    #[test]
    fn nothing_is_drawn_past_the_end_of_the_buffer() {
        let (e, mut s) = setup("one\n", 10, 6);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[2], "          ", "blank, not a tilde");
        assert_eq!(lines[3], "          ");
    }

    #[test]
    fn a_scrolled_window_draws_from_its_top_line() {
        let text: String = (0..50).map(|n| format!("line {n}\n")).collect();
        let (mut e, mut s) = setup(&text, 20, 6);
        e.windows.current_mut().top_line = 10;
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("line 10"), "got `{}`", lines[0]);
        assert!(lines[3].starts_with("line 13"));
    }

    #[test]
    fn tabs_are_painted_out_to_the_next_tab_stop() {
        let (mut e, mut s) = setup("\tx\n", 20, 4);
        e.with_current_buffer(|b| b.set_tab_width(4));
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "    x               ");
    }

    #[test]
    fn control_characters_show_as_a_caret_pair() {
        let (e, mut s) = setup("a\u{1}b\n", 20, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "a^Ab                ");
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let (e, mut s) = setup("漢字x\n", 20, 4);
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("漢字x"), "got `{}`", lines[0]);
        assert!(
            s.get(1, 0).unwrap().continuation,
            "the second half of the first char"
        );
    }

    #[test]
    fn horizontal_scrolling_clips_from_the_left() {
        let (mut e, mut s) = setup("abcdefghijklmnop\n", 10, 4);
        e.windows.current_mut().left_column = 4;
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "efghijklmn");
    }

    #[test]
    fn a_long_line_is_clipped_at_the_right_edge() {
        let (e, mut s) = setup(&"x".repeat(100), 10, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "xxxxxxxxxx");
    }

    #[test]
    fn line_numbers_are_drawn_when_the_setting_asks_for_them() {
        let (mut e, mut s) = setup("one\ntwo\n", 20, 5);
        e.settings.line_numbers = true;
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("1 one"), "got `{}`", lines[0]);
        assert!(lines[1].starts_with("2 two"), "got `{}`", lines[1]);
    }

    #[test]
    fn line_numbers_are_right_aligned_to_the_widest() {
        let text: String = (0..120).map(|n| format!("line {n}\n")).collect();
        let (mut e, mut s) = setup(&text, 20, 5);
        e.settings.line_numbers = true;
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("  1 line 0"), "got `{}`", lines[0]);
    }

    #[test]
    fn the_current_line_number_is_drawn_in_its_own_face() {
        let (mut e, mut s) = setup("one\ntwo\n", 20, 5);
        e.settings.line_numbers = true;
        e.with_current_buffer(|b| b.set_point(b.line_start(1)));
        draw(&e, &mut s);
        assert_eq!(
            face_at(&s, 0, 1),
            e.theme.resolve("line-number-current-line")
        );
        assert_eq!(face_at(&s, 0, 0), e.theme.resolve("line-number"));
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn syntax_highlighting_colours_the_spans_it_covers() {
        let (mut e, mut s) = setup("fn main() {}\n", 20, 4);
        let id = e.current_buffer_id();
        // `fn` as a keyword, `main` as a function name.
        e.highlights.insert(
            id,
            (
                e.current_buffer().revision(),
                0..usize::MAX,
                vec![
                    Highlight::new(0, 2, "font-lock-keyword"),
                    Highlight::new(3, 7, "font-lock-function-name"),
                ],
            ),
        );
        draw(&e, &mut s);
        assert_eq!(face_at(&s, 0, 0), e.theme.resolve("font-lock-keyword"));
        assert_eq!(
            face_at(&s, 3, 0),
            e.theme.resolve("font-lock-function-name")
        );
        assert_eq!(
            face_at(&s, 8, 0),
            e.theme.resolve("default"),
            "outside every span"
        );
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn stale_highlighting_is_still_drawn() {
        let (mut e, mut s) = setup("fn main\n", 20, 4);
        let id = e.current_buffer_id();
        e.highlights.insert(
            id,
            (
                0,
                0..usize::MAX,
                vec![Highlight::new(0, 2, "font-lock-keyword")],
            ),
        );
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        assert!(e.highlights_are_stale(id));
        draw(&e, &mut s);
        // Colours one keystroke behind beat no colours at all.
        assert_ne!(face_at(&s, 1, 0), e.theme.resolve("default"));
    }

    #[test]
    fn the_region_is_drawn_in_the_region_face() {
        let (mut e, mut s) = setup("hello world\n", 20, 4);
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            b.set_point(5);
        });
        draw(&e, &mut s);
        let region = e.theme.resolve("region").background;
        assert_eq!(face_at(&s, 0, 0).background, region);
        assert_eq!(face_at(&s, 4, 0).background, region);
        assert_ne!(face_at(&s, 5, 0).background, region, "the end is exclusive");
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn a_region_keeps_the_syntax_colour_underneath_it() {
        let (mut e, mut s) = setup("fn main\n", 20, 4);
        let id = e.current_buffer_id();
        e.highlights.insert(
            id,
            (
                e.current_buffer().revision(),
                0..usize::MAX,
                vec![Highlight::new(0, 2, "font-lock-keyword")],
            ),
        );
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            b.set_point(7);
        });
        draw(&e, &mut s);
        let face = face_at(&s, 0, 0);
        assert_eq!(
            face.foreground,
            e.theme.resolve("font-lock-keyword").foreground
        );
        assert_eq!(face.background, e.theme.resolve("region").background);
    }

    #[test]
    fn a_region_spanning_a_newline_is_drawn_to_the_edge() {
        let (mut e, mut s) = setup("one\ntwo\n", 12, 5);
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            b.set_point(6);
        });
        draw(&e, &mut s);
        let region = e.theme.resolve("region").background;
        assert_eq!(
            face_at(&s, 8, 0).background,
            region,
            "past the end of `one`"
        );
    }

    #[test]
    fn a_search_match_is_highlighted() {
        let (mut e, mut s) = setup("alpha beta\n", 20, 4);
        e.isearch = Some(crate::commands::search::Isearch::at(
            "beta",
            maxgus_text::SearchKind::Literal,
            maxgus_text::SearchDirection::Forward,
            0,
            Some(Range::new(6, 10)),
        ));
        draw(&e, &mut s);
        assert_eq!(
            face_at(&s, 6, 0).background,
            e.theme.resolve("isearch").background
        );
        assert_ne!(
            face_at(&s, 0, 0).background,
            e.theme.resolve("isearch").background
        );
    }

    #[test]
    fn trailing_whitespace_is_marked() {
        let (e, mut s) = setup("text   \nnext\n", 20, 5);
        draw(&e, &mut s);
        let marked = e.theme.resolve("trailing-whitespace").background;
        assert_eq!(face_at(&s, 5, 0).background, marked);
        assert_ne!(face_at(&s, 0, 0).background, marked);
    }

    #[cfg(feature = "lsp")]
    #[test]
    fn diagnostics_are_underlined_where_they_sit() {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 30, 5),
        );
        let id = editor
            .buffers
            .visit_file("/project/main.rs", "let x = 1;\n");
        editor.switch_to_buffer(id).unwrap();
        editor.diagnostics.replace(
            maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/main.rs")),
            vec![maxgus_lsp::Diagnostic::new(
                maxgus_lsp::LspRange::new(
                    maxgus_lsp::LspPosition::new(0, 4),
                    maxgus_lsp::LspPosition::new(0, 5),
                ),
                maxgus_lsp::Severity::Error,
                "unused",
            )],
        );
        let mut s = Surface::new(Size::new(30, 5));
        draw(&editor, &mut s);
        assert_eq!(face_at(&s, 4, 0).attributes.underline, Some(true));
        assert_ne!(face_at(&s, 0, 0).attributes.underline, Some(true));
    }

    #[cfg(feature = "lsp")]
    #[test]
    fn the_mode_line_shows_diagnostic_counts() {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 60, 5),
        );
        let id = editor
            .buffers
            .visit_file("/project/main.rs", "let x = 1;\n");
        editor.switch_to_buffer(id).unwrap();
        let range = maxgus_lsp::LspRange::empty(maxgus_lsp::LspPosition::ZERO);
        editor.diagnostics.replace(
            maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/main.rs")),
            vec![
                maxgus_lsp::Diagnostic::new(range, maxgus_lsp::Severity::Error, "a"),
                maxgus_lsp::Diagnostic::new(range, maxgus_lsp::Severity::Error, "b"),
                maxgus_lsp::Diagnostic::new(range, maxgus_lsp::Severity::Warning, "c"),
            ],
        );
        let mut s = Surface::new(Size::new(60, 5));
        let lines = rendered(&editor, &mut s);
        assert!(
            lines[3].contains(&format!("{} 2", crate::icons::ERROR)),
            "two errors, got `{}`",
            lines[3]
        );
        assert!(
            lines[3].contains(&format!("{} 1", crate::icons::WARNING)),
            "one warning, got `{}`",
            lines[3]
        );
    }

    #[test]
    fn the_selected_window_has_the_brighter_mode_line() {
        let (mut e, mut s) = setup("text", 40, 12);
        e.split_window(crate::window::Direction::Vertical).unwrap();
        draw(&e, &mut s);
        let selected_row = e.windows.current().rect.bottom() - 1;
        let other = e
            .windows
            .ids()
            .into_iter()
            .find(|w| *w != e.windows.current_id())
            .unwrap();
        let other_row = e.windows.get(other).unwrap().rect.bottom() - 1;
        // Read at the right-hand edge, past every segment: the segments take
        // their own foreground, and it is the bar behind them that says which
        // window has the keyboard.
        assert_eq!(
            face_at(&s, 39, selected_row),
            e.theme.resolve("mode-line"),
            "the selected window's bar"
        );
        assert_eq!(
            face_at(&s, 39, other_row),
            e.theme.resolve("mode-line-inactive"),
            "the other window's bar"
        );
    }

    #[test]
    fn two_windows_are_both_drawn() {
        let (mut e, mut s) = setup("alpha\nbeta\ngamma\n", 40, 12);
        e.split_window(crate::window::Direction::Vertical).unwrap();
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("alpha"));
        // The second window starts halfway down and shows the same buffer.
        assert!(lines[6].starts_with("alpha"), "got `{}`", lines[6]);
    }

    #[test]
    fn a_side_window_is_drawn_beside_the_others() {
        // An ordinary buffer, not the tree's: this is about the layout, and
        // a panel buffer is drawn from its own state rather than its text.
        let (mut e, mut s) = setup("body text\n", 40, 8);
        let side = e.buffers.create_with_text("*notes*", "v project\n  src\n");
        e.windows.add_side_window(side, 12);
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("v project"), "got `{}`", lines[0]);
        assert!(
            lines[0][12..].starts_with("body text"),
            "got `{}`",
            lines[0]
        );
    }

    #[test]
    fn the_tree_draws_a_glyph_for_what_each_row_is() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 24);
        e.tree_window = Some(window);
        e.tree = vec![
            node("/p", "p", maxgus_tree::NodeKind::Directory, 0, true),
            node(
                "/p/main.rs",
                "main.rs",
                maxgus_tree::NodeKind::File,
                1,
                false,
            ),
            node(
                "/p/notes.md",
                "notes.md",
                maxgus_tree::NodeKind::File,
                1,
                false,
            ),
        ];
        let lines = tree_rows(&mut e, &mut s);

        assert!(
            lines[0].contains(crate::icons::DIRECTORY_OPEN),
            "an open directory has its own glyph, got `{}`",
            lines[0]
        );
        assert!(
            lines[1].contains(crate::icons::for_language("rust")),
            "a Rust file has Rust's glyph, got `{}`",
            lines[1]
        );
        assert!(
            lines[2].contains(crate::icons::for_language("markdown")),
            "and markdown has its own, got `{}`",
            lines[2]
        );
    }

    #[test]
    fn turning_the_glyphs_off_leaves_the_tree_plain() {
        // A terminal without a Nerd Font would draw boxes, so this has to be
        // a setting and the setting has to work.
        let (mut e, mut s) = setup("body\n", 40, 8);
        e.settings.nerd_font_icons = false;
        let tree = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 24);
        e.tree_window = Some(window);
        e.tree = vec![node(
            "/p/main.rs",
            "main.rs",
            maxgus_tree::NodeKind::File,
            0,
            false,
        )];
        let row = tree_rows(&mut e, &mut s)[0].clone();
        assert!(row.contains("main.rs"), "got `{row}`");
        assert!(
            !row.contains(crate::icons::for_language("rust")),
            "the glyph is still there, got `{row}`"
        );
    }

    /// Lays the panel out, draws it, and returns the tree section's rows on
    /// their own. The tree is no longer the whole of that window — it sits
    /// under a heading, beside the outline and the buffer list — so a test
    /// about tree rows asks for tree rows rather than for screen lines.
    fn tree_rows(e: &mut Editor, s: &mut Surface) -> Vec<String> {
        e.render_tree_buffer();
        draw(e, s);
        s.to_lines()
            .into_iter()
            .skip(first_tree_row(e) as usize)
            .collect()
    }

    /// The screen row the first tree node is drawn on. Its own window now,
    /// so it is the first row of that window.
    fn first_tree_row(_e: &Editor) -> u16 {
        0
    }

    #[test]
    fn the_tree_is_drawn_in_its_own_faces() {
        let (mut e, mut s) = setup("body text\n", 40, 10);
        let tree = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 20);
        e.tree_window = Some(window);
        e.tree = vec![
            maxgus_tree::VisibleNode {
                path: "/project".into(),
                name: "project".into(),
                kind: maxgus_tree::NodeKind::Directory,
                depth: 0,
                expanded: true,
                expandable: true,
                git: None,
                is_root: true,
            },
            maxgus_tree::VisibleNode {
                path: "/project/src".into(),
                name: "src".into(),
                kind: maxgus_tree::NodeKind::Directory,
                depth: 1,
                expanded: false,
                expandable: true,
                git: None,
                is_root: false,
            },
            maxgus_tree::VisibleNode {
                path: "/project/main.rs".into(),
                name: "main.rs".into(),
                kind: maxgus_tree::NodeKind::File,
                depth: 1,
                expanded: false,
                expandable: false,
                git: Some(maxgus_tree::GitStatus::Modified),
                is_root: false,
            },
        ];
        let lines = tree_rows(&mut e, &mut s);
        assert!(lines[0].starts_with('v'), "the arrow, got `{}`", lines[0]);
        assert!(lines[0].contains("project"), "got `{}`", lines[0]);
        assert!(lines[1].contains("> "), "got `{}`", lines[1]);
        assert!(lines[1].contains("src"), "got `{}`", lines[1]);
        assert!(lines[2].contains("main.rs"), "got `{}`", lines[2]);

        // The root, a directory and a file are told apart by their faces,
        // read at the first column of each name — found rather than counted,
        // since the arrow and the glyph before it are not a fixed width.
        let column_of = |row: usize, name: &str| -> u16 {
            let line = &lines[row];
            let byte = line
                .find(name)
                .unwrap_or_else(|| panic!("`{name}` in `{line}`"));
            line[..byte].chars().count() as u16
        };
        let top = first_tree_row(&e);
        let root = face_at(&s, column_of(0, "project"), top);
        let directory = face_at(&s, column_of(1, "src"), top + 1);
        let file = face_at(&s, column_of(2, "main.rs"), top + 2);
        assert_eq!(root.foreground, e.theme.resolve("tree-root").foreground);
        assert_eq!(
            directory.foreground,
            e.theme.resolve("tree-directory").foreground
        );
        assert_eq!(file.foreground, e.theme.resolve("tree-file").foreground);
        assert_ne!(
            root.foreground, file.foreground,
            "they must be distinguishable"
        );
    }

    #[test]
    fn a_modified_file_shows_its_git_status_in_colour() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 20);
        e.tree_window = Some(window);
        e.tree = vec![maxgus_tree::VisibleNode {
            path: "/project/main.rs".into(),
            name: "main.rs".into(),
            kind: maxgus_tree::NodeKind::File,
            depth: 0,
            expanded: false,
            expandable: false,
            git: Some(maxgus_tree::GitStatus::Modified),
            is_root: false,
        }];
        let row: String = tree_rows(&mut e, &mut s)[0].clone();
        assert!(row.contains('M'), "no git indicator: `{row}`");
        // By character, not by byte: the row carries a file glyph now, and a
        // byte offset stopped being a column the moment it did.
        let at = row.chars().count() as u16
            - 1
            - row
                .chars()
                .rev()
                .position(|c| c == 'M')
                .expect("the indicator") as u16;
        assert_eq!(
            face_at(&s, at, first_tree_row(&e)).foreground,
            e.theme.resolve("tree-git-modified").foreground
        );
    }

    #[test]
    fn the_selected_tree_row_is_marked() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "a\nb\n");
        let window = e.windows.add_side_window(tree, 20);
        e.tree_window = Some(window);
        e.tree = ["a", "b"]
            .iter()
            .enumerate()
            .map(|(i, name)| maxgus_tree::VisibleNode {
                path: format!("/{name}").into(),
                name: (*name).to_string(),
                kind: maxgus_tree::NodeKind::File,
                depth: 0,
                expanded: false,
                expandable: false,
                git: None,
                is_root: i == 0,
            })
            .collect();
        e.render_panel_buffer();
        e.move_tree_cursor_to_line(1);
        draw(&e, &mut s);

        let top = first_tree_row(&e);
        let marked = e.theme.resolve("tree-selected").background;
        assert_eq!(
            face_at(&s, 0, top + 1).background,
            marked,
            "the cursor row is not marked"
        );
        assert_ne!(
            face_at(&s, 0, top).background,
            marked,
            "and other rows are not"
        );
    }

    #[test]
    fn the_completion_list_is_a_popup_at_the_top_of_the_frame() {
        let (mut e, mut s) = setup("text", 40, 10);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let candidates: Vec<String> = ["save-buffer", "save-some-buffers"]
            .iter()
            .map(|c| c.to_string())
            .collect();
        // The first completion grows the input to the common prefix; only
        // when it cannot grow further is the list offered.
        e.minibuffer.complete(&candidates);
        e.minibuffer.complete(&candidates);
        assert!(
            e.minibuffer.completion().visible,
            "the list should be offered"
        );

        let lines: Vec<String> = rendered(&e, &mut s)
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert!(lines[0].starts_with('╭'), "no top border: `{}`", lines[0]);
        // The prompt is the first line inside the box, behind a count of
        // where the highlight is in the list. TAB completion highlights
        // nothing until it starts cycling, so this one reads zero.
        assert!(
            lines[1].contains("0/2 M-x save-"),
            "prompt line is `{}`",
            lines[1]
        );
        assert!(
            lines[2].contains("save-buffer"),
            "first candidate: `{}`",
            lines[2]
        );
        assert!(
            lines[3].contains("save-some-buffers"),
            "second candidate: `{}`",
            lines[3]
        );
        assert!(
            lines[4].starts_with('╰'),
            "no bottom border: `{}`",
            lines[4]
        );
        // Having moved into the popup, the prompt is not also at the bottom:
        // one prompt, in one place.
        assert!(
            lines[9].is_empty(),
            "the echo area still has `{}`",
            lines[9]
        );
    }

    /// A prompt over `count` candidates, with the list up.
    fn listing(count: usize, width: u16, height: u16) -> (Editor, Surface) {
        let (mut editor, surface) = setup("text", width, height);
        let candidates: Vec<String> = (0..count).map(|n| format!("candidate-{n:02}")).collect();
        editor.completion_candidates = candidates.clone();
        editor.prompt_for(
            "noop",
            crate::MinibufferKind::Command,
            "M-x ",
            "",
            candidates,
        );
        (editor, surface)
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_in_view() {
        // Twelve candidates in a frame with room for four rows: moving past
        // the fourth has to bring the fifth into view, one row at a time,
        // rather than leaving the highlight somewhere off the box.
        let (mut e, mut s) = listing(12, 40, 10);
        let rows = e.completion_rows();
        assert!(rows < 12, "the frame should not fit the whole list");

        // The list opens with the first row highlighted, so `rows` moves put
        // the highlight one row below the box.
        for _ in 0..rows {
            e.move_completion_selection(1);
        }
        assert_eq!(e.minibuffer.completion().selected, Some(rows));
        let lines = rendered(&e, &mut s);
        let selected = format!("candidate-{rows:02}");
        assert!(
            lines.iter().any(|line| line.contains(&selected)),
            "the selected row is not drawn:\n{lines:#?}"
        );
        // One row at a time: the row above it is still there, the first is not.
        let previous = format!("candidate-{:02}", rows - 1);
        assert!(
            lines.iter().any(|line| line.contains(&previous)),
            "it scrolled by more than a row:\n{lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("candidate-00")),
            "the list did not scroll at all:\n{lines:#?}"
        );
    }

    #[test]
    fn the_popup_is_centred_across_the_frame() {
        for width in [80, 100, 121, 200] {
            let (e, _) = listing(12, width, 20);
            let popup = completion_popup(&e, e.frame).expect("a popup");
            let left = popup.x;
            let right = width - popup.right();
            // Even, give or take the odd column that cannot be split.
            assert!(
                left.abs_diff(right) <= 1,
                "{width} columns: {left} to the left, {right} to the right"
            );
            assert!(popup.right() <= width, "it runs off the right edge");
        }
    }

    #[test]
    fn a_frame_too_narrow_to_centre_still_fits_the_popup() {
        // Below the floor the box is the whole frame, and centring it must
        // not push it off the edge by half of nothing.
        let (e, _) = listing(12, 30, 20);
        let popup = completion_popup(&e, e.frame).expect("a popup");
        assert_eq!(popup.x, 0, "there is nothing to centre it in");
        assert_eq!(popup.right(), 30);
    }

    #[test]
    fn the_highlight_is_on_the_selected_row_after_scrolling() {
        // Drawn from an offset, the row a candidate is on is no longer its
        // index. The highlight has to move with the list, or it marks
        // whatever happens to have scrolled into that row.
        let (mut e, mut s) = listing(12, 40, 10);
        let rows = e.completion_rows();
        for _ in 0..rows + 2 {
            e.move_completion_selection(1);
        }
        let selected = e
            .minibuffer
            .completion()
            .current()
            .expect("something is selected")
            .to_string();

        let lines = rendered(&e, &mut s);
        let face = e.theme.resolve("completion-selected");
        // The one row painted in the selected face, and what is written on it.
        let painted: Vec<usize> = (2..2 + rows)
            .filter(|y| face_at(&s, 2, *y as u16) == face)
            .collect();
        assert_eq!(
            painted.len(),
            1,
            "expected one highlighted row:\n{lines:#?}"
        );
        assert!(
            lines[painted[0]].contains(&selected),
            "the highlight is on `{}`, not on `{selected}`",
            lines[painted[0]].trim()
        );
    }

    #[test]
    fn the_list_wraps_round_at_both_ends() {
        let (mut e, mut s) = listing(12, 40, 10);
        // Backwards from the first: the end of the list, in view.
        e.move_completion_selection(-1);
        assert_eq!(e.minibuffer.completion().selected, Some(11));
        let lines = rendered(&e, &mut s);
        assert!(
            lines.iter().any(|line| line.contains("candidate-11")),
            "the last candidate is not in view:\n{lines:#?}"
        );

        // And forwards off the end: back to the top of the list.
        e.move_completion_selection(1);
        assert_eq!(e.minibuffer.completion().selected, Some(0));
        let lines = rendered(&e, &mut s);
        assert!(
            lines.iter().any(|line| line.contains("candidate-00")),
            "the list did not scroll back to the top:\n{lines:#?}"
        );
    }

    #[test]
    fn the_popup_leaves_the_frame_something() {
        // Full width, the box is the whole screen and nothing of the buffer
        // is left to read behind it.
        for width in [80, 100, 120, 200] {
            let (e, _) = listing(12, width, 20);
            let popup = completion_popup(&e, e.frame).expect("a popup");
            assert!(
                popup.width < width,
                "the popup is the whole frame at {width} columns"
            );
            assert!(
                popup.width * 2 <= width * 3 / 2,
                "the popup takes {} of {width} columns",
                popup.width
            );
            assert!(popup.width >= 40, "too narrow to read at {width} columns");
        }
    }

    #[test]
    fn a_narrow_frame_still_gets_a_popup() {
        let (e, _) = listing(12, 30, 20);
        let popup = completion_popup(&e, e.frame).expect("a popup");
        assert!(popup.width <= 30, "wider than the frame");
        assert!(popup.width >= 20, "unusably narrow: {}", popup.width);
    }

    #[test]
    fn the_candidate_being_cycled_is_marked() {
        let (mut e, mut s) = setup("text", 40, 10);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let candidates: Vec<String> = ["save-buffer", "save-some-buffers"]
            .iter()
            .map(|c| c.to_string())
            .collect();
        e.minibuffer.complete(&candidates);
        e.minibuffer.complete(&candidates);
        e.minibuffer.cycle_completion(true);

        draw(&e, &mut s);
        let chosen = e.theme.resolve("completion-selected").background;
        // Inside the border, the highlight runs the whole width of the row.
        assert_eq!(
            face_at(&s, 1, 2).background,
            chosen,
            "the first candidate is chosen"
        );
        assert_eq!(
            face_at(&s, 38, 2).background,
            chosen,
            "the highlight stops short"
        );
        assert_ne!(face_at(&s, 1, 3).background, chosen);
    }

    #[test]
    fn a_long_candidate_list_is_capped_and_says_how_many_are_left() {
        let (mut e, mut s) = setup("text", 40, 12);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        let candidates: Vec<String> = (0..30).map(|n| format!("command-{n:02}")).collect();
        e.minibuffer.complete(&candidates);
        e.minibuffer.complete(&candidates);

        let lines = rendered(&e, &mut s);
        // The count says how many match; the box shows as many as fit
        // without taking over the screen.
        assert!(
            lines[1].contains("0/30"),
            "no count of the matches: `{}`",
            lines[1]
        );
        // The prompt line holds `command-` too, so only rows whose first
        // column after the border is the name itself are counted.
        let rows = lines.iter().filter(|l| l.starts_with("│command-")).count();
        assert!(
            (1..=6).contains(&rows),
            "{rows} candidate rows:
{lines:#?}"
        );
    }

    #[test]
    fn nothing_is_drawn_when_there_are_no_completions_to_show() {
        let (mut e, mut s) = setup("text\nmore\n", 40, 8);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        let lines: Vec<String> = rendered(&e, &mut s)
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert_eq!(lines[0], "text", "the buffer is untouched");
        assert_eq!(lines[1], "more");
    }

    /// Puts an incremental search in progress, with `current` as its match.
    fn searching(e: &mut Editor, query: &str, current: Option<Range>) {
        e.isearch = Some(crate::commands::search::Isearch::at(
            query,
            maxgus_text::SearchKind::Literal,
            maxgus_text::SearchDirection::Forward,
            0,
            current,
        ));
    }

    #[test]
    fn every_match_is_marked_and_the_current_one_more_strongly() {
        let (mut e, mut s) = setup("beta alpha beta gamma beta\n", 40, 6);
        searching(&mut e, "beta", Some(Range::new(0, 4)));
        draw(&e, &mut s);

        let current = e.theme.resolve("isearch").background;
        let other = e.theme.resolve("lazy-highlight").background;
        assert_eq!(
            face_at(&s, 0, 0).background,
            current,
            "the match point is on"
        );
        assert_eq!(face_at(&s, 11, 0).background, other, "a later match");
        assert_eq!(face_at(&s, 22, 0).background, other, "and another");
        assert_ne!(current, other, "they must be distinguishable");
        assert_ne!(
            face_at(&s, 5, 0).background,
            other,
            "text between them is not marked"
        );
    }

    #[test]
    fn a_failing_search_marks_nothing_in_the_buffer() {
        let (mut e, mut s) = setup("alpha beta\n", 40, 6);
        let mut search = crate::commands::search::Isearch::at(
            "zzz",
            maxgus_text::SearchKind::Literal,
            maxgus_text::SearchDirection::Forward,
            0,
            None,
        );
        search.failing = true;
        e.isearch = Some(search);
        draw(&e, &mut s);
        let marked = e.theme.resolve("lazy-highlight").background;
        assert!(
            (0..10).all(|x| face_at(&s, x, 0).background != marked),
            "a search that matches nothing should mark nothing"
        );
    }

    #[test]
    fn a_failing_search_says_so_in_its_own_face() {
        let (mut e, mut s) = setup("alpha\n", 40, 6);
        let mut search = crate::commands::search::Isearch::at(
            "zzz",
            maxgus_text::SearchKind::Literal,
            maxgus_text::SearchDirection::Forward,
            0,
            None,
        );
        search.failing = true;
        e.isearch = Some(search);
        draw(&e, &mut s);
        assert_eq!(face_at(&s, 0, 5), e.theme.resolve("isearch-fail"));
    }

    #[test]
    fn the_delimiter_matching_the_one_at_point_is_marked() {
        let (mut e, mut s) = setup("fn f() { g(1) }\n", 40, 6);
        // Point on the opening brace.
        e.with_current_buffer(|b| b.set_point(7));
        draw(&e, &mut s);

        let marked = e.theme.resolve("match-paren").foreground;
        assert_eq!(
            face_at(&s, 7, 0).foreground,
            marked,
            "the brace under point"
        );
        assert_eq!(face_at(&s, 14, 0).foreground, marked, "and its partner");
        assert_ne!(face_at(&s, 9, 0).foreground, marked, "not the text between");
    }

    #[test]
    fn a_delimiter_just_before_point_is_marked_too() {
        let (mut e, mut s) = setup("(abc)\n", 40, 6);
        // Point after the closing bracket, where Emacs still marks the pair.
        e.with_current_buffer(|b| b.set_point(5));
        draw(&e, &mut s);
        let marked = e.theme.resolve("match-paren").foreground;
        assert_eq!(face_at(&s, 4, 0).foreground, marked);
        assert_eq!(face_at(&s, 0, 0).foreground, marked);
    }

    #[test]
    fn an_unmatched_delimiter_marks_nothing() {
        let (mut e, mut s) = setup("(abc\n", 40, 6);
        e.with_current_buffer(|b| b.set_point(0));
        draw(&e, &mut s);
        assert_ne!(
            face_at(&s, 0, 0).foreground,
            e.theme.resolve("match-paren").foreground,
            "an unbalanced bracket has no partner to point at"
        );
    }

    #[test]
    fn the_fill_column_is_marked_when_the_setting_asks_for_it() {
        let (mut e, mut s) = setup("short\n", 40, 6);
        e.settings.fill_column_indicator = true;
        e.settings.fill_column = 20;
        draw(&e, &mut s);

        let face = e.theme.resolve("fill-column-indicator");
        assert_eq!(face_at(&s, 20, 0), face);
        assert_eq!(s.get(20, 0).unwrap().ch, '│');
        assert_ne!(face_at(&s, 19, 0), face, "only the one column");
    }

    #[test]
    fn the_fill_column_is_not_marked_by_default() {
        let (mut e, mut s) = setup("short\n", 40, 6);
        e.settings.fill_column = 20;
        draw(&e, &mut s);
        assert_eq!(s.get(20, 0).unwrap().ch, ' ');
    }

    #[test]
    fn messages_are_coloured_by_what_they_say() {
        assert_eq!(message_tone("Wrote /tmp/a.rs (12 bytes)"), Some("success"));
        assert_eq!(message_tone("Applied 3 change(s)"), Some("success"));
        assert_eq!(message_tone("Buffer is read-only"), Some("warning"));
        assert_eq!(message_tone("Unsaved: main.rs"), Some("warning"));
        assert_eq!(
            message_tone("Mark set"),
            None,
            "an ordinary message is ordinary"
        );
    }

    #[test]
    fn a_prompt_is_shown_in_the_echo_area() {
        let (mut e, mut s) = setup("text", 30, 5);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[4], "M-x save                      ");
        assert_eq!(face_at(&s, 0, 4), e.theme.resolve("minibuffer-prompt"));
    }

    #[test]
    fn an_error_message_is_shown_in_the_error_face() {
        let (mut e, mut s) = setup("text", 30, 5);
        e.error("No such file");
        draw(&e, &mut s);
        assert_eq!(face_at(&s, 0, 4), e.theme.resolve("error"));
    }

    #[test]
    fn a_search_takes_over_the_echo_area() {
        let (mut e, mut s) = setup("alpha", 30, 5);
        e.isearch = Some(crate::commands::search::Isearch::at(
            "alp",
            maxgus_text::SearchKind::Literal,
            maxgus_text::SearchDirection::Forward,
            0,
            Some(Range::new(0, 3)),
        ));
        e.message("stale message");
        let lines = rendered(&e, &mut s);
        assert!(lines[4].starts_with("I-search: alp"), "got `{}`", lines[4]);
    }

    #[test]
    fn a_half_typed_key_sequence_is_echoed() {
        let (mut e, mut s) = setup("text", 30, 5);
        e.pending_keys = Some("C-x r".into());
        let lines = rendered(&e, &mut s);
        assert!(lines[4].starts_with("C-x r"), "got `{}`", lines[4]);
    }

    #[test]
    fn a_frame_with_no_room_for_windows_still_draws_the_echo_area() {
        let (mut e, mut s) = setup("text", 20, 1);
        e.windows.layout(Rect::new(0, 0, 20, 1));
        e.message("hello");
        let lines = rendered(&e, &mut s);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "hello               ");
    }

    #[test]
    fn a_zero_height_frame_draws_nothing_and_does_not_panic() {
        let (e, mut s) = setup("text", 20, 0);
        let lines = rendered(&e, &mut s);
        assert!(lines.is_empty());
    }

    #[test]
    fn an_empty_buffer_draws_a_blank_screen_with_a_mode_line() {
        let (e, mut s) = setup("", 20, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "                    ");
        assert!(lines[2].contains("test"), "the mode line is still there");
    }
}
