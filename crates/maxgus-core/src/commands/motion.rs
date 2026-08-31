//! Motion commands.
//!
//! Every one of these honours the prefix argument, and the ones that come in
//! pairs treat a negative argument as an invocation of their opposite — `C-u -
//! C-f` moves backward, as it does in Emacs.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_text::Motion;

/// Registers the motion commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "forward-char",
            "Move point forward one character.",
            forward_char
        ),
        command!(
            "backward-char",
            "Move point backward one character.",
            backward_char
        ),
        command!(
            "next-line",
            "Move point down one line, keeping the goal column.",
            next_line
        ),
        command!(
            "previous-line",
            "Move point up one line, keeping the goal column.",
            previous_line
        ),
        command!(
            "move-beginning-of-line",
            "Move point to the beginning of the line.",
            beginning_of_line
        ),
        command!(
            "move-end-of-line",
            "Move point to the end of the line.",
            end_of_line
        ),
        command!(
            "back-to-indentation",
            "Move point to the first non-blank character.",
            back_to_indentation
        ),
        command!("forward-word", "Move point forward one word.", forward_word),
        command!(
            "backward-word",
            "Move point backward one word.",
            backward_word
        ),
        command!(
            "forward-sentence",
            "Move point forward one sentence.",
            forward_sentence
        ),
        command!(
            "backward-sentence",
            "Move point backward one sentence.",
            backward_sentence
        ),
        command!(
            "forward-paragraph",
            "Move point forward one paragraph.",
            forward_paragraph
        ),
        command!(
            "backward-paragraph",
            "Move point backward one paragraph.",
            backward_paragraph
        ),
        command!(
            "forward-sexp",
            "Move point forward one balanced expression.",
            forward_sexp
        ),
        command!(
            "backward-sexp",
            "Move point backward one balanced expression.",
            backward_sexp
        ),
        command!(
            "beginning-of-defun",
            "Move point to the start of the enclosing definition.",
            beginning_of_defun
        ),
        command!(
            "end-of-defun",
            "Move point past the end of the enclosing definition.",
            end_of_defun
        ),
        command!(
            "beginning-of-buffer",
            "Move point to the beginning of the buffer.",
            beginning_of_buffer
        ),
        command!(
            "end-of-buffer",
            "Move point to the end of the buffer.",
            end_of_buffer
        ),
        command!(
            "goto-char",
            "Move point to a given character offset.",
            goto_char
        ),
        command!(
            "goto-line",
            "Move point to the beginning of a given line.",
            goto_line
        ),
        command!(
            "scroll-up-command",
            "Scroll the window down one screenful.",
            scroll_up
        ),
        command!(
            "scroll-down-command",
            "Scroll the window up one screenful.",
            scroll_down
        ),
        command!(
            "recenter-top-bottom",
            "Centre point, then cycle to the top and bottom.",
            recenter
        ),
        command!(
            "set-mark-command",
            "Set the mark at point, or pop the mark ring.",
            set_mark
        ),
        command!(
            "exchange-point-and-mark",
            "Swap point and the mark.",
            exchange_point_and_mark
        ),
        command!(
            "mark-whole-buffer",
            "Put point at the beginning and the mark at the end.",
            mark_whole_buffer
        ),
        command!(
            "mark-word",
            "Extend the region over the next word.",
            mark_word
        ),
        command!(
            "mark-sexp",
            "Extend the region over the next balanced expression.",
            mark_sexp
        ),
        command!(
            "mark-paragraph",
            "Put point before this paragraph and the mark after it.",
            mark_paragraph
        ),
        command!(
            "mark-defun",
            "Put point before this definition and the mark after it.",
            mark_defun
        ),
        command!(
            "count-words",
            "Report the lines, words and characters in the region.",
            count_words
        ),
        command!(
            "pop-mark",
            "Move point to the previous mark-ring entry.",
            pop_mark
        ),
        command!(
            "what-cursor-position",
            "Describe the character and position under point.",
            what_cursor_position
        ),
    ]);
}

/// Runs `forward` or `backward` depending on the sign of the argument, so a
/// negative prefix reverses any paired motion.
fn directional(
    editor: &mut Editor,
    args: &Args,
    forward: impl FnOnce(&mut Editor, usize) -> Result<()>,
    backward: impl FnOnce(&mut Editor, usize) -> Result<()>,
) -> Result<()> {
    let count = args.signed_count();
    if count < 0 {
        backward(editor, count.unsigned_abs() as usize)
    } else {
        forward(editor, count as usize)
    }
}

/// Moves point with `f`, then scrolls so it is visible.
fn move_point(editor: &mut Editor, f: impl FnOnce(&maxgus_text::Buffer) -> usize) {
    editor.with_current_buffer(|buffer| {
        let to = f(buffer);
        buffer.set_point(to);
    });
    editor.follow_point();
}

// ---- characters and lines ----------------------------------------------

