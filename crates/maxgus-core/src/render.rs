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
use maxgus_tui::{Rect, Surface, char_width};

/// The text area of a window, in cells: what it draws its buffer into, which
/// is its own rectangle without the mode line and without the echo area.
///
/// Public because a window that can draw a fraction of a line needs to know
/// which fraction of the screen is allowed to move, and deriving that twice
/// is how the two come to disagree.
pub fn text_area(editor: &Editor, id: crate::window::WindowId) -> Option<Rect> {
    let window = editor.windows.get(id)?;
    let (body, _) = editor.frame.split_bottom(1);
    Some(window.rect.intersect(&body)?.split_bottom(1).0)
}

/// The text areas of the windows showing code: where a front end that can
/// draw ligatures should.
///
/// Code, and not the rest, because a ligature is a claim about what the
/// characters mean: `->` in Rust is an arrow, and joining it says so. `->`
/// in a help page, `--color` on a shell line and `C-<left>` in the list of
/// bindings are the characters they are made of, and a font joining them
/// there is contradicting the text.
pub fn code_areas(editor: &Editor) -> Vec<Rect> {
    editor
        .windows
        .iter()
        .filter(|window| {
            editor.buffers.get(window.buffer).is_some_and(|buffer| {
                buffer
                    .language()
                    .is_some_and(|language| !maxgus_text::is_prose(language))
            })
        })
        .filter_map(|window| text_area(editor, window.id))
        .collect()
}

/// Where a window is in its buffer, for a front end that draws a bar to
/// say so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPosition {
    pub window: crate::window::WindowId,
    /// The text area the bar belongs beside.
    pub area: Rect,
    /// How much of the buffer is above the window, as a fraction of it.
    pub above: f32,
    /// How much of the buffer the window shows, as a fraction of it.
    pub shown: f32,
}

/// Every window that is not showing its whole buffer, and where in the
/// buffer it is. Counted in lines rather than rows: a wrapped line is one
/// line of the buffer whatever it takes to draw, and a bar is a rough
/// answer anyway.
pub fn scroll_positions(editor: &Editor) -> Vec<ScrollPosition> {
    editor
        .windows
        .iter()
        .filter_map(|window| {
            let buffer = editor.buffers.get(window.buffer)?;
            let total = buffer.len_lines();
            let height = window.text_height();
            if height == 0 || height >= total {
                return None;
            }
            let area = text_area(editor, window.id)?;
            Some(ScrollPosition {
                window: window.id,
                area,
                above: window.top_line as f32 / total as f32,
                shown: height as f32 / total as f32,
            })
        })
        .collect()
}

/// The windows with another to their right: where a divider belongs.
///
/// The terminal has no pixel to draw one in — the windows simply abut, and
/// each one's gutter keeps them apart. A front end that has pixels can do
/// better, and this is which edges to do it on.
pub fn divided_windows(editor: &Editor) -> Vec<Rect> {
    let (body, _) = editor.frame.split_bottom(1);
    editor
        .windows
        .iter()
        .filter_map(|window| window.rect.intersect(&body))
        .filter(|rect| rect.x + rect.width < body.x + body.width)
        .collect()
}

/// The line just beyond the edge of the current window, drawn as that window
/// would draw it.
///
/// Smooth scrolling needs it. The window's text is drawn a fraction of a line
/// out of place, so a fraction of a line opens up at one edge, and what
/// belongs there is the line arriving — which the editor did not draw,
/// because it is not in the window yet. `direction` is which edge: `1` for
/// the line arriving at the bottom, `-1` for the one at the top.
///
/// `scratch` is somewhere to draw into, kept by the caller so this does not
/// allocate a screen every frame. Returns nothing at the ends of a buffer,
/// where the gap really is empty.
pub fn edge_row(
    editor: &mut Editor,
    id: crate::window::WindowId,
    direction: isize,
    scratch: &mut Surface,
) -> Option<Vec<maxgus_tui::Cell>> {
    edge_rows(editor, id, direction, 1, scratch)?.pop()
}

/// The `count` lines just beyond the edge of a window, in screen order.
///
/// One line is all a wheel notch ever needs, because the view never gets
/// more than a line ahead of the drawing. A command is different: it can
/// move the view a page, and `scroll-animation-far-lines` then asks for the
/// last few lines of that jump to be drawn as a slide — which opens a gap
/// several lines deep, with several lines' worth of nothing to put in it.
///
/// One redraw whatever `count` is: the frame is drawn once with the window
/// moved the whole way, and the rows are taken off the edge it moved from.
/// Drawing it once per line was the obvious way and would have made a
/// four-line slide cost four screens a frame.
pub fn edge_rows(
    editor: &mut Editor,
    id: crate::window::WindowId,
    direction: isize,
    count: usize,
    scratch: &mut Surface,
) -> Option<Vec<Vec<maxgus_tui::Cell>>> {
    let area = text_area(editor, id)?;
    if area.height == 0 || area.width == 0 || count == 0 {
        return None;
    }
    // Never more than the window holds: past that the "line arriving" is a
    // line that was already on screen, which would be drawn twice.
    let count = count.min(area.height as usize);
    let window = editor.windows.get(id)?;
    let (was, buffer_id) = (window.top_line, window.buffer);
    let moved = was.checked_add_signed(direction * count as isize)?;
    // Past the last line there is nothing arriving, and drawing it would only
    // produce the blank the gap already is.
    let lines = editor.buffers.get(buffer_id).map_or(0, |b| b.len_lines());
    if direction > 0 && moved + area.height as usize > lines {
        return None;
    }
    editor.windows.get_mut(id)?.top_line = moved;
    if scratch.size() != editor.frame.size() {
        scratch.resize(editor.frame.size());
    }
    draw(editor, scratch);
    if let Some(window) = editor.windows.get_mut(id) {
        window.top_line = was;
    }

    // Moving down the buffer, what is arriving is at the bottom of the frame
    // just drawn; moving up, at the top. Returned top to bottom either way,
    // so the caller lays them out in the order it has them: the first goes
    // one row below the window when moving down, and `count` rows above it
    // when moving up.
    let first = match direction > 0 {
        true => area.y + area.height - count as u16,
        false => area.y,
    };
    Some(
        (0..count as u16)
            .map(|n| {
                (area.x..area.x + area.width)
                    .map(|x| scratch.get(x, first + n).cloned().unwrap_or_default())
                    .collect()
            })
            .collect(),
    )
}

/// Paints the whole frame.
pub fn draw(editor: &Editor, surface: &mut Surface) {
    draw_background(editor, surface);
    draw_floating(editor, surface);
}

/// Everything that is not floating over something else: the windows, their
/// mode lines, and the echo area.
///
/// Apart from [`draw_floating`] because a window that blurs what is behind a
/// popup needs *what is behind it*, and by the time the popup has been
/// composited into the grid there is nothing behind it any more. Drawing the
/// two halves in turn costs no more than drawing them together — the same
/// cells are painted either way — and between the two calls is the only
/// moment the backdrop exists.
pub fn draw_background(editor: &Editor, surface: &mut Surface) {
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
    draw_echo_area(editor, surface, echo);
}

/// The things that go over the top of the windows, and where they went.
///
/// The rectangles come back so a front end that can blur knows which parts
/// of the frame are floating. Reporting them rather than working them out
/// again is the point: each of these decides its own size from the text it
/// has to show, and a second copy of that arithmetic somewhere else would be
/// a second copy to keep right.
pub fn draw_floating(editor: &Editor, surface: &mut Surface) -> Vec<Rect> {
    let frame = surface.area();
    if frame.height == 0 {
        return Vec::new();
    }
    let (body, echo) = frame.split_bottom(1);
    let mut floating = Vec::new();
    // The popup goes over the top of the windows rather than resizing them, so
    // opening the list does not reflow what is being edited. It carries the
    // prompt with it, and the echo area stays out of the way while it is up.
    #[cfg(feature = "full")]
    if let Some(active) = editor.transient.as_ref() {
        floating.extend(draw_transient(editor, surface, frame, active));
    }
    // What the language server said about the symbol under point, beside it
    // rather than over it.
    #[cfg(feature = "full")]
    if let Some(doc) = editor.doc.as_ref() {
        floating.extend(draw_doc(editor, surface, body, doc));
    }
    // And what could follow what is being typed, at the cursor. Drawn after
    // the doc box, because a list you are choosing from matters more than a
    // description of what you have already written.
    #[cfg(feature = "full")]
    if let Some(list) = editor.autocomplete.as_ref() {
        floating.extend(draw_autocomplete(editor, surface, body, list));
    }
    // What the next key can be, when someone has stopped in the middle of a
    // sequence. Over the windows like the popup, and under the echo area,
    // which is still showing the keys typed so far.
    // A menu that was asked for outright wins over one a pause opened: the
    // pause is a guess at what was wanted and the question mark is not.
    match editor.key_menu.as_ref() {
        Some(menu) => floating.extend(draw_key_menu(editor, surface, body, menu)),
        None => {
            if let Some(prefix) = editor.which_key.as_ref() {
                floating.extend(draw_which_key(editor, surface, body, prefix));
            }
        }
    }
    // The file browser, over everything: it is the thing being looked at
    // while it is up, not a note beside something else.
    if let Some(browser) = editor.browser.as_ref() {
        floating.extend(draw_browser(editor, surface, frame, browser));
    }
    // The completion popup takes the echo area with it: the prompt it is
    // answering rides along the top of the box instead.
    if let Some(area) = completion_popup(editor, frame) {
        draw_completion_popup(editor, surface, area);
        surface.clear_rect(echo, editor.theme.resolve("default"));
        floating.push(area);
    }
    floating
}

