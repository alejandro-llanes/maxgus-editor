//! Editing commands: insertion, deletion, the kill ring, undo, case and
//! whitespace.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_text::{CharClass, Motion, Range};

/// Registers the editing commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "self-insert-command",
            "Insert the character that invoked this command.",
            self_insert,
            non_interactive
        ),
        command!("newline", "Insert a newline.", newline),
        command!(
            "electric-newline-and-maybe-indent",
            "Insert a newline and indent it.",
            newline_and_indent
        ),
        command!(
            "open-line",
            "Insert a newline after point, leaving point where it is.",
            open_line
        ),
        command!(
            "quoted-insert",
            "Insert the next character literally.",
            quoted_insert
        ),
        command!(
            "delete-char",
            "Delete the character after point.",
            delete_char
        ),
        command!(
            "delete-backward-char",
            "Delete the character before point.",
            delete_backward_char
        ),
        command!("kill-line", "Kill to the end of the line.", kill_line),
        command!(
            "kill-whole-line",
            "Kill the whole line including its newline.",
            kill_whole_line
        ),
        command!("kill-word", "Kill to the end of the next word.", kill_word),
        command!(
            "backward-kill-word",
            "Kill back to the start of the previous word.",
            backward_kill_word
        ),
        command!(
            "kill-region",
            "Kill the text between point and the mark.",
            kill_region
        ),
        command!(
            "kill-ring-save",
            "Copy the region to the kill ring.",
            kill_ring_save
        ),
        command!(
            "kill-sexp",
            "Kill the balanced expression after point.",
            kill_sexp
        ),
        command!(
            "kill-sentence",
            "Kill to the end of the sentence.",
            kill_sentence
        ),
        command!(
            "backward-kill-sentence",
            "Kill back to the start of the sentence.",
            backward_kill_sentence
        ),
        command!(
            "zap-to-char",
            "Kill up to and including a given character.",
            zap_to_char
        ),
        command!("yank", "Insert the most recent kill.", yank),
        command!(
            "yank-pop",
            "Replace the last yank with an earlier kill.",
            yank_pop
        ),
        command!("undo", "Undo the last change.", undo),
        command!("undo-redo", "Redo the last undone change.", undo_redo),
        command!(
            "transpose-chars",
            "Swap the characters around point.",
            transpose_chars
        ),
        command!(
            "transpose-words",
            "Swap the words around point.",
            transpose_words
        ),
        command!(
            "transpose-lines",
            "Swap the lines around point.",
            transpose_lines
        ),
        command!(
            "upcase-word",
            "Convert the next word to upper case.",
            upcase_word
        ),
        command!(
            "downcase-word",
            "Convert the next word to lower case.",
            downcase_word
        ),
        command!(
            "capitalize-word",
            "Capitalise the next word.",
            capitalize_word
        ),
        command!(
            "upcase-region",
            "Convert the region to upper case.",
            upcase_region
        ),
        command!(
            "downcase-region",
            "Convert the region to lower case.",
            downcase_region
        ),
        command!(
            "delete-horizontal-space",
            "Delete the whitespace around point.",
            delete_horizontal_space
        ),
        command!(
            "just-one-space",
            "Leave exactly one space around point.",
            just_one_space
        ),
        command!(
            "delete-indentation",
            "Join this line to the previous one.",
            delete_indentation
        ),
        command!(
            "delete-blank-lines",
            "Delete the blank lines around point.",
            delete_blank_lines
        ),
        command!(
            "duplicate-line-or-region",
            "Copy the region, or this line, and put the copy after it.",
            duplicate_line_or_region
        ),
        command!(
            "indent-for-tab-command",
            "Indent the line, or insert indentation.",
            indent_for_tab
        ),
        command!(
            "indent-rigidly",
            "Shift the region by the indentation width.",
            indent_rigidly
        ),
        command!(
            "indent-region",
            "Indent every line of the region.",
            indent_region
        ),
        command!(
            "split-line",
            "Split the line at point, indenting the rest to this column.",
            split_line
        ),
        command!(
            "tab-to-tab-stop",
            "Insert space up to the next tab stop.",
            tab_to_tab_stop
        ),
        command!(
            "keyboard-quit",
            "Abandon whatever is in progress.",
            keyboard_quit
        ),
    ]);
}

/// The string one indentation level is made of.
fn indent_unit(editor: &Editor) -> String {
    if editor.settings.indent_with_tabs {
        "\t".to_string()
    } else {
        " ".repeat(editor.settings.tab_width)
    }
}

// ---- insertion ----------------------------------------------------------

fn self_insert(editor: &mut Editor, args: &Args) -> Result<()> {
    // A snippet's field is selected so that typing takes its place.
    editor.take_snippet_field()?;
    let Some(c) = args.key.and_then(|k| k.as_char()) else {
        return Err(crate::CoreError::Message(
            "That key does not insert a character".into(),
        ));
    };
    let text: String = std::iter::repeat_n(c, args.count()).collect();
    editor.with_current_buffer(|b| b.insert_at_point(&text))?;
    editor.follow_point();
    Ok(())
}

fn newline(editor: &mut Editor, args: &Args) -> Result<()> {
    let text = "\n".repeat(args.count());
    editor.with_current_buffer(|b| b.insert_at_point(&text))?;
    editor.follow_point();
    Ok(())
}

/// `C-j`: a newline followed by the previous line's indentation.
fn newline_and_indent(editor: &mut Editor, args: &Args) -> Result<()> {
    for _ in 0..args.count() {
        let indentation = {
            let buffer = editor.current_buffer();
            let point = buffer.point();
            let start = Motion::line_start(buffer.rope(), point);
            let first = Motion::back_to_indentation(buffer.rope(), point);
            buffer.slice(Range::new(start, first.min(point)))
        };
        editor.with_current_buffer(|b| {
            b.transact(false, |b| {
                b.insert_at_point("\n")?;
                b.insert_at_point(&indentation)
            })
        })?;
    }
    editor.follow_point();
    Ok(())
}

/// `C-o`: opens a line after point without moving it.
fn open_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let text = "\n".repeat(args.count());
    editor.with_current_buffer(|b| {
        let at = b.point();
        b.insert(at, &text)?;
        // `insert` pushes point along when it lands at or before it; open-line
        // is defined to leave point before the newline.
        b.set_point(at);
        Ok::<(), maxgus_text::TextError>(())
    })?;
    editor.follow_point();
    Ok(())
}

/// `C-q`: inserts the next key literally, so a control character or a bound
/// key can be typed into the buffer.
fn quoted_insert(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(key) = args.read_char else {
        editor.read_char("quoted-insert", "C-q-");
        return Ok(());
    };
    // Any character the key names, control characters included.
    let c = match key.code {
        maxgus_keys::KeyCode::Char(c)
            if key.modifiers.contains(maxgus_keys::Modifiers::CONTROL) =>
        {
            // `C-a` is U+0001, and so on up the alphabet.
            let upper = c.to_ascii_uppercase();
            char::from(upper as u8 & 0x1f)
        }
        maxgus_keys::KeyCode::Char(c) => c,
        maxgus_keys::KeyCode::Enter => '\n',
        maxgus_keys::KeyCode::Tab => '\t',
        _ => return Err(crate::CoreError::Message("That key inserts nothing".into())),
    };
    let text: String = std::iter::repeat_n(c, args.count()).collect();
    editor.with_current_buffer(|b| b.insert_at_point(&text))?;
    editor.follow_point();
    Ok(())
}