fn forward_char(editor: &mut Editor, args: &Args) -> Result<()> {
    directional(
        editor,
        args,
        |e, n| {
            move_point(e, |b| Motion::forward_char(b.rope(), b.point(), n));
            Ok(())
        },
        |e, n| {
            move_point(e, |b| Motion::backward_char(b.rope(), b.point(), n));
            Ok(())
        },
    )
}

fn backward_char(editor: &mut Editor, args: &Args) -> Result<()> {
    directional(
        editor,
        args,
        |e, n| {
            move_point(e, |b| Motion::backward_char(b.rope(), b.point(), n));
            Ok(())
        },
        |e, n| {
            move_point(e, |b| Motion::forward_char(b.rope(), b.point(), n));
            Ok(())
        },
    )
}

/// Moves `delta` lines, keeping the display column point started at. The goal
/// column persists across consecutive line motions, so walking down through a
/// short line and out the other side returns to the original column.
fn line_motion(editor: &mut Editor, delta: isize) -> Result<()> {
    // The window owns the goal column; take it before touching the buffer.
    let existing = editor.windows.current().goal_column;
    editor.with_current_buffer(|buffer| {
        let point = buffer.point();
        let goal = existing.unwrap_or_else(|| buffer.display_column(point));
        let line = buffer.line_of(point);
        let target = line
            .saturating_add_signed(delta)
            .min(buffer.len_lines().saturating_sub(1));
        let to = buffer.offset_at_display_column(target, goal);
        buffer.set_point_keeping_goal(to);
        buffer.set_goal_column(Some(goal));
    });
    editor.follow_point();
    Ok(())
}

fn next_line(editor: &mut Editor, args: &Args) -> Result<()> {
    line_motion(editor, args.signed_count() as isize)
}

fn previous_line(editor: &mut Editor, args: &Args) -> Result<()> {
    line_motion(editor, -(args.signed_count() as isize))
}

fn beginning_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    move_point(editor, |b| Motion::line_start(b.rope(), b.point()));
    Ok(())
}

fn end_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    move_point(editor, |b| Motion::line_end(b.rope(), b.point()));
    Ok(())
}

fn back_to_indentation(editor: &mut Editor, _: &Args) -> Result<()> {
    move_point(editor, |b| Motion::back_to_indentation(b.rope(), b.point()));
    Ok(())
}

// ---- words, sentences, paragraphs --------------------------------------

macro_rules! paired_motion {
    ($forward:ident, $backward:ident, $advance:path, $retreat:path) => {
        fn $forward(editor: &mut Editor, args: &Args) -> Result<()> {
            directional(
                editor,
                args,
                |e, n| {
                    move_point(e, |b| $advance(b.rope(), b.point(), n));
                    Ok(())
                },
                |e, n| {
                    move_point(e, |b| $retreat(b.rope(), b.point(), n));
                    Ok(())
                },
            )
        }

        fn $backward(editor: &mut Editor, args: &Args) -> Result<()> {
            directional(
                editor,
                args,
                |e, n| {
                    move_point(e, |b| $retreat(b.rope(), b.point(), n));
                    Ok(())
                },
                |e, n| {
                    move_point(e, |b| $advance(b.rope(), b.point(), n));
                    Ok(())
                },
            )
        }
    };
}

paired_motion!(
    forward_word,
    backward_word,
    Motion::forward_word,
    Motion::backward_word
);
paired_motion!(
    forward_sentence,
    backward_sentence,
    Motion::forward_sentence,
    Motion::backward_sentence
);
paired_motion!(
    forward_paragraph,
    backward_paragraph,
    Motion::forward_paragraph,
    Motion::backward_paragraph
);

// ---- balanced expressions ----------------------------------------------

/// Scans over sexps, reporting unbalanced delimiters rather than moving
/// somewhere arbitrary.
fn sexp_motion(editor: &mut Editor, n: usize, forward: bool) -> Result<()> {
    let found = editor.with_current_buffer(|buffer| {
        let point = buffer.point();
        let to = if forward {
            Motion::forward_sexp(buffer.rope(), point, n)
        } else {
            Motion::backward_sexp(buffer.rope(), point, n)
        };
        if let Some(to) = to {
            buffer.set_point(to);
        }
        to.is_some()
    });
    if !found {
        return Err(crate::CoreError::Message(
            "Containing expression ends prematurely".into(),
        ));
    }
    editor.follow_point();
    Ok(())
}

fn forward_sexp(editor: &mut Editor, args: &Args) -> Result<()> {
    directional(
        editor,
        args,
        |e, n| sexp_motion(e, n, true),
        |e, n| sexp_motion(e, n, false),
    )
}

fn backward_sexp(editor: &mut Editor, args: &Args) -> Result<()> {
    directional(
        editor,
        args,
        |e, n| sexp_motion(e, n, false),
        |e, n| sexp_motion(e, n, true),
    )
}