/// The list of suggestions, at the cursor.
///
/// Below the line being typed where there is room and above it where there
/// is not, and lined up with the word rather than centred, so the list sits
/// where the eye already is.
#[cfg(feature = "full")]
fn draw_autocomplete(
    editor: &Editor,
    surface: &mut Surface,
    body: Rect,
    list: &crate::autocomplete::Autocomplete,
) -> Option<Rect> {
    let window = editor.windows.current();
    let area = window.rect.intersect(&body)?;
    let (text_area, _) = area.split_bottom(1);
    if text_area.width < 16 || text_area.height < 4 {
        return None;
    }
    let (top, shown) = list.visible();
    let rows: Vec<&crate::autocomplete::Item> = shown.collect();
    if rows.is_empty() {
        return None;
    }

    // Wide enough for the widest row, and never more than half the window:
    // the code being written is the thing being looked at.
    let widest = rows
        .iter()
        .map(|item| item.label.chars().count() + label_extra(item))
        .max()
        .unwrap_or(1);
    let width = (widest + 4).clamp(16, (text_area.width as usize / 2).max(16)) as u16;
    let width = width.min(text_area.width);
    let height = rows.len() as u16 + 2;

    // At the cursor, and below it unless the bottom of the window is nearer
    // than the list is tall.
    let (cursor_x, cursor_y) = editor.cursor_position();
    let below = cursor_y.saturating_add(1);
    let y = match below + height <= text_area.bottom() {
        true => below,
        false => cursor_y.saturating_sub(height).max(text_area.y),
    };
    // Lined up with the start of the word, pulled back where it would spill
    // off the right edge.
    let x = cursor_x
        .min(text_area.right().saturating_sub(width))
        .max(text_area.x);
    let box_area = Rect::new(x, y, width, height.min(text_area.height));

    let theme = &editor.theme;
    let plain = theme.resolve("default");
    surface.clear_rect(box_area, plain);
    draw_border(surface, box_area, theme.resolve("completion-border"));
    let inner = box_area.inset(1);
    let selected_face = theme.resolve("completion-selected");
    let kind_face = theme.resolve("completion-annotation");
    let detail_face = theme.resolve("shadow");

    for (n, item) in rows.iter().enumerate() {
        let y = inner.y + n as u16;
        // `selected_row` counts from the first row on show, which is what
        // `n` counts too.
        let chosen = n == list.selected_row();
        // The selected row is painted across the whole width, so it reads
        // as a row rather than as a coloured word.
        if chosen {
            surface.clear_rect(Rect::new(inner.x, y, inner.width, 1), selected_face);
        }
        let base = match chosen {
            true => selected_face,
            false => plain,
        };
        let mut at = surface.set_string(inner.x + 1, y, &item.label, base, inner.width);
        // The kind, then whatever of the type fits after it.
        if !item.kind.is_empty() {
            let kind = format!(" {}", item.kind);
            let face = match chosen {
                true => selected_face,
                false => kind_face,
            };
            at = surface.set_string(at, y, &kind, face, inner.right().saturating_sub(at));
        }
        if !item.detail.is_empty() {
            let face = match chosen {
                true => selected_face,
                false => detail_face,
            };
            let room = inner.right().saturating_sub(at + 1);
            if room > 3 {
                surface.set_string(at, y, &format!(" {}", item.detail), face, room);
            }
        }
    }
    // How much of the list is off the bottom, so a long one says so.
    if list.len() > rows.len() {
        let count = format!("{}/{}", top + list.selected_row() + 1, list.len());
        let x = inner
            .right()
            .saturating_sub(count.chars().count() as u16 + 1);
        surface.set_string(
            x,
            box_area.bottom().saturating_sub(1),
            &count,
            theme.resolve("completion-count"),
            count.chars().count() as u16,
        );
    }
    Some(area)
}

/// How much room a row wants beyond its label.
#[cfg(feature = "full")]
fn label_extra(item: &crate::autocomplete::Item) -> usize {
    let kind = match item.kind.is_empty() {
        true => 0,
        false => item.kind.chars().count() + 1,
    };
    let detail = match item.detail.is_empty() {
        true => 0,
        // A long type is cut rather than making the list as wide as it is.
        false => item.detail.chars().count().min(24) + 1,
    };
    kind + detail
}

/// The box holding what the language server said about a symbol.
///
/// `lsp-ui-doc`: beside the line the symbol is on rather than over it, on
/// whichever side has room. What arrives is markdown — a heading, a rule, a
/// bulleted list of parameters and the signature in a fenced block — and it
/// is drawn as those things rather than as the punctuation that spells them.
#[cfg(feature = "full")]
fn draw_doc(editor: &Editor, surface: &mut Surface, body: Rect, doc: &crate::Doc) -> Option<Rect> {
    let window = editor.windows.get(doc.window)?;
    let area = window.rect.intersect(&body)?;
    let (text_area, _) = area.split_bottom(1);
    if text_area.width < 20 || text_area.height < 6 {
        return None;
    }
    // Three fifths of the window at most, and never wider than it needs.
    // The width wanted is the width of the *rendered* document, not of the
    // markdown: `### function \`add\`` is four characters narrower once the
    // punctuation is gone.
    let widest = crate::markup::natural_width(&doc.text);
    let width = (widest + 4).clamp(20, (text_area.width as usize * 3 / 5).max(20)) as u16;
    let width = width.min(text_area.width);
    let lines = crate::markup::render(&doc.text, width.saturating_sub(4) as usize);
    // Half the window: enough for a heading, a signature, the parameters
    // and a sentence — which is what a reply is — while leaving as much of
    // the code on screen as the box covers.
    let most = (text_area.height as usize / 2).max(3);
    let height = (lines.len().min(most) + 2) as u16;

    // The row the symbol is on, and whether the box fits under it.
    let row = text_area.y + doc.line.saturating_sub(window.top_line) as u16;
    let below = row.saturating_add(1);
    let y = match below + height <= text_area.bottom() {
        true => below,
        // Above it, or pinned to the top when there is no room either way.
        false => row.saturating_sub(height).max(text_area.y),
    };
    // Against the right edge, where the code usually is not.
    let x = text_area.right().saturating_sub(width).max(text_area.x);
    let box_area = Rect::new(x, y, width, height.min(text_area.height));

    let theme = &editor.theme;
    let panel = theme.resolve("doc");
    let border = theme.resolve("doc-border");
    surface.clear_rect(box_area, panel);
    draw_border(surface, box_area, border);
    draw_border_title(
        surface,
        box_area,
        "Documentation",
        theme.resolve("doc-title"),
    );
    let inner = box_area.inset(1);
    // One column of padding inside the border, so text does not touch it.
    let (left, room) = (inner.x + 1, inner.width.saturating_sub(2));

    let buffer_bg = theme.resolve("default").background;
    for (n, line) in lines.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + n as u16;
        match line {
            // Drawn edge to edge, and joined to the border, so it reads as
            // a division of the box rather than a row of hyphens in it.
            crate::markup::Line::Rule => {
                for x in inner.x..inner.x + inner.width {
                    surface.set_char(x, y, '─', border);
                }
                surface.set_char(box_area.x, y, '├', border);
                surface.set_char(box_area.right().saturating_sub(1), y, '┤', border);
            }
            crate::markup::Line::Text(spans) => {
                let mut at = left;
                for span in spans {
                    let mut face = on_panel(theme.resolve(span.face), panel, buffer_bg);
                    if span.bold {
                        face.attributes.bold = Some(true);
                    }
                    if span.italic {
                        face.attributes.italic = Some(true);
                    }
                    let left_here = (left + room).saturating_sub(at);
                    at = surface.set_string(at, y, &span.text, face, left_here);
                }
            }
        }
    }
    // Say when there is more than fits, rather than ending mid-sentence.
    // The row is cleared first: the line being replaced is usually longer
    // than the notice, and its tail would otherwise read as part of it.
    if lines.len() > inner.height as usize && inner.height > 0 {
        let more = format!("… {} more lines", lines.len() - inner.height as usize + 1);
        let y = inner.y + inner.height - 1;
        surface.clear_rect(Rect::new(inner.x, y, inner.width, 1), panel);
        // The row replaced may have been a rule, whose ends are in the
        // border rather than inside it and so survived the clearing: a box
        // with two stubs poking into it where a line used to cross.
        surface.set_char(box_area.x, y, '│', border);
        surface.set_char(box_area.right().saturating_sub(1), y, '│', border);
        let shadow = on_panel(theme.resolve("shadow"), panel, buffer_bg);
        surface.set_string(left, y, &more, shadow, room);
    }
    Some(area)
}

/// Puts `face` on a panel: its own colours, but the panel's background.
///
/// A box with a background of its own is a box every face drawn into it
/// punches a hole in, because a face that says nothing about a background
/// inherits the buffer's. So one that did not choose is given the panel's,
/// and one that did — `doc-code`, which is a panel of its own — keeps what
/// it chose. That is the whole rule, and it is the difference between a box
/// that lifts off the text and a box with the text showing through it.
#[cfg(feature = "full")]
fn on_panel(mut face: Face, panel: Face, buffer: Option<maxgus_faces::Color>) -> Face {
    if face.background == buffer {
        face.background = panel.background;
    }
    face
}