// ---- deletion -----------------------------------------------------------

fn delete_char(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return delete_backward_char(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    editor.with_current_buffer(|b| {
        let from = b.point();
        let to = (from + n as usize).min(b.point_max());
        b.delete(Range::new(from, to))
    })?;
    editor.follow_point();
    Ok(())
}

fn delete_backward_char(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return delete_char(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    editor.with_current_buffer(|b| {
        let to = b.point();
        let from = to.saturating_sub(n as usize).max(b.point_min());
        b.delete(Range::new(from, to))
    })?;
    editor.follow_point();
    Ok(())
}

// ---- the kill ring ------------------------------------------------------

/// Kills `range`, adding it to the kill ring. `before` marks a backward kill,
/// which prepends when appending to an existing entry.
fn kill_range(editor: &mut Editor, range: Range, before: bool) -> Result<()> {
    if range.is_empty() {
        return Ok(());
    }
    let text = editor.with_current_buffer(|b| b.delete(range))?;
    editor.kill(&text, before);
    editor.follow_point();
    Ok(())
}

/// `C-k`: to the end of the line, or the newline itself when already there.
fn kill_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        if args.prefix.is_present() {
            // With an argument, kill that many whole lines.
            let line = buffer.line_of(point);
            let target = (line + args.count()).min(buffer.len_lines().saturating_sub(1));
            Range::new(point, buffer.line_start(target).max(point))
        } else {
            let end = Motion::line_end(buffer.rope(), point);
            if end > point {
                Range::new(point, end)
            } else {
                // At the end of the line, take the newline.
                Range::new(point, (point + 1).min(buffer.point_max()))
            }
        }
    };
    kill_range(editor, range, false)
}

fn kill_whole_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let line = buffer.line_of(point);
        let start = buffer.line_start(line);
        let end = buffer.line_start((line + args.count()).min(buffer.len_lines()));
        Range::new(start, end.max(start))
    };
    kill_range(editor, range, false)
}

fn kill_word(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return backward_kill_word(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        Range::new(
            point,
            Motion::forward_word(buffer.rope(), point, n as usize),
        )
    };
    kill_range(editor, range, false)
}

fn kill_sentence(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return backward_kill_sentence(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        Range::new(
            point,
            Motion::forward_sentence(buffer.rope(), point, n as usize),
        )
    };
    kill_range(editor, range, false)
}

fn backward_kill_sentence(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return kill_sentence(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        Range::new(
            Motion::backward_sentence(buffer.rope(), point, n as usize),
            point,
        )
    };
    // Killed backwards, so it joins the front of the last kill rather than the
    // back, the way `M-DEL` does.
    kill_range(editor, range, true)
}

fn backward_kill_word(editor: &mut Editor, args: &Args) -> Result<()> {
    let n = args.signed_count();
    if n < 0 {
        return kill_word(editor, &Args::new(crate::Prefix::Numeric(-n), args.key));
    }
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        Range::new(
            Motion::backward_word(buffer.rope(), point, n as usize),
            point,
        )
    };
    kill_range(editor, range, true)
}

fn kill_sexp(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let to = Motion::forward_sexp(buffer.rope(), point, args.count()).ok_or_else(|| {
            crate::CoreError::Message("Containing expression ends prematurely".into())
        })?;
        Range::new(point, to)
    };
    kill_range(editor, range, false)
}

fn kill_region(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = editor.region()?;
    kill_range(editor, range, false)?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

/// `M-w`: the region goes to the kill ring but stays in the buffer.
fn kill_ring_save(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = editor.region()?;
    let text = editor.current_buffer().slice(range);
    editor.kill(&text, false);
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message("Saved");
    Ok(())
}

/// `M-z`: kills through the next occurrence of a character.
fn zap_to_char(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(key) = args.read_char else {
        editor.read_char("zap-to-char", "Zap to char: ");
        return Ok(());
    };
    let Some(target) = key.as_char() else {
        return Err(crate::CoreError::Message(
            "That key names no character".into(),
        ));
    };
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let rope = buffer.rope();
        let mut at = point;
        // Each repetition zaps through one further occurrence.
        for _ in 0..args.count() {
            let mut found = None;
            let mut i = at;
            while i < buffer.point_max() {
                if rope.char(i) == target {
                    found = Some(i + 1);
                    break;
                }
                i += 1;
            }
            match found {
                Some(next) => at = next,
                None => {
                    return Err(crate::CoreError::Message(format!(
                        "Searching for character `{target}` failed"
                    )));
                }
            }
        }
        Range::new(point, at)
    };
    kill_range(editor, range, false)
}

fn yank(editor: &mut Editor, args: &Args) -> Result<()> {
    // A raw `C-u` rotates the ring before yanking, as `C-u C-y` does.
    if args.prefix.is_raw() {
        editor.kill_ring.rotate(1);
    }
    let Some(text) = editor.kill_ring.front().map(str::to_string) else {
        return Err(crate::CoreError::Message("Kill ring is empty".into()));
    };
    editor.with_current_buffer(|b| {
        let at = b.point();
        b.insert_at_point(&text)?;
        // The mark records where the yank began, so `C-x C-x` selects it.
        b.set_mark_inactive(at);
        Ok::<(), maxgus_text::TextError>(())
    })?;
    editor.follow_point();
    Ok(())
}

/// `M-y`: only meaningful straight after a yank.
fn yank_pop(editor: &mut Editor, _: &Args) -> Result<()> {
    if !matches!(
        editor.last_command.as_deref(),
        Some("yank") | Some("yank-pop")
    ) {
        return Err(crate::CoreError::Message(
            "Previous command was not a yank".into(),
        ));
    }
    let Some(previous) = editor.kill_ring.front().map(str::to_string) else {
        return Err(crate::CoreError::Message("Kill ring is empty".into()));
    };
    let Some(replacement) = editor.kill_ring.rotate(1).map(str::to_string) else {
        return Err(crate::CoreError::Message("Kill ring is empty".into()));
    };
    editor.with_current_buffer(|b| {
        let end = b.point();
        let start = end.saturating_sub(previous.chars().count());
        b.replace(Range::new(start, end), &replacement)?;
        b.set_mark_inactive(start);
        Ok::<(), maxgus_text::TextError>(())
    })?;
    editor.follow_point();
    Ok(())
}

// ---- undo ---------------------------------------------------------------

fn undo(editor: &mut Editor, args: &Args) -> Result<()> {
    let mut any = false;
    for _ in 0..args.count() {
        if !editor.with_current_buffer(|b| b.undo())? {
            break;
        }
        any = true;
    }
    editor.follow_point();
    if !any {
        return Err(crate::CoreError::Message(
            "No further undo information".into(),
        ));
    }
    editor.message("Undo");
    Ok(())
}

