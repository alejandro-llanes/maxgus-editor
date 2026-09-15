//! Rectangles: the text between two columns, over a run of lines.
//!
//! The rectangle is the one point and the mark make the corners of, as in
//! Emacs, whether or not the mark is active. Columns are display columns, so
//! a tab or a wide character counts for the cells it takes rather than for
//! one character.
//!
//! `C-x r k` kills one and `C-x r y` puts it back somewhere else, which is
//! what pulls a column out of a table or indents a block by hand;
//! `C-x r t` writes the same string down each line of it, which is how a
//! column of prefixes is added in one go.

use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::{CoreError, MinibufferKind, Result, command};
use maxgus_text::{Buffer, Motion, Range};

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "kill-rectangle",
            "Kill the rectangle between point and the mark, keeping it for yank-rectangle.",
            kill_rectangle
        ),
        command!(
            "delete-rectangle",
            "Delete the rectangle between point and the mark.",
            delete_rectangle
        ),
        command!(
            "copy-rectangle-as-kill",
            "Keep the rectangle between point and the mark for yank-rectangle.",
            copy_rectangle_as_kill
        ),
        command!(
            "yank-rectangle",
            "Insert the last killed rectangle with its top left corner at point.",
            yank_rectangle
        ),
        command!(
            "open-rectangle",
            "Push the text in the rectangle to the right, leaving it blank.",
            open_rectangle
        ),
        command!(
            "clear-rectangle",
            "Blank out the rectangle with spaces.",
            clear_rectangle
        ),
        command!(
            "string-rectangle",
            "Replace each line of the rectangle with a string.",
            string_rectangle
        ),
        command!(
            "rectangle-number-lines",
            "Number the lines of the rectangle down its left edge.",
            number_lines
        ),
    ]);
}

/// Where a rectangle is: its first and last lines and the display columns
/// between which it lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub first: usize,
    pub last: usize,
    pub left: usize,
    pub right: usize,
}

impl Bounds {
    /// The rectangle with `a` and `b` at opposite corners.
    pub fn of(buffer: &Buffer, a: usize, b: usize) -> Bounds {
        let (start, end) = (a.min(b), a.max(b));
        let (x, y) = (buffer.display_column(a), buffer.display_column(b));
        Bounds {
            first: buffer.line_of(start),
            last: buffer.line_of(end),
            left: x.min(y),
            right: x.max(y),
        }
    }

    pub fn width(&self) -> usize {
        self.right - self.left
    }
}

/// The corners of the rectangle to act on, or why there is none.
fn bounds(editor: &mut Editor) -> Result<Bounds> {
    editor.sync_to_buffer();
    let buffer = editor.current_buffer();
    let Some(mark) = buffer.mark() else {
        return Err(CoreError::Message(
            "The mark is not set now, so there is no rectangle".into(),
        ));
    };
    Ok(Bounds::of(buffer, mark, buffer.point()))
}

/// The text of each line of the rectangle, padded with spaces to its width
/// where a line stops short of the right edge — so putting it back somewhere
/// keeps its shape.
pub fn extract(buffer: &Buffer, bounds: Bounds) -> Vec<String> {
    (bounds.first..=bounds.last)
        .map(|line| {
            let from = buffer.offset_at_display_column(line, bounds.left);
            let to = buffer.offset_at_display_column(line, bounds.right);
            let (from, to) = (from.min(to), to.max(from));
            let taken = buffer.display_column(to) - buffer.display_column(from);
            let short = bounds.width().saturating_sub(taken);
            format!(
                "{}{}",
                buffer.slice(Range::new(from, to)),
                " ".repeat(short)
            )
        })
        .collect()
}

