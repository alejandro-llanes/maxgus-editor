//! Text commands that depend on the buffer's language: comments and filling.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_text::{Motion, Range};

/// Registers the text commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "delete-trailing-whitespace",
            "Take the spaces off the ends of every line.",
            delete_trailing_whitespace
        ),
        command!(
            "comment-dwim",
            "Comment or uncomment the region, or this line.",
            comment_dwim
        ),
        command!(
            "comment-line",
            "Comment or uncomment this line.",
            comment_line
        ),
        command!(
            "fill-paragraph",
            "Wrap the paragraph to the fill column.",
            fill_paragraph
        ),
        command!(
            "fill-region",
            "Wrap the region to the fill column.",
            fill_region
        ),
    ]);
}

/// The line-comment marker for a language.
pub fn comment_prefix(language: Option<&str>) -> &'static str {
    match language {
        Some("rust" | "c" | "cpp" | "javascript" | "typescript" | "go" | "css" | "kdl") => "//",
        Some("python" | "bash" | "toml" | "yaml" | "make" | "dockerfile") => "#",
        Some("html" | "markdown") => "<!--",
        // Anything unrecognised gets the most widely understood marker.
        _ => "#",
    }
}

/// The closing marker, for languages whose comments are delimited.
fn comment_suffix(prefix: &str) -> &'static str {
    if prefix == "<!--" { " -->" } else { "" }
}

/// The lines the command should act on: the region's, or the current one.
fn target_lines(editor: &mut Editor) -> (usize, usize) {
    if let Ok(range) = editor.region() {
        let buffer = editor.current_buffer();
        let first = buffer.line_of(range.start);
        // A region ending at a line start does not include that line.
        let last_offset = range.end.saturating_sub(1).max(range.start);
        (first, buffer.line_of(last_offset))
    } else {
        let line = editor
            .current_buffer()
            .line_of(editor.current_buffer().point());
        (line, line)
    }
}

/// Comments or uncomments `first..=last`, whichever leaves them consistent.
fn toggle_comments(editor: &mut Editor, first: usize, last: usize) -> Result<()> {
    let prefix = comment_prefix(editor.current_buffer().language());
    let suffix = comment_suffix(prefix);

    // Every non-blank line already commented means the command uncomments.
    let (all_commented, indent) = {
        let buffer = editor.current_buffer();
        let lines: Vec<String> = (first..=last).map(|l| buffer.line_text(l)).collect();
        let interesting: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
        let all = !interesting.is_empty()
            && interesting
                .iter()
                .all(|l| l.trim_start().starts_with(prefix));
        // Comments go in at the shallowest indentation, so they line up.
        let indent = interesting
            .iter()
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        (all, indent)
    };

    editor.with_current_buffer(|buffer| {
        buffer.transact(false, |buffer| {
            // Work backwards so earlier offsets stay valid.
            for line in (first..=last).rev() {
                let text = buffer.line_text(line);
                if text.trim().is_empty() {
                    continue;
                }
                let start = buffer.line_start(line);
                let end = Motion::line_end(buffer.rope(), start);
                let replacement = if all_commented {
                    uncomment(&text, prefix, suffix)
                } else {
                    let (head, tail) = text.split_at(indent.min(text.len()));
                    format!("{head}{prefix} {tail}{suffix}")
                };
                buffer.replace(Range::new(start, end), &replacement)?;
            }
            Ok::<(), maxgus_text::TextError>(())
        })
    })?;
    Ok(())
}

/// Strips one comment marker from `text`, keeping its indentation.
fn uncomment(text: &str, prefix: &str, suffix: &str) -> String {
    let indent = &text[..text.len() - text.trim_start().len()];
    let body = text.trim_start();
    let Some(body) = body.strip_prefix(prefix) else {
        return text.to_string();
    };
    // The space inserted when commenting is taken back out.
    let body = body.strip_prefix(' ').unwrap_or(body);
    let body = match suffix.is_empty() {
        true => body,
        false => body.strip_suffix(suffix).unwrap_or(body),
    };
    format!("{indent}{body}")
}

