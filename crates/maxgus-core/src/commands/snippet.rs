//! Expanding snippets, and moving through their fields.
//!
//! `TAB` after a snippet's key expands it and selects the first field. `TAB`
//! again moves to the next, `S-TAB` back, and `C-g` gives up on the whole
//! thing. Editing anywhere outside the snippet ends it too — a field that has
//! been typed past is not a field any more.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::minibuffer::MinibufferKind;
use crate::snippet::parse;
use crate::{CoreError, Result};

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "snippet-expand",
            "Expand the snippet named by the word before point.",
            expand
        ),
        command!(
            "insert-snippet",
            "Choose a snippet by name and insert it.",
            insert
        ),
        command!(
            "snippet-next-field",
            "Move to the next field of the snippet being filled in.",
            next_field,
            non_interactive
        ),
        command!(
            "snippet-previous-field",
            "Move back to the previous field.",
            previous_field,
            non_interactive
        ),
        command!(
            "snippet-abort",
            "Stop filling in the snippet, leaving the text as it is.",
            abort,
            non_interactive
        ),
    ]);
}

/// Expanding, for `TAB` to try before it indents.
pub fn expand_command(editor: &mut Editor, args: &Args) -> Result<()> {
    expand(editor, args)
}

/// Moving on, for `TAB` while a snippet is being filled in.
pub fn next_field_command(editor: &mut Editor, args: &Args) -> Result<()> {
    next_field(editor, args)
}

/// The word before point, which is what a key is typed as.
fn key_before_point(editor: &Editor) -> Option<(String, usize)> {
    let buffer = editor.current_buffer();
    let point = editor.windows.current().point;
    let text: Vec<char> = buffer.text().chars().collect();
    let mut start = point.min(text.len());
    while start > 0 {
        let c = text[start - 1];
        if c.is_alphanumeric() || c == '_' || c == '-' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == point {
        return None;
    }
    Some((text[start..point].iter().collect(), start))
}

/// `TAB`: expands the key before point, if it is one.
fn expand(editor: &mut Editor, _: &Args) -> Result<()> {
    let (key, start) = key_before_point(editor)
        .ok_or_else(|| CoreError::Message("No snippet key before point".into()))?;
    let mode = editor.current_mode_name();
    let snippet = editor
        .snippets
        .iter()
        .find(|s| s.key == key && s.mode.as_deref().is_none_or(|m| Some(m) == mode.as_deref()))
        .cloned()
        .ok_or_else(|| CoreError::Message(format!("No snippet for `{key}`")))?;
    let point = editor.windows.current().point;
    editor.with_current_buffer(move |b| {
        b.set_point(start);
        b.delete(maxgus_text::Range::new(start, point))
    })?;
    insert_expansion(editor, &snippet.body, start)
}

/// `M-x insert-snippet`: choose one by name.
fn insert(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let mode = editor.current_mode_name();
        let candidates: Vec<String> = editor
            .snippets
            .iter()
            .filter(|s| s.mode.as_deref().is_none_or(|m| Some(m) == mode.as_deref()))
            .map(|s| format!("{} ({})", s.name, s.key))
            .collect();
        if candidates.is_empty() {
            return Err(CoreError::Message("No snippets for this mode".into()));
        }
        editor.prompt_for(
            "insert-snippet",
            MinibufferKind::Choice,
            "Snippet: ".to_string(),
            "",
            candidates,
        );
        return Ok(());
    };
    // The prompt offers `name (key)`; either half identifies it.
    let wanted = name.split(" (").next().unwrap_or(&name).to_string();
    let body = editor
        .snippets
        .iter()
        .find(|s| s.name == wanted || s.key == wanted)
        .map(|s| s.body.clone())
        .ok_or_else(|| CoreError::Message(format!("No snippet called `{wanted}`")))?;
    let at = editor.windows.current().point;
    insert_expansion(editor, &body, at)
}

/// Puts the expanded text in and starts filling it in.
fn insert_expansion(editor: &mut Editor, body: &str, at: usize) -> Result<()> {
    let expansion = parse(body);
    let text = expansion.text.clone();
    editor.with_current_buffer(move |b| {
        b.set_point(at);
        b.insert_at_point(&text)
    })?;
    let fields: Vec<(usize, usize)> = expansion
        .fields
        .iter()
        .map(|field| (at + field.start, at + field.end))
        .collect();
    if fields.is_empty() {
        let end = at + expansion.end();
        editor.move_point_to(end);
        return Ok(());
    }
    editor.snippet_fields = fields;
    editor.snippet_field = 0;
    go_to_field(editor);
    Ok(())
}

/// Puts point on the current field, selecting its default so typing replaces
/// it — which is what a default is for.
fn go_to_field(editor: &mut Editor) {
    let Some((start, end)) = editor.snippet_fields.get(editor.snippet_field).copied() else {
        return;
    };
    editor.move_point_to(end);
    if start != end {
        editor.with_current_buffer(move |b| {
            b.set_mark(start);
            b.set_point(end);
        });
    } else {
        editor.with_current_buffer(|b| b.deactivate_mark());
    }
    let of = editor.snippet_fields.len();
    editor.message(format!("Field {} of {of}", editor.snippet_field + 1));
}

fn next_field(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.snippet_fields.is_empty() {
        return Err(CoreError::Message("No snippet is being filled in".into()));
    }
    if editor.snippet_field + 1 >= editor.snippet_fields.len() {
        editor.end_snippet();
        editor.message("Snippet finished".to_string());
        return Ok(());
    }
    editor.snippet_field += 1;
    go_to_field(editor);
    Ok(())
}

fn previous_field(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.snippet_fields.is_empty() {
        return Err(CoreError::Message("No snippet is being filled in".into()));
    }
    if editor.snippet_field == 0 {
        return Err(CoreError::Message("This is the first field".into()));
    }
    editor.snippet_field -= 1;
    go_to_field(editor);
    Ok(())
}

fn abort(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.snippet_fields.is_empty() {
        return Err(CoreError::Message("No snippet is being filled in".into()));
    }
    editor.end_snippet();
    editor.message("Snippet abandoned".to_string());
    Ok(())
}