fn undo_redo(editor: &mut Editor, args: &Args) -> Result<()> {
    let mut any = false;
    for _ in 0..args.count() {
        if !editor.with_current_buffer(|b| b.redo())? {
            break;
        }
        any = true;
    }
    editor.follow_point();
    if !any {
        return Err(crate::CoreError::Message(
            "No further redo information".into(),
        ));
    }
    editor.message("Redo");
    Ok(())
}

// ---- transposition ------------------------------------------------------

/// Swaps the two spans `first` and `second`, which must not overlap, and
/// leaves point after the second.
fn swap(editor: &mut Editor, first: Range, second: Range) -> Result<()> {
    if first.is_empty() || second.is_empty() || first.end > second.start {
        return Err(crate::CoreError::Message("Nothing to transpose".into()));
    }
    let (a, b) = {
        let buffer = editor.current_buffer();
        (buffer.slice(first), buffer.slice(second))
    };
    editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            // Replace the later span first so the earlier offsets stay valid.
            buffer.replace(second, &a)?;
            buffer.replace(first, &b)?;
            Ok::<(), maxgus_text::TextError>(())
        })
    })?;
    let landing = second.end - first.len() + b.chars().count();
    editor.with_current_buffer(|buffer| buffer.set_point(landing.min(buffer.point_max())));
    editor.follow_point();
    Ok(())
}

fn transpose_chars(editor: &mut Editor, _: &Args) -> Result<()> {
    let (first, second) = {
        let buffer = editor.current_buffer();
        let mut point = buffer.point();
        // At the end of a line Emacs transposes the two characters before it.
        if point >= buffer.point_max() || buffer.char_after(point) == Some('\n') {
            point = point.saturating_sub(1);
        }
        if point == 0 {
            return Err(crate::CoreError::Message("Beginning of buffer".into()));
        }
        (Range::new(point - 1, point), Range::new(point, point + 1))
    };
    swap(editor, first, second)
}

fn transpose_words(editor: &mut Editor, _: &Args) -> Result<()> {
    let (first, second) = {
        let buffer = editor.current_buffer();
        let rope = buffer.rope();
        let point = buffer.point();
        // The word before point and the word after it.
        let second_end = Motion::forward_word(rope, point, 1);
        let second_start = Motion::backward_word(rope, second_end, 1);
        // The previous word ends where the separator before `second` begins.
        let mut first_end = second_start;
        while first_end > 0 && !CharClass::of(rope.char(first_end - 1)).is_word() {
            first_end -= 1;
        }
        let first_start = Motion::backward_word(rope, first_end, 1);
        if first_start >= second_start {
            return Err(crate::CoreError::Message("Nothing to transpose".into()));
        }
        (
            Range::new(first_start, first_end),
            Range::new(second_start, second_end),
        )
    };
    swap(editor, first, second)
}

fn transpose_lines(editor: &mut Editor, _: &Args) -> Result<()> {
    let (first, second) = {
        let buffer = editor.current_buffer();
        let line = buffer.line_of(buffer.point());
        if line == 0 {
            return Err(crate::CoreError::Message("Beginning of buffer".into()));
        }
        let previous = Range::new(buffer.line_start(line - 1), buffer.line_start(line));
        let end = buffer.line_start((line + 1).min(buffer.len_lines()));
        let current = Range::new(buffer.line_start(line), end.max(buffer.line_start(line)));
        (previous, current)
    };
    swap(editor, first, second)
}

// ---- case ---------------------------------------------------------------

/// Applies `f` to the next `n` words, leaving point after them.
fn case_words(editor: &mut Editor, n: usize, f: impl Fn(&str) -> String) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        Range::new(point, Motion::forward_word(buffer.rope(), point, n))
    };
    if range.is_empty() {
        return Ok(());
    }
    let replacement = f(&editor.current_buffer().slice(range));
    editor.with_current_buffer(|b| b.replace(range, &replacement))?;
    editor.with_current_buffer(|b| b.set_point(range.start + replacement.chars().count()));
    editor.follow_point();
    Ok(())
}

fn upcase_word(editor: &mut Editor, args: &Args) -> Result<()> {
    case_words(editor, args.count(), |s| s.to_uppercase())
}

fn downcase_word(editor: &mut Editor, args: &Args) -> Result<()> {
    case_words(editor, args.count(), |s| s.to_lowercase())
}

fn capitalize_word(editor: &mut Editor, args: &Args) -> Result<()> {
    case_words(editor, args.count(), capitalize)
}

/// Upper-cases the first letter of each word and lower-cases the rest, which
/// is what `capitalize-word` does.
fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut starting = true;
    for c in text.chars() {
        if CharClass::of(c).is_word() {
            if starting {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            starting = false;
        } else {
            out.push(c);
            starting = true;
        }
    }
    out
}

/// Applies `f` to the region, leaving point and the mark where they were.
fn case_region(editor: &mut Editor, f: impl Fn(&str) -> String) -> Result<()> {
    let range = editor.region()?;
    let replacement = f(&editor.current_buffer().slice(range));
    let point = editor.current_buffer().point();
    editor.with_current_buffer(|b| b.replace(range, &replacement))?;
    editor.with_current_buffer(|b| b.set_point(point.min(b.point_max())));
    Ok(())
}

fn upcase_region(editor: &mut Editor, _: &Args) -> Result<()> {
    case_region(editor, |s| s.to_uppercase())
}

fn downcase_region(editor: &mut Editor, _: &Args) -> Result<()> {
    case_region(editor, |s| s.to_lowercase())
}

// ---- whitespace ---------------------------------------------------------

/// The run of spaces and tabs around point.
fn horizontal_space(editor: &Editor) -> Range {
    let buffer = editor.current_buffer();
    let rope = buffer.rope();
    let point = buffer.point();
    let blank = |c: char| c == ' ' || c == '\t';

    let mut start = point;
    while start > buffer.point_min() && rope.char(start - 1).pipe_is(blank) {
        start -= 1;
    }
    let mut end = point;
    while end < buffer.point_max() && rope.char(end).pipe_is(blank) {
        end += 1;
    }
    Range::new(start, end)
}

/// A tiny helper so the whitespace scan reads as a predicate application.
trait Pipe {
    fn pipe_is(self, f: impl Fn(char) -> bool) -> bool;
}

impl Pipe for char {
    fn pipe_is(self, f: impl Fn(char) -> bool) -> bool {
        f(self)
    }
}

fn delete_horizontal_space(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = horizontal_space(editor);
    editor.with_current_buffer(|b| b.delete(range))?;
    editor.follow_point();
    Ok(())
}

fn just_one_space(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = horizontal_space(editor);
    let spaces = " ".repeat(
        args.prefix
            .positive_count()
            .max(if args.prefix.is_present() { 0 } else { 1 }),
    );
    editor.with_current_buffer(|b| b.replace(range, &spaces))?;
    editor.with_current_buffer(|b| b.set_point(range.start + spaces.chars().count()));
    editor.follow_point();
    Ok(())
}

