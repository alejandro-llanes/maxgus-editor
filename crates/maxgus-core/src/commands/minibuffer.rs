//! Minibuffer commands.
//!
//! These are what the minibuffer keymap binds while a prompt is open. They
//! edit the prompt line rather than the buffer, and `RET` hands the collected
//! text back to whichever command asked for it.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
};

/// Registers the minibuffer commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!("minibuffer-self-insert", "Insert the typed character into the prompt.", self_insert, non_interactive),
        command!("minibuffer-complete-and-exit", "Accept what has been typed.", complete_and_exit, non_interactive),
        command!("minibuffer-keyboard-quit", "Abandon the prompt.", keyboard_quit, non_interactive),
        command!("minibuffer-complete", "Complete what has been typed.", complete, non_interactive),
        command!("minibuffer-complete-backward", "Cycle completions backwards.", complete_backward, non_interactive),
        command!("minibuffer-complete-word", "Complete, or insert a space.", complete_word, non_interactive),
        command!("minibuffer-delete-backward-char", "Delete the character before point.", delete_backward, non_interactive),
        command!("minibuffer-delete-char", "Delete the character after point.", delete_forward, non_interactive),
        command!("minibuffer-beginning-of-line", "Move to the start of the prompt.", beginning_of_line, non_interactive),
        command!("minibuffer-end-of-line", "Move to the end of the prompt.", end_of_line, non_interactive),
        command!("minibuffer-forward-char", "Move forward one character.", forward_char, non_interactive),
        command!("minibuffer-backward-char", "Move backward one character.", backward_char, non_interactive),
        command!("minibuffer-kill-line", "Kill to the end of the prompt.", kill_line, non_interactive),
        command!("minibuffer-backward-kill-word", "Kill the word before point.", backward_kill_word, non_interactive),
        command!("minibuffer-previous-history", "Recall the previous entry.", previous_history, non_interactive),
        command!("minibuffer-next-history", "Recall the next entry.", next_history, non_interactive),
        command!("minibuffer-yank", "Insert the most recent kill.", yank, non_interactive),
        command!(
            "minibuffer-next-candidate",
            "Move down the candidate list, or forward through the history.",
            next_candidate,
            non_interactive
        ),
        command!(
            "minibuffer-previous-candidate",
            "Move up the candidate list, or back through the history.",
            previous_candidate,
            non_interactive
        ),
        command!(
            "minibuffer-next-candidate-page",
            "Move a page down the candidate list.",
            next_candidate_page,
            non_interactive
        ),
        command!(
            "minibuffer-previous-candidate-page",
            "Move a page up the candidate list.",
            previous_candidate_page,
            non_interactive
        ),
    ]);
}

fn self_insert(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(c) = args.key.and_then(|k| k.as_char()) else {
        return Ok(());
    };
    editor.minibuffer.insert_char(c);
    // A single-key prompt answers as soon as the character arrives.
    if editor.minibuffer.kind().is_some_and(|k| k.is_single_key()) {
        return complete_and_exit(editor, args);
    }
    editor.refresh_completions();
    Ok(())
}

/// `RET`: closes the prompt and re-enters the command that opened it.
fn complete_and_exit(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(text) = editor.accept_prompt() else {
        return Ok(());
    };
    let Some((command, prefix)) = editor.pending_input.take() else {
        // A prompt opened with no continuation just closes.
        return Ok(());
    };
    // The command is re-entered through the same path a key would take, so it
    // sees the prefix argument it was originally invoked with.
    editor.deferred = Some((command, Args::with_input(prefix, text)));
    Ok(())
}

/// How many rows the popup shows, which is how far a page moves.
fn page(editor: &Editor) -> isize {
    editor.completion_rows().max(1) as isize
}

/// The arrows walk the candidate list when there is one, and the history when
/// there is not — so they stay useful in a prompt that does not complete.
fn move_candidate(editor: &mut Editor, delta: isize) -> Result<()> {
    if editor.minibuffer.completion().visible && editor.minibuffer.move_selection(delta) {
        return Ok(());
    }
    match delta > 0 {
        true => next_history(editor, &Args::new(crate::Prefix::None, None)),
        false => previous_history(editor, &Args::new(crate::Prefix::None, None)),
    }
}

fn next_candidate(editor: &mut Editor, _: &Args) -> Result<()> {
    move_candidate(editor, 1)
}

fn previous_candidate(editor: &mut Editor, _: &Args) -> Result<()> {
    move_candidate(editor, -1)
}

fn next_candidate_page(editor: &mut Editor, _: &Args) -> Result<()> {
    let page = page(editor);
    move_candidate(editor, page)
}

fn previous_candidate_page(editor: &mut Editor, _: &Args) -> Result<()> {
    let page = page(editor);
    move_candidate(editor, -page)
}

fn keyboard_quit(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.abort_prompt();
    Ok(())
}