/// The panel that says what can follow a half-typed sequence.
///
/// Along the bottom of the windows, in as many even columns as the width
/// allows: `which-key` puts it where the eye already is when the hand has
/// stopped. What will not fit is counted rather than quietly dropped — a
/// panel that shows twenty of thirty keys and says so is useful, and one
/// that shows twenty and implies that is all of them is a liar.
fn draw_which_key(
    editor: &Editor,
    surface: &mut Surface,
    body: Rect,
    prefix: &str,
) -> Option<Rect> {
    /// Between one column and the next.
    const GAP: usize = 2;

    let entries = crate::which_key::continuations(editor, prefix);
    if entries.is_empty() || body.width < 16 || body.height < 5 {
        return None;
    }
    let theme = &editor.theme;
    let plain = theme.resolve("default");

    /// A column narrower than this is a column of ellipses.
    const NARROWEST: usize = 18;

    // Columns wide enough for the longest entry, unless packing them tighter
    // is what makes everything fit — which-key would rather cut a name than
    // hide a key, and so would anyone looking for the key.
    let inside = body.width.saturating_sub(2) as usize;
    let natural = entries
        .iter()
        .map(|entry| entry.key.chars().count() + 3 + entry.label.chars().count())
        .max()
        .unwrap_or(1)
        .min(inside);
    // Two thirds of the screen at the outside: a panel that swallows the
    // buffer is worse than not knowing what `C-x r` does.
    let most = ((body.height as usize * 2) / 3).max(1);
    let roomy = ((inside + GAP) / (natural + GAP)).max(1);
    let packed = ((inside + GAP) / (NARROWEST + GAP)).max(roomy);
    let columns = entries.len().div_ceil(most).max(roomy).min(packed);
    let cell = ((inside + GAP) / columns).saturating_sub(GAP).max(1);
    let rows = entries.len().div_ceil(columns).min(most);
    let room = rows * columns;
    let over = entries.len().saturating_sub(room);

    let height = (rows + 2) as u16;
    let area = Rect::new(
        body.x,
        body.bottom().saturating_sub(height).max(body.y),
        body.width,
        height.min(body.height),
    );
    surface.clear_rect(area, plain);
    draw_border(surface, area, theme.resolve("completion-border"));
    let inner = area.inset(1);
    let key_face = theme.resolve("completion-key");
    let group_face = theme.resolve("which-key-group");

    // The last cell is spent on the count when there is one to give.
    let shown = match over {
        0 => entries.len(),
        _ => room.saturating_sub(1),
    };
    for (n, entry) in entries.iter().take(shown).enumerate() {
        let (column, row) = (n / rows, n % rows);
        let x = inner.x + (column * (cell + GAP)) as u16;
        let y = inner.y + row as u16;
        which_key_cell(surface, x, y, cell, entry, key_face, group_face, plain);
    }
    if over > 0 {
        let n = shown;
        let x = inner.x + ((n / rows) * (cell + GAP)) as u16;
        let y = inner.y + (n % rows) as u16;
        let more = format!("… {} more", over + 1);
        surface.set_string(x, y, &more, group_face, cell as u16);
    }
    Some(area)
}

/// The whole of a keymap, in the box `which-key` draws into.
///
/// treemacs' helpful hydra, which is what `?` in the tree is: named columns
/// — Navigation, Nodes, Files — rather than the single level a half-typed
/// prefix shows, because this is being read rather than glanced at, and a
/// list of fifty keys in no order is a list nobody reads twice.
///
/// Sections are kept whole. One that will not fit in what is left of a
/// column starts the next one instead of being broken across the two, which
/// is the difference between a heading and a heading with its keys
/// somewhere else. What still does not fit is counted, the way the
/// which-key panel counts it.
fn draw_key_menu(
    editor: &Editor,
    surface: &mut Surface,
    body: Rect,
    menu: &crate::which_key::Menu,
) -> Option<Rect> {
    /// Between one column and the next.
    const GAP: usize = 3;
    /// Between a key and what it does.
    const LEAD: usize = 2;

    if menu.sections.is_empty() || body.width < 16 || body.height < 6 {
        return None;
    }
    let theme = &editor.theme;
    let plain = theme.resolve("default");

    // Three quarters of the frame. which-key takes two thirds because it
    // arrives uninvited and goes again; this was asked for, and a panel
    // asked for may have more of the screen than one that interrupted.
    let most = ((body.height as usize * 3) / 4).saturating_sub(2).max(1);
    let inside = body.width.saturating_sub(2) as usize;

    // The shortest packing that fits, rather than the first one tried.
    //
    // Height and width trade against each other: taller columns are fewer
    // columns and so a narrower panel. Filling each column to the brim and
    // then discovering the last one has nowhere to go drops sections while
    // the panel is visibly half empty — so every height is tried, shortest
    // first, and the first that fits across is the one drawn. The panel is
    // then as short as it can be while still saying everything.
    let mut columns = pack(&menu.sections, most);
    for height in 1..=most {
        let candidate = pack(&menu.sections, height);
        if columns_fit(&candidate, inside, GAP, LEAD) {
            columns = candidate;
            break;
        }
    }

    // Each column is as wide as its own widest row rather than as the
    // widest row anywhere, which is how treemacs' hydra reads: a column of
    // three-letter toggles does not get the width of a column of sentences.
    let widths: Vec<usize> = columns
        .iter()
        .map(|column| menu_column_width(column, LEAD).min(inside))
        .collect();
    let mut fits = 0;
    let mut at = 0;
    for width in &widths {
        let wants = at + width + usize::from(at > 0) * GAP;
        if fits > 0 && wants > inside {
            break;
        }
        at = wants;
        fits += 1;
    }
    let over: usize = columns
        .split_off(fits.max(1).min(columns.len()))
        .iter()
        .flat_map(|column| column.iter())
        .map(|section| section.entries.len())
        .sum();
    let rows = columns
        .iter()
        .map(|column| {
            column.iter().map(|s| s.height()).sum::<usize>() + column.len().saturating_sub(1)
        })
        .max()
        .unwrap_or(1);

    let height = (rows + 2 + usize::from(over > 0)) as u16;
    let area = Rect::new(
        body.x,
        body.bottom().saturating_sub(height).max(body.y),
        body.width,
        height.min(body.height),
    );
    surface.clear_rect(area, plain);
    let border = theme.resolve("completion-border");
    draw_border(surface, area, border);
    draw_border_title(surface, area, &menu.title, theme.resolve("menu-heading"));
    let inner = area.inset(1);
    let key_face = theme.resolve("completion-key");
    let heading = theme.resolve("menu-heading");

    let mut x = inner.x;
    for (n, column) in columns.iter().enumerate() {
        let cell = widths[n];
        let mut y = inner.y;
        for (first, section) in column.iter().enumerate() {
            if first > 0 {
                y += 1;
            }
            if y >= inner.bottom() {
                break;
            }
            surface.set_string(x, y, &section.title, heading, cell as u16);
            y += 1;
            let widest = section
                .entries
                .iter()
                .map(|(key, _)| key.chars().count())
                .max()
                .unwrap_or(0);
            for (key, what) in &section.entries {
                if y >= inner.bottom() {
                    break;
                }
                surface.set_string(x, y, key, key_face, cell as u16);
                let at = x + (widest + LEAD) as u16;
                let left = (cell as u16).saturating_sub(at.saturating_sub(x));
                surface.set_string(at, y, what, plain, left);
                y += 1;
            }
        }
        x += (cell + GAP) as u16;
    }
    // The same honesty the which-key panel owes: a panel that shows most of
    // a keymap and implies it is all of it is a liar.
    if over > 0 {
        let more = format!("… {over} more");
        let y = inner.bottom().saturating_sub(1);
        surface.set_string(inner.x, y, &more, theme.resolve("shadow"), inner.width);
    }
    Some(area)
}