fn beginning_of_defun(editor: &mut Editor, args: &Args) -> Result<()> {
    for _ in 0..args.count() {
        move_point(editor, |b| Motion::beginning_of_defun(b.rope(), b.point()));
    }
    Ok(())
}

fn end_of_defun(editor: &mut Editor, args: &Args) -> Result<()> {
    for _ in 0..args.count() {
        move_point(editor, |b| Motion::end_of_defun(b.rope(), b.point()));
    }
    Ok(())
}

// ---- whole buffer -------------------------------------------------------

/// Both buffer-end commands push the mark first, so `C-u C-SPC` comes back.
fn jump_to(editor: &mut Editor, target: impl FnOnce(&maxgus_text::Buffer) -> usize) {
    editor.with_current_buffer(|buffer| {
        // The place being left goes on the mark ring, so `C-u C-SPC` returns.
        let from = buffer.point();
        buffer.push_mark(from);
        let to = target(buffer);
        buffer.set_point(to);
    });
    editor.follow_point();
}

fn beginning_of_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    jump_to(editor, |b| b.point_min());
    Ok(())
}

fn end_of_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    jump_to(editor, |b| b.point_max());
    Ok(())
}

fn goto_char(editor: &mut Editor, args: &Args) -> Result<()> {
    if !args.prefix.is_present() {
        return Err(crate::CoreError::Message(
            "goto-char needs a position".into(),
        ));
    }
    let target = args.prefix.count().max(0) as usize;
    jump_to(editor, |b| b.clamp(target));
    Ok(())
}

fn goto_line(editor: &mut Editor, args: &Args) -> Result<()> {
    if !args.prefix.is_present() {
        return Err(crate::CoreError::Message(
            "goto-line needs a line number".into(),
        ));
    }
    // Line numbers are one-based everywhere the user sees them.
    let line = (args.prefix.count().max(1) as usize) - 1;
    jump_to(editor, |b| {
        b.line_start(line.min(b.len_lines().saturating_sub(1)))
    });
    Ok(())
}

// ---- scrolling ----------------------------------------------------------

fn scroll_up(editor: &mut Editor, _: &Args) -> Result<()> {
    scroll_selected_window_down(editor);
    Ok(())
}

/// Pages the selected window down, dragging point onto the first visible line.
///
/// Shared with `scroll-other-window`, which selects its target and calls this,
/// so the two cannot disagree about where point ends up.
pub(crate) fn scroll_selected_window_down(editor: &mut Editor) {
    editor.scroll_rows(page_step(editor));
    // Point follows the window, landing on the first visible line.
    let top = editor.windows.current().top_line;
    editor.with_current_buffer(|b| {
        if b.line_of(b.point()) < top {
            b.set_point(b.line_start(top));
        }
    });
}

/// A screenful less the two rows of overlap `next-screen-context-lines`
/// provides, in screen rows — which is what a page is, wrapped or not.
fn page_step(editor: &Editor) -> isize {
    editor
        .windows
        .current()
        .text_height()
        .saturating_sub(2)
        .max(1) as isize
}

fn scroll_down(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.scroll_rows(-page_step(editor));
    let bottom = editor.bottom_visible_line();
    editor.with_current_buffer(|b| {
        if b.line_of(b.point()) > bottom {
            b.set_point(b.line_start(bottom));
        }
    });
    Ok(())
}

/// `recenter-top-bottom` cycles: centre, then top, then bottom, as long as it
/// is invoked repeatedly.
fn recenter(editor: &mut Editor, args: &Args) -> Result<()> {
    let height = editor.windows.current().text_height();
    if args.prefix.is_present() {
        // A count from the top, or from the bottom when it is negative.
        let position = args.signed_count();
        let above = match position < 0 {
            true => height.saturating_sub(position.unsigned_abs() as usize),
            false => position as usize,
        };
        editor.scroll_point_to_row(above);
        return Ok(());
    }
    // The cycle position is read off where point currently sits in the
    // window, so no extra state is needed to know which stop is next.
    let repeating = matches!(editor.last_command.as_deref(), Some("recenter-top-bottom"));
    let above = match repeating.then(|| editor.point_row()).flatten() {
        Some(row) if row == height / 2 => 0,
        Some(0) => height.saturating_sub(1),
        _ => height / 2,
    };
    editor.scroll_point_to_row(above);
    Ok(())
}

// ---- the mark -----------------------------------------------------------

/// `C-SPC` sets the mark; `C-u C-SPC` jumps to the previous one instead.
fn set_mark(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.prefix.is_raw() {
        return pop_mark(editor, args);
    }
    editor.with_current_buffer(|buffer| {
        let point = buffer.point();
        buffer.set_mark(point);
    });
    editor.message("Mark set");
    Ok(())
}