/// `M-^`: joins this line to the previous one with a single space.
fn delete_indentation(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let line = buffer.line_of(buffer.point());
        if line == 0 {
            return Err(crate::CoreError::Message("Beginning of buffer".into()));
        }
        let previous_end = Motion::line_end(buffer.rope(), buffer.line_start(line - 1));
        let first = Motion::back_to_indentation(buffer.rope(), buffer.line_start(line));
        Range::new(previous_end, first)
    };
    // A join leaves one space unless the previous line ended empty.
    let separator = if range.start == 0
        || editor
            .current_buffer()
            .char_before(range.start)
            .is_some_and(char::is_whitespace)
    {
        ""
    } else {
        " "
    };
    editor.with_current_buffer(|b| b.replace(range, separator))?;
    editor.with_current_buffer(|b| b.set_point(range.start + separator.chars().count()));
    editor.follow_point();
    Ok(())
}

/// `C-x C-o`: collapses a run of blank lines to one, or deletes it entirely
/// when point is already on a lone blank line.
fn delete_blank_lines(editor: &mut Editor, _: &Args) -> Result<()> {
    let (range, replacement) = {
        let buffer = editor.current_buffer();
        let line = buffer.line_of(buffer.point());
        let blank = |n: usize| buffer.line_text(n).trim().is_empty();
        if !blank(line) {
            return Ok(());
        }
        let mut first = line;
        while first > 0 && blank(first - 1) {
            first -= 1;
        }
        let mut last = line;
        while last + 1 < buffer.len_lines() && blank(last + 1) {
            last += 1;
        }
        let start = buffer.line_start(first);
        let end = buffer.line_start((last + 1).min(buffer.len_lines()));
        // A single blank line goes away; a run collapses to one.
        let replacement = if first == last { "" } else { "\n" };
        (Range::new(start, end.max(start)), replacement)
    };
    editor.with_current_buffer(|b| b.replace(range, replacement))?;
    editor.follow_point();
    Ok(())
}

// ---- indentation --------------------------------------------------------

/// TAB: indents to the previous line's indentation, or inserts one level when
/// the line already matches it.
fn indent_for_tab(editor: &mut Editor, args: &Args) -> Result<()> {
    // A snippet being filled in owns `TAB`: moving to the next field is what
    // it is for, and indenting the line under a field would be nonsense.
    if editor.in_snippet() {
        return crate::commands::snippet::next_field_command(editor, args);
    }
    // Otherwise the word before point may be a snippet's key, and `TAB` is
    // how yasnippet expands one. Nothing happens when it is not: indenting
    // is what `TAB` does the rest of the time.
    if crate::commands::snippet::expand_command(editor, args).is_ok() {
        return Ok(());
    }
    let unit = indent_unit(editor);
    let (range, target, point_was) = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let line = buffer.line_of(point);
        let start = buffer.line_start(line);
        let first = Motion::back_to_indentation(buffer.rope(), start);
        let current = buffer.slice(Range::new(start, first));
        let previous = if line == 0 {
            String::new()
        } else {
            let above = buffer.line_start(line - 1);
            buffer.slice(Range::new(
                above,
                Motion::back_to_indentation(buffer.rope(), above),
            ))
        };
        // Matching the line above already? Then add a level.
        let target = if current == previous {
            format!("{current}{unit}")
        } else {
            previous
        };
        (Range::new(start, first), target, point)
    };
    let before = range.len();
    let after = target.chars().count();
    editor.with_current_buffer(|b| b.replace(range, &target))?;
    // Point keeps its position relative to the text, as Emacs' TAB does.
    let moved = point_was + after - before.min(point_was);
    editor.with_current_buffer(|b| b.set_point(moved.min(b.point_max())));
    editor.follow_point();
    Ok(())
}

/// `C-x TAB`: shifts every line of the region by one indentation level, or
/// back out again with a negative argument.
/// `C-M-\`: gives every line of the region the indentation of the line above
/// it, which is the rule TAB uses when a line does not already match.
///
/// Worked top down and re-reading each line as it goes, because indenting one
/// line changes what the next one is measured against.
fn indent_region(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = editor.region()?;
    let lines = {
        let buffer = editor.current_buffer();
        let first = buffer.line_of(range.start);
        let last = buffer.line_of(range.end.saturating_sub(1).max(range.start));
        first..=last
    };
    editor.with_current_buffer(|buffer| {
        // One undo group for the whole region, as `indent-rigidly` does.
        buffer.transact(false, |buffer| {
            // Forwards here, unlike `indent-rigidly`: each line is measured
            // against the one above it, which must already have been indented.
            for line in lines {
                if buffer.line_text(line).trim().is_empty() {
                    // Emacs leaves a blank line blank rather than filling it
                    // with whitespace nothing follows.
                    continue;
                }
                let start = buffer.line_start(line);
                let first_char = Motion::back_to_indentation(buffer.rope(), start);
                // The nearest non-blank line above, not simply the one above:
                // a blank line carries no indentation, and taking it at face
                // value would flatten everything after it.
                let above = (0..line)
                    .rev()
                    .find(|n| !buffer.line_text(*n).trim().is_empty())
                    .map(|n| {
                        let at = buffer.line_start(n);
                        buffer.slice(Range::new(
                            at,
                            Motion::back_to_indentation(buffer.rope(), at),
                        ))
                    })
                    .unwrap_or_default();
                buffer.replace(Range::new(start, first_char), &above)?;
            }
            Ok::<(), maxgus_text::TextError>(())
        })
    })?;
    editor.follow_point();
    Ok(())
}

/// `C-M-o`: pushes the rest of the line down, indented to where point is, and
/// leaves point exactly where it was.
fn split_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let column = editor
        .current_buffer()
        .display_column(editor.current_buffer().point());
    let text = format!("{}{}", "\n".repeat(args.count()), " ".repeat(column));
    editor.with_current_buffer(|b| {
        let at = b.point();
        b.insert(at, &text)?;
        // Same reason as `open-line`: the insertion carries point along, and
        // the point of this command is that it does not move.
        b.set_point(at);
        Ok::<(), maxgus_text::TextError>(())
    })?;
    editor.follow_point();
    Ok(())
}

/// `M-i`: space out to the next multiple of the tab width.
fn tab_to_tab_stop(editor: &mut Editor, _: &Args) -> Result<()> {
    let width = editor.settings.tab_width.max(1);
    let column = editor
        .current_buffer()
        .display_column(editor.current_buffer().point());
    // Always moves: sitting on a stop already means going to the next one.
    let spaces = width - (column % width);
    editor.with_current_buffer(|b| b.insert_at_point(&" ".repeat(spaces)))?;
    editor.follow_point();
    Ok(())
}

fn indent_rigidly(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = editor.region()?;
    let unit = indent_unit(editor);
    let width = unit.chars().count();
    let outdent = args.signed_count() < 0;

    let lines = {
        let buffer = editor.current_buffer();
        let first = buffer.line_of(range.start);
        let last = buffer.line_of(range.end.saturating_sub(1).max(range.start));
        first..=last
    };
    editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            // Work from the last line back, so earlier offsets stay valid.
            for line in lines.rev() {
                let start = buffer.line_start(line);
                if outdent {
                    let first = Motion::back_to_indentation(buffer.rope(), start);
                    let removable = (first - start).min(width);
                    buffer.delete(Range::new(start, start + removable))?;
                } else {
                    buffer.insert(start, &unit)?;
                }
            }
            Ok::<(), maxgus_text::TextError>(())
        })
    })?;
    Ok(())
}