fn comment_dwim(editor: &mut Editor, _: &Args) -> Result<()> {
    let (first, last) = target_lines(editor);
    toggle_comments(editor, first, last)?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

fn comment_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let buffer = editor.current_buffer();
    let first = buffer.line_of(buffer.point());
    let last = (first + args.count() - 1).min(buffer.len_lines().saturating_sub(1));
    toggle_comments(editor, first, last)
}

// ---- filling ------------------------------------------------------------

/// Wraps `text` to `width` columns, keeping `indent` on every line.
///
/// Words longer than the width are left whole rather than broken, which is
/// what keeps a long URL or identifier intact.
pub fn fill(text: &str, width: usize, indent: &str) -> String {
    let width = width.max(indent.len() + 1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            format!("{indent}{word}")
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && candidate.chars().count() > width {
            lines.push(std::mem::take(&mut current));
            current = format!("{indent}{word}");
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}

/// Fills `first..=last`, treating them as one paragraph.
fn fill_lines(editor: &mut Editor, first: usize, last: usize) -> Result<()> {
    let width = editor.fill_column_for(editor.current_buffer_id());
    let (range, indent, text) = {
        let buffer = editor.current_buffer();
        let start = buffer.line_start(first);
        let end = Motion::line_end(buffer.rope(), buffer.line_start(last));
        if end <= start {
            return Ok(());
        }
        let head = buffer.line_text(first);
        let indent = head[..head.len() - head.trim_start().len()].to_string();
        (
            Range::new(start, end),
            indent,
            buffer.slice(Range::new(start, end)),
        )
    };
    if text.trim().is_empty() {
        return Ok(());
    }
    let filled = fill(&text, width, &indent);
    if filled == text {
        editor.message("Paragraph is already filled");
        return Ok(());
    }
    editor.with_current_buffer(|b| b.replace(range, &filled))?;
    editor.with_current_buffer(|b| {
        let to = range.start + filled.chars().count();
        b.set_point(to.min(b.point_max()));
    });
    editor.follow_point();
    Ok(())
}

fn fill_paragraph(editor: &mut Editor, _: &Args) -> Result<()> {
    let (first, last) = {
        let buffer = editor.current_buffer();
        let here = buffer.line_of(buffer.point());
        let blank = |l: usize| buffer.line_text(l).trim().is_empty();
        if blank(here) {
            return Ok(());
        }
        let mut first = here;
        while first > 0 && !blank(first - 1) {
            first -= 1;
        }
        let mut last = here;
        while last + 1 < buffer.len_lines() && !blank(last + 1) {
            last += 1;
        }
        (first, last)
    };
    fill_lines(editor, first, last)
}

fn fill_region(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = editor.region()?;
    let (first, last) = {
        let buffer = editor.current_buffer();
        (
            buffer.line_of(range.start),
            buffer.line_of(range.end.saturating_sub(1).max(range.start)),
        )
    };
    fill_lines(editor, first, last)?;
    editor.with_current_buffer(|b| b.deactivate_mark());
    Ok(())
}

// ---- syntax ------------------------------------------------------------

/// Registers the commands that read the syntax tree.
#[cfg(feature = "syntax")]
pub fn register_syntax(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "describe-syntax-at-point",
            "Say what the parser makes of the text at point.",
            describe_syntax
        ),
        command!(
            "expand-region",
            "Extend the region to the enclosing syntactic unit.",
            expand_region
        ),
    ]);
}

/// `C-h s`: reports the grammar's name for the construct under point.
#[cfg(feature = "syntax")]
fn describe_syntax(editor: &mut Editor, _: &Args) -> Result<()> {
    let (kind, span) = syntax_at_point(editor)?;
    let text = {
        let buffer = editor.current_buffer();
        let preview: String = buffer.slice(span).chars().take(40).collect();
        format!("{kind}: `{}`", preview.replace('\n', "\\n"))
    };
    editor.message(text);
    Ok(())
}

