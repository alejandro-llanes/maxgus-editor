//! Buffer commands: switching, killing, narrowing and the buffer list.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    window::Direction,
};
use maxgus_text::Range;

/// The name of the buffer `C-x C-b` builds.
pub const BUFFER_LIST_NAME: &str = "*Buffer List*";

/// Registers the buffer commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "kill-buffer-in-all-windows",
            "Kill this buffer, and show something else wherever it was.",
            kill_in_all_windows
        ),
        command!(
            "switch-to-buffer",
            "Display another buffer in this window.",
            switch_to_buffer
        ),
        command!(
            "switch-to-buffer-other-window",
            "Display another buffer in a new window.",
            switch_other_window
        ),
        command!("kill-buffer", "Kill a buffer.", kill_buffer),
        command!("list-buffers", "Show a list of every buffer.", list_buffers),
        command!(
            "next-buffer",
            "Display the next buffer in the list.",
            next_buffer
        ),
        command!(
            "previous-buffer",
            "Display the previous buffer in the list.",
            previous_buffer
        ),
        command!(
            "read-only-mode",
            "Toggle whether this buffer can be edited.",
            read_only_mode
        ),
        command!(
            "rename-buffer",
            "Give this buffer another name.",
            rename_buffer
        ),
        command!(
            "narrow-to-region",
            "Restrict editing to the region.",
            narrow_to_region
        ),
        command!(
            "narrow-to-defun",
            "Restrict editing to the enclosing definition.",
            narrow_to_defun
        ),
        command!("widen", "Remove any restriction on editing.", widen),
    ]);
}

/// Prompts for a buffer name, offering the most recently used one as the
/// default — the way `C-x b` does.
fn prompt_for_buffer(editor: &mut Editor, command: &str, verb: &str) {
    let current = editor.current_buffer_id();
    let default = editor
        .buffers
        .other(current)
        .and_then(|id| editor.buffers.get(id))
        .map(|b| b.name().to_string());
    let prompt = match &default {
        Some(name) => format!("{verb} (default {name}): "),
        None => format!("{verb}: "),
    };
    let candidates = editor.buffers.visible_names();
    editor.prompt_for(command, MinibufferKind::Buffer, prompt, "", candidates);
}

/// Resolves a name typed at a buffer prompt. An empty answer means the
/// default; a name that does not exist creates the buffer, as Emacs does.
fn resolve_buffer(editor: &mut Editor, name: &str) -> maxgus_text::BufferId {
    if name.is_empty() {
        let current = editor.current_buffer_id();
        if let Some(other) = editor.buffers.other(current) {
            return other;
        }
        return current;
    }
    match editor.buffers.find_by_name(name) {
        Some(id) => id,
        None => editor.buffers.create(name),
    }
}

fn switch_to_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        prompt_for_buffer(editor, "switch-to-buffer", "Switch to buffer");
        return Ok(());
    };
    let id = resolve_buffer(editor, &name);
    editor.switch_to_buffer(id)
}

fn switch_other_window(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        prompt_for_buffer(
            editor,
            "switch-to-buffer-other-window",
            "Switch to buffer in other window",
        );
        return Ok(());
    };
    let id = resolve_buffer(editor, &name);
    // Reuse an existing second window rather than piling up splits.
    if editor.windows.len() < 2 {
        editor.split_window(Direction::Vertical)?;
    }
    editor.other_window(1);
    editor.switch_to_buffer(id)
}