// ---- quitting -----------------------------------------------------------

/// `C-g`: abandons the prompt, the region, the prefix argument and any queued
/// work, and says so.
fn keyboard_quit(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.minibuffer.is_active() {
        editor.abort_prompt();
        return Ok(());
    }
    editor.pending_char = None;
    editor.prefix = crate::Prefix::None;
    editor.key_menu = None;
    editor.tasks.clear();
    editor.with_current_buffer(|b| b.deactivate_mark());
    if editor.in_snippet() {
        editor.end_snippet();
        editor.message("Snippet abandoned");
        return Ok(());
    }
    // `C-g` is how you get back to one cursor, as it is in
    // `multiple-cursors`: the alternative is typing everywhere by accident.
    if !editor.cursors.is_empty() {
        editor.cursors.clear();
        editor.message("One cursor");
        return Ok(());
    }
    editor.message("Quit");
    Ok(())
}

/// `C-d`: the region again below itself, or this line again below itself.
///
/// One command for both because that is how it is reached: with something
/// selected it means the selection, and without it means the line, which is
/// what a person pressing it expects either way.
fn duplicate_line_or_region(editor: &mut Editor, args: &Args) -> Result<()> {
    let times = args.count().max(1);
    let (text, at) = {
        let buffer = editor.current_buffer();
        match buffer.region() {
            Some(region) if !region.is_empty() => (buffer.slice(region), region.end),
            _ => {
                let point = buffer.point();
                let line = buffer.line_of(point);
                let start = buffer.line_start(line);
                let end = Motion::line_end(buffer.rope(), start);
                (format!("\n{}", buffer.slice(Range::new(start, end))), end)
            }
        }
    };
    let copies = text.repeat(times);
    let length = copies.chars().count();
    editor.with_current_buffer(move |b| {
        b.set_point(at);
        b.insert_at_point(&copies)
    })?;
    // Point ends on the copy, which is what is about to be edited.
    editor.move_point_to(at + length);
    editor.follow_point();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, Prefix};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    /// An editor holding `text` with point at the start, plus a dispatcher
    /// with the motion and editing commands.
    fn setup(text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.create_with_text("test", text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));

        let mut registry = Registry::new();
        super::super::motion::register(&mut registry);
        register(&mut registry);
        (Dispatcher::new(registry), editor)
    }

    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    fn run_n(d: &mut Dispatcher, e: &mut Editor, command: &str, n: i32) {
        e.prefix = Prefix::Numeric(n);
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    fn fails(d: &mut Dispatcher, e: &mut Editor, command: &str) -> String {
        match d.execute(e, command, None) {
            Dispatch::Failed { message, .. } => message,
            other => panic!("`{command}` should have failed, got {other:?}"),
        }
    }

    fn text(e: &Editor) -> String {
        e.current_buffer().text()
    }

    fn point(e: &Editor) -> usize {
        e.windows.current().point
    }

    fn goto(e: &mut Editor, at: usize) {
        e.with_current_buffer(|b| b.set_point(at));
    }

    fn mark_region(e: &mut Editor, from: usize, to: usize) {
        e.with_current_buffer(|b| {
            b.set_point(from);
            b.set_mark(from);
            b.set_point(to);
        });
    }

    // ---- insertion ----

    #[test]
    fn self_insert_inserts_the_key_that_invoked_it() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "a");
        d.handle_keys(&mut e, "b");
        assert_eq!(text(&e), "ab");
        assert_eq!(point(&e), 2);
    }

    #[test]
    fn self_insert_repeats_with_a_prefix_argument() {
        let (mut d, mut e) = setup("");
        e.prefix = Prefix::Numeric(5);
        d.execute(
            &mut e,
            "self-insert-command",
            Some(maxgus_keys::Key::char('x')),
        );
        assert_eq!(text(&e), "xxxxx");
    }

    #[test]
    fn self_insert_refuses_a_key_that_names_no_character() {
        let (mut d, mut e) = setup("");
        let out = d.execute(
            &mut e,
            "self-insert-command",
            Some(maxgus_keys::Key::ctrl('x')),
        );
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn newline_inserts_and_repeats() {
        let (mut d, mut e) = setup("ab");
        goto(&mut e, 1);
        run(&mut d, &mut e, "newline");
        assert_eq!(text(&e), "a\nb");
        run_n(&mut d, &mut e, "newline", 2);
        assert_eq!(text(&e), "a\n\n\nb");
    }

    #[test]
    fn newline_and_indent_copies_the_current_indentation() {
        let (mut d, mut e) = setup("    indented");
        run(&mut d, &mut e, "move-end-of-line");
        run(&mut d, &mut e, "electric-newline-and-maybe-indent");
        assert_eq!(text(&e), "    indented\n    ");
        assert_eq!(point(&e), 17);
    }

    #[test]
    fn open_line_leaves_point_before_the_newline() {
        let (mut d, mut e) = setup("ab");
        goto(&mut e, 1);
        run(&mut d, &mut e, "open-line");
        assert_eq!(text(&e), "a\nb");
        assert_eq!(point(&e), 1, "point did not move");
    }

    #[test]
    fn quoted_insert_takes_the_next_key_literally() {
        let (mut d, mut e) = setup("");
        // The first invocation only arms the read.
        run(&mut d, &mut e, "quoted-insert");
        assert!(e.pending_char.is_some());
        assert_eq!(text(&e), "");

        d.handle_keys(&mut e, "C-a");
        assert_eq!(text(&e), "\u{1}", "the control character itself");
        assert!(e.pending_char.is_none());
    }

    #[test]
    fn quoted_insert_can_insert_a_tab_or_a_newline() {
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "quoted-insert");
        d.handle_keys(&mut e, "TAB");
        assert_eq!(text(&e), "\t");
        run(&mut d, &mut e, "quoted-insert");
        d.handle_keys(&mut e, "RET");
        assert_eq!(text(&e), "\t\n");
    }

    #[test]
    fn a_read_char_command_keeps_its_prefix_argument() {
        let (mut d, mut e) = setup("");
        e.prefix = Prefix::Numeric(3);
        d.execute(&mut e, "quoted-insert", None);
        d.handle_keys(&mut e, "x");
        assert_eq!(text(&e), "xxx");
    }

    // ---- deletion ----

    #[test]
    fn delete_char_removes_forward_and_clamps() {
        let (mut d, mut e) = setup("abc");
        run(&mut d, &mut e, "delete-char");
        assert_eq!(text(&e), "bc");
        run_n(&mut d, &mut e, "delete-char", 99);
        assert_eq!(text(&e), "");
    }

    #[test]
    fn delete_backward_removes_behind_point() {
        let (mut d, mut e) = setup("abc");
        goto(&mut e, 3);
        run(&mut d, &mut e, "delete-backward-char");
        assert_eq!(text(&e), "ab");
        assert_eq!(point(&e), 2);
    }

    #[test]
    fn a_negative_argument_reverses_deletion() {
        let (mut d, mut e) = setup("abcdef");
        goto(&mut e, 3);
        run_n(&mut d, &mut e, "delete-char", -2);
        assert_eq!(text(&e), "adef", "deleted backwards");
        run_n(&mut d, &mut e, "delete-backward-char", -1);
        assert_eq!(text(&e), "aef", "deleted forwards");
    }

    #[test]
    fn deleting_at_the_buffer_edge_does_nothing() {
        let (mut d, mut e) = setup("a");
        run(&mut d, &mut e, "delete-backward-char");
        assert_eq!(text(&e), "a");
        goto(&mut e, 1);
        run(&mut d, &mut e, "delete-char");
        assert_eq!(text(&e), "a");
    }

    // ---- the kill ring ----

    #[test]
    fn kill_line_takes_the_rest_of_the_line_then_the_newline() {
        let (mut d, mut e) = setup("hello world\nsecond");
        goto(&mut e, 5);
        run(&mut d, &mut e, "kill-line");
        assert_eq!(text(&e), "hello\nsecond");
        assert_eq!(e.kill_ring.front(), Some(" world"));

        run(&mut d, &mut e, "kill-line");
        assert_eq!(text(&e), "hellosecond", "the second kill took the newline");
    }

    #[test]
    fn consecutive_kills_collect_into_one_entry() {
        let (mut d, mut e) = setup("one\ntwo\nthree");
        d.handle_keys(&mut e, "C-k");
        d.handle_keys(&mut e, "C-k");
        d.handle_keys(&mut e, "C-k");
        assert_eq!(e.kill_ring.len(), 1);
        assert_eq!(e.kill_ring.front(), Some("one\ntwo"));
    }

    #[test]
    fn kill_line_with_an_argument_takes_whole_lines() {
        let (mut d, mut e) = setup("one\ntwo\nthree\nfour");
        run_n(&mut d, &mut e, "kill-line", 2);
        assert_eq!(text(&e), "three\nfour");
    }

    #[test]
    fn kill_whole_line_takes_the_line_and_its_newline() {
        let (mut d, mut e) = setup("one\ntwo\nthree");
        goto(&mut e, 5);
        run(&mut d, &mut e, "kill-whole-line");
        assert_eq!(text(&e), "one\nthree");
        assert_eq!(e.kill_ring.front(), Some("two\n"));
    }

    #[test]
    fn word_kills_work_in_both_directions() {
        let (mut d, mut e) = setup("alpha beta gamma");
        run(&mut d, &mut e, "kill-word");
        assert_eq!(text(&e), " beta gamma");
        assert_eq!(e.kill_ring.front(), Some("alpha"));

        run(&mut d, &mut e, "move-end-of-line");
        run(&mut d, &mut e, "backward-kill-word");
        assert_eq!(text(&e), " beta ");
    }

    #[test]
    fn a_backward_kill_prepends_when_appending_to_the_run() {
        let (mut d, mut e) = setup("alpha beta");
        goto(&mut e, 10);
        d.handle_keys(&mut e, "M-DEL");
        d.handle_keys(&mut e, "M-DEL");
        assert_eq!(e.kill_ring.len(), 1);
        assert_eq!(e.kill_ring.front(), Some("alpha beta"), "in reading order");
    }

    #[test]
    fn kill_region_needs_a_region_and_says_so() {
        let (mut d, mut e) = setup("hello world");
        let message = fails(&mut d, &mut e, "kill-region");
        assert!(message.contains("mark is not set"), "got `{message}`");
    }

    #[test]
    fn kill_region_removes_the_region_and_deactivates_the_mark() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 6);
        run(&mut d, &mut e, "kill-region");
        assert_eq!(text(&e), "world");
        assert_eq!(e.kill_ring.front(), Some("hello "));
        assert!(e.region().is_err(), "the mark is no longer active");
    }

    #[test]
    fn kill_ring_save_copies_without_removing() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 5);
        run(&mut d, &mut e, "kill-ring-save");
        assert_eq!(text(&e), "hello world");
        assert_eq!(e.kill_ring.front(), Some("hello"));
        assert_eq!(e.minibuffer.display(), "Saved");
    }

    #[test]
    fn kill_sexp_takes_a_balanced_expression() {
        let (mut d, mut e) = setup("(a (b c)) rest");
        run(&mut d, &mut e, "kill-sexp");
        assert_eq!(text(&e), " rest");
        assert_eq!(e.kill_ring.front(), Some("(a (b c))"));
    }

    #[test]
    fn kill_sexp_reports_unbalanced_delimiters() {
        let (mut d, mut e) = setup("(a b");
        fails(&mut d, &mut e, "kill-sexp");
        assert_eq!(text(&e), "(a b", "nothing was removed");
    }

    #[test]
    fn zap_to_char_kills_through_the_character() {
        let (mut d, mut e) = setup("hello world");
        run(&mut d, &mut e, "zap-to-char");
        assert!(e.pending_char.is_some());
        d.handle_keys(&mut e, "o");
        assert_eq!(text(&e), " world");
        assert_eq!(e.kill_ring.front(), Some("hello"));
    }

    #[test]
    fn zap_to_char_repeats_with_an_argument() {
        let (mut d, mut e) = setup("hello world");
        e.prefix = Prefix::Numeric(2);
        d.execute(&mut e, "zap-to-char", None);
        d.handle_keys(&mut e, "o");
        assert_eq!(text(&e), "rld", "through the second `o`");
    }

    #[test]
    fn zap_to_a_missing_character_fails_without_editing() {
        let (mut d, mut e) = setup("hello");
        d.execute(&mut e, "zap-to-char", None);
        let out = d.handle_keys(&mut e, "z");
        assert!(matches!(out, Dispatch::Failed { .. }));
        assert_eq!(text(&e), "hello");
    }

    // ---- yanking ----

    #[test]
    fn yank_inserts_the_most_recent_kill_and_marks_it() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 6);
        run(&mut d, &mut e, "kill-region");
        goto(&mut e, 5);
        run(&mut d, &mut e, "yank");
        assert_eq!(text(&e), "worldhello ");
        assert_eq!(
            e.current_buffer().mark(),
            Some(5),
            "the yank start was marked"
        );
    }

    #[test]
    fn yanking_an_empty_ring_is_an_error() {
        let (mut d, mut e) = setup("text");
        assert!(fails(&mut d, &mut e, "yank").contains("empty"));
    }

    #[test]
    fn yank_pop_replaces_the_last_yank_with_an_earlier_kill() {
        let (mut d, mut e) = setup("");
        e.kill_ring.kill_new("first");
        e.kill_ring.kill_new("second");

        d.execute(&mut e, "yank", None);
        assert_eq!(text(&e), "second");
        d.execute(&mut e, "yank-pop", None);
        assert_eq!(text(&e), "first");
        d.execute(&mut e, "yank-pop", None);
        assert_eq!(text(&e), "second", "the ring wrapped");
    }

    #[test]
    fn yank_pop_only_follows_a_yank() {
        let (mut d, mut e) = setup("text");
        e.kill_ring.kill_new("something");
        let message = fails(&mut d, &mut e, "yank-pop");
        assert!(message.contains("not a yank"), "got `{message}`");
    }

    #[test]
    fn a_raw_prefix_rotates_the_ring_before_yanking() {
        let (mut d, mut e) = setup("");
        e.kill_ring.kill_new("older");
        e.kill_ring.kill_new("newer");
        e.prefix = Prefix::Universal(1);
        d.execute(&mut e, "yank", None);
        assert_eq!(text(&e), "older");
    }

    // ---- undo ----

    #[test]
    fn undo_reverses_edits_and_redo_reapplies_them() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "a");
        d.handle_keys(&mut e, "b");
        assert_eq!(text(&e), "ab");

        run(&mut d, &mut e, "undo");
        assert_eq!(text(&e), "", "the typed run undid together");
        assert_eq!(e.minibuffer.display(), "Undo");

        run(&mut d, &mut e, "undo-redo");
        assert_eq!(text(&e), "ab");
    }

    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let (mut d, mut e) = setup("untouched");
        assert!(fails(&mut d, &mut e, "undo").contains("No further undo"));
        assert!(fails(&mut d, &mut e, "undo-redo").contains("No further redo"));
    }

    #[test]
    fn undo_repeats_with_an_argument() {
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "newline");
        run(&mut d, &mut e, "newline");
        run(&mut d, &mut e, "newline");
        assert_eq!(text(&e), "\n\n\n");
        run_n(&mut d, &mut e, "undo", 2);
        assert_eq!(text(&e), "\n");
    }

    // ---- transposition ----

    #[test]
    fn transpose_chars_swaps_the_pair_around_point() {
        let (mut d, mut e) = setup("abcd");
        goto(&mut e, 2);
        run(&mut d, &mut e, "transpose-chars");
        assert_eq!(text(&e), "acbd");
        assert_eq!(point(&e), 3, "point ends after the pair");
    }

    #[test]
    fn transpose_chars_at_the_end_of_a_line_uses_the_two_before_it() {
        let (mut d, mut e) = setup("ab\ncd");
        goto(&mut e, 2);
        run(&mut d, &mut e, "transpose-chars");
        assert_eq!(text(&e), "ba\ncd");
    }

    #[test]
    fn transpose_chars_at_the_start_of_the_buffer_is_an_error() {
        let (mut d, mut e) = setup("ab");
        assert!(fails(&mut d, &mut e, "transpose-chars").contains("Beginning"));
    }

    #[test]
    fn transpose_words_swaps_the_words_around_point() {
        let (mut d, mut e) = setup("alpha beta");
        goto(&mut e, 5);
        run(&mut d, &mut e, "transpose-words");
        assert_eq!(text(&e), "beta alpha");
    }

    #[test]
    fn transpose_lines_swaps_with_the_line_above() {
        let (mut d, mut e) = setup("one\ntwo\nthree\n");
        goto(&mut e, 4);
        run(&mut d, &mut e, "transpose-lines");
        assert_eq!(text(&e), "two\none\nthree\n");
    }

    #[test]
    fn transpose_lines_on_the_first_line_is_an_error() {
        let (mut d, mut e) = setup("one\ntwo\n");
        assert!(fails(&mut d, &mut e, "transpose-lines").contains("Beginning"));
    }

    // ---- case ----

    #[test]
    fn word_case_commands_convert_and_advance() {
        let (mut d, mut e) = setup("hello world");
        run(&mut d, &mut e, "upcase-word");
        assert_eq!(text(&e), "HELLO world");
        assert_eq!(point(&e), 5);
        run(&mut d, &mut e, "upcase-word");
        assert_eq!(text(&e), "HELLO WORLD");
    }

    #[test]
    fn downcase_and_capitalize_behave_as_expected() {
        let (mut d, mut e) = setup("HELLO WORLD");
        run(&mut d, &mut e, "downcase-word");
        assert_eq!(text(&e), "hello WORLD");
        goto(&mut e, 0);
        run_n(&mut d, &mut e, "capitalize-word", 2);
        assert_eq!(text(&e), "Hello World");
    }

    #[test]
    fn capitalisation_restarts_at_each_word() {
        assert_eq!(capitalize("hello wORLD-again"), "Hello World-Again");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("  spaced  "), "  Spaced  ");
    }

    #[test]
    fn region_case_commands_leave_point_alone() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 5);
        let before = point(&e);
        run(&mut d, &mut e, "upcase-region");
        assert_eq!(text(&e), "HELLO world");
        assert_eq!(point(&e), before);
        run(&mut d, &mut e, "downcase-region");
        assert_eq!(text(&e), "hello world");
    }

    #[test]
    fn region_case_commands_need_a_region() {
        let (mut d, mut e) = setup("text");
        assert!(fails(&mut d, &mut e, "upcase-region").contains("mark"));
    }

    // ---- whitespace ----

    #[test]
    fn delete_horizontal_space_removes_the_run_around_point() {
        let (mut d, mut e) = setup("a   \t  b");
        goto(&mut e, 3);
        run(&mut d, &mut e, "delete-horizontal-space");
        assert_eq!(text(&e), "ab");
    }

    #[test]
    fn just_one_space_collapses_the_run() {
        let (mut d, mut e) = setup("a   \t  b");
        goto(&mut e, 3);
        run(&mut d, &mut e, "just-one-space");
        assert_eq!(text(&e), "a b");
        assert_eq!(point(&e), 2);
    }

    #[test]
    fn just_one_space_with_an_argument_leaves_that_many() {
        let (mut d, mut e) = setup("a   b");
        goto(&mut e, 2);
        run_n(&mut d, &mut e, "just-one-space", 3);
        assert_eq!(text(&e), "a   b");
        goto(&mut e, 2);
        run_n(&mut d, &mut e, "just-one-space", 0);
        assert_eq!(text(&e), "ab");
    }

    #[test]
    fn delete_indentation_joins_with_one_space() {
        let (mut d, mut e) = setup("first line\n    second line");
        goto(&mut e, 15);
        run(&mut d, &mut e, "delete-indentation");
        assert_eq!(text(&e), "first line second line");
    }

    #[test]
    fn delete_indentation_on_the_first_line_is_an_error() {
        let (mut d, mut e) = setup("only line");
        assert!(fails(&mut d, &mut e, "delete-indentation").contains("Beginning"));
    }

    #[test]
    fn delete_blank_lines_removes_a_lone_blank_line() {
        let (mut d, mut e) = setup("one\n\ntwo\n");
        goto(&mut e, 4);
        run(&mut d, &mut e, "delete-blank-lines");
        assert_eq!(text(&e), "one\ntwo\n");
    }

    #[test]
    fn delete_blank_lines_collapses_a_run_to_one() {
        let (mut d, mut e) = setup("one\n\n\n\ntwo\n");
        goto(&mut e, 5);
        run(&mut d, &mut e, "delete-blank-lines");
        assert_eq!(text(&e), "one\n\ntwo\n");
    }

    #[test]
    fn delete_blank_lines_on_a_non_blank_line_does_nothing() {
        let (mut d, mut e) = setup("one\ntwo\n");
        run(&mut d, &mut e, "delete-blank-lines");
        assert_eq!(text(&e), "one\ntwo\n");
    }

    // ---- indentation ----

    #[test]
    fn tab_indents_to_match_the_line_above() {
        let (mut d, mut e) = setup("    first\nsecond");
        goto(&mut e, 12);
        run(&mut d, &mut e, "indent-for-tab-command");
        assert_eq!(text(&e), "    first\n    second");
    }

    #[test]
    fn tab_on_an_already_matching_line_adds_a_level() {
        let (mut d, mut e) = setup("    first\n    second");
        goto(&mut e, 16);
        run(&mut d, &mut e, "indent-for-tab-command");
        assert_eq!(text(&e), "    first\n        second");
    }

    #[test]
    fn kill_sentence_takes_to_the_end_of_the_sentence() {
        let (mut d, mut e) = setup("One thing. Two thing.\n");
        run(&mut d, &mut e, "kill-sentence");
        // The separating space stays behind: `forward-sentence` stops on the
        // terminator, and Emacs leaves the gap for the next sentence.
        assert_eq!(text(&e), " Two thing.\n");
        assert_eq!(e.kill_ring.front(), Some("One thing."));
    }

    #[test]
    fn backward_kill_sentence_takes_back_to_the_start() {
        let (mut d, mut e) = setup("One thing. Two thing.\n");
        e.with_current_buffer(|b| b.set_point(21));
        run(&mut d, &mut e, "backward-kill-sentence");
        assert_eq!(text(&e), "One thing. \n");
    }

    #[test]
    fn backward_kill_sentence_inside_leading_whitespace_kills_nothing_and_does_not_panic() {
        // `backward-sentence` used to run *forward* out of leading whitespace,
        // which made a backwards range here and panicked. Found by the binding
        // sweep, not by a test written for it.
        let (mut d, mut e) = setup("   hello there.\n");
        e.with_current_buffer(|b| b.set_point(1));
        run(&mut d, &mut e, "backward-kill-sentence");
        assert_eq!(
            text(&e),
            "  hello there.\n",
            "back to the buffer start, one space"
        );
    }

    #[test]
    fn a_negative_argument_turns_each_sentence_kill_into_the_other() {
        let (mut d, mut e) = setup("One thing. Two thing.\n");
        e.with_current_buffer(|b| b.set_point(21));
        run_n(&mut d, &mut e, "kill-sentence", -1);
        assert_eq!(text(&e), "One thing. \n");
    }

    #[test]
    fn indent_region_gives_each_line_the_indentation_above_it() {
        // Lines 1 and 2 only: each takes the indentation of the line above,
        // so the ragged ones line up under the first.
        let (mut d, mut e) = setup("    base\n  ragged\n        wild\n");
        mark_region(&mut e, 9, 26);
        run(&mut d, &mut e, "indent-region");
        assert_eq!(text(&e), "    base\n    ragged\n    wild\n");
    }

    #[test]
    fn indent_region_leaves_blank_lines_empty() {
        // Two things at once: the blank line keeps no whitespace, and the
        // line after it measures against the last line that had any rather
        // than against the blank one, which would have flattened it.
        let (mut d, mut e) = setup("    base\n  ragged\n\n  after\n");
        mark_region(&mut e, 9, 27);
        run(&mut d, &mut e, "indent-region");
        assert_eq!(text(&e), "    base\n    ragged\n\n    after\n");
    }

    #[test]
    fn indent_region_undoes_in_one_step() {
        let (mut d, mut e) = setup("    base\n  ragged\n        wild\n");
        let before = text(&e);
        mark_region(&mut e, 9, 26);
        run(&mut d, &mut e, "indent-region");
        assert_ne!(text(&e), before);
        run(&mut d, &mut e, "undo");
        assert_eq!(text(&e), before, "the whole region is one undo group");
    }

    #[test]
    fn split_line_pushes_the_rest_down_and_leaves_point_alone() {
        let (mut d, mut e) = setup("abcdef\n");
        e.with_current_buffer(|b| b.set_point(3));
        run(&mut d, &mut e, "split-line");
        assert_eq!(
            text(&e),
            "abc\n   def\n",
            "indented to the column point was in"
        );
        assert_eq!(e.current_buffer().point(), 3, "point did not move");
    }

    #[test]
    fn tab_to_tab_stop_spaces_out_to_the_next_stop() {
        let (mut d, mut e) = setup("ab\n");
        e.with_current_buffer(|b| b.set_point(2));
        run(&mut d, &mut e, "tab-to-tab-stop");
        assert_eq!(text(&e), "ab  \n", "column 2 to the stop at 4");
    }

    #[test]
    fn tab_to_tab_stop_on_a_stop_moves_to_the_next_one() {
        // Emacs always moves; landing on a stop is not a reason to do nothing.
        let (mut d, mut e) = setup("\n");
        run(&mut d, &mut e, "tab-to-tab-stop");
        assert_eq!(text(&e), "    \n");
    }

    #[test]
    fn indent_rigidly_shifts_every_line_of_the_region() {
        let (mut d, mut e) = setup("one\ntwo\nthree\n");
        mark_region(&mut e, 0, 8);
        run(&mut d, &mut e, "indent-rigidly");
        assert_eq!(text(&e), "    one\n    two\nthree\n");
    }

    #[test]
    fn indent_rigidly_with_a_negative_argument_outdents() {
        let (mut d, mut e) = setup("        one\n        two\n");
        mark_region(&mut e, 0, 13);
        run_n(&mut d, &mut e, "indent-rigidly", -1);
        assert_eq!(text(&e), "    one\n    two\n");
    }

    #[test]
    fn indentation_honours_the_tabs_setting() {
        let (mut d, mut e) = setup("one\n");
        e.settings.indent_with_tabs = true;
        mark_region(&mut e, 0, 3);
        run(&mut d, &mut e, "indent-rigidly");
        assert_eq!(text(&e), "\tone\n");
    }

    // ---- quitting ----

    #[test]
    fn keyboard_quit_abandons_everything_in_progress() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 5);
        e.prefix = Prefix::Numeric(9);
        e.spawn(crate::Task::Tree(crate::TreeAction::Refresh));
        e.read_char("zap-to-char", "Zap: ");

        run(&mut d, &mut e, "keyboard-quit");
        assert_eq!(e.minibuffer.display(), "Quit");
        assert!(e.region().is_err(), "the region was deactivated");
        assert_eq!(e.prefix, Prefix::None);
        assert!(e.tasks.is_empty());
        assert!(e.pending_char.is_none());
    }

    #[test]
    fn keyboard_quit_closes_an_open_prompt() {
        let (mut d, mut e) = setup("text");
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        run(&mut d, &mut e, "keyboard-quit");
        assert!(!e.minibuffer.is_active());
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));
    }

    // ---- read-only ----

    #[test]
    fn a_read_only_buffer_refuses_every_edit() {
        let (mut d, mut e) = setup("protected");
        e.with_current_buffer(|b| b.set_read_only(true));
        for command in ["newline", "delete-char", "kill-line", "upcase-word"] {
            let out = d.execute(&mut e, command, None);
            assert!(
                matches!(out, Dispatch::Failed { .. }),
                "`{command}` should have been refused"
            );
        }
        assert_eq!(text(&e), "protected");
    }
}