/// The file browser: a box over the frame that narrows as you type.
///
/// Centred and wide, because it is the thing being looked at rather than a
/// note in the corner. The directory is in the top border, the filter is
/// the first row inside it with the cursor sitting in it, and the keys are
/// along the bottom — a box that has to be explained somewhere else is a
/// box people close again.
fn draw_browser(
    editor: &Editor,
    surface: &mut Surface,
    frame: Rect,
    browser: &crate::browser::Browser,
) -> Option<Rect> {
    if frame.width < 30 || frame.height < 8 {
        return None;
    }
    let theme = &editor.theme;
    let icons = editor.settings.nerd_font_icons;

    // Three fifths of the frame, and no wider than a path needs.
    let width = ((frame.width as usize * 3) / 5).clamp(30, frame.width as usize) as u16;
    // Two rows of chrome inside the border — the filter and its rule — and
    // the rows themselves, up to two thirds of the frame.
    let most = ((frame.height as usize * 2) / 3).saturating_sub(4).max(1);
    let rows = browser.rows().len().clamp(1, most);
    let height = (rows + 4) as u16;
    let area = Rect::new(
        frame.x + (frame.width.saturating_sub(width)) / 2,
        frame.y + (frame.height.saturating_sub(height)) / 3,
        width,
        height.min(frame.height),
    );

    let plain = theme.resolve("default");
    let border = theme.resolve("completion-border");
    surface.clear_rect(area, plain);
    draw_border(surface, area, border);
    // The directory, shortened at the front: the end of a long path is the
    // part that says where you are.
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = browser.directory.to_string_lossy();
    let shown = match !home.is_empty() && shown.starts_with(&home) {
        true => format!("~{}", &shown[home.len()..]),
        false => shown.into_owned(),
    };
    // The question goes first and keeps its room: a box that says where you
    // are but not what it wants is a box you have to remember why you
    // opened. It is the path that gets shortened when the two will not fit.
    let asked = match browser.prompt.is_empty() {
        true => String::new(),
        false => format!("{} · ", browser.prompt),
    };
    // A search shows a root and every directory under it, which is a
    // different thing from showing what is in that root.
    let shown = match browser.searched {
        true => format!("under {shown}"),
        false => shown,
    };
    let room = (area.width as usize)
        .saturating_sub(16 + asked.chars().count())
        .max(8);
    let shown = match shown.chars().count() > room {
        true => format!(
            "…{}",
            shown
                .chars()
                .skip(shown.chars().count() - room)
                .collect::<String>()
        ),
        false => shown,
    };
    let shown = format!("{asked}{shown}");
    draw_border_title(surface, area, &shown, theme.resolve("menu-heading"));

    let inner = area.inset(1);
    let (shown_count, total) = browser.tally();
    // The tally in the top right: three of forty is a different thing from
    // a directory with three files in it.
    // `+` when the walk stopped at its limit rather than at the end: a list
    // that is not all of them should not claim to be.
    let more = match browser.capped {
        true => "+",
        false => "",
    };
    let tally = format!(" {shown_count}/{total}{more} ");
    let at = area
        .right()
        .saturating_sub(tally.chars().count() as u16 + 2);
    if at > area.x + shown.chars().count() as u16 + 4 {
        surface.set_string(
            at,
            area.y,
            &tally,
            theme.resolve("completion-count"),
            area.width,
        );
    }

    // The filter, with the cursor in it.
    let mut x = inner.x + 1;
    if icons {
        x = surface.set_string(
            x,
            inner.y,
            "\u{f002} ",
            theme.resolve("minibuffer-prompt"),
            inner.width,
        );
    }
    x = surface.set_string(
        x,
        inner.y,
        &browser.filter,
        theme.resolve("default"),
        inner.right().saturating_sub(x),
    );
    if x < inner.right() {
        surface.set_char(x, inner.y, ' ', theme.resolve("cursor"));
    }
    // A rule under it, joined to the border so it reads as a division of
    // the box rather than a row of dashes in it.
    let rule = inner.y + 1;
    for x in inner.x..inner.right() {
        surface.set_char(x, rule, '─', border);
    }
    surface.set_char(area.x, rule, '├', border);
    surface.set_char(area.right().saturating_sub(1), rule, '┤', border);

    let list = Rect::new(
        inner.x,
        rule + 1,
        inner.width,
        inner.bottom().saturating_sub(rule + 1),
    );
    if browser.rows().is_empty() {
        let note = match (browser.pending, browser.searched) {
            (true, true) => "Searching…",
            (true, false) => "Reading…",
            (false, _) => "Nothing matches",
        };
        surface.set_string(
            list.x + 1,
            list.y,
            note,
            theme.resolve("panel-note"),
            list.width,
        );
    }
    // Scrolled so the cursor is always on screen, which a list you hold an
    // arrow down on stops being otherwise.
    let height = list.height as usize;
    let top = browser.selected.saturating_sub(height.saturating_sub(1));
    for row in 0..list.height {
        let index = top + row as usize;
        let Some(entry) = browser.rows().get(index) else {
            break;
        };
        let y = list.y + row;
        let selected = index == browser.selected;
        if selected {
            surface.clear_rect(
                Rect::new(list.x, y, list.width, 1),
                theme.resolve("completion-selected"),
            );
        }
        draw_browser_row(editor, surface, browser, *entry, list, y, selected, icons);
    }

    // The keys, along the bottom border.
    // Longest first: a narrow box drops back to the keys it cannot do
    // without, rather than to nothing at all.
    let hints: &[&str] = match (browser.is_choosing(), browser.searched) {
        (true, true) => &[
            " ↑↓ move · → in · ← back · RET choose ",
            " ↑↓ move · RET choose ",
        ],
        (true, false) => &[
            " ↑↓ move · → in · ← out · C-s search ~ · RET choose ",
            " ↑↓ move · → in · ← out · RET choose ",
        ],
        (false, _) => &[" ↑↓ move · → in · ← out · RET open "],
    };
    if let Some(hint) = hints
        .iter()
        .find(|hint| (hint.chars().count() as u16) < area.width.saturating_sub(4))
    {
        surface.set_string(
            area.x + 2,
            area.bottom().saturating_sub(1),
            hint,
            theme.resolve("shadow"),
            area.width,
        );
    }
    Some(area)
}

/// One entry: what it is, what it is called, how big and how old.
#[allow(clippy::too_many_arguments)]
fn draw_browser_row(
    editor: &Editor,
    surface: &mut Surface,
    browser: &crate::browser::Browser,
    row: crate::browser::Row,
    area: Rect,
    y: u16,
    selected: bool,
    icons: bool,
) {
    let theme = &editor.theme;
    // A selected row keeps the selection's background whatever the face of
    // the thing on it says, or the bar would have holes in it.
    let background = match selected {
        true => theme.resolve("completion-selected").background,
        false => None,
    };
    let face = |name: &str| {
        let mut face = theme.resolve(name);
        if let Some(background) = background {
            face.background = Some(background);
        }
        face
    };

    let entry = browser.entry(row);
    let (glyph, name, kind) = match (row, entry) {
        // The directory being looked at, offered as an answer to a question
        // about which directory. Open, where the rest are shut: it is the
        // one you are standing in.
        (crate::browser::Row::Here, _) => (
            crate::icons::DIRECTORY_OPEN,
            ".".to_string(),
            "tree-directory",
        ),
        (_, None) => (crate::icons::DIRECTORY, "..".to_string(), "tree-directory"),
        (_, Some(entry)) if entry.is_dir => (
            crate::icons::DIRECTORY,
            format!("{}/", entry.name),
            "tree-directory",
        ),
        (_, Some(entry)) if entry.link.is_some() => {
            (crate::icons::SYMLINK, entry.name.clone(), "tree-symlink")
        }
        (_, Some(entry)) => (
            crate::icons::for_file(std::path::Path::new(&entry.name)),
            entry.name.clone(),
            "tree-file",
        ),
    };

    let mut x = area.x + 1;
    if icons {
        x = surface.set_string(x, y, &format!("{glyph} "), face(kind), area.right() - x);
    }
    // The size and the date sit against the right edge, so they line up as
    // columns rather than trailing after names of every length.
    let detail = match row {
        // `.` is a row nobody has seen in a file browser before, because
        // no other file browser answers with a directory. Saying what it
        // is costs one dimmed phrase.
        crate::browser::Row::Here => "this directory".to_string(),
        // A walk has no detail to show: it read the names and not the
        // directories, which is most of why it is quick.
        _ if browser.searched => String::new(),
        _ => entry
            .filter(|entry| !entry.is_dir)
            .map(|entry| format!("{:>7}  {}", human_size(entry.size), entry.modified))
            .or_else(|| entry.map(|entry| format!("{:>7}  {}", "", entry.modified)))
            .unwrap_or_default(),
    };
    let detail_at = area
        .right()
        .saturating_sub(detail.chars().count() as u16 + 1);
    let room = detail_at.saturating_sub(x + 1).max(1);
    surface.set_string(x, y, &name, face(kind), room);
    if !detail.is_empty() && detail_at > x + 1 {
        surface.set_string(
            detail_at,
            y,
            &detail,
            face("shadow"),
            area.right() - detail_at,
        );
    }
}

/// A size as a person reads it: `6.2k`, `1.4M`.
fn human_size(bytes: u64) -> String {
    const STEP: f64 = 1024.0;
    let bytes = bytes as f64;
    for (limit, suffix) in [(STEP, ""), (STEP * STEP, "k"), (STEP * STEP * STEP, "M")] {
        if bytes < limit {
            let scaled = bytes / (limit / STEP);
            return match suffix.is_empty() || scaled >= 10.0 {
                true => format!("{scaled:.0}{suffix}"),
                false => format!("{scaled:.1}{suffix}"),
            };
        }
    }
    format!("{:.1}G", bytes / (STEP * STEP * STEP))
}

/// Fills columns `height` rows tall with whole sections./// Fills columns `height` rows tall with whole sections.
///
/// A section that will not fit in what is left starts the next column
/// rather than being broken across the two: a heading in one column with
/// its keys in another is a heading over nothing.
fn pack(
    sections: &[crate::which_key::Section],
    height: usize,
) -> Vec<Vec<&crate::which_key::Section>> {
    let mut columns: Vec<Vec<&crate::which_key::Section>> = Vec::new();
    let mut used = 0;
    for section in sections {
        // A blank row between one section and the next, but not above the
        // first in a column: a heading against the border reads as a title.
        // Which is why the separator is charged after the column is chosen
        // and not before — a section that starts a new column has nothing
        // above it to be separated from, and paying for a row it does not
        // use made every column but the first one row short.
        let wants = section.height() + usize::from(used > 0);
        if columns.is_empty() || used + wants > height {
            columns.push(Vec::new());
            used = 0;
        }
        used += section.height() + usize::from(used > 0);
        columns.last_mut().expect("just pushed").push(section);
    }
    columns
}

/// The width a menu column wants: its widest section, and no more.
fn menu_column_width(column: &[&crate::which_key::Section], lead: usize) -> usize {
    column
        .iter()
        .map(|section| section.width(lead))
        .max()
        .unwrap_or(1)
}