fn pop_mark(editor: &mut Editor, _: &Args) -> Result<()> {
    let moved = editor.with_current_buffer(|buffer| match buffer.pop_mark_ring() {
        Some(target) => {
            buffer.set_point(target);
            buffer.deactivate_mark();
            true
        }
        None => false,
    });
    if !moved {
        return Err(crate::CoreError::Message(
            "No mark set in this buffer".into(),
        ));
    }
    editor.follow_point();
    Ok(())
}

fn exchange_point_and_mark(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.with_current_buffer(|buffer| buffer.exchange_point_and_mark())?;
    editor.follow_point();
    Ok(())
}

fn mark_whole_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.with_current_buffer(|buffer| {
        let (min, max) = (buffer.point_min(), buffer.point_max());
        buffer.set_point(min);
        buffer.set_mark(max);
        // Emacs leaves point at the beginning and the mark at the end.
        buffer.set_point(min);
    });
    editor.follow_point();
    Ok(())
}

/// Extends the region over the next `n` of whatever `advance` moves across.
fn mark_thing(
    editor: &mut Editor,
    args: &Args,
    advance: impl Fn(&maxgus_text::Buffer, usize) -> Option<usize>,
) -> Result<()> {
    let n = args.count();
    let extended = editor.with_current_buffer(|buffer| {
        // With the mark already active, extend from it rather than resetting.
        let from = if buffer.is_mark_active() {
            buffer.mark().unwrap_or_else(|| buffer.point())
        } else {
            buffer.point()
        };
        let anchor = if buffer.is_mark_active() {
            buffer.point().max(from)
        } else {
            buffer.point()
        };
        let saved = buffer.point();
        buffer.set_point(anchor);
        let target = advance(buffer, n);
        buffer.set_point(saved);
        match target {
            Some(to) => {
                buffer.set_mark(to);
                true
            }
            None => false,
        }
    });
    if !extended {
        return Err(crate::CoreError::Message(
            "Containing expression ends prematurely".into(),
        ));
    }
    Ok(())
}

/// The shape `mark-paragraph` and `mark-defun` share: point goes to the front
/// of the thing around it and the mark to the back, which is the other way
/// round from `mark-word` and `mark-sexp` — those grow forward from point.
///
/// Repeating the command extends by another thing rather than re-marking the
/// same one, which is what makes `M-h M-h` cover two paragraphs.
fn mark_enclosing(
    editor: &mut Editor,
    args: &Args,
    start: impl Fn(&maxgus_text::Buffer, usize) -> usize,
    end: impl Fn(&maxgus_text::Buffer, usize) -> usize,
) -> Result<()> {
    let n = args.count();
    editor.with_current_buffer(|buffer| {
        // An active mark means this is a repeat, so the far end moves on from
        // where it already is and point stays where it was put.
        let (from, extend_at) = match buffer.is_mark_active() {
            true => (
                buffer.point(),
                buffer.mark().unwrap_or_else(|| buffer.point()),
            ),
            false => (buffer.point(), buffer.point()),
        };
        let saved = buffer.point();
        buffer.set_point(extend_at);
        let to = end(buffer, n);
        buffer.set_point(from);
        let head = match buffer.is_mark_active() {
            // Point has already been moved to the front by the first call.
            true => saved,
            false => start(buffer, 1),
        };
        buffer.set_point(head);
        buffer.set_mark(to.max(head));
    });
    editor.follow_point();
    Ok(())
}

fn mark_paragraph(editor: &mut Editor, args: &Args) -> Result<()> {
    mark_enclosing(
        editor,
        args,
        |b, n| Motion::backward_paragraph(b.rope(), b.point(), n),
        |b, n| Motion::forward_paragraph(b.rope(), b.point(), n),
    )
}

fn mark_defun(editor: &mut Editor, args: &Args) -> Result<()> {
    mark_enclosing(
        editor,
        args,
        |b, _| Motion::beginning_of_defun(b.rope(), b.point()),
        |b, n| {
            let mut at = b.point();
            for _ in 0..n {
                at = Motion::end_of_defun(b.rope(), at);
            }
            at
        },
    )
}

fn mark_word(editor: &mut Editor, args: &Args) -> Result<()> {
    mark_thing(editor, args, |b, n| {
        Some(Motion::forward_word(b.rope(), b.point(), n))
    })
}

fn mark_sexp(editor: &mut Editor, args: &Args) -> Result<()> {
    mark_thing(editor, args, |b, n| {
        Motion::forward_sexp(b.rope(), b.point(), n)
    })
}