/// Replaces each line's part of the rectangle with what `with` makes of it,
/// from the last line up so the earlier offsets stay good, as one undo step.
fn rewrite(editor: &mut Editor, bounds: Bounds, with: impl Fn(usize) -> String) -> Result<()> {
    editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            for line in (bounds.first..=bounds.last).rev() {
                let start = buffer.line_start(line);
                let end = Motion::line_end(buffer.rope(), start);
                let reach = buffer.display_column(end);
                let from = buffer.offset_at_display_column(line, bounds.left);
                let to = buffer.offset_at_display_column(line, bounds.right);
                let replacement = with(line - bounds.first);
                // A line that stops before the rectangle begins is padded out
                // to it, but only when there is something to put there.
                let pad = match reach < bounds.left && !replacement.trim_end().is_empty() {
                    true => " ".repeat(bounds.left - reach),
                    false => String::new(),
                };
                let replacement = match to >= end && replacement.trim_end().is_empty() {
                    // Nothing but spaces at the end of a line is trailing
                    // whitespace nobody wants.
                    true => String::new(),
                    false => replacement,
                };
                buffer.replace(
                    Range::new(from.min(to), to.max(from)),
                    &format!("{pad}{replacement}"),
                )?;
            }
            Ok::<(), maxgus_text::TextError>(())
        })
    })?;
    Ok(())
}

/// Inserts `rows` as a rectangle with its top left corner at point: each row
/// on the next line down at the same column, lines added at the end of the
/// buffer and spaces added to lines too short to reach it. Point ends at the
/// bottom right corner, as Emacs leaves it.
pub fn insert(editor: &mut Editor, rows: &[String]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    editor.sync_to_buffer();
    let (line, column) = {
        let buffer = editor.current_buffer();
        (
            buffer.line_of(buffer.point()),
            buffer.display_column(buffer.point()),
        )
    };
    let end = editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            let mut end = buffer.point();
            for (index, row) in rows.iter().enumerate() {
                let target = line + index;
                if target >= buffer.len_lines() {
                    let at = buffer.point_max();
                    buffer.insert(at, "\n")?;
                }
                let start = buffer.line_start(target);
                let line_end = Motion::line_end(buffer.rope(), start);
                let reach = buffer.display_column(line_end);
                let at = buffer.offset_at_display_column(target, column);
                let pad = " ".repeat(column.saturating_sub(reach));
                let text = format!("{pad}{row}");
                buffer.insert(at, &text)?;
                end = at + text.chars().count();
            }
            Ok::<usize, maxgus_text::TextError>(end)
        })
    })?;
    editor.move_point_to(end);
    editor.follow_point();
    Ok(())
}

fn kill_rectangle(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let rows = extract(editor.current_buffer(), bounds);
    rewrite(editor, bounds, |_| String::new())?;
    editor.kill(&rows.join("\n"), false);
    let count = rows.len();
    editor.killed_rectangle = rows;
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message(format!(
        "Killed a rectangle of {}; C-x r y puts it back",
        crate::count(count, "line")
    ));
    Ok(())
}