/// Whether every column would fit side by side in `inside` columns.
fn columns_fit(
    columns: &[Vec<&crate::which_key::Section>],
    inside: usize,
    gap: usize,
    lead: usize,
) -> bool {
    let widths = columns.iter().map(|column| menu_column_width(column, lead));
    let gaps = gap * columns.len().saturating_sub(1);
    widths.sum::<usize>() + gaps <= inside
}

/// One `key → label` in the which-key panel, cut to `cell` columns.
#[allow(clippy::too_many_arguments)]
fn which_key_cell(
    surface: &mut Surface,
    x: u16,
    y: u16,
    cell: usize,
    entry: &crate::which_key::Continuation,
    key_face: Face,
    group_face: Face,
    plain: Face,
) {
    let key_width = entry.key.chars().count();
    let mut at = surface.set_string(x, y, &entry.key, key_face, cell as u16);
    at = surface.set_string(at, y, " → ", plain, 3);
    let room = cell.saturating_sub(key_width + 3);
    if room == 0 {
        return;
    }
    let label: String = match entry.label.chars().count() > room {
        // A name cut without saying so reads as a different name.
        true => entry
            .label
            .chars()
            .take(room.saturating_sub(1))
            .chain(std::iter::once('…'))
            .collect(),
        false => entry.label.clone(),
    };
    let face = match entry.group {
        true => group_face,
        false => plain,
    };
    surface.set_string(at, y, &label, face, room as u16);
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
    #[cfg(feature = "full")]
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
    #[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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
            draw_selection_mark(surface, area, y, theme.resolve("tree-selection-mark"));
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
            draw_selection_mark(surface, area, y, theme.resolve("tree-selection-mark"));
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

/// The mark that says whether a row can be opened, and whether it is.
///
/// A chevron where the font has one, and `>`/`v` where it does not — which
/// are letters pretending to be arrows, and are what this drew before.
/// Always two columns wide either way, so the indentation of everything
/// after it does not move when the glyphs are turned off.
fn expander(expandable: bool, expanded: bool, icons: bool) -> String {
    if !expandable {
        return "  ".to_string();
    }
    match (icons, expanded) {
        (true, true) => format!("{} ", crate::icons::CHEVRON_DOWN),
        (true, false) => format!("{} ", crate::icons::CHEVRON_RIGHT),
        (false, true) => "v ".to_string(),
        (false, false) => "> ".to_string(),
    }
}

/// The vertical rules that say how deep a row is nested.
///
/// A panel of indented names is a panel where the eye has to measure
/// whitespace to see what belongs to what. A line down each level is what
/// turns the indentation into a shape. treemacs calls it an indent guide,
/// and it is the difference between a list and a tree.
///
/// Drawn under everything else on the row, so a name or a glyph that
/// reaches back over one covers it rather than being cut by it.
fn draw_indent_guides(surface: &mut Surface, area: Rect, y: u16, depth: usize, face: Face) {
    for level in 0..depth {
        let x = area.x + (level as u16 * 2);
        if x >= area.right() {
            break;
        }
        surface.set_char(x, y, '│', face);
    }
}

/// The bar down the left of the selected row.
///
/// The row already has a background of its own, which says *that* it is
/// selected. This says where the selection is when the eye is somewhere
/// else on the screen: a solid mark in an accent colour reads from the
/// corner of the eye in a way a change of background does not.
fn draw_selection_mark(surface: &mut Surface, area: Rect, y: u16, face: Face) {
    surface.set_char(area.x, y, '▎', face);
}

fn draw_tree_row(
    editor: &Editor,
    surface: &mut Surface,
    node: &maxgus_tree::VisibleNode,
    area: Rect,
    y: u16,
    face: &dyn Fn(&str) -> Face,
) {
    // One column at the left for the selection mark, so the rows do not
    // shift sideways as the cursor moves down them.
    let area = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(1),
        area.height,
    );
    draw_indent_guides(surface, area, y, node.depth, face("tree-indent"));
    let mut x = area.x + (node.depth as u16 * 2).min(area.width);
    // The chevron marks what can be opened.
    let icons = editor.settings.nerd_font_icons;
    let mark = expander(node.expandable, node.expanded, icons);
    x = surface.set_string(x, y, &mark, face("tree-arrow"), area.right() - x);
    // The glyph says what kind of thing it is at a glance, in the face of
    // the node itself so a directory's icon reads as a directory.
    if icons {
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
    let area = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(1),
        area.height,
    );
    // One level deeper than the tree's, because every symbol sits under the
    // heading rather than beside it.
    draw_indent_guides(surface, area, y, symbol.depth + 1, face("tree-indent"));
    let indent = ((symbol.depth as u16 + 1) * 2).min(area.width);
    let mut x = area.x + indent;
    let mark = expander(symbol.has_children(), symbol.expanded, icons);
    x = surface.set_string(x, y, &mark, face("tree-arrow"), area.right() - x);
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

/// How wide a line may be before it wraps, or `None` when lines are clipped.
///
/// The text area less the line-number gutter — a wrapped line has to break
/// where the text ends, not where the window does. Shared with the
/// scrolling, which has to agree with the drawing about how many rows a
/// line takes or point goes off screen.
pub(crate) fn wrap_width(editor: &Editor, window: &Window, buffer: &Buffer) -> Option<usize> {
    if editor.settings.truncate_lines {
        return None;
    }
    Some(text_columns(editor, window, buffer))
}

/// The columns a window has for text once the line numbers have taken
/// theirs, and the last column its own: that is where a row says it goes
/// on (`\`) or that the line does (`$`), as a terminal Emacs does, so the
/// text stops short of it.
pub(crate) fn text_columns(editor: &Editor, window: &Window, buffer: &Buffer) -> usize {
    let area = text_area(editor, window.id).unwrap_or(window.rect);
    area.width
        .saturating_sub(line_number_width(editor, buffer))
        .saturating_sub(1) as usize
}

/// One screen row's worth of a line.
///
/// A window that truncates has one of these per line; a window that wraps
/// has one per row a line takes. Everything that draws goes through the
/// list, so neither the text, the line numbers nor the cursors can disagree
/// about which row anything is on.
#[derive(Debug, Clone, Copy)]
struct Visual {
    /// Which row down the window this is, so everything that draws on it
    /// works the position out the same way.
    row: u16,
    line: usize,
    /// The first character on this row.
    start: usize,
    /// One past the last: where the next row begins, or the line's end.
    end: usize,
    /// The display column `start` sits at, counted from the line's start.
    column: usize,
    /// Taken off a character's display column to place it — the row's own
    /// column when wrapping, the horizontal scroll when truncating.
    origin: usize,
    /// False on a continuation row, which carries no line number.
    first: bool,
}

/// The rows a window shows, top to bottom, at most `height` of them.
fn visual_rows(editor: &Editor, window: &Window, buffer: &Buffer, height: u16) -> Vec<Visual> {
    let lines = buffer.len_lines();
    let mut out = Vec::new();
    let Some(width) = wrap_width(editor, window, buffer) else {
        for row in 0..height as usize {
            let line = window.top_line + row;
            if line >= lines {
                break;
            }
            let start = buffer.line_start(line);
            out.push(Visual {
                row: row as u16,
                line,
                start,
                end: maxgus_text::Motion::line_end(buffer.rope(), start),
                column: 0,
                origin: window.left_column,
                first: true,
            });
        }
        return out;
    };

    let mut line = window.top_line;
    let mut index = window.top_row;
    while out.len() < height as usize && line < lines {
        // Once per line rather than once per row: a line that wraps twenty
        // times would otherwise be walked twenty times over.
        let rows = crate::wrap::rows_of(buffer, line, width);
        let end_of_line = maxgus_text::Motion::line_end(buffer.rope(), buffer.line_start(line));
        while index < rows.len() && out.len() < height as usize {
            out.push(Visual {
                row: out.len() as u16,
                line,
                start: rows[index].offset,
                end: rows.get(index + 1).map_or(end_of_line, |next| next.offset),
                column: rows[index].column,
                origin: rows[index].column,
                first: index == 0,
            });
            index += 1;
        }
        line += 1;
        index = 0;
    }
    out
}

/// The width the line-number column takes, including its trailing space.
///
/// Shared with `Editor::cursor_position`, which has to move the cursor over by
/// the same amount the text is moved over — otherwise it sits in the gutter,
/// three columns adrift of the character it is on.
pub(crate) fn line_number_width(editor: &Editor, buffer: &Buffer) -> u16 {
    if !editor.settings.line_numbers || !has_line_numbers(editor, buffer) {
        return 0;
    }
    // Enough digits for the last line, plus a separating space.
    let digits = buffer.len_lines().max(1).to_string().len();
    (digits + 1) as u16
}

/// Whether a buffer is drawn with a line-number column at all.
///
/// The views a subsystem paints itself — the tree and its panels, the git
/// views — have no gutter, and a cursor placed as though they
/// had one sits a few columns to the right of the character it is on.
fn has_line_numbers(editor: &Editor, buffer: &Buffer) -> bool {
    let name = buffer.name();
    if name == crate::commands::tree::TREE_BUFFER_NAME
        || name == crate::commands::tree::SYMBOLS_BUFFER_NAME
        || name == crate::commands::tree::BUFFERS_BUFFER_NAME
    {
        return false;
    }
    #[cfg(feature = "full")]
    if name == crate::commands::git::STATUS_BUFFER_NAME
        || editor.git_diffs.contains_key(name)
        || editor.git_lists.contains_key(name)
    {
        return false;
    }
    let _ = editor;
    true
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
    #[cfg(feature = "full")]
    let diagnostics = resolve_diagnostics(editor, buffer);
    #[cfg(not(feature = "full"))]
    let diagnostics: Vec<(Range, &'static str)> = Vec::new();
    // The other matches of a running search, and the delimiter matching the
    // one under point. Both are resolved once for the window, like the
    // diagnostics: computing them per line would repeat the work per row.
    let first_line = window.top_line;
    // One line per row is the most a window can show, so this is never too
    // small — a wrapping window shows fewer lines, not more.
    let last_line = (window.top_line + area.height as usize).min(buffer.len_lines());
    let matches = resolve_search_matches(editor, buffer, first_line, last_line);
    let paren = matching_delimiter(editor, buffer, window);
    let decorations = resolve_decorations(editor, buffer, first_line, last_line);

    // The fill column, marked before the text so the text draws over it.
    if editor.settings.fill_column_indicator {
        draw_fill_column(editor, surface, window, area, gutter);
    }

    // Past the end of the buffer there are no rows, and Emacs draws nothing
    // there — not tildes.
    let rows = visual_rows(editor, window, buffer, area.height);
    for visual in &rows {
        let y = area.y + visual.row;
        if gutter > 0 && visual.first {
            draw_line_number(surface, theme, visual.line, point_line, area.x, y, gutter);
        }
        draw_line(
            editor,
            surface,
            window,
            buffer,
            visual,
            &LineArea { area, gutter },
            &Overlays {
                diagnostics: &diagnostics,
                decorations: &decorations,
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
        let Some((x, y)) = cell_of(*cursor, buffer, &rows, &LineArea { area, gutter }) else {
            continue;
        };
        let mut cell = surface.get(x, y).copied().unwrap_or_default();
        cell.face = face;
        surface.set(x, y, cell);
    }

    draw_beacon(
        editor,
        surface,
        window,
        buffer,
        &rows,
        &LineArea { area, gutter },
    );
}

/// Paints the light beside the cursor, when one is lit.
///
/// Over the text rather than instead of it: the characters stay readable and
/// only their background changes, which is what `beacon` does with an overlay.
fn draw_beacon(
    editor: &Editor,
    surface: &mut Surface,
    window: &Window,
    buffer: &Buffer,
    rows: &[Visual],
    area: &LineArea,
) {
    let Some(beacon) = editor.beacon.filter(|beacon| beacon.window == window.id) else {
        return;
    };
    let shape = editor.beacon_shape();
    let consumed = shape.consumed(beacon.elapsed);
    let light = editor.beacon_light();
    let background = editor
        .theme
        .resolve("default")
        .background
        .and_then(maxgus_faces::Color::to_rgb)
        .unwrap_or((0, 0, 0));
    let Some((x, y)) = cell_of(beacon.offset, buffer, rows, area) else {
        return;
    };
    for index in 0..shape.size {
        let Some(colour) = crate::beacon::cell_colour(&shape, &light, background, index, consumed)
        else {
            break;
        };
        let column = x + index as u16;
        if column >= area.area.right() {
            break;
        }
        let mut cell = surface.get(column, y).copied().unwrap_or_default();
        cell.face.background = Some(maxgus_faces::Color::Rgb(colour.0, colour.1, colour.2));
        surface.set(column, y, cell);
    }
}

/// Which of the rows on screen holds `offset`, and how far along it.
///
/// The rows are asked rather than the arithmetic done, because with wrapping
/// there is no arithmetic to do: a row is not a line, and only the list that
/// was drawn knows which is which.
fn place_in(rows: &[Visual], buffer: &Buffer, offset: usize) -> Option<(u16, usize)> {
    for (index, visual) in rows.iter().enumerate() {
        // The last row of a line also holds the position *after* its last
        // character, which is where point sits at the end of a line.
        let last = rows
            .get(index + 1)
            .is_none_or(|next| next.line != visual.line);
        let within =
            offset >= visual.start && (offset < visual.end || (last && offset <= visual.end));
        if within {
            let column = buffer.display_column(offset).checked_sub(visual.origin)?;
            return Some((visual.row, column));
        }
    }
    None
}

/// The screen cell an offset is drawn in, or `None` when it is not on screen.
fn cell_of(offset: usize, buffer: &Buffer, rows: &[Visual], area: &LineArea) -> Option<(u16, u16)> {
    let (row, column) = place_in(rows, buffer, offset.min(buffer.len_chars()))?;
    if row >= area.area.height {
        return None;
    }
    let x = area.area.x + area.gutter + column as u16;
    if x >= area.area.right() {
        return None;
    }
    Some((x, area.area.y + row))
}

/// Where point sits in its window: the row down from the top of the text and
/// the column across it, both already adjusted for wrapping, the gutter and
/// any horizontal scroll.
///
/// Shared with `Editor::cursor_position`, which puts the hardware cursor
/// there and must land on the same cell the character was drawn in.
pub(crate) fn point_cell(
    editor: &Editor,
    window: &Window,
    buffer: &Buffer,
    offset: usize,
) -> (u16, u16) {
    let height = text_area(editor, window.id).map_or(window.rect.height, |area| area.height);
    let rows = visual_rows(editor, window, buffer, height);
    place_in(&rows, buffer, offset.min(buffer.len_chars()))
        .map(|(row, column)| (row, column as u16))
        // Off screen, which redisplay is about to correct: the top-left is
        // as good a guess as any and better than an arithmetic one that
        // assumed a row per line.
        .unwrap_or((0, 0))
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
    /// The faces a mode lays over its own buffer — dired's marks and
    /// directories — under everything the user does to the text.
    decorations: &'a [(Range, &'static str)],
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
    visual: &Visual,
    place: &LineArea,
    overlays: &Overlays<'_>,
) {
    let LineArea { area, gutter } = *place;
    let y = area.y + visual.row;
    let layers = Layers::new(editor, window, buffer, visual.line, overlays);

    let left = area.x + gutter;
    // The last column is kept for the edge marks, so text stops before it.
    let edge = area.right().saturating_sub(1);
    let right = edge.max(left);
    // The display column of the first character on this row, counted from
    // the start of the line so tabs land on their usual stops.
    let mut column = visual.column;
    let mut offset = visual.start;

    while offset < visual.end {
        let c = buffer.rope().char(offset);
        let width = buffer.char_display_width(c, column);
        let face = layers.face_at(offset, c);

        // Skip what horizontal scrolling has moved off the left edge. A
        // wrapping window has no horizontal scroll, and its origin is the
        // row's own column, so nothing is ever skipped there.
        if column + width > visual.origin {
            let x = left + (column.saturating_sub(visual.origin) as u16);
            if x >= right {
                break;
            }
            match c {
                // A tab paints as blanks up to the next tab stop.
                '\t' => {
                    for i in 0..width {
                        let at = x + i as u16;
                        if at < right {
                            surface.set(at, y, cell(' ', face));
                        }
                    }
                }
                // Control characters show as `^X`, as Emacs draws them.
                c if (c as u32) < 0x20 => {
                    let caret = format!("^{}", (b'@' + c as u8) as char);
                    surface.set_string(x, y, &caret, face, right - x);
                }
                c => {
                    surface.set_char(x, y, c, face);
                }
            }
        }
        column += width;
        offset += 1;
    }

    let line_end = maxgus_text::Motion::line_end(buffer.rope(), buffer.line_start(visual.line));
    // The region and search highlights extend across the newline, so a
    // selected line reads as selected all the way to the right edge. Only
    // on the row the line actually ends on: a continuation row runs into
    // the next one, and there is no newline under it to extend across.
    if offset >= line_end
        && let Some(face) = layers.eol_face()
    {
        let x = left + (column.saturating_sub(visual.origin) as u16);
        for at in x..area.right() {
            surface.set(at, y, cell(' ', face));
        }
    }

    // What a terminal Emacs puts in the last column: `\` on a row that
    // wraps on to the next, `$` on a line cut off by the edge. Without it a
    // line that wraps and two lines that happen to align read the same.
    if offset < line_end && edge >= left {
        let mark = if editor.settings.truncate_lines {
            '$'
        } else {
            '\\'
        };
        surface.set(edge, y, cell(mark, editor.theme.resolve("fringe")));
    }
}

fn cell(ch: char, face: Face) -> maxgus_tui::Cell {
    maxgus_tui::Cell::new(ch, face)
}

/// The face layers in effect for one line.
struct Layers<'a> {
    theme: &'a Theme,
    default: Face,
    /// Syntax spans overlapping this line, in byte offsets.
    #[cfg(feature = "full")]
    highlights: Vec<&'a maxgus_syntax::Highlight>,
    /// Read to turn a character offset into the byte offset the spans use.
    #[cfg(feature = "full")]
    rope: &'a ropey::Rope,
    region: Option<Range>,
    /// The search match point is on.
    current: Option<Range>,
    /// The other matches on this line.
    others: Vec<Range>,
    /// Delimiter positions to mark on this line.
    parens: Vec<usize>,
    diagnostics: Vec<(Range, &'static str)>,
    /// The mode's own faces on this line.
    decorations: Vec<(Range, &'static str)>,
    /// Trailing whitespace on this line, when the face is worth showing.
    trailing: Option<Range>,
    /// True when the region or a match runs past the end of this line.
    region_spans_eol: bool,
}

impl<'a> Layers<'a> {
    #[cfg_attr(not(feature = "full"), allow(unused_variables))]
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

        #[cfg(feature = "full")]
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

        // The match point is on is drawn differently from the others — the
        // one a search is at, or the one query-replace is asking about.
        let current = editor
            .isearch
            .as_ref()
            .and_then(|s| s.current)
            .or_else(|| {
                editor
                    .query_replace
                    .as_ref()
                    .and_then(|s| s.current.as_ref().map(|m| m.range))
            })
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
        let decorations: Vec<(Range, &'static str)> = overlays
            .decorations
            .iter()
            .filter(|(r, _)| r.overlaps(&line_range))
            .copied()
            .collect();
        let trailing = trailing_whitespace(buffer, line_range);

        Layers {
            theme: &editor.theme,
            default: editor.theme.resolve("default"),
            #[cfg(feature = "full")]
            highlights,
            #[cfg(feature = "full")]
            rope,
            region,
            current,
            others,
            parens,
            diagnostics,
            decorations,
            trailing,
            region_spans_eol,
        }
    }

    /// The composed face for the character at `offset`.
    fn face_at(&self, offset: usize, _c: char) -> Face {
        let mut face = self.default;

        // Syntax highlighting, looked up by byte offset: the spans come from
        // tree-sitter, which counts bytes.
        #[cfg(feature = "full")]
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
        if let Some((_, name)) = self.decorations.iter().find(|(r, _)| r.contains(offset)) {
            face.overlay(&self.theme.resolve_overlay(name));
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

#[cfg(feature = "full")]
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

/// The faces a mode lays over its own buffer, for the lines on screen.
///
/// Dired is the one so far: it writes plain text into its buffer, and says
/// here which of it is a directory, a link, or a row that is marked.
fn resolve_decorations(
    editor: &Editor,
    buffer: &Buffer,
    first_line: usize,
    last_line: usize,
) -> Vec<(Range, &'static str)> {
    if buffer.name() != crate::commands::dired::DIRED_BUFFER_NAME {
        return Vec::new();
    }
    let Some(view) = editor.dired.as_ref() else {
        return Vec::new();
    };
    (first_line..last_line)
        .filter_map(|line| Some((buffer.line_start(line), view.row(line)?)))
        .flat_map(|(start, row)| {
            view.row_faces(row)
                .into_iter()
                .map(move |(column, length, face)| {
                    (Range::new(start + column, start + column + length), face)
                })
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
    let column = editor.fill_column_for(window.buffer);
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
    let start = buffer.line_start(first_line);
    let end = buffer.line_start(last_line);
    // A listing marks the matches it was made from, the way `occur` does.
    if let Some(listing) = editor.listings.get(buffer.name()) {
        return listing
            .matches
            .iter()
            .filter(|m| m.start < end && m.end > start)
            .copied()
            .collect();
    }
    // The query being replaced marks its other matches too, so what `!`
    // would do to the rest of the screen can be seen before answering it.
    let query = match (editor.isearch.as_ref(), editor.query_replace.as_ref()) {
        (Some(search), _) => {
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
            query
        }
        (None, Some(replace)) if replace.current.is_some() => replace.query.clone(),
        _ => return Vec::new(),
    };
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
    let (mut right, left): (Vec<_>, Vec<_>) = segments.into_iter().partition(|s| s.right);
    let width = |text: &str| -> u16 { text.chars().map(|c| char_width(c) as u16).sum() };

    // A segment is a word: it goes on the bar whole or not at all, since
    // `18:0 To` cut off mid-word says less than nothing. The buffer's name
    // is the one exception, kept by losing its front — the end of a path
    // is the part that tells files apart — because it is what a narrow
    // window most needs the bar to say.
    let mut x = area.x;
    for segment in &left {
        let room = area.right() - x;
        if width(&segment.text) <= room {
            x = surface.set_string(x, area.y, &segment.text, paint(segment.face), room);
            continue;
        }
        if segment.shortens && room > 1 {
            let text = shortened_from_the_front(&segment.text, room);
            x = surface.set_string(x, area.y, &text, paint(segment.face), room);
        }
        break;
    }

    // The right-hand group sits against the edge, and gives way from its
    // inner end — problems, then the branch, then the language — until
    // what is left fits beside the left-hand group.
    while !right.is_empty() && x + right.iter().map(|s| width(&s.text)).sum::<u16>() > area.right()
    {
        right.remove(0);
    }
    let mut at = area.right() - right.iter().map(|s| width(&s.text)).sum::<u16>();
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

/// The last `room` columns of `text`, behind an ellipsis that says the
/// front went.
fn shortened_from_the_front(text: &str, room: u16) -> String {
    let mut kept = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let wide = char_width(c) as u16;
        if used + wide > room {
            break;
        }
        used += wide;
        kept.push(c);
    }
    std::iter::once('\u{2026}')
        .chain(kept.into_iter().rev())
        .collect()
}

#[cfg(feature = "full")]
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
) -> Option<Rect> {
    let transient = active.current()?;
    let theme = &editor.theme;

    type Line = Vec<(String, &'static str)>;
    let mut groups: Vec<Vec<Line>> = Vec::new();
    for group in transient.groups {
        let mut lines: Vec<Line> = vec![vec![(group.title.to_string(), "transient-heading")]];
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
        groups.push(lines);
    }

    // The groups go side by side when there is room, as magit lays them
    // out, and a group is never cut in two: its heading and its keys are
    // read together, and a column break between them left the keys under
    // the wrong heading.
    let widest = groups
        .iter()
        .flatten()
        .map(|line| {
            line.iter()
                .map(|(text, _)| text.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let column = (widest as u16 + 2).max(24);
    let columns = (frame.width / column).max(1) as usize;
    let columns = columns.min(groups.len()).max(1);
    let total: usize = groups.iter().map(Vec::len).sum::<usize>() + groups.len() - 1;
    let target = total.div_ceil(columns);
    // Consecutive groups fill a column until the next one would push it past
    // its share, then start the next column, while there are columns left.
    let mut packed: Vec<Vec<Line>> = vec![Vec::new()];
    for lines in groups {
        let filled = packed.last().map_or(0, Vec::len);
        let overflows = filled > 0 && filled + 1 + lines.len() > target;
        if overflows && packed.len() < columns {
            packed.push(lines);
            continue;
        }
        let current = packed.last_mut().expect("one column at least");
        if !current.is_empty() {
            current.push(Vec::new());
        }
        current.extend(lines);
    }
    let rows = packed.iter().map(Vec::len).max().unwrap_or(0);
    let height = (rows as u16 + 2).min(frame.height.saturating_sub(2));
    if height < 3 {
        return None;
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

    for (index, lines) in packed.iter().enumerate() {
        let offset = index as u16 * column;
        if inner.x + offset >= inner.right() {
            break;
        }
        for (row, line) in lines.iter().enumerate() {
            let y = inner.y + row as u16;
            if y >= inner.bottom() {
                break;
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
    Some(area)
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
    // does not push its documentation across the screen away from it. A
    // name is only cut short to make room for what annotates it: a list of
    // paths with nothing beside them has the whole box to spell them in.
    let annotated = annotations
        .iter()
        .any(|(k, d)| !k.is_empty() || !d.is_empty());
    let most = match annotated {
        true => inner.width * 2 / 3,
        false => inner.width,
    };
    let names = column_width(shown.iter().map(String::as_str), most);
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
            // The shortest of them. A command now often has two — the
            // classic Emacs key and the one under Doom's leader — and the
            // shorter is the one worth showing in a column this narrow.
            let key = editor
                .keymaps
                .where_is(candidate)
                .iter()
                .map(|sequence| sequence.notation())
                .min_by_key(|notation| (notation.chars().count(), notation.clone()))
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
/// Writes `title` into the top border, so a panel says what it is.
///
/// `╭─ File tree ─────╮`, with a space either side of the words: the border
/// is a line and a word laid straight on a line is hard to read. A title
/// too long for the box is dropped rather than cut — a heading that has lost
/// its last word is worse than no heading, because it still looks like one.
fn draw_border_title(surface: &mut Surface, area: Rect, title: &str, face: Face) {
    if area.width < 2 {
        return;
    }
    let text = format!(" {title} ");
    let room = area.width.saturating_sub(4) as usize;
    if title.is_empty() || text.chars().count() > room {
        return;
    }
    surface.set_string(area.x + 2, area.y, &text, face, room as u16);
}

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
mod menu_layout_tests {
    use super::*;
    use crate::which_key::Section;

    fn section(title: &str, entries: usize) -> Section {
        Section {
            title: title.to_string(),
            entries: (0..entries)
                .map(|n| (format!("k{n}"), format!("does thing {n}")))
                .collect(),
        }
    }

    #[test]
    fn a_column_holds_as_many_rows_as_it_was_given() {
        // The bug this is here for: the blank row between two sections was
        // charged to the section that then went to the *next* column, where
        // it had nothing above it to be separated from. Every column but
        // the first was one row short, and the sections squeezed out that
        // way were reported as "… 3 more" under a panel with room for them.
        let sections = vec![section("a", 3), section("b", 3), section("c", 3)];
        // Each is 4 rows; two of them plus a separator is 9.
        let packed = pack(&sections, 9);
        assert_eq!(packed.len(), 2, "{packed:#?}");
        assert_eq!(packed[0].len(), 2, "the first column lost a section");
        assert_eq!(packed[1].len(), 1);

        for column in &packed {
            let rows: usize = column.iter().map(|s| s.height()).sum::<usize>() + column.len() - 1;
            assert!(rows <= 9, "a column of {rows} rows was given 9");
        }
    }

    #[test]
    fn a_taller_column_is_a_narrower_panel() {
        // Which is the whole reason the height is searched rather than
        // fixed: one more row down can be a whole column less across.
        let sections = vec![section("a", 3), section("b", 3)];
        assert_eq!(pack(&sections, 4).len(), 2);
        assert_eq!(pack(&sections, 9).len(), 1);
    }

    #[test]
    fn a_section_taller_than_a_column_still_gets_one() {
        // Rather than looping, or being dropped: a panel that silently
        // omits a section is the failure the count is there to prevent.
        let sections = vec![section("huge", 20)];
        let packed = pack(&sections, 5);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].len(), 1);
    }

    #[test]
    fn columns_fit_measures_the_gaps_between_them_too() {
        let sections = vec![section("a", 1), section("b", 1)];
        let packed = pack(&sections, 2);
        assert_eq!(packed.len(), 2);
        let width = menu_column_width(&packed[0], 2);
        // Exactly the two columns, and not the gap between them.
        assert!(!columns_fit(&packed, width * 2, 3, 2));
        assert!(columns_fit(&packed, width * 2 + 3, 3, 2));
    }
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
    #[cfg(feature = "full")]
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

    #[test]
    fn a_window_showing_its_whole_buffer_gets_no_scroll_bar() {
        let (editor, _) = setup("one\ntwo\nthree\n", 40, 10);
        assert!(scroll_positions(&editor).is_empty());
    }

    #[test]
    fn a_scrolled_window_says_how_far_down_it_is_and_how_much_it_shows() {
        // A hundred lines in a window eight rows tall: the bar is eight
        // hundredths long, and starts where the window does.
        let text = (1..=100)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (mut editor, _) = setup(&text, 40, 10);
        let id = editor.windows.current_id();
        let at_top = scroll_positions(&editor);
        assert_eq!(at_top.len(), 1);
        assert_eq!(at_top[0].window, id);
        assert_eq!(at_top[0].above, 0.0);
        assert!((at_top[0].shown - 0.08).abs() < 1e-6, "{:?}", at_top[0]);
        assert_eq!(
            at_top[0].area,
            text_area(&editor, id).unwrap(),
            "the bar goes beside the text, not the mode line"
        );
        editor.scroll_window_lines(id, 50);
        let down = scroll_positions(&editor);
        assert!((down[0].above - 0.5).abs() < 1e-6, "{:?}", down[0]);
        assert_eq!(down[0].shown, at_top[0].shown);
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
        assert_eq!(lines[0], "efghijklm$");
    }

    #[test]
    fn a_long_line_is_clipped_at_the_right_edge() {
        // With a `$` in the last column to say so, as a terminal Emacs
        // does; a line that just fits has no mark.
        let (e, mut s) = setup(&"x".repeat(100), 10, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "xxxxxxxxx$");
        let (e, mut s) = setup(&"x".repeat(9), 10, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "xxxxxxxxx ");
    }

    #[test]
    fn a_row_that_wraps_ends_in_a_backslash() {
        let (mut e, mut s) = setup("abcdefghijklmnop\nshort\n", 10, 6);
        e.settings.truncate_lines = false;
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "abcdefghi\\");
        assert_eq!(lines[1], "jklmnop   ");
        assert_eq!(lines[2], "short     ");
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

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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
    fn a_narrow_mode_line_leaves_things_off_rather_than_cutting_them() {
        // Twenty columns: room for the flags and a name, not for the
        // position. `18:0 To` cut short is worse than no position at all.
        let plain = Settings {
            nerd_font_icons: false,
            ..Settings::default()
        };
        let mut editor = Editor::new(
            plain.clone(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 20, 5),
        );
        let id = editor
            .buffers
            .visit_file("/project/main.rs", "let x = 1;\n");
        editor.switch_to_buffer(id).unwrap();
        let mut s = Surface::new(Size::new(20, 5));
        let lines = rendered(&editor, &mut s);
        assert_eq!(lines[3].trim_end(), " -- 11 main.rs");

        // Narrower than the name: the name gives up its front, and its
        // end, the part that says which file, is what shows.
        let mut editor = Editor::new(
            plain,
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 16, 5),
        );
        let id = editor
            .buffers
            .visit_file("/project/the-long-way-round.rs", "let x = 1;\n");
        editor.switch_to_buffer(id).unwrap();
        let mut s = Surface::new(Size::new(16, 5));
        let lines = rendered(&editor, &mut s);
        assert_eq!(lines[3].trim_end(), " -- 11 …round.rs");
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
        // A column at the left is kept for the selection mark, so the rows
        // do not shift sideways as the cursor moves down them. Then the
        // chevron, in whichever form the settings ask for.
        let icons = e.settings.nerd_font_icons;
        let open = expander(true, true, icons);
        let shut = expander(true, false, icons);
        assert!(
            lines[0]
                .trim_start_matches('▎')
                .starts_with(open.trim_end()),
            "the open chevron, got `{}`",
            lines[0]
        );
        assert!(lines[0].contains("project"), "got `{}`", lines[0]);
        assert!(
            lines[1].contains(shut.trim_end()),
            "the shut chevron, got `{}`",
            lines[1]
        );
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
        // Read past the mark column, which is a foreground of its own.
        assert_eq!(
            face_at(&s, 2, top + 1).background,
            marked,
            "the cursor row is not marked"
        );
        assert_ne!(
            face_at(&s, 2, top).background,
            marked,
            "and other rows are not"
        );
        // And the bar itself, which is what says where the selection is
        // when the eye is somewhere else on the screen.
        assert_eq!(
            s.get(0, top + 1).map(|cell| cell.ch),
            Some('▎'),
            "the selected row has no mark down its left"
        );
        assert_ne!(
            s.get(0, top).map(|cell| cell.ch),
            Some('▎'),
            "an unselected row has one"
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
    fn query_replace_marks_the_match_it_is_asking_about_and_the_rest() {
        let (mut e, mut s) = setup("one two one", 40, 5);
        let query =
            maxgus_text::SearchQuery::new("one", maxgus_text::SearchKind::Literal, None).unwrap();
        let current = query.search_forward(e.current_buffer().rope(), 0).unwrap();
        e.query_replace = Some(crate::commands::search::QueryReplace {
            query,
            replacement: "1".into(),
            current: Some(current),
            replaced: 0,
            replace_all: false,
        });
        draw(&e, &mut s);
        let asked = e.theme.resolve("isearch").background;
        let other = e.theme.resolve("lazy-highlight").background;
        assert_eq!(face_at(&s, 0, 0).background, asked);
        assert_eq!(face_at(&s, 8, 0).background, other);
        assert_ne!(face_at(&s, 4, 0).background, other, "`two` is not a match");
    }

    #[cfg(feature = "full")]
    #[test]
    fn a_menu_puts_its_groups_side_by_side_and_never_splits_one() {
        let (mut e, mut s) = setup("text", 80, 24);
        e.transient = Some(crate::transient::Active::new("commit"));
        let lines = rendered(&e, &mut s);
        let arguments = lines.iter().position(|l| l.contains("Arguments")).unwrap();
        let create = lines.iter().position(|l| l.contains("Create")).unwrap();
        assert_eq!(arguments, create, "the two headings share a row");
        // Every key of a group sits under its heading, in the same column.
        let column = lines[create].find("Create").unwrap();
        assert!(lines[create + 1][column..].trim_start().starts_with('c'));
        assert!(lines[create + 5][column..].trim_start().starts_with('f'));
        assert!(
            lines[arguments + 4]
                .trim_start_matches(['│', ' '])
                .starts_with("-n"),
            "got `{}`",
            lines[arguments + 4]
        );

        // Too narrow for two columns: the groups are stacked, one after the
        // other, with a blank line between them.
        let (mut e, mut s) = setup("text", 40, 24);
        e.transient = Some(crate::transient::Active::new("commit"));
        let lines = rendered(&e, &mut s);
        let arguments = lines.iter().position(|l| l.contains("Arguments")).unwrap();
        let create = lines.iter().position(|l| l.contains("Create")).unwrap();
        assert_eq!(create, arguments + 6);
        assert!(lines[arguments + 5].trim_matches(['│', ' ']).is_empty());
    }

    #[test]
    fn an_empty_buffer_draws_a_blank_screen_with_a_mode_line() {
        let (e, mut s) = setup("", 20, 4);
        let lines = rendered(&e, &mut s);
        assert_eq!(lines[0], "                    ");
        assert!(lines[2].contains("test"), "the mode line is still there");
    }

    #[test]
    fn ligatures_belong_to_the_code_windows_and_dividers_to_the_left_ones() {
        let (mut e, _) = setup("fn main() {}", 80, 24);
        assert!(
            code_areas(&e).is_empty(),
            "a buffer with no language is not code"
        );
        assert!(divided_windows(&e).is_empty(), "one window has no seam");
        e.with_current_buffer(|b| b.set_language(Some("txt".into())));
        assert!(code_areas(&e).is_empty(), "a text file is not code");

        e.with_current_buffer(|b| b.set_language(Some("rust".into())));
        let right = e
            .split_window(crate::window::Direction::Horizontal)
            .unwrap();
        e.select_window(right);
        let help = e.buffers.create_with_text("*Help*", "C-x C-f -> find-file");
        e.switch_to_buffer(help).unwrap();

        let code = code_areas(&e);
        assert_eq!(code.len(), 1, "only the code window: {code:?}");
        assert_eq!(
            code[0],
            Rect::new(0, 0, 40, 22),
            "its text, not its mode line"
        );
        let seams = divided_windows(&e);
        assert_eq!(seams.len(), 1, "one seam between two windows: {seams:?}");
        assert_eq!(
            seams[0],
            Rect::new(0, 0, 40, 23),
            "the left window, mode line and all"
        );
    }
}