/// `M-=`: counts the region, or the whole buffer when there is no region —
/// which is what Emacs does when the mark is not active.
fn count_words(editor: &mut Editor, _: &Args) -> Result<()> {
    let (what, text) = {
        let buffer = editor.current_buffer();
        match buffer.region() {
            Some(range) => ("Region", buffer.slice(range)),
            None => (
                "Buffer",
                buffer.slice(maxgus_text::Range::new(
                    buffer.point_min(),
                    buffer.point_max(),
                )),
            ),
        }
    };
    let characters = text.chars().count();
    // An empty selection is nought lines; anything else has a line for its
    // last, unterminated, part.
    let lines = match characters {
        0 => 0,
        _ => text.matches('\n').count() + usize::from(!text.ends_with('\n')),
    };
    let words = text
        .split(|c: char| !maxgus_text::CharClass::of(c).is_word())
        .filter(|w| !w.is_empty())
        .count();
    editor.message(format!(
        "{what} has {lines} line{}, {words} word{}, and {characters} character{}",
        plural(lines),
        plural(words),
        plural(characters)
    ));
    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `C-x =`: reports the character under point and where it sits.
fn what_cursor_position(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let total = buffer.len_chars();
        let position = buffer.position_of(point);
        let column = buffer.display_column(point);
        let percent = point
            .checked_mul(100)
            .and_then(|n| n.checked_div(total))
            .unwrap_or(0);
        match buffer.char_after(point) {
            Some(c) => format!(
                "Char: {} (U+{:04X})  point={} of {} ({}%)  line {} column {}",
                describe_char(c),
                c as u32,
                point,
                total,
                percent,
                position.line + 1,
                column
            ),
            None => format!(
                "point={} of {} (end of buffer)  line {} column {}",
                point,
                total,
                position.line + 1,
                column
            ),
        }
    };
    editor.message(text);
    Ok(())
}

