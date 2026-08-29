//! Register commands: the `C-x r` family.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_text::{Range, Register};

/// Registers the register commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "point-to-register",
            "Record point in a register.",
            point_to_register
        ),
        command!(
            "jump-to-register",
            "Go to the position in a register.",
            jump_to_register
        ),
        command!(
            "copy-to-register",
            "Copy the region into a register.",
            copy_to_register
        ),
        command!(
            "insert-register",
            "Insert a register's contents.",
            insert_register
        ),
        command!(
            "number-to-register",
            "Store a number in a register.",
            number_to_register
        ),
        command!(
            "increment-register",
            "Add to the number in a register.",
            increment_register
        ),
        command!(
            "copy-rectangle-to-register",
            "Copy a rectangle into a register.",
            copy_rectangle
        ),
        command!(
            "list-registers",
            "Show what every register holds.",
            list_registers
        ),
    ]);
}

/// Every register command takes a one-character name; this asks for it.
fn ask_for_register(editor: &mut Editor, command: &str, verb: &str) {
    editor.prompt_for(
        command,
        MinibufferKind::Char,
        format!("{verb} to register: "),
        "",
        Vec::new(),
    );
}

/// The register name from an answered prompt.
fn register_name(args: &Args) -> Result<char> {
    args.input
        .as_ref()
        .and_then(|s| s.chars().next())
        .ok_or_else(|| crate::CoreError::Message("No register named".into()))
}

fn point_to_register(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.input.is_none() {
        ask_for_register(editor, "point-to-register", "Point");
        return Ok(());
    }
    let key = register_name(args)?;
    editor.sync_to_buffer();
    let (buffer, position, offset) = {
        let buffer = editor.current_buffer();
        let offset = buffer.point();
        (
            buffer.name().to_string(),
            buffer.position_of(offset),
            offset,
        )
    };
    editor.registers.set(
        key,
        Register::Position {
            buffer,
            position,
            offset,
        },
    );
    editor.message(format!("Point saved in register {key}"));
    Ok(())
}

fn jump_to_register(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.input.is_none() {
        ask_for_register(editor, "jump-to-register", "Jump");
        return Ok(());
    }
    let key = register_name(args)?;
    let Some(Register::Position { buffer, offset, .. }) = editor.registers.get(key).cloned() else {
        return Err(crate::CoreError::Message(format!(
            "Register {key} holds no position"
        )));
    };
    let Some(id) = editor.buffers.find_by_name(&buffer) else {
        return Err(crate::CoreError::Message(format!(
            "Buffer `{buffer}` is gone"
        )));
    };
    editor.switch_to_buffer(id)?;
    // The old position goes on the mark ring, so the jump can be undone.
    editor.with_current_buffer(|b| {
        let from = b.point();
        b.push_mark(from);
        b.set_point(offset);
    });
    editor.follow_point();
    Ok(())
}

fn copy_to_register(editor: &mut Editor, args: &Args) -> Result<()> {
    // The region is read before prompting, so it is the one the user could see.
    let range = editor.region()?;
    if args.input.is_none() {
        ask_for_register(editor, "copy-to-register", "Copy");
        return Ok(());
    }
    let key = register_name(args)?;
    let text = editor.current_buffer().slice(range);
    let length = text.chars().count();
    editor.registers.set(key, Register::Text(text));
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message(format!("Copied {length} characters to register {key}"));
    Ok(())
}

fn insert_register(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.input.is_none() {
        ask_for_register(editor, "insert-register", "Insert");
        return Ok(());
    }
    let key = register_name(args)?;
    let text = editor.registers.text_of(key)?;
    editor.with_current_buffer(|b| {
        let at = b.point();
        b.insert_at_point(&text)?;
        // The mark records where the insertion began.
        b.set_mark_inactive(at);
        Ok::<(), maxgus_text::TextError>(())
    })?;
    editor.follow_point();
    Ok(())
}

fn number_to_register(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.input.is_none() {
        ask_for_register(editor, "number-to-register", "Number");
        return Ok(());
    }
    let key = register_name(args)?;
    // The prefix argument is the number, defaulting to zero as in Emacs.
    let value = if args.prefix.is_present() {
        args.prefix.count() as i64
    } else {
        0
    };
    editor.registers.set(key, Register::Number(value));
    editor.message(format!("Register {key} now holds {value}"));
    Ok(())
}