fn kill_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let current = editor.current_buffer().name().to_string();
        let candidates = editor.buffers.visible_names();
        editor.prompt_for(
            "kill-buffer",
            MinibufferKind::Buffer,
            format!("Kill buffer (default {current}): "),
            "",
            candidates,
        );
        return Ok(());
    };
    let id = if name.is_empty() {
        editor.current_buffer_id()
    } else {
        editor
            .buffers
            .find_by_name(&name)
            .ok_or_else(|| crate::CoreError::Message(format!("No buffer named `{name}`")))?
    };
    // Refuse to discard unsaved work without being told twice.
    let unsaved = editor
        .buffers
        .get(id)
        .is_some_and(|b| b.is_modified() && b.path().is_some());
    if unsaved && !args.prefix.is_present() {
        let name = editor
            .buffers
            .get(id)
            .expect("checked above")
            .name()
            .to_string();
        return Err(crate::CoreError::Message(format!(
            "Buffer `{name}` has unsaved changes; C-u C-x k kills it anyway"
        )));
    }
    editor.kill_buffer(id)?;
    Ok(())
}

/// `C-x C-b`: builds a read-only listing, in the same columns Emacs uses.
fn list_buffers(editor: &mut Editor, _: &Args) -> Result<()> {
    let mut listing = String::from("CRM Buffer                Size  Mode         File\n");
    listing.push_str(&"-".repeat(72));
    listing.push('\n');
    let current = editor.current_buffer_id();
    for id in editor.buffers.ids().to_vec() {
        let Some(buffer) = editor.buffers.get(id) else {
            continue;
        };
        let mode = editor.mode_name(id);
        let flags = format!(
            "{}{}{} ",
            if buffer.id == current { '.' } else { ' ' },
            if buffer.is_read_only() { '%' } else { ' ' },
            if buffer.is_modified() { '*' } else { ' ' },
        );
        listing.push_str(&format!(
            "{flags}{:<20} {:>5}  {:<12} {}\n",
            buffer.name(),
            buffer.len_chars(),
            mode,
            buffer
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ));
    }

    // Reuse the listing buffer rather than making a new one each time.
    let id = match editor.buffers.find_by_name(BUFFER_LIST_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &listing)?;
            id
        }
        None => editor.buffers.create_with_text(BUFFER_LIST_NAME, &listing),
    };
    editor
        .buffers
        .get_mut(id)
        .expect("just created")
        .set_read_only(true);
    editor.switch_to_buffer(id)
}

fn step_buffer(editor: &mut Editor, forward: bool, count: usize) -> Result<()> {
    let mut id = editor.current_buffer_id();
    for _ in 0..count {
        let next = if forward {
            editor.buffers.next(id)
        } else {
            editor.buffers.previous(id)
        };
        match next {
            Some(next) => id = next,
            None => return Err(crate::CoreError::Message("No other buffer".into())),
        }
    }
    // Stepping through the list must not reorder it, or the walk would loop
    // between two buffers; so the window changes without touching the order.
    editor.sync_to_buffer();
    let point = editor.buffers.get(id).map(|b| b.point()).unwrap_or(0);
    let window = editor.windows.current_mut();
    window.buffer = id;
    window.point = point;
    window.top_line = 0;
    editor.follow_point();
    Ok(())
}

fn next_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    step_buffer(editor, true, args.count())
}

fn previous_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    step_buffer(editor, false, args.count())
}

fn read_only_mode(editor: &mut Editor, args: &Args) -> Result<()> {
    let now = editor.with_current_buffer(|b| {
        // A prefix argument sets rather than toggles: positive turns it on.
        let next = if args.prefix.is_present() {
            args.signed_count() > 0
        } else {
            !b.is_read_only()
        };
        b.set_read_only(next);
        next
    });
    editor.message(if now {
        "Buffer is read-only"
    } else {
        "Buffer is writable"
    });
    Ok(())
}

fn rename_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let current = editor.current_buffer().name().to_string();
        editor.prompt_for(
            "rename-buffer",
            MinibufferKind::Text,
            "Rename buffer to: ",
            &current,
            Vec::new(),
        );
        return Ok(());
    };
    if name.is_empty() {
        return Err(crate::CoreError::Message(
            "Buffer name cannot be empty".into(),
        ));
    }
    let id = editor.current_buffer_id();
    let actual = editor.buffers.rename(id, &name)?;
    editor.message(format!("Buffer renamed to `{actual}`"));
    Ok(())
}