fn delete_rectangle(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    rewrite(editor, bounds, |_| String::new())?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

fn copy_rectangle_as_kill(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let rows = extract(editor.current_buffer(), bounds);
    let count = rows.len();
    editor.killed_rectangle = rows;
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message(format!(
        "Copied a rectangle of {}",
        crate::count(count, "line")
    ));
    Ok(())
}

fn yank_rectangle(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.killed_rectangle.is_empty() {
        return Err(CoreError::Message(
            "No rectangle has been killed or copied".into(),
        ));
    }
    let rows = editor.killed_rectangle.clone();
    insert(editor, &rows)
}

fn open_rectangle(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let rows = extract(editor.current_buffer(), bounds);
    let blank = " ".repeat(bounds.width());
    rewrite(editor, bounds, |index| format!("{blank}{}", rows[index]))?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

fn clear_rectangle(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let blank = " ".repeat(bounds.width());
    rewrite(editor, bounds, |_| blank.clone())?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

fn string_rectangle(editor: &mut Editor, args: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let Some(text) = args.input.clone() else {
        editor.prompt_for(
            "string-rectangle",
            MinibufferKind::Text,
            "String rectangle: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    rewrite(editor, bounds, |_| text.clone())?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

fn number_lines(editor: &mut Editor, _: &Args) -> Result<()> {
    let bounds = bounds(editor)?;
    let rows = extract(editor.current_buffer(), bounds);
    let count = bounds.last - bounds.first + 1;
    let digits = count.to_string().len();
    rewrite(editor, bounds, |index| {
        format!("{:>digits$} {}", index + 1, rows[index])
    })?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup(text: &str) -> (Dispatcher, crate::Editor) {
        let mut editor = crate::Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.create_with_text("t", text);
        editor.switch_to_buffer(id).unwrap();
        (
            Dispatcher::new(crate::commands::standard_registry()),
            editor,
        )
    }

    /// Mark at `from`, point at `to`.
    fn corners(e: &mut crate::Editor, from: usize, to: usize) {
        e.with_current_buffer(|b| {
            b.set_point(from);
            b.set_mark(from);
            b.set_point(to);
        });
        e.windows.current_mut().point = to;
    }

    fn run(d: &mut Dispatcher, e: &mut crate::Editor, name: &str) {
        let out = d.execute(e, name, None);
        assert!(!matches!(out, Dispatch::Failed { .. }), "{name}: {out:?}");
    }

    #[test]
    fn a_killed_rectangle_comes_out_of_every_line_and_goes_back_in_shape() {
        let (mut d, mut e) = setup("abcd\nefgh\nijkl\n");
        // From `b` to the column after `k`: columns 1 to 3 of three lines.
        corners(&mut e, 1, 13);
        run(&mut d, &mut e, "kill-rectangle");
        assert_eq!(e.current_buffer().text(), "ad\neh\nil\n");
        assert_eq!(e.killed_rectangle, vec!["bc", "fg", "jk"]);

        // Back in at the end of the first line.
        e.move_point_to(2);
        run(&mut d, &mut e, "yank-rectangle");
        assert_eq!(e.current_buffer().text(), "adbc\nehfg\niljk\n");
    }

    #[test]
    fn a_rectangle_reaching_past_short_lines_keeps_its_width() {
        let (mut d, mut e) = setup("long line\nab\nlong line\n");
        corners(&mut e, 2, 18);
        run(&mut d, &mut e, "copy-rectangle-as-kill");
        assert_eq!(e.killed_rectangle, vec!["ng ", "   ", "ng "]);
    }

    #[test]
    fn string_rectangle_writes_down_each_line() {
        let (mut d, mut e) = setup("one\ntwo\nthree\n");
        corners(&mut e, 0, 8);
        d.execute(&mut e, "string-rectangle", None);
        assert!(e.minibuffer.is_active());
        for c in "- ".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.current_buffer().text(), "- one\n- two\n- three\n");
    }

    #[test]
    fn opening_and_clearing_and_numbering() {
        let (mut d, mut e) = setup("abcd\nefgh\n");
        corners(&mut e, 1, 8);
        run(&mut d, &mut e, "open-rectangle");
        assert_eq!(e.current_buffer().text(), "a  bcd\ne  fgh\n");

        let (mut d, mut e) = setup("abcd\nefgh\n");
        corners(&mut e, 1, 8);
        run(&mut d, &mut e, "clear-rectangle");
        assert_eq!(e.current_buffer().text(), "a  d\ne  h\n");

        let (mut d, mut e) = setup("a\nb\nc\n");
        corners(&mut e, 0, 4);
        run(&mut d, &mut e, "rectangle-number-lines");
        assert_eq!(e.current_buffer().text(), "1 a\n2 b\n3 c\n");
    }

    #[test]
    fn yanking_below_the_last_line_adds_lines_for_the_rows() {
        let (mut d, mut e) = setup("x");
        e.killed_rectangle = vec!["ab".into(), "cd".into()];
        e.move_point_to(1);
        run(&mut d, &mut e, "yank-rectangle");
        assert_eq!(e.current_buffer().text(), "xab\n cd");
        run(&mut d, &mut e, "undo");
        assert_eq!(e.current_buffer().text(), "x", "one undo takes it all back");
    }

    #[test]
    fn with_no_mark_there_is_no_rectangle() {
        let (mut d, mut e) = setup("abc");
        assert!(matches!(
            d.execute(&mut e, "kill-rectangle", None),
            Dispatch::Failed { .. }
        ));
    }
}