/// Renders a character the way `what-cursor-position` does, spelling out the
/// ones that have no visible form.
fn describe_char(c: char) -> String {
    match c {
        '\n' => "^J".to_string(),
        '\t' => "^I".to_string(),
        c if (c as u32) < 0x20 => format!("^{}", (b'@' + c as u8) as char),
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dispatcher;
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_text::Range;
    use maxgus_tui::Rect;

    /// An editor showing `text` with point at the start, plus a dispatcher
    /// holding only the motion commands.
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
        register(&mut registry);
        (Dispatcher::new(registry), editor)
    }

    /// Runs `command` with no prefix argument, asserting it succeeded.
    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, crate::Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    /// Runs `command` with a numeric prefix argument.
    fn run_n(d: &mut Dispatcher, e: &mut Editor, command: &str, n: i32) {
        e.prefix = crate::Prefix::Numeric(n);
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, crate::Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    fn point(e: &Editor) -> usize {
        e.windows.current().point
    }

    #[test]
    fn mark_paragraph_puts_point_before_it_and_the_mark_after() {
        // The other way round from `mark-word`, which grows forward from point.
        let (mut d, mut e) = setup("one two\nthree\n\nsecond para\n");
        e.with_current_buffer(|b| b.set_point(9));
        run(&mut d, &mut e, "mark-paragraph");

        let region = e.current_buffer().region().expect("a region");
        assert_eq!(region.start, 0, "point went to the front of the paragraph");
        let text = e.current_buffer().slice(region);
        assert!(text.contains("one two"), "got {text:?}");
        assert!(text.contains("three"), "got {text:?}");
        assert!(
            !text.contains("second para"),
            "it stopped at the blank line: {text:?}"
        );
    }

    #[test]
    fn marking_a_paragraph_again_takes_in_the_next_one() {
        let (mut d, mut e) = setup("first\n\nsecond\n\nthird\n");
        e.with_current_buffer(|b| b.set_point(0));
        run(&mut d, &mut e, "mark-paragraph");
        let once = e.current_buffer().region().expect("a region");
        run(&mut d, &mut e, "mark-paragraph");
        let twice = e.current_buffer().region().expect("a region");
        assert_eq!(
            twice.start, once.start,
            "point stays where the first put it"
        );
        assert!(
            twice.end > once.end,
            "the far end moved on: {once:?} then {twice:?}"
        );
    }

    #[test]
    fn mark_defun_covers_the_definition_around_point() {
        let (mut d, mut e) = setup("fn one() {\n    let x = 1;\n}\n\nfn two() {\n}\n");
        e.with_current_buffer(|b| b.set_point(16));
        run(&mut d, &mut e, "mark-defun");

        let region = e.current_buffer().region().expect("a region");
        let text = e.current_buffer().slice(region);
        assert!(text.contains("fn one()"), "got {text:?}");
        assert!(
            !text.contains("fn two()"),
            "it stopped at the next definition: {text:?}"
        );
    }

    #[test]
    fn count_words_reports_the_buffer_when_nothing_is_marked() {
        let (mut d, mut e) = setup("one two three\nfour five\n");
        run(&mut d, &mut e, "count-words");
        let echo = e.minibuffer.display();
        assert!(echo.starts_with("Buffer has"), "got `{echo}`");
        assert!(echo.contains("2 lines"), "got `{echo}`");
        assert!(echo.contains("5 words"), "got `{echo}`");
    }

    #[test]
    fn count_words_reports_the_region_when_there_is_one() {
        let (mut d, mut e) = setup("one two three\nfour five\n");
        // `set_mark` activates the region; `push_mark` deliberately does not,
        // because a jump-back position is not a selection.
        e.with_current_buffer(|b| {
            b.set_point(0);
            b.set_mark(7);
        });
        run(&mut d, &mut e, "count-words");
        let echo = e.minibuffer.display();
        assert!(echo.starts_with("Region has"), "got `{echo}`");
        assert!(echo.contains("2 words"), "`one two` is two words: `{echo}`");
    }

    #[test]
    fn count_words_says_one_line_not_one_lines() {
        let (mut d, mut e) = setup("solo\n");
        run(&mut d, &mut e, "count-words");
        let echo = e.minibuffer.display();
        assert!(echo.contains("1 line,"), "got `{echo}`");
        assert!(echo.contains("1 word,"), "got `{echo}`");
    }

    #[test]
    fn every_motion_command_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for name in [
            "forward-char",
            "next-line",
            "forward-word",
            "beginning-of-buffer",
            "set-mark-command",
        ] {
            assert!(registry.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn character_motion_moves_and_clamps() {
        let (mut d, mut e) = setup("abc");
        run(&mut d, &mut e, "forward-char");
        assert_eq!(point(&e), 1);
        run_n(&mut d, &mut e, "forward-char", 10);
        assert_eq!(point(&e), 3, "clamps at the end");
        run_n(&mut d, &mut e, "backward-char", 10);
        assert_eq!(point(&e), 0, "clamps at the start");
    }

    #[test]
    fn a_negative_argument_reverses_a_paired_motion() {
        let (mut d, mut e) = setup("abcdef");
        run_n(&mut d, &mut e, "forward-char", 4);
        assert_eq!(point(&e), 4);
        run_n(&mut d, &mut e, "forward-char", -2);
        assert_eq!(point(&e), 2, "a negative argument moves backward");
        run_n(&mut d, &mut e, "backward-char", -3);
        assert_eq!(point(&e), 5, "and vice versa");
    }

    #[test]
    fn line_motion_keeps_the_goal_column_across_a_short_line() {
        let (mut d, mut e) = setup("aaaaaaaaaa\nbb\ncccccccccc");
        run_n(&mut d, &mut e, "forward-char", 8);
        assert_eq!(point(&e), 8);

        run(&mut d, &mut e, "next-line");
        assert_eq!(point(&e), 13, "clamped to the end of the short line");

        run(&mut d, &mut e, "next-line");
        assert_eq!(point(&e), 22, "back out to column eight");
    }

    #[test]
    fn a_horizontal_motion_resets_the_goal_column() {
        let (mut d, mut e) = setup("aaaaaaaaaa\nbb\ncccccccccc");
        run_n(&mut d, &mut e, "forward-char", 8);
        run(&mut d, &mut e, "next-line");
        run(&mut d, &mut e, "move-beginning-of-line");
        run(&mut d, &mut e, "next-line");
        assert_eq!(point(&e), 14, "column zero of the third line");
    }

    #[test]
    fn line_motion_clamps_at_the_first_and_last_line() {
        let (mut d, mut e) = setup("one\ntwo\nthree");
        run_n(&mut d, &mut e, "previous-line", 5);
        assert_eq!(e.current_buffer().line_of(point(&e)), 0);
        run_n(&mut d, &mut e, "next-line", 50);
        assert_eq!(e.current_buffer().line_of(point(&e)), 2);
    }

    #[test]
    fn line_motion_accounts_for_tab_expansion() {
        let (mut d, mut e) = setup("\tx\nabcdefgh");
        e.with_current_buffer(|b| b.set_tab_width(4));
        run_n(&mut d, &mut e, "forward-char", 1);
        assert_eq!(e.current_buffer().display_column(point(&e)), 4);
        run(&mut d, &mut e, "next-line");
        assert_eq!(point(&e), 3 + 4, "column four of the second line");
    }

    #[test]
    fn line_commands_reach_the_ends_and_the_indentation() {
        let (mut d, mut e) = setup("    indented text\nnext");
        run_n(&mut d, &mut e, "forward-char", 10);
        run(&mut d, &mut e, "move-beginning-of-line");
        assert_eq!(point(&e), 0);
        run(&mut d, &mut e, "back-to-indentation");
        assert_eq!(point(&e), 4);
        run(&mut d, &mut e, "move-end-of-line");
        assert_eq!(point(&e), 17, "before the newline");
    }

    #[test]
    fn word_motion_honours_the_count() {
        let (mut d, mut e) = setup("alpha beta gamma delta");
        run(&mut d, &mut e, "forward-word");
        assert_eq!(point(&e), 5);
        run_n(&mut d, &mut e, "forward-word", 2);
        assert_eq!(point(&e), 16);
        run_n(&mut d, &mut e, "backward-word", 3);
        assert_eq!(point(&e), 0);
    }

    #[test]
    fn paragraph_and_sentence_motion_work() {
        let (mut d, mut e) = setup("One. Two.\n\nSecond paragraph.\n");
        run(&mut d, &mut e, "forward-sentence");
        assert_eq!(point(&e), 4, "just past the full stop");
        run(&mut d, &mut e, "beginning-of-buffer");
        run(&mut d, &mut e, "forward-paragraph");
        assert_eq!(e.current_buffer().line_of(point(&e)), 1, "the blank line");
    }

    #[test]
    fn sexp_motion_traverses_balanced_delimiters() {
        let (mut d, mut e) = setup("(a (b c)) rest");
        run(&mut d, &mut e, "forward-sexp");
        assert_eq!(point(&e), 9);
        run(&mut d, &mut e, "backward-sexp");
        assert_eq!(point(&e), 0);
    }

    #[test]
    fn unbalanced_delimiters_are_reported_rather_than_guessed_at() {
        let (mut d, mut e) = setup("(a b");
        let out = d.execute(&mut e, "forward-sexp", None);
        assert!(matches!(out, crate::Dispatch::Failed { .. }));
        assert_eq!(point(&e), 0, "point did not move");
        assert!(e.minibuffer.message_is_error());
    }

    #[test]
    fn defun_motion_anchors_on_column_zero() {
        let (mut d, mut e) = setup("fn a() {\n    body\n}\n\nfn b() {\n    body\n}\n");
        let inside_b = e.current_buffer().line_start(5) + 2;
        e.with_current_buffer(|b| b.set_point(inside_b));
        run(&mut d, &mut e, "beginning-of-defun");
        assert_eq!(e.current_buffer().line_of(point(&e)), 4);
    }

    #[test]
    fn buffer_end_commands_push_the_mark_so_the_jump_can_be_undone() {
        let (mut d, mut e) = setup("0123456789");
        run_n(&mut d, &mut e, "forward-char", 5);
        run(&mut d, &mut e, "end-of-buffer");
        assert_eq!(point(&e), 10);
        assert_eq!(
            e.current_buffer().mark(),
            Some(5),
            "the old position was marked"
        );
        run(&mut d, &mut e, "beginning-of-buffer");
        assert_eq!(point(&e), 0);
    }

    #[test]
    fn goto_line_is_one_based_and_clamps() {
        let (mut d, mut e) = setup("one\ntwo\nthree\nfour");
        run_n(&mut d, &mut e, "goto-line", 3);
        assert_eq!(e.current_buffer().line_of(point(&e)), 2);
        run_n(&mut d, &mut e, "goto-line", 999);
        assert_eq!(
            e.current_buffer().line_of(point(&e)),
            3,
            "clamps to the last line"
        );
        run_n(&mut d, &mut e, "goto-line", 1);
        assert_eq!(point(&e), 0);
    }

    #[test]
    fn goto_commands_need_an_argument() {
        let (mut d, mut e) = setup("text");
        assert!(matches!(
            d.execute(&mut e, "goto-line", None),
            crate::Dispatch::Failed { .. }
        ));
        assert!(matches!(
            d.execute(&mut e, "goto-char", None),
            crate::Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn goto_char_moves_to_an_offset() {
        let (mut d, mut e) = setup("0123456789");
        run_n(&mut d, &mut e, "goto-char", 4);
        assert_eq!(point(&e), 4);
        run_n(&mut d, &mut e, "goto-char", 999);
        assert_eq!(point(&e), 10, "clamps to the end");
    }

    #[test]
    fn paging_moves_the_window_and_drags_point_along() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let (mut d, mut e) = setup(&text);
        run(&mut d, &mut e, "scroll-up-command");
        let window = e.windows.current();
        assert!(window.top_line > 0);
        assert!(
            e.current_buffer().line_of(point(&e)) >= window.top_line,
            "point stayed on screen"
        );
        run(&mut d, &mut e, "scroll-down-command");
        assert_eq!(e.windows.current().top_line, 0);
    }

    #[test]
    fn recentring_puts_point_in_the_middle_then_cycles() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let (mut d, mut e) = setup(&text);
        e.with_current_buffer(|b| b.set_point(b.line_start(100)));
        e.follow_point();

        run(&mut d, &mut e, "recenter-top-bottom");
        let height = e.windows.current().text_height();
        assert_eq!(e.windows.current().top_line, 100 - height / 2, "centred");

        run(&mut d, &mut e, "recenter-top-bottom");
        assert_eq!(e.windows.current().top_line, 100, "then the top");

        run(&mut d, &mut e, "recenter-top-bottom");
        assert_eq!(
            e.windows.current().top_line,
            100 - (height - 1),
            "then the bottom"
        );
    }

    #[test]
    fn recentring_with_an_argument_positions_from_the_top() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let (mut d, mut e) = setup(&text);
        e.with_current_buffer(|b| b.set_point(b.line_start(100)));
        run_n(&mut d, &mut e, "recenter-top-bottom", 3);
        assert_eq!(e.windows.current().top_line, 97);
    }

    #[test]
    fn setting_the_mark_activates_the_region() {
        let (mut d, mut e) = setup("hello world");
        run(&mut d, &mut e, "set-mark-command");
        assert_eq!(e.minibuffer.display(), "Mark set");
        run_n(&mut d, &mut e, "forward-char", 5);
        assert_eq!(e.region().unwrap(), Range::new(0, 5));
    }

    #[test]
    fn a_raw_prefix_makes_c_spc_pop_the_mark_ring_instead() {
        let (mut d, mut e) = setup("0123456789");
        run(&mut d, &mut e, "set-mark-command");
        run_n(&mut d, &mut e, "forward-char", 5);
        run(&mut d, &mut e, "set-mark-command");
        run_n(&mut d, &mut e, "forward-char", 3);
        assert_eq!(point(&e), 8);

        e.prefix = crate::Prefix::Universal(1);
        run(&mut d, &mut e, "set-mark-command");
        assert_eq!(point(&e), 0, "back to the first mark");
    }

    #[test]
    fn popping_an_empty_mark_ring_is_an_error() {
        let (mut d, mut e) = setup("text");
        assert!(matches!(
            d.execute(&mut e, "pop-mark", None),
            crate::Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn exchanging_point_and_mark_swaps_them() {
        let (mut d, mut e) = setup("hello world");
        run(&mut d, &mut e, "set-mark-command");
        run_n(&mut d, &mut e, "forward-char", 5);
        run(&mut d, &mut e, "exchange-point-and-mark");
        assert_eq!(point(&e), 0);
        assert_eq!(e.current_buffer().mark(), Some(5));
    }

    #[test]
    fn exchanging_without_a_mark_is_an_error() {
        let (mut d, mut e) = setup("text");
        assert!(matches!(
            d.execute(&mut e, "exchange-point-and-mark", None),
            crate::Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn marking_the_whole_buffer_covers_it() {
        let (mut d, mut e) = setup("some text here");
        run(&mut d, &mut e, "mark-whole-buffer");
        assert_eq!(point(&e), 0);
        assert_eq!(e.region().unwrap(), Range::new(0, 14));
    }

    #[test]
    fn marking_a_word_extends_the_region_over_it() {
        let (mut d, mut e) = setup("alpha beta gamma");
        run(&mut d, &mut e, "mark-word");
        assert_eq!(e.region().unwrap(), Range::new(0, 5));
        // Repeating extends the region by one more word.
        run(&mut d, &mut e, "mark-word");
        assert_eq!(e.region().unwrap(), Range::new(0, 10));

        // With an argument and no active mark, it marks that many words.
        e.with_current_buffer(|b| {
            b.deactivate_mark();
            b.set_point(0);
        });
        run_n(&mut d, &mut e, "mark-word", 2);
        assert_eq!(e.region().unwrap(), Range::new(0, 10));
    }

    #[test]
    fn marking_a_sexp_covers_a_balanced_expression() {
        let (mut d, mut e) = setup("(a b) rest");
        run(&mut d, &mut e, "mark-sexp");
        assert_eq!(e.region().unwrap(), Range::new(0, 5));
    }

    #[test]
    fn marking_an_unbalanced_sexp_is_an_error() {
        let (mut d, mut e) = setup("(a b");
        assert!(matches!(
            d.execute(&mut e, "mark-sexp", None),
            crate::Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn what_cursor_position_describes_the_character_under_point() {
        let (mut d, mut e) = setup("Hi\n");
        run(&mut d, &mut e, "what-cursor-position");
        let text = e.minibuffer.display();
        assert!(text.contains("Char: H"), "got `{text}`");
        assert!(text.contains("U+0048"), "got `{text}`");
        assert!(text.contains("line 1"), "got `{text}`");
    }

    #[test]
    fn what_cursor_position_names_the_invisible_characters() {
        let (mut d, mut e) = setup("a\tb");
        run_n(&mut d, &mut e, "forward-char", 1);
        run(&mut d, &mut e, "what-cursor-position");
        assert!(
            e.minibuffer.display().contains("^I"),
            "got `{}`",
            e.minibuffer.display()
        );
        assert_eq!(describe_char('\n'), "^J");
        assert_eq!(describe_char('\u{1}'), "^A");
    }

    #[test]
    fn what_cursor_position_reports_the_end_of_the_buffer() {
        let (mut d, mut e) = setup("ab");
        run(&mut d, &mut e, "end-of-buffer");
        run(&mut d, &mut e, "what-cursor-position");
        assert!(e.minibuffer.display().contains("end of buffer"));
    }

    #[test]
    fn motion_scrolls_the_window_to_keep_point_visible() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let (mut d, mut e) = setup(&text);
        run_n(&mut d, &mut e, "next-line", 100);
        let window = e.windows.current();
        assert!(window.shows_line(100), "the window followed point");
    }
}