fn narrow_to_region(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = editor.region()?;
    if range.is_empty() {
        return Err(crate::CoreError::Message("Region is empty".into()));
    }
    editor.with_current_buffer(|b| {
        b.narrow(range);
        b.deactivate_mark();
    });
    editor.follow_point();
    Ok(())
}

fn narrow_to_defun(editor: &mut Editor, _: &Args) -> Result<()> {
    let range = {
        let buffer = editor.current_buffer();
        let point = buffer.point();
        let start = maxgus_text::Motion::beginning_of_defun(buffer.rope(), point + 1);
        let end = maxgus_text::Motion::end_of_defun(buffer.rope(), point);
        Range::new(start, end.max(start))
    };
    if range.is_empty() {
        return Err(crate::CoreError::Message(
            "No definition around point".into(),
        ));
    }
    editor.with_current_buffer(|b| b.narrow(range));
    editor.follow_point();
    Ok(())
}

fn widen(editor: &mut Editor, _: &Args) -> Result<()> {
    let was = editor.with_current_buffer(|b| {
        let was = b.is_narrowed();
        b.widen();
        was
    });
    if !was {
        return Err(crate::CoreError::Message("Buffer is not narrowed".into()));
    }
    editor.follow_point();
    Ok(())
}

/// `C-x K`: kills the buffer and leaves no window showing it.
fn kill_in_all_windows(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    let name = editor.current_buffer().name().to_string();
    editor.kill_buffer(id)?;
    editor.message(format!("Killed {name}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, Prefix};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup() -> (Dispatcher, Editor) {
        let editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let mut registry = Registry::new();
        register(&mut registry);
        super::super::minibuffer::register(&mut registry);
        super::super::motion::register(&mut registry);
        super::super::edit::register(&mut registry);
        super::super::window::register(&mut registry);
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

    /// Runs `command`, then answers its prompt with `answer`.
    fn run_answering(d: &mut Dispatcher, e: &mut Editor, command: &str, answer: &str) {
        d.execute(e, command, None);
        assert!(e.minibuffer.is_active(), "`{command}` should have prompted");
        for c in answer.chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(e, "RET");
    }

    #[test]
    fn every_buffer_binding_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for name in [
            "switch-to-buffer",
            "kill-buffer",
            "list-buffers",
            "read-only-mode",
            "widen",
        ] {
            assert!(registry.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn switching_prompts_and_then_switches() {
        let (mut d, mut e) = setup();
        e.buffers.create("notes");
        run_answering(&mut d, &mut e, "switch-to-buffer", "notes");
        assert_eq!(e.current_buffer().name(), "notes");
    }

    #[test]
    fn the_prompt_offers_the_most_recent_other_buffer_as_the_default() {
        let (mut d, mut e) = setup();
        let notes = e.buffers.create("notes");
        e.switch_to_buffer(notes).unwrap();
        let scratch = e.buffers.find_by_name(crate::SCRATCH_NAME).unwrap();
        e.switch_to_buffer(scratch).unwrap();

        d.execute(&mut e, "switch-to-buffer", None);
        assert!(
            e.minibuffer.prompt().contains("default notes"),
            "got `{}`",
            e.minibuffer.prompt()
        );

        // An empty answer takes the default.
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.current_buffer().name(), "notes");
    }

    #[test]
    fn switching_to_a_name_that_does_not_exist_creates_it() {
        let (mut d, mut e) = setup();
        run_answering(&mut d, &mut e, "switch-to-buffer", "brand-new");
        assert_eq!(e.current_buffer().name(), "brand-new");
        assert!(e.current_buffer().is_empty());
    }

    #[test]
    fn the_buffer_prompt_completes_against_existing_names() {
        let (mut d, mut e) = setup();
        e.buffers.create("notes-one");
        e.buffers.create("notes-two");
        d.execute(&mut e, "switch-to-buffer", None);
        d.handle_keys(&mut e, "n");
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "notes-", "the common prefix");
    }

    #[test]
    fn switching_in_another_window_splits_once_and_reuses_it() {
        let (mut d, mut e) = setup();
        e.buffers.create("notes");
        run_answering(&mut d, &mut e, "switch-to-buffer-other-window", "notes");
        assert_eq!(e.windows.len(), 2);
        assert_eq!(e.current_buffer().name(), "notes");

        e.buffers.create("more");
        run_answering(&mut d, &mut e, "switch-to-buffer-other-window", "more");
        assert_eq!(e.windows.len(), 2, "no second split");
    }

    #[test]
    fn killing_prompts_and_defaults_to_the_current_buffer() {
        let (mut d, mut e) = setup();
        let notes = e.buffers.create("notes");
        e.switch_to_buffer(notes).unwrap();
        d.execute(&mut e, "kill-buffer", None);
        assert!(e.minibuffer.prompt().contains("default notes"));
        d.handle_keys(&mut e, "RET");
        assert!(e.buffers.find_by_name("notes").is_none());
    }

    #[test]
    fn killing_an_unknown_buffer_says_so() {
        let (mut d, mut e) = setup();
        e.buffers.create("other");
        d.execute(&mut e, "kill-buffer", None);
        for c in "nonexistent".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn killing_a_buffer_with_unsaved_changes_needs_confirming() {
        let (mut d, mut e) = setup();
        let id = e.buffers.visit_file("/tmp/a.rs", "original");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("edited").unwrap());

        d.execute(&mut e, "kill-buffer", None);
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
        assert!(e.buffers.get(id).is_some(), "the buffer survived");

        // A prefix argument kills it anyway.
        e.prefix = Prefix::Universal(1);
        d.execute(&mut e, "kill-buffer", None);
        d.handle_keys(&mut e, "RET");
        assert!(e.buffers.get(id).is_none());
    }

    #[test]
    fn an_unmodified_file_buffer_is_killed_without_confirmation() {
        let (mut d, mut e) = setup();
        let id = e.buffers.visit_file("/tmp/a.rs", "original");
        e.switch_to_buffer(id).unwrap();
        d.execute(&mut e, "kill-buffer", None);
        d.handle_keys(&mut e, "RET");
        assert!(e.buffers.get(id).is_none());
    }

    #[test]
    fn the_buffer_list_shows_every_buffer_and_is_read_only() {
        let (mut d, mut e) = setup();
        e.buffers.visit_file("/project/main.rs", "fn main() {}");
        e.buffers.create("notes");

        run(&mut d, &mut e, "list-buffers");
        assert_eq!(e.current_buffer().name(), BUFFER_LIST_NAME);
        assert!(e.current_buffer().is_read_only());

        let text = e.current_buffer().text();
        assert!(text.contains("main.rs"), "got `{text}`");
        assert!(text.contains("notes"), "got `{text}`");
        assert!(text.contains("/project/main.rs"), "the file column");
        assert!(text.contains("rust"), "the mode column");
    }

    #[test]
    fn listing_twice_reuses_the_same_buffer() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "list-buffers");
        run(&mut d, &mut e, "list-buffers");
        assert_eq!(
            e.buffers
                .iter()
                .filter(|b| b.name() == BUFFER_LIST_NAME)
                .count(),
            1,
            "no `*Buffer List*<2>`"
        );
    }

    #[test]
    fn the_listing_marks_the_current_and_modified_buffers() {
        let (mut d, mut e) = setup();
        let id = e.buffers.visit_file("/tmp/a.rs", "");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("edited").unwrap());
        run(&mut d, &mut e, "list-buffers");
        let text = e.current_buffer().text();
        let line = text.lines().find(|l| l.contains("a.rs")).unwrap();
        assert!(line.contains('*'), "the modified flag, got `{line}`");
    }

    #[test]
    fn stepping_walks_the_buffer_list_without_reordering_it() {
        let (mut d, mut e) = setup();
        e.buffers.create("a");
        e.buffers.create("b");
        let order_before = e.buffers.ids().to_vec();

        run(&mut d, &mut e, "next-buffer");
        let after_one = e.current_buffer_id();
        run(&mut d, &mut e, "next-buffer");
        assert_ne!(e.current_buffer_id(), after_one);
        assert_eq!(
            e.buffers.ids(),
            order_before.as_slice(),
            "the order is untouched"
        );

        run(&mut d, &mut e, "previous-buffer");
        assert_eq!(e.current_buffer_id(), after_one);
    }

    #[test]
    fn read_only_mode_toggles_and_can_be_set_explicitly() {
        let (mut d, mut e) = setup();
        assert!(!e.current_buffer().is_read_only());
        run(&mut d, &mut e, "read-only-mode");
        assert!(e.current_buffer().is_read_only());
        assert_eq!(e.minibuffer.display(), "Buffer is read-only");
        run(&mut d, &mut e, "read-only-mode");
        assert!(!e.current_buffer().is_read_only());

        e.prefix = Prefix::Numeric(1);
        d.execute(&mut e, "read-only-mode", None);
        assert!(e.current_buffer().is_read_only());
        e.prefix = Prefix::Numeric(-1);
        d.execute(&mut e, "read-only-mode", None);
        assert!(!e.current_buffer().is_read_only());
    }

    #[test]
    fn renaming_starts_from_the_current_name_and_uniquifies() {
        let (mut d, mut e) = setup();
        e.buffers.create("taken");
        let id = e.buffers.create("original");
        e.switch_to_buffer(id).unwrap();

        d.execute(&mut e, "rename-buffer", None);
        assert_eq!(
            e.minibuffer.input(),
            "original",
            "pre-filled with the current name"
        );
        e.minibuffer.kill_whole();
        for c in "taken".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.current_buffer().name(), "taken<2>");
    }

    #[test]
    fn renaming_to_nothing_is_refused() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "rename-buffer", None);
        e.minibuffer.kill_whole();
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn narrowing_restricts_editing_to_the_region() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("test", "0123456789");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| {
            b.set_point(2);
            b.set_mark(2);
            b.set_point(6);
        });

        run(&mut d, &mut e, "narrow-to-region");
        assert!(e.current_buffer().is_narrowed());
        assert_eq!(e.current_buffer().point_min(), 2);
        assert_eq!(e.current_buffer().point_max(), 6);

        run(&mut d, &mut e, "widen");
        assert!(!e.current_buffer().is_narrowed());
        assert_eq!(e.current_buffer().point_max(), 10);
    }

    #[test]
    fn narrowing_needs_a_region() {
        let (mut d, mut e) = setup();
        assert!(fails(&mut d, &mut e, "narrow-to-region").contains("mark"));
    }

    #[test]
    fn narrowing_to_an_empty_region_is_refused() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("test", "0123456789");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| {
            b.set_point(3);
            b.set_mark(3);
        });
        assert!(fails(&mut d, &mut e, "narrow-to-region").contains("empty"));
    }

    #[test]
    fn widening_a_buffer_that_is_not_narrowed_says_so() {
        let (mut d, mut e) = setup();
        assert!(fails(&mut d, &mut e, "widen").contains("not narrowed"));
    }

    #[test]
    fn narrowing_to_a_definition_covers_it() {
        let (mut d, mut e) = setup();
        let source = "fn a() {\n    one\n}\n\nfn b() {\n    two\n}\n";
        let id = e.buffers.create_with_text("test.rs", source);
        e.switch_to_buffer(id).unwrap();
        let inside = e.current_buffer().line_start(5) + 2;
        e.with_current_buffer(|b| b.set_point(inside));

        run(&mut d, &mut e, "narrow-to-defun");
        let buffer = e.current_buffer();
        assert!(buffer.is_narrowed());
        let visible = buffer.slice(Range::new(buffer.point_min(), buffer.point_max()));
        assert!(visible.contains("fn b"), "got `{visible}`");
        assert!(!visible.contains("fn a"), "got `{visible}`");
    }
}
