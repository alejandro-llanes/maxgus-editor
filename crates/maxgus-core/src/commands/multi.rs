//! Making and unmaking extra cursors.
//!
//! `C->` puts one at the next occurrence of whatever is selected, `C-<` at the
//! previous, and `C-c C-<` at every one of them at once. From there, typing
//! types everywhere. `C-g` puts them away.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::multi::{all_occurrences, next_occurrence, previous_occurrence};
use crate::{CoreError, Result};

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "mark-next-like-this",
            "Put a cursor at the next occurrence of the selection.",
            mark_next
        ),
        command!(
            "mark-previous-like-this",
            "Put a cursor at the previous occurrence of the selection.",
            mark_previous
        ),
        command!(
            "mark-all-like-this",
            "Put a cursor at every occurrence of the selection.",
            mark_all
        ),
        command!(
            "cursor-at-next-line",
            "Put a cursor on the line below, in the same column.",
            cursor_below
        ),
        command!(
            "cursor-at-previous-line",
            "Put a cursor on the line above, in the same column.",
            cursor_above
        ),
        command!(
            "unmark-cursor",
            "Take away the last cursor that was added.",
            unmark
        ),
        command!("remove-all-cursors", "Go back to one cursor.", remove_all),
    ]);
}

/// What the extra cursors are matched against: the region, or the word point
/// is on when there is no region — which is what a rename usually starts as.
fn subject(editor: &mut Editor) -> Result<String> {
    if let Some(text) = editor.region_text().filter(|t| !t.is_empty()) {
        return Ok(text);
    }
    let word = editor
        .word_at_point()
        .filter(|w| !w.is_empty())
        .ok_or_else(|| CoreError::Message("Nothing here to match".into()))?;
    // The word becomes the region, so what the cursors are matching is
    // visible rather than guessed at.
    let point = editor.windows.current().point;
    let text = editor.current_buffer().text();
    if let Some(found) = previous_occurrence(&text, &word, point + 1)
        && found.start <= point
        && point <= found.end
    {
        editor.with_current_buffer(move |b| {
            b.set_mark(found.start);
            b.set_point(found.end);
        });
        editor.windows.current_mut().point = found.end;
    }
    Ok(word)
}

fn mark_next(editor: &mut Editor, _: &Args) -> Result<()> {
    mark(editor, true)
}

fn mark_previous(editor: &mut Editor, _: &Args) -> Result<()> {
    mark(editor, false)
}

fn mark(editor: &mut Editor, forward: bool) -> Result<()> {
    let wanted = subject(editor)?;
    let text = editor.current_buffer().text();
    let point = editor.windows.current().point;
    // From beyond the occurrence point is on, so `C->` twice reaches two
    // different ones rather than the same one twice.
    let mut from = match forward {
        true => point,
        false => point.saturating_sub(wanted.chars().count() + 1),
    };
    for _ in 0..editor.cursors.len() + 2 {
        let found = match forward {
            true => next_occurrence(&text, &wanted, from),
            false => previous_occurrence(&text, &wanted, from),
        }
        .ok_or_else(|| CoreError::Message(format!("No more matches for `{wanted}`")))?;
        if editor.cursors.add(found.end, point) {
            editor.message(format!("{} cursors", editor.cursors.len() + 1));
            return Ok(());
        }
        // That one is already taken: keep looking.
        from = match forward {
            true => found.start + 1,
            false => found.start.saturating_sub(1),
        };
    }
    Err(CoreError::Message(format!(
        "Every `{wanted}` already has a cursor"
    )))
}

fn mark_all(editor: &mut Editor, _: &Args) -> Result<()> {
    let wanted = subject(editor)?;
    let text = editor.current_buffer().text();
    let point = editor.windows.current().point;
    let found = all_occurrences(&text, &wanted);
    if found.is_empty() {
        return Err(CoreError::Message(format!("No matches for `{wanted}`")));
    }
    for occurrence in &found {
        editor.cursors.add(occurrence.end, point);
    }
    editor.message(format!("{} cursors", editor.cursors.len() + 1));
    Ok(())
}

/// `C-S-<down>`: a cursor on the next line, in the same column.
fn cursor_below(editor: &mut Editor, _: &Args) -> Result<()> {
    cursor_on_next_line(editor, 1)
}

fn cursor_above(editor: &mut Editor, _: &Args) -> Result<()> {
    cursor_on_next_line(editor, -1)
}

fn cursor_on_next_line(editor: &mut Editor, delta: isize) -> Result<()> {
    let point = editor.windows.current().point;
    let (offset, last) = {
        let buffer = editor.current_buffer();
        let line = buffer.line_of(point);
        let column = point - buffer.line_start(line);
        // From the furthest cursor in that direction, so pressing it twice
        // makes two cursors rather than fighting over one line.
        let furthest = editor
            .cursors
            .offsets()
            .iter()
            .copied()
            .chain(std::iter::once(point))
            .map(|offset| buffer.line_of(offset))
            .fold(line, |best, line| match delta > 0 {
                true => best.max(line),
                false => best.min(line),
            });
        let target = match delta > 0 {
            true => furthest + 1,
            false => match furthest {
                0 => return Err(CoreError::Message("No line above".into())),
                other => other - 1,
            },
        };
        if target >= buffer.len_lines() {
            return Err(CoreError::Message("No line below".into()));
        }
        let start = buffer.line_start(target);
        let end = maxgus_text::Motion::line_end(buffer.rope(), start);
        ((start + column).min(end), buffer.len_chars())
    };
    let _ = last;
    if !editor.cursors.add(offset, point) {
        return Err(CoreError::Message("There is already a cursor there".into()));
    }
    editor.message(format!("{} cursors", editor.cursors.len() + 1));
    Ok(())
}

fn unmark(editor: &mut Editor, _: &Args) -> Result<()> {
    let last = *editor
        .cursors
        .offsets()
        .last()
        .ok_or_else(|| CoreError::Message("There is only one cursor".into()))?;
    editor.cursors.remove(last);
    editor.message(format!("{} cursors", editor.cursors.len() + 1));
    Ok(())
}

fn remove_all(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.cursors.is_empty() {
        return Err(CoreError::Message("There is only one cursor".into()));
    }
    editor.cursors.clear();
    editor.message("One cursor".to_string());
    Ok(())
}