/// `C-=`: grows the region to the next enclosing node.
#[cfg(feature = "syntax")]
fn expand_region(editor: &mut Editor, _: &Args) -> Result<()> {
    let (_, span) = syntax_at_point(editor)?;
    editor.with_current_buffer(|buffer| {
        buffer.set_point(span.start);
        buffer.set_mark(span.start);
        buffer.set_point(span.end);
    });
    editor.follow_point();
    Ok(())
}

/// The node kind and span at point, from the buffer's syntax tree.
///
/// The tree lives in the executor, so this parses on demand. It is only for
/// commands the user invokes deliberately, never for redisplay.
#[cfg(feature = "syntax")]
fn syntax_at_point(editor: &Editor) -> Result<(String, Range)> {
    let buffer = editor.current_buffer();
    let Some(language) = buffer.language() else {
        return Err(crate::CoreError::Message("Buffer has no language".into()));
    };
    let mut highlighter = maxgus_syntax::Highlighter::new(language)
        .map_err(|_| crate::CoreError::Message(format!("No grammar for {language}")))?;
    let text = buffer.text();
    highlighter
        .parse(&text)
        .map_err(|e| crate::CoreError::Message(e.to_string()))?;

    let rope = buffer.rope();
    // An active region asks about the node enclosing it, so repeating the
    // command walks outward; otherwise it asks about point.
    let bytes = match buffer.region() {
        Some(region) if !region.is_empty() => {
            rope.char_to_byte(region.start)..rope.char_to_byte(region.end)
        }
        _ => {
            let at = rope.char_to_byte(buffer.point());
            at..at
        }
    };
    let (start, end) = highlighter
        .enclosing_node_range(bytes)
        .ok_or_else(|| crate::CoreError::Message("Nothing here to describe".into()))?;
    let kind = highlighter
        .node_kind_at(start)
        .unwrap_or("node")
        .to_string();
    Ok((
        kind,
        Range::new(rope.byte_to_char(start), rope.byte_to_char(end)),
    ))
}

#[cfg(feature = "syntax")]
#[cfg(test)]
mod syntax_tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup(name: &str, text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.visit_file(format!("/project/{name}"), text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));
        (
            Dispatcher::new(crate::commands::standard_registry()),
            editor,
        )
    }

    #[test]
    fn describing_the_syntax_names_the_construct() {
        let (mut d, mut e) = setup("main.rs", "fn main() {\n    let x = 1;\n}\n");
        e.with_current_buffer(|b| b.set_point(b.line_start(1) + 8));
        let out = d.execute(&mut e, "describe-syntax-at-point", None);
        assert!(!matches!(out, Dispatch::Failed { .. }), "{out:?}");
        let said = e.minibuffer.display();
        assert!(said.contains("identifier"), "got `{said}`");
        assert!(said.contains('x'), "got `{said}`");
    }

    #[test]
    fn expanding_the_region_covers_the_enclosing_node() {
        let (mut d, mut e) = setup("main.rs", "fn main() {\n    let x = 1;\n}\n");
        e.with_current_buffer(|b| b.set_point(b.line_start(1) + 8));

        d.execute(&mut e, "expand-region", None);
        let first = e.region().expect("a region was made");
        assert_eq!(e.current_buffer().slice(first), "x");

        // Again, and it takes in more.
        d.execute(&mut e, "expand-region", None);
        let second = e.region().expect("a region");
        assert!(
            second.len() > first.len(),
            "{second:?} did not grow from {first:?}"
        );
        assert!(e.current_buffer().slice(second).contains("let x = 1"));
    }

    #[test]
    fn a_buffer_with_no_grammar_says_so() {
        let (mut d, mut e) = setup("notes.txt", "plain text\n");
        let out = d.execute(&mut e, "describe-syntax-at-point", None);
        assert!(matches!(out, Dispatch::Failed { .. }), "{out:?}");
    }
}

