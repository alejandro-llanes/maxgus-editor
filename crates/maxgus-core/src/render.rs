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
    match completion_popup(editor, frame) {
        Some(area) => {
            draw_completion_popup(editor, surface, area);
            surface.clear_rect(echo, default);
        }
        None => draw_echo_area(editor, surface, echo),
    }
}

/// Paints one window: its contents and its mode line.
fn draw_window(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let Some(buffer) = editor.buffers.get(window.buffer) else { return };
    let selected = window.id == editor.windows.current_id();
    let (text_area, mode_line_area) = area.split_bottom(1);
    // The tree is drawn from its own snapshot rather than as buffer text, so
    // each node can carry the face its kind and git status call for.
    if Some(window.id) == editor.tree_window {
        draw_tree(editor, surface, window, text_area);
    } else {
        draw_text(editor, surface, window, buffer, text_area);
    }
    draw_mode_line(editor, surface, window, mode_line_area, selected);
}

/// Paints the file tree: one node per row, each in the face its kind asks for.
fn draw_tree(editor: &Editor, surface: &mut Surface, window: &Window, area: Rect) {
    let theme = &editor.theme;
    let cursor_line = editor.tree_cursor_line();

    for row in 0..area.height {
        let line = window.top_line + row as usize;
        let Some(node) = editor.tree.get(line) else { break };
        let y = area.y + row;

        // The selected row is marked across its whole width, so the cursor is
        // findable without hunting for the terminal's own.
        let selected = line == cursor_line;
        if selected {
            surface.clear_rect(
                Rect::new(area.x, y, area.width, 1),
                theme.resolve("tree-selected"),
            );
        }
        let face = |name: &str| {
            let mut face = theme.resolve(name);
            if selected {
                face.overlay(&theme.resolve_overlay("tree-selected"));
            }
            face
        };

        let mut x = area.x + (node.depth as u16 * 2).min(area.width);
        // The arrow marks what can be opened.
        x = surface.set_string(x, y, node.arrow(), face("tree-arrow"), area.right() - x);
        // The glyph says what kind of thing it is at a glance, in the face of
        // the node itself so a directory's icon reads as a directory.
        if editor.settings.nerd_font_icons {
            let glyph = tree_glyph(node);
            let icon = format!("{glyph} ");
            x = surface.set_string(x, y, &icon, face(node.face()), area.right().saturating_sub(x));
        }
        x = surface.set_string(x, y, &node.name, face(node.face()), area.right().saturating_sub(x));

        // The git indicator sits at the right edge, where a column of them
        // reads as a column.
        if let Some(status) = node.git {
            let at = area.right().saturating_sub(2).max(x + 1);
            if at < area.right() {
                surface.set_char(at, y, status.indicator(), face(status.face()));
            }
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
fn draw_text(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    buffer: &Buffer,
    area: Rect,
) {
    let theme = &editor.theme;
    let gutter = line_number_width(editor, buffer);
    let point_line = buffer.line_of(window.point.min(buffer.len_chars()));
    // Diagnostics are resolved once for the whole window. Doing it per line
    // would repeat the work for every row on screen.
    let diagnostics = resolve_diagnostics(editor, buffer);
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
            &Overlays { diagnostics: &diagnostics, matches: &matches, paren },
        );
    }
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
    highlights: Vec<&'a maxgus_syntax::Highlight>,
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

        let line_start_byte = rope.char_to_byte(start);
        let line_end_byte = rope.char_to_byte(end);
        let highlights = editor
            .highlights_for(window.buffer)
            .iter()
            .filter(|h| h.start < line_end_byte && line_start_byte < h.end)
            .collect();

        // The region is only shown in the window whose buffer owns it.
        let region = buffer.region().filter(|r| r.overlaps(&line_range) || r.is_empty());
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
        let diagnostics: Vec<(Range, &'static str)> =
            overlays.diagnostics.iter().filter(|(r, _)| r.overlaps(&line_range)).copied().collect();
        let trailing = trailing_whitespace(buffer, line_range);

        Layers {
            theme: &editor.theme,
            default: editor.theme.resolve("default"),
            highlights,
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
        if !self.highlights.is_empty() {
            let byte = self.rope.char_to_byte(offset);
            if let Some(span) = self.highlights.iter().find(|h| h.start <= byte && byte < h.end) {
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

/// Every diagnostic for `buffer`, as character ranges with the face to use.
///
/// This is computed once per window. It used to be done per line, which meant
/// rendering the whole buffer to a string for each row on screen — fine on a
/// small file, ruinous on a large one.
fn resolve_diagnostics(editor: &Editor, buffer: &Buffer) -> Vec<(Range, &'static str)> {
    let Some(path) = buffer.path() else { return Vec::new() };
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
    let Some(offset) = column.checked_sub(window.left_column) else { return };
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
    let Some(search) = editor.isearch.as_ref() else { return Vec::new() };
    if search.query.is_empty() || search.failing {
        return Vec::new();
    }
    let Ok(query) = maxgus_text::SearchQuery::new(
        &search.query,
        search.kind,
        editor.settings.case_fold_search,
    ) else {
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

    let mut x = area.x;
    for segment in editor.mode_line_segments(window.id) {
        if x >= area.right() {
            break;
        }
        // Each segment keeps the bar's background and takes its own
        // foreground, so the row still reads as one bar. An unselected window
        // gives all of them the inactive face: colour is how the selected
        // window is told apart, and colouring both would take that away.
        let painted = match selected {
            true => {
                let mut own = editor.theme.resolve_overlay(segment.face);
                own.background = face.background;
                let mut merged = face;
                merged.overlay(&own);
                merged
            }
            false => face,
        };
        x = surface.set_string(x, area.y, &segment.text, painted, area.right() - x);
    }
}

/// Paints the list of completions above the echo area.
///
/// Emacs opens a `*Completions*` window; on a terminal a few rows over the
/// bottom of the frame say the same thing without disturbing the layout. Until
/// this existed, `TAB` on an ambiguous prefix appeared to do nothing at all.
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
    if !editor.minibuffer.kind().is_some_and(crate::MinibufferKind::completes) {
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
    // A frame too short to leave any of the buffer visible keeps the prompt
    // in the echo area, which needs no room at all.
    (frame.height > height && frame.width > 8).then(|| Rect::new(frame.x, frame.y, frame.width, height))
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
    let shown = &completion.candidates[..completion.len().min(rows)];
    let annotations: Vec<(String, String)> = shown.iter().map(|c| annotate(editor, c)).collect();
    // The columns are as wide as their widest entry, so a list of short names
    // does not push its documentation across the screen away from it.
    let names = column_width(shown.iter().map(String::as_str), inner.width / 2);
    let keys = column_width(annotations.iter().map(|(k, _)| k.as_str()), inner.width / 4);

    for (row, candidate) in shown.iter().enumerate() {
        let y = inner.y + 1 + row as u16;
        let chosen = completion.selected == Some(row);
        let face = if chosen { theme.resolve("completion-selected") } else { default };
        // The highlight runs the full width of the box, as in the mock-up:
        // a row's worth of colour is far easier to track with the arrow keys
        // than a word's worth.
        surface.clear_rect(Rect::new(inner.x, y, inner.width, 1), face);
        surface.set_string(inner.x, y, candidate, face, names);

        let (key, doc) = &annotations[row];
        let mut x = inner.x + names + 1;
        if keys > 0 {
            let key_face = if chosen { face } else { theme.resolve("completion-key") };
            surface.set_string(x, y, key, key_face, keys);
            x += keys + 1;
        }
        if x < inner.right() {
            let doc_face = if chosen { face } else { theme.resolve("completion-annotation") };
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
        assert!(lines[3].contains("test"), "the mode line, got `{}`", lines[3]);
        // The bar is a painted row rather than one padded with dashes, so it
        // is the background that has to reach the edge.
        let bar = e.theme.resolve("mode-line").background;
        for x in 0..60u16 {
            assert_eq!(face_at(&s, x, 3).background, bar, "the bar stops at column {x}");
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
        assert!(s.get(1, 0).unwrap().continuation, "the second half of the first char");
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
        assert_eq!(face_at(&s, 0, 1), e.theme.resolve("line-number-current-line"));
        assert_eq!(face_at(&s, 0, 0), e.theme.resolve("line-number"));
    }

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
        assert_eq!(face_at(&s, 3, 0), e.theme.resolve("font-lock-function-name"));
        assert_eq!(face_at(&s, 8, 0), e.theme.resolve("default"), "outside every span");
    }

    #[test]
    fn stale_highlighting_is_still_drawn() {
        let (mut e, mut s) = setup("fn main\n", 20, 4);
        let id = e.current_buffer_id();
        e.highlights.insert(id, (0, 0..usize::MAX, vec![Highlight::new(0, 2, "font-lock-keyword")]));
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

    #[test]
    fn a_region_keeps_the_syntax_colour_underneath_it() {
        let (mut e, mut s) = setup("fn main\n", 20, 4);
        let id = e.current_buffer_id();
        e.highlights.insert(
            id,
            (e.current_buffer().revision(), 0..usize::MAX, vec![Highlight::new(0, 2, "font-lock-keyword")]),
        );
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(0);
            b.set_point(7);
        });
        draw(&e, &mut s);
        let face = face_at(&s, 0, 0);
        assert_eq!(face.foreground, e.theme.resolve("font-lock-keyword").foreground);
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
        assert_eq!(face_at(&s, 8, 0).background, region, "past the end of `one`");
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
        assert_eq!(face_at(&s, 6, 0).background, e.theme.resolve("isearch").background);
        assert_ne!(face_at(&s, 0, 0).background, e.theme.resolve("isearch").background);
    }

    #[test]
    fn trailing_whitespace_is_marked() {
        let (e, mut s) = setup("text   \nnext\n", 20, 5);
        draw(&e, &mut s);
        let marked = e.theme.resolve("trailing-whitespace").background;
        assert_eq!(face_at(&s, 5, 0).background, marked);
        assert_ne!(face_at(&s, 0, 0).background, marked);
    }

    #[test]
    fn diagnostics_are_underlined_where_they_sit() {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 30, 5),
        );
        let id = editor.buffers.visit_file("/project/main.rs", "let x = 1;\n");
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

    #[test]
    fn the_mode_line_shows_diagnostic_counts() {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 60, 5),
        );
        let id = editor.buffers.visit_file("/project/main.rs", "let x = 1;\n");
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
        let other = e.windows.ids().into_iter().find(|w| *w != e.windows.current_id()).unwrap();
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
        let (mut e, mut s) = setup("body text\n", 40, 8);
        let tree = e.buffers.create_with_text("*treefile*", "v project\n  src\n");
        e.windows.add_side_window(tree, 12);
        let lines = rendered(&e, &mut s);
        assert!(lines[0].starts_with("v project"), "got `{}`", lines[0]);
        assert!(lines[0][12..].starts_with("body text"), "got `{}`", lines[0]);
    }

    #[test]
    fn the_tree_draws_a_glyph_for_what_each_row_is() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e.buffers.create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 24);
        e.tree_window = Some(window);
        e.tree = vec![
            node("/p", "p", maxgus_tree::NodeKind::Directory, 0, true),
            node("/p/main.rs", "main.rs", maxgus_tree::NodeKind::File, 1, false),
            node("/p/notes.md", "notes.md", maxgus_tree::NodeKind::File, 1, false),
        ];
        draw(&e, &mut s);
        let lines = s.to_lines();

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
        let tree = e.buffers.create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
        let window = e.windows.add_side_window(tree, 24);
        e.tree_window = Some(window);
        e.tree = vec![node("/p/main.rs", "main.rs", maxgus_tree::NodeKind::File, 0, false)];
        draw(&e, &mut s);

        let row = s.to_lines()[0].clone();
        assert!(row.contains("main.rs"), "got `{row}`");
        assert!(
            !row.contains(crate::icons::for_language("rust")),
            "the glyph is still there, got `{row}`"
        );
    }

    #[test]
    fn the_tree_is_drawn_in_its_own_faces() {
        let (mut e, mut s) = setup("body text\n", 40, 10);
        let tree = e.buffers.create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
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
        draw(&e, &mut s);

        let lines = s.to_lines();
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
            let byte = line.find(name).unwrap_or_else(|| panic!("`{name}` in `{line}`"));
            line[..byte].chars().count() as u16
        };
        let root = face_at(&s, column_of(0, "project"), 0);
        let directory = face_at(&s, column_of(1, "src"), 1);
        let file = face_at(&s, column_of(2, "main.rs"), 2);
        assert_eq!(root.foreground, e.theme.resolve("tree-root").foreground);
        assert_eq!(directory.foreground, e.theme.resolve("tree-directory").foreground);
        assert_eq!(file.foreground, e.theme.resolve("tree-file").foreground);
        assert_ne!(root.foreground, file.foreground, "they must be distinguishable");
    }

    #[test]
    fn a_modified_file_shows_its_git_status_in_colour() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e.buffers.create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "");
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
        draw(&e, &mut s);

        let row: String = s.to_lines()[0].clone();
        assert!(row.contains('M'), "no git indicator: `{row}`");
        // By character, not by byte: the row carries a file glyph now, and a
        // byte offset stopped being a column the moment it did.
        let at = row.chars().count() as u16
            - 1
            - row.chars().rev().position(|c| c == 'M').expect("the indicator") as u16;
        assert_eq!(
            face_at(&s, at, 0).foreground,
            e.theme.resolve("tree-git-modified").foreground
        );
    }

    #[test]
    fn the_selected_tree_row_is_marked() {
        let (mut e, mut s) = setup("body\n", 40, 8);
        let tree = e.buffers.create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "a\nb\n");
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
        e.move_tree_cursor_to_line(1);
        draw(&e, &mut s);

        let marked = e.theme.resolve("tree-selected").background;
        assert_eq!(face_at(&s, 0, 1).background, marked, "the cursor row is not marked");
        assert_ne!(face_at(&s, 0, 0).background, marked, "and other rows are not");
    }

    #[test]
    fn the_completion_list_is_a_popup_at_the_top_of_the_frame() {
        let (mut e, mut s) = setup("text", 40, 10);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let candidates: Vec<String> =
            ["save-buffer", "save-some-buffers"].iter().map(|c| c.to_string()).collect();
        // The first completion grows the input to the common prefix; only
        // when it cannot grow further is the list offered.
        e.minibuffer.complete(&candidates);
        e.minibuffer.complete(&candidates);
        assert!(e.minibuffer.completion().visible, "the list should be offered");

        let lines: Vec<String> =
            rendered(&e, &mut s).into_iter().map(|l| l.trim_end().to_string()).collect();
        assert!(lines[0].starts_with('╭'), "no top border: `{}`", lines[0]);
        // The prompt is the first line inside the box, behind a count of
        // where the highlight is in the list. TAB completion highlights
        // nothing until it starts cycling, so this one reads zero.
        assert!(lines[1].contains("0/2 M-x save-"), "prompt line is `{}`", lines[1]);
        assert!(lines[2].contains("save-buffer"), "first candidate: `{}`", lines[2]);
        assert!(lines[3].contains("save-some-buffers"), "second candidate: `{}`", lines[3]);
        assert!(lines[4].starts_with('╰'), "no bottom border: `{}`", lines[4]);
        // Having moved into the popup, the prompt is not also at the bottom:
        // one prompt, in one place.
        assert!(lines[9].is_empty(), "the echo area still has `{}`", lines[9]);
    }

    #[test]
    fn the_candidate_being_cycled_is_marked() {
        let (mut e, mut s) = setup("text", 40, 10);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let candidates: Vec<String> =
            ["save-buffer", "save-some-buffers"].iter().map(|c| c.to_string()).collect();
        e.minibuffer.complete(&candidates);
        e.minibuffer.complete(&candidates);
        e.minibuffer.cycle_completion(true);

        draw(&e, &mut s);
        let chosen = e.theme.resolve("completion-selected").background;
        // Inside the border, the highlight runs the whole width of the row.
        assert_eq!(face_at(&s, 1, 2).background, chosen, "the first candidate is chosen");
        assert_eq!(face_at(&s, 38, 2).background, chosen, "the highlight stops short");
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
        assert!(lines[1].contains("0/30"), "no count of the matches: `{}`", lines[1]);
        // The prompt line holds `command-` too, so only rows whose first
        // column after the border is the name itself are counted.
        let rows = lines.iter().filter(|l| l.starts_with("│command-")).count();
        assert!((1..=6).contains(&rows), "{rows} candidate rows:
{lines:#?}");
    }

    #[test]
    fn nothing_is_drawn_when_there_are_no_completions_to_show() {
        let (mut e, mut s) = setup("text\nmore\n", 40, 8);
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        let lines: Vec<String> =
            rendered(&e, &mut s).into_iter().map(|l| l.trim_end().to_string()).collect();
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
        assert_eq!(face_at(&s, 0, 0).background, current, "the match point is on");
        assert_eq!(face_at(&s, 11, 0).background, other, "a later match");
        assert_eq!(face_at(&s, 22, 0).background, other, "and another");
        assert_ne!(current, other, "they must be distinguishable");
        assert_ne!(face_at(&s, 5, 0).background, other, "text between them is not marked");
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
        assert_eq!(face_at(&s, 7, 0).foreground, marked, "the brace under point");
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
        assert_eq!(message_tone("Mark set"), None, "an ordinary message is ordinary");
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