/// TAB: completes against whatever the prompt was given.
fn complete(editor: &mut Editor, _: &Args) -> Result<()> {
    if !editor.minibuffer.kind().is_some_and(|k| k.completes()) {
        return Ok(());
    }
    let candidates = editor.completion_candidates.clone();
    if candidates.is_empty() {
        editor.minibuffer.show_error("No completions");
        return Ok(());
    }
    // Once TAB has put the list up, TAB cycles through it. A list that is
    // merely *visible* — every completing prompt shows one from the moment it
    // opens — must not turn this first TAB into a cycle.
    if editor.minibuffer.completion().cycling {
        editor.minibuffer.cycle_completion(true);
        return Ok(());
    }
    if !editor.minibuffer.complete(&candidates) && editor.minibuffer.completion().is_empty() {
        editor.minibuffer.show_error("No match");
    }
    Ok(())
}

fn complete_backward(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.cycle_completion(false);
    Ok(())
}

/// `SPC`: completes in file and buffer prompts, inserts a space elsewhere.
fn complete_word(editor: &mut Editor, args: &Args) -> Result<()> {
    match editor.minibuffer.kind() {
        Some(kind) if kind.completes() && kind != crate::MinibufferKind::File => {
            complete(editor, args)
        }
        _ => {
            // File names may contain spaces, so a space is just a space.
            editor.minibuffer.insert_char(' ');
            Ok(())
        }
    }
}

fn delete_backward(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.delete_backward();
    editor.refresh_completions();
    Ok(())
}

fn delete_forward(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.delete_forward();
    editor.refresh_completions();
    Ok(())
}

fn beginning_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.move_start();
    Ok(())
}

fn end_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.move_end();
    Ok(())
}

fn forward_char(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.move_right();
    Ok(())
}

fn backward_char(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.move_left();
    Ok(())
}

fn kill_line(editor: &mut Editor, _: &Args) -> Result<()> {
    let killed = editor.minibuffer.kill_to_end();
    if !killed.is_empty() {
        editor.kill_ring.kill_new(killed);
    }
    editor.refresh_completions();
    Ok(())
}

fn backward_kill_word(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.minibuffer.delete_word_backward();
    editor.refresh_completions();
    Ok(())
}

fn previous_history(editor: &mut Editor, _: &Args) -> Result<()> {
    if !editor.minibuffer.history_previous() {
        editor.minibuffer.show_error("Beginning of history");
    }
    editor.refresh_completions();
    Ok(())
}

fn next_history(editor: &mut Editor, _: &Args) -> Result<()> {
    if !editor.minibuffer.history_next() {
        editor.minibuffer.show_error("End of history");
    }
    editor.refresh_completions();
    Ok(())
}