/// `C-c c w`: trailing whitespace out of the whole buffer, now.
///
/// The setting of the same name does this on save; this is for a buffer that
/// is not going to be saved yet, or one whose project has switched it off.
fn delete_trailing_whitespace(editor: &mut Editor, _: &Args) -> Result<()> {
    let (cleaned, before) = {
        let buffer = editor.current_buffer();
        let text = buffer.text();
        let cleaned: String = text
            .split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        (cleaned, text)
    };
    if cleaned == before {
        return Err(crate::CoreError::Message("No trailing whitespace".into()));
    }
    let removed = before.chars().count() - cleaned.chars().count();
    let point = editor.windows.current().point;
    editor.with_current_buffer(move |b| b.replace_all(&cleaned))?;
    editor.move_point_to(point.min(editor.current_buffer().len_chars()));
    editor.message(format!("Removed {removed} character(s)"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup(name: &str, text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.visit_file(format!("/project/{name}"), text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));
        (
            Dispatcher::new(crate::commands::standard_registry()),
            editor,
        )
    }

    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
    }

    fn text(e: &Editor) -> String {
        e.current_buffer().text()
    }

    fn mark_region(e: &mut Editor, from: usize, to: usize) {
        e.with_current_buffer(|b| {
            b.set_point(from);
            b.set_mark(from);
            b.set_point(to);
        });
    }

    #[test]
    fn the_comment_marker_follows_the_language() {
        assert_eq!(comment_prefix(Some("rust")), "//");
        assert_eq!(comment_prefix(Some("python")), "#");
        assert_eq!(comment_prefix(Some("html")), "<!--");
        assert_eq!(comment_prefix(None), "#", "a sensible default");
        assert_eq!(comment_suffix("<!--"), " -->");
        assert_eq!(comment_suffix("//"), "");
    }

    #[test]
    fn commenting_a_line_and_taking_it_back() {
        let (mut d, mut e) = setup("main.rs", "let x = 1;\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "// let x = 1;\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "let x = 1;\n");
    }

    #[test]
    fn commenting_uses_the_language_marker() {
        let (mut d, mut e) = setup("script.py", "x = 1\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "# x = 1\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "x = 1\n");
    }

    #[test]
    fn a_delimited_comment_gets_both_markers() {
        let (mut d, mut e) = setup("page.html", "<p>hi</p>\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "<!-- <p>hi</p> -->\n");
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "<p>hi</p>\n");
    }

    #[test]
    fn commenting_a_region_covers_every_line() {
        let (mut d, mut e) = setup("main.rs", "one\ntwo\nthree\n");
        mark_region(&mut e, 0, 8);
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "// one\n// two\nthree\n");
    }

    #[test]
    fn markers_line_up_at_the_shallowest_indentation() {
        let (mut d, mut e) = setup("main.rs", "    one\n        two\n");
        mark_region(&mut e, 0, 19);
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "    // one\n    //     two\n");
    }

    #[test]
    fn a_partly_commented_region_is_commented_rather_than_uncommented() {
        let (mut d, mut e) = setup("main.rs", "// one\ntwo\n");
        mark_region(&mut e, 0, 10);
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(
            text(&e),
            "// // one\n// two\n",
            "everything ends up commented"
        );
    }

    #[test]
    fn blank_lines_inside_a_region_are_left_alone() {
        let (mut d, mut e) = setup("main.rs", "one\n\ntwo\n");
        mark_region(&mut e, 0, 8);
        run(&mut d, &mut e, "comment-dwim");
        assert_eq!(text(&e), "// one\n\n// two\n");
    }

    #[test]
    fn uncommenting_takes_back_only_one_marker_and_its_space() {
        assert_eq!(uncomment("// text", "//", ""), "text");
        assert_eq!(
            uncomment("//text", "//", ""),
            "text",
            "a marker with no space"
        );
        assert_eq!(
            uncomment("    // text", "//", ""),
            "    text",
            "indentation survives"
        );
        assert_eq!(
            uncomment("// // text", "//", ""),
            "// text",
            "one at a time"
        );
        assert_eq!(
            uncomment("plain", "//", ""),
            "plain",
            "nothing to take back"
        );
    }

    #[test]
    fn comment_line_can_take_a_count() {
        let (mut d, mut e) = setup("main.rs", "one\ntwo\nthree\n");
        e.prefix = crate::Prefix::Numeric(2);
        d.execute(&mut e, "comment-line", None);
        assert_eq!(text(&e), "// one\n// two\nthree\n");
    }

    // ---- filling ----

    #[test]
    fn filling_wraps_at_the_fill_column() {
        assert_eq!(fill("a b c d e", 5, ""), "a b c\nd e");
        assert_eq!(fill("one two three", 20, ""), "one two three");
        assert_eq!(fill("", 20, ""), "");
    }

    #[test]
    fn filling_keeps_the_indentation_on_every_line() {
        assert_eq!(fill("a b c d", 8, "  "), "  a b c\n  d");
    }

    #[test]
    fn a_word_longer_than_the_column_is_left_whole() {
        let filled = fill("short enormouslylongword after", 10, "");
        assert!(filled.contains("enormouslylongword"), "got `{filled}`");
        assert_eq!(filled.lines().count(), 3);
    }

    #[test]
    fn filling_a_paragraph_stops_at_blank_lines() {
        let (mut d, mut e) = setup(
            "notes.txt",
            "one two three four five six seven\n\nsecond paragraph\n",
        );
        e.settings.fill_column = 12;
        run(&mut d, &mut e, "fill-paragraph");
        let out = text(&e);
        assert!(
            out.starts_with("one two\nthree four\nfive six\nseven\n"),
            "got `{out}`"
        );
        assert!(
            out.ends_with("second paragraph\n"),
            "the second paragraph was untouched"
        );
    }

    #[test]
    fn filling_joins_short_lines_back_together() {
        let (mut d, mut e) = setup("notes.txt", "one\ntwo\nthree\n");
        e.settings.fill_column = 40;
        run(&mut d, &mut e, "fill-paragraph");
        assert_eq!(text(&e), "one two three\n");
    }

    #[test]
    fn filling_an_already_filled_paragraph_says_so() {
        let (mut d, mut e) = setup("notes.txt", "one two three\n");
        e.settings.fill_column = 40;
        run(&mut d, &mut e, "fill-paragraph");
        run(&mut d, &mut e, "fill-paragraph");
        assert_eq!(e.minibuffer.display(), "Paragraph is already filled");
    }

    #[test]
    fn filling_on_a_blank_line_does_nothing() {
        let (mut d, mut e) = setup("notes.txt", "\nparagraph\n");
        run(&mut d, &mut e, "fill-paragraph");
        assert_eq!(text(&e), "\nparagraph\n");
    }

    #[test]
    fn filling_a_region_wraps_just_that_span() {
        let (mut d, mut e) = setup("notes.txt", "aaa bbb ccc ddd\nlast line\n");
        e.settings.fill_column = 7;
        mark_region(&mut e, 0, 15);
        run(&mut d, &mut e, "fill-region");
        let out = text(&e);
        assert!(out.starts_with("aaa bbb\nccc ddd\n"), "got `{out}`");
        assert!(out.ends_with("last line\n"));
    }

    #[test]
    fn filling_a_region_needs_a_region() {
        let (mut d, mut e) = setup("notes.txt", "text\n");
        assert!(matches!(
            d.execute(&mut e, "fill-region", None),
            Dispatch::Failed { .. }
        ));
    }
}