fn increment_register(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.input.is_none() {
        ask_for_register(editor, "increment-register", "Increment");
        return Ok(());
    }
    let key = register_name(args)?;
    let by = if args.prefix.is_present() {
        args.prefix.count() as i64
    } else {
        1
    };
    let value = editor.registers.increment(key, by)?;
    editor.message(format!("Register {key} now holds {value}"));
    Ok(())
}

/// `C-x r r`: takes the rectangle between point and the mark.
fn copy_rectangle(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = editor.region()?;
    if args.input.is_none() {
        ask_for_register(editor, "copy-rectangle-to-register", "Copy rectangle");
        return Ok(());
    }
    let key = register_name(args)?;
    let rows = rectangle_rows(editor, range);
    let count = rows.len();
    editor.registers.set(key, Register::Rectangle(rows));
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message(format!("Copied {count} rows to register {key}"));
    Ok(())
}

/// The rows of the rectangle `range` spans, taken between the display columns
/// of its two corners.
fn rectangle_rows(editor: &Editor, range: Range) -> Vec<String> {
    let buffer = editor.current_buffer();
    let (first_line, last_line) = (buffer.line_of(range.start), buffer.line_of(range.end));
    let left = buffer.display_column(range.start);
    let right = buffer.display_column(range.end);
    let (left, right) = (left.min(right), left.max(right));

    (first_line..=last_line)
        .map(|line| {
            let from = buffer.offset_at_display_column(line, left);
            let to = buffer.offset_at_display_column(line, right);
            buffer.slice(Range::new(from.min(to), to.max(from)))
        })
        .collect()
}