fn yank(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(text) = editor.kill_ring.front().map(str::to_string) else {
        return Err(crate::CoreError::Message("Kill ring is empty".into()));
    };
    // A prompt is one line; a multi-line kill is flattened into it.
    editor.minibuffer.insert(&text.replace('\n', " "));
    editor.refresh_completions();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, MinibufferKind, Prefix};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    /// A command that records what it was handed, so the round trip through a
    /// prompt can be observed.
    fn echo_input(editor: &mut Editor, args: &Args) -> Result<()> {
        match &args.input {
            Some(text) => {
                editor.message(format!("got `{text}` x{}", args.count()));
                Ok(())
            }
            None => {
                editor.prompt_for(
                    "echo-input",
                    MinibufferKind::Command,
                    "Echo: ",
                    "",
                    vec!["save-buffer".into(), "save-some-buffers".into(), "find-file".into()],
                );
                Ok(())
            }
        }
    }

    fn setup() -> (Dispatcher, Editor) {
        let editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let mut registry = Registry::new();
        register(&mut registry);
        super::super::motion::register(&mut registry);
        super::super::edit::register(&mut registry);
        registry.register(command!("echo-input", "Echo the input.", echo_input));
        (Dispatcher::new(registry), editor)
    }

    #[test]
    fn every_minibuffer_binding_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for (keys, command) in crate::keymap::MINIBUFFER_BINDINGS {
            assert!(registry.contains(command), "`{keys}` runs unregistered `{command}`");
        }
        assert!(registry.contains("minibuffer-self-insert"), "the fallback binding");
    }

    #[test]
    fn a_prompt_collects_text_and_hands_it_to_the_command() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        assert!(e.minibuffer.is_active());
        assert_eq!(e.minibuffer.display(), "Echo: ");

        for key in ["h", "i"] {
            d.handle_keys(&mut e, key);
        }
        assert_eq!(e.minibuffer.input(), "hi");

        d.handle_keys(&mut e, "RET");
        assert!(!e.minibuffer.is_active());
        assert_eq!(e.minibuffer.display(), "got `hi` x1");
    }

    #[test]
    fn the_prefix_argument_survives_the_round_trip() {
        let (mut d, mut e) = setup();
        e.prefix = Prefix::Numeric(4);
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "x");
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.minibuffer.display(), "got `x` x4");
    }

    #[test]
    fn quitting_a_prompt_abandons_the_waiting_command() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "x");
        d.handle_keys(&mut e, "C-g");
        assert!(!e.minibuffer.is_active());
        assert_eq!(e.minibuffer.display(), "Quit");
        assert!(e.pending_input.is_none(), "the command was dropped");
    }

    #[test]
    fn the_global_map_returns_once_the_prompt_closes() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("minibuffer-forward-char"));
        d.handle_keys(&mut e, "RET");
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));
    }

    #[test]
    fn editing_commands_work_on_the_prompt_line() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        for key in ["a", "b", "c"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "C-a");
        assert_eq!(e.minibuffer.point(), 0);
        d.handle_keys(&mut e, "C-d");
        assert_eq!(e.minibuffer.input(), "bc");
        d.handle_keys(&mut e, "C-e");
        d.handle_keys(&mut e, "DEL");
        assert_eq!(e.minibuffer.input(), "b");
        d.handle_keys(&mut e, "C-f");
        d.handle_keys(&mut e, "C-b");
        assert_eq!(e.minibuffer.point(), 0);
    }

    #[test]
    fn killing_the_prompt_line_puts_the_text_on_the_kill_ring() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        for key in ["a", "b", "c"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "C-a");
        d.handle_keys(&mut e, "C-k");
        assert_eq!(e.minibuffer.input(), "");
        assert_eq!(e.kill_ring.front(), Some("abc"));
        d.handle_keys(&mut e, "C-y");
        assert_eq!(e.minibuffer.input(), "abc");
    }

    #[test]
    fn a_multi_line_kill_is_flattened_into_the_prompt() {
        let (mut d, mut e) = setup();
        e.kill_ring.kill_new("one\ntwo");
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "C-y");
        assert_eq!(e.minibuffer.input(), "one two");
    }

    #[test]
    fn word_deletion_works_on_the_prompt() {
        let (mut d, mut e) = setup();
        // A file prompt, where SPC inserts a space rather than completing.
        e.prompt_for("echo-input", MinibufferKind::File, "Find file: ", "", Vec::new());
        for key in ["a", "b", "SPC", "c", "d"] {
            d.handle_keys(&mut e, key);
        }
        assert_eq!(e.minibuffer.input(), "ab cd");
        d.handle_keys(&mut e, "M-DEL");
        assert_eq!(e.minibuffer.input(), "ab ");
    }

    #[test]
    fn tab_completes_against_the_supplied_candidates() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        for key in ["s", "a"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "save-", "the common prefix of two matches");
    }

    #[test]
    fn a_second_tab_shows_and_then_cycles_the_candidates() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        for key in ["s", "a", "TAB"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "TAB");
        assert!(e.minibuffer.completion().visible);
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "save-buffer");
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "save-some-buffers");
        d.handle_keys(&mut e, "S-TAB");
        assert_eq!(e.minibuffer.input(), "save-buffer", "and backwards");
    }

    #[test]
    fn completing_a_unique_prefix_fills_it_in() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "f");
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "find-file");
    }

    #[test]
    fn completing_nothing_that_matches_says_so() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "z");
        d.handle_keys(&mut e, "TAB");
        assert!(e.minibuffer.message_is_error());
    }

    #[test]
    fn history_is_recalled_across_prompts() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        for key in ["o", "n", "e"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "RET");

        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "M-p");
        assert_eq!(e.minibuffer.input(), "one");
        d.handle_keys(&mut e, "M-n");
        assert_eq!(e.minibuffer.input(), "", "back to what was typed");
    }

    #[test]
    fn walking_past_the_end_of_history_says_so() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "M-p");
        assert!(e.minibuffer.message_is_error());
    }

    #[test]
    fn a_space_completes_in_a_command_prompt_but_not_a_file_prompt() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "echo-input", None);
        d.handle_keys(&mut e, "s");
        d.handle_keys(&mut e, "SPC");
        assert_eq!(e.minibuffer.input(), "save-", "SPC completed");

        e.abort_prompt();
        e.prompt_for("echo-input", MinibufferKind::File, "Find file: ", "", Vec::new());
        d.handle_keys(&mut e, "a");
        d.handle_keys(&mut e, "SPC");
        assert_eq!(e.minibuffer.input(), "a ", "file names may contain spaces");
    }

    #[test]
    fn a_single_key_prompt_answers_on_the_first_character() {
        let (mut d, mut e) = setup();
        e.prompt_for("echo-input", MinibufferKind::Char, "Register: ", "", Vec::new());
        d.handle_keys(&mut e, "a");
        assert!(!e.minibuffer.is_active(), "no RET needed");
        assert_eq!(e.minibuffer.display(), "got `a` x1");
    }

    #[test]
    fn a_prompt_with_no_waiting_command_simply_closes() {
        let (mut d, mut e) = setup();
        e.prompt(MinibufferKind::Text, "Anything: ");
        d.handle_keys(&mut e, "x");
        let out = d.handle_keys(&mut e, "RET");
        assert!(!matches!(out, Dispatch::Failed { .. }));
        assert!(!e.minibuffer.is_active());
    }

    #[test]
    fn a_prompt_can_start_pre_filled() {
        let (mut d, mut e) = setup();
        e.prompt_for("echo-input", MinibufferKind::File, "Find file: ", "/tmp/", Vec::new());
        assert_eq!(e.minibuffer.input(), "/tmp/");
        d.handle_keys(&mut e, "a");
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.minibuffer.display(), "got `/tmp/a` x1");
    }
}