fn list_registers(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.registers.is_empty() {
        editor.message("No registers are set");
        return Ok(());
    }
    let mut text = String::from("Registers\n\n");
    for (key, _) in editor.registers.iter() {
        if let Some(line) = editor.registers.describe(key) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    super::help::show_help(editor, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, Prefix};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup(text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.create_with_text("test", text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));
        let registry = crate::commands::standard_registry();
        editor.command_names = registry.interactive_names();
        (Dispatcher::new(registry), editor)
    }

    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
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

    /// Runs a register command and answers its one-character prompt.
    fn run_with_register(d: &mut Dispatcher, e: &mut Editor, command: &str, key: &str) {
        d.execute(e, command, None);
        assert!(
            e.minibuffer.is_active(),
            "`{command}` should have asked for a register"
        );
        d.handle_keys(e, key);
    }

    fn mark_region(e: &mut Editor, from: usize, to: usize) {
        e.with_current_buffer(|b| {
            b.set_point(from);
            b.set_mark(from);
            b.set_point(to);
        });
    }

    #[test]
    fn the_register_prompt_takes_a_single_character() {
        let (mut d, mut e) = setup("text");
        d.execute(&mut e, "point-to-register", None);
        assert!(e.minibuffer.prompt().contains("register"));
        d.handle_keys(&mut e, "a");
        assert!(!e.minibuffer.is_active(), "one key was enough");
        assert!(e.registers.get('a').is_some());
    }

    #[test]
    fn point_can_be_saved_and_jumped_back_to() {
        let (mut d, mut e) = setup("0123456789");
        e.with_current_buffer(|b| b.set_point(6));
        run_with_register(&mut d, &mut e, "point-to-register", "a");
        assert!(e.minibuffer.display().contains("Point saved"));

        e.with_current_buffer(|b| b.set_point(0));
        run_with_register(&mut d, &mut e, "jump-to-register", "a");
        assert_eq!(e.windows.current().point, 6);
        assert_eq!(
            e.current_buffer().mark(),
            Some(0),
            "the old position was marked"
        );
    }

    #[test]
    fn jumping_to_an_empty_register_says_so() {
        let (mut d, mut e) = setup("text");
        d.execute(&mut e, "jump-to-register", None);
        let out = d.handle_keys(&mut e, "z");
        assert!(matches!(out, Dispatch::Failed { .. }));
        assert!(e.minibuffer.display().contains("no position"));
    }

    #[test]
    fn jumping_to_a_register_whose_buffer_is_gone_says_so() {
        let (mut d, mut e) = setup("text");
        run_with_register(&mut d, &mut e, "point-to-register", "a");
        let id = e.current_buffer_id();
        e.buffers.rename(id, "renamed").unwrap();
        d.execute(&mut e, "jump-to-register", None);
        let out = d.handle_keys(&mut e, "a");
        assert!(matches!(out, Dispatch::Failed { .. }));
        assert!(e.minibuffer.display().contains("is gone"));
    }

    #[test]
    fn the_region_can_be_copied_into_a_register_and_inserted_back() {
        let (mut d, mut e) = setup("hello world");
        mark_region(&mut e, 0, 5);
        run_with_register(&mut d, &mut e, "copy-to-register", "t");
        assert!(e.minibuffer.display().contains("Copied 5 characters"));
        assert_eq!(
            e.current_buffer().text(),
            "hello world",
            "the text stayed put"
        );

        e.with_current_buffer(|b| b.set_point(11));
        run_with_register(&mut d, &mut e, "insert-register", "t");
        assert_eq!(e.current_buffer().text(), "hello worldhello");
        assert_eq!(
            e.current_buffer().mark(),
            Some(11),
            "the insertion start was marked"
        );
    }

    #[test]
    fn copying_needs_a_region() {
        let (mut d, mut e) = setup("text");
        assert!(fails(&mut d, &mut e, "copy-to-register").contains("mark"));
    }

    #[test]
    fn inserting_an_empty_register_says_so() {
        let (mut d, mut e) = setup("text");
        d.execute(&mut e, "insert-register", None);
        let out = d.handle_keys(&mut e, "z");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn numbers_can_be_stored_and_incremented() {
        let (mut d, mut e) = setup("");
        e.prefix = Prefix::Numeric(10);
        d.execute(&mut e, "number-to-register", None);
        d.handle_keys(&mut e, "n");
        assert!(e.minibuffer.display().contains("holds 10"));

        run_with_register(&mut d, &mut e, "increment-register", "n");
        assert!(e.minibuffer.display().contains("holds 11"));

        e.prefix = Prefix::Numeric(5);
        d.execute(&mut e, "increment-register", None);
        d.handle_keys(&mut e, "n");
        assert!(e.minibuffer.display().contains("holds 16"));
    }

    #[test]
    fn a_number_register_defaults_to_zero() {
        let (mut d, mut e) = setup("");
        run_with_register(&mut d, &mut e, "number-to-register", "n");
        assert!(e.minibuffer.display().contains("holds 0"));
    }

    #[test]
    fn a_number_register_can_be_inserted_as_text() {
        let (mut d, mut e) = setup("");
        e.prefix = Prefix::Numeric(42);
        d.execute(&mut e, "number-to-register", None);
        d.handle_keys(&mut e, "n");
        run_with_register(&mut d, &mut e, "insert-register", "n");
        assert_eq!(e.current_buffer().text(), "42");
    }

    #[test]
    fn incrementing_a_text_register_is_refused() {
        let (mut d, mut e) = setup("hello");
        mark_region(&mut e, 0, 5);
        run_with_register(&mut d, &mut e, "copy-to-register", "t");
        d.execute(&mut e, "increment-register", None);
        let out = d.handle_keys(&mut e, "t");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn a_rectangle_takes_the_same_columns_from_every_row() {
        let (mut d, mut e) = setup("abcdef\nghijkl\nmnopqr\n");
        // Columns 2..4 of the first three lines.
        e.with_current_buffer(|b| {
            b.set_point(2);
            b.set_mark(2);
            b.set_point(14 + 4);
        });
        run_with_register(&mut d, &mut e, "copy-rectangle-to-register", "r");
        assert!(e.minibuffer.display().contains("Copied 3 rows"));

        e.with_current_buffer(|b| {
            let end = b.len_chars();
            b.set_point(end);
        });
        run_with_register(&mut d, &mut e, "insert-register", "r");
        assert!(
            e.current_buffer().text().ends_with("cd\nij\nop"),
            "got `{}`",
            e.current_buffer().text()
        );
    }

    #[test]
    fn a_rectangle_over_one_line_is_just_that_span() {
        let (mut d, mut e) = setup("abcdef\n");
        mark_region(&mut e, 1, 4);
        run_with_register(&mut d, &mut e, "copy-rectangle-to-register", "r");
        assert_eq!(e.registers.text_of('r').unwrap(), "bcd");
    }

    #[test]
    fn registers_can_be_listed() {
        let (mut d, mut e) = setup("hello");
        assert!(e.registers.is_empty());
        run(&mut d, &mut e, "list-registers");
        assert_eq!(e.minibuffer.display(), "No registers are set");

        mark_region(&mut e, 0, 5);
        run_with_register(&mut d, &mut e, "copy-to-register", "t");
        e.prefix = Prefix::Numeric(7);
        d.execute(&mut e, "number-to-register", None);
        d.handle_keys(&mut e, "n");

        run(&mut d, &mut e, "list-registers");
        let text = e.current_buffer().text();
        assert!(text.contains("t: text \"hello\""), "got `{text}`");
        assert!(text.contains("n: number 7"), "got `{text}`");
    }
}
