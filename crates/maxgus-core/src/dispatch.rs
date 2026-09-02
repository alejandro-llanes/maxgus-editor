//! Key dispatch.
//!
//! Keys accumulate into a sequence until the active keymaps resolve it to a
//! command, report it as still being a live prefix, or report it as unbound.
//! This is also where the bookkeeping Emacs commands rely on happens: the
//! prefix argument is cleared after every command that is not itself a prefix
//! command, and `last-command` is recorded so consecutive kills append.

use crate::{
    command::{Args, Registry},
    editor::Editor,
    prefix::Prefix,
};
use maxgus_keys::{Key, KeySequence, Lookup};

/// How many times one command may hand straight on to another before the
/// chain is treated as a loop.
pub const MAX_DEFERRED_CHAIN: usize = 8;

/// Commands that build up a prefix argument, and so must not clear it.
pub const PREFIX_COMMANDS: &[&str] = &["universal-argument", "digit-argument", "negative-argument"];

/// Commands that run again at every extra cursor.
///
/// A list rather than a rule, because there is no rule: a command that edits
/// or moves is what several cursors are for, and one that prompts, switches
/// buffer or splits a window means nothing done five times. Anything not
/// named here runs once and puts the extra cursors away, which is what
/// `multiple-cursors` does with a command it has not been told about.
pub const MULTI_CURSOR_COMMANDS: &[&str] = &[
    // Typing.
    "self-insert-command",
    "newline",
    "open-line",
    "delete-char",
    "delete-backward-char",
    "delete-horizontal-space",
    "just-one-space",
    "quoted-insert",
    "indent-for-tab-command",
    // Killing, which is also how a rename deletes the old word.
    "kill-line",
    "kill-whole-line",
    "kill-word",
    "backward-kill-word",
    "kill-region",
    "yank",
    // Moving, so the cursors can be lined up before they are typed at.
    "forward-char",
    "backward-char",
    "forward-word",
    "backward-word",
    "move-beginning-of-line",
    "move-end-of-line",
    "next-line",
    "previous-line",
    // Case, which is the other half of a rename.
    "upcase-word",
    "downcase-word",
    "capitalize-word",
    // Commenting a set of lines at once.
    "comment-line",
];

/// Commands that leave the extra cursors alone without being run at them.
///
/// The multi-cursor commands themselves, and the ones that would be absurd
/// run five times but should not put the cursors away either — a prefix
/// argument, or asking what a key does.
pub const KEEP_CURSORS_COMMANDS: &[&str] = &[
    "mark-next-like-this",
    "mark-previous-like-this",
    "mark-all-like-this",
    "cursor-at-next-line",
    "cursor-at-previous-line",
    "unmark-cursor",
    "universal-argument",
    "digit-argument",
    "negative-argument",
    "describe-key",
    "describe-bindings",
    "execute-extended-command",
];

/// Commands that append to the kill ring when run consecutively, which is what
/// makes repeated `C-k` collect one entry rather than many.
pub const KILL_COMMANDS: &[&str] = &[
    "kill-line",
    "kill-whole-line",
    "kill-word",
    "backward-kill-word",
    "kill-region",
    "kill-sexp",
    "zap-to-char",
];

/// What handling one key produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// A command ran.
    Executed { command: String },
    /// The sequence so far is a live prefix; `echo` is what to show.
    Prefix { echo: String },
    /// Nothing is bound to the sequence.
    Undefined { keys: String },
    /// A command ran and failed.
    Failed { command: String, message: String },
}

impl Dispatch {
    pub fn is_prefix(&self) -> bool {
        matches!(self, Dispatch::Prefix { .. })
    }

    pub fn command(&self) -> Option<&str> {
        match self {
            Dispatch::Executed { command } | Dispatch::Failed { command, .. } => Some(command),
            _ => None,
        }
    }
}

/// Accumulates keys and runs the commands they resolve to.
#[derive(Debug)]
pub struct Dispatcher {
    pub registry: Registry,
    /// Keys typed toward a binding that is not yet complete.
    pending: KeySequence,
}

impl Dispatcher {
    pub fn new(registry: Registry) -> Dispatcher {
        Dispatcher {
            registry,
            pending: KeySequence::empty(),
        }
    }

    /// The half-typed sequence, for the echo area.
    pub fn pending(&self) -> &KeySequence {
        &self.pending
    }

    /// Abandons any half-typed sequence, as `C-g` does.
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    /// Handles one key.
    pub fn handle_key(&mut self, editor: &mut Editor, key: Key) -> Dispatch {
        // A message is read once: the next key takes it off the echo area,
        // as Emacs does, so "Wrote main.rs" is not still there an hour later
        // looking like news. A prompt is not a message, and stays.
        if !editor.minibuffer.is_active() {
            editor.minibuffer.clear_message();
        }
        // A command waiting on `read-char` takes the next key whole, before
        // any keymap sees it.
        if let Some((command, prefix)) = editor.pending_char.take() {
            self.pending.clear();
            return self.execute_with(editor, &command, Args::with_read_char(prefix, key));
        }
        // Keys typed while recording go into the macro, except the ones that
        // end the recording — those are filtered when the macro is closed.
        if !editor.replaying_macro
            && let Some(keys) = editor.recording_macro.as_mut()
        {
            keys.push(key);
        }
        self.pending.push(key);
        // A terminal that cannot send Meta sends ESC first; fold it back so
        // `ESC x` finds the `M-x` binding.
        let sequence = self.pending.canonicalize_escape_prefix();

        match editor.keymaps.lookup(&sequence) {
            Lookup::Prefix => Dispatch::Prefix {
                echo: self.pending.notation(),
            },
            Lookup::Undefined => {
                let keys = self.pending.notation();
                self.pending.clear();
                // An unbound sequence also abandons the prefix argument.
                editor.prefix = Prefix::None;
                editor.last_command = None;
                Dispatch::Undefined { keys }
            }
            Lookup::Command(name) => {
                self.pending.clear();
                self.execute(editor, &name, Some(key))
            }
        }
    }

    /// Runs `name` as though it had been invoked from a key, doing the same
    /// bookkeeping. This is the path `M-x` and keyboard macros take.
    pub fn execute(&mut self, editor: &mut Editor, name: &str, key: Option<Key>) -> Dispatch {
        let args = Args::new(editor.prefix, key);
        self.execute_with(editor, name, args)
    }

    /// Runs `name` once at each extra cursor.
    ///
    /// Point is moved to the cursor, the command is run, and the cursor is
    /// updated to wherever point ended up — so a motion moves every cursor
    /// and an edit happens at each of them. The real point is put back at the
    /// end, shifted by whatever the edits did to the text before it.
    fn run_at_other_cursors(&mut self, editor: &mut Editor, name: &str, args: &Args) {
        let mut point = editor.windows.current().point;
        // Where the window was looking. Each run follows its cursor, and a
        // cursor off the bottom of the window scrolled it there and left it:
        // typing at three places sent the window to the last of them.
        let (top_line, top_row, left_column) = {
            let window = editor.windows.current();
            (window.top_line, window.top_row, window.left_column)
        };
        // Where each cursor ends up. Everything already in here is further
        // through the buffer than the cursor being run, so each edit moves
        // all of them — and the real point, when the edit is before it.
        let mut moved: Vec<usize> = Vec::new();
        for cursor in editor.cursors.descending() {
            editor.windows.current_mut().point = cursor;
            editor.with_current_buffer(|b| b.set_point(cursor));
            let length = editor.current_buffer().len_chars();

            // One cursor failing is not a reason to abandon the rest: a
            // `C-d` at the end of the buffer has nothing to delete and the
            // others still do.
            let _ = self.registry.execute(editor, name, args);

            let landed = editor.windows.current().point;
            let delta = editor.current_buffer().len_chars() as isize - length as isize;
            if delta != 0 {
                let at = cursor.min(landed);
                for offset in &mut moved {
                    *offset = crate::multi::shift_by_delta(*offset, at, delta);
                }
                point = crate::multi::shift_by_delta(point, at, delta);
            }
            moved.push(landed);
        }
        let length = editor.current_buffer().len_chars();
        editor.cursors.clear();
        for cursor in moved {
            editor.cursors.add(cursor.min(length), point.min(length));
        }
        let point = point.min(length);
        let window = editor.windows.current_mut();
        window.point = point;
        window.top_line = top_line;
        window.top_row = top_row;
        window.left_column = left_column;
        editor.with_current_buffer(move |b| b.set_point(point));
        editor.follow_point();
    }

    /// Runs `name` with fully specified arguments.
    pub fn execute_with(&mut self, editor: &mut Editor, name: &str, args: Args) -> Dispatch {
        // A kill appends only when the previous command was also a kill.
        editor.kill_appending = KILL_COMMANDS.contains(&name)
            && editor
                .last_command
                .as_deref()
                .is_some_and(|last| KILL_COMMANDS.contains(&last));
        editor.this_command = Some(name.to_string());

        // What the command is about to do has to be measured, because every
        // other cursor is an offset into the text it is about to change.
        let multi = !editor.cursors.is_empty() && MULTI_CURSOR_COMMANDS.contains(&name);
        let before = multi.then(|| {
            (
                editor.windows.current().point,
                editor.current_buffer().len_chars(),
            )
        });

        // Where the cursor was, so the beacon can tell whether it jumped.
        // This is where `beacon` hangs its own `post-command-hook`.
        let beacon_before = editor.settings.beacon.then(|| editor.beacon_watch());

        // A snippet's fields are offsets into text a command is about to
        // change, and have to move with it.
        let snippet_before = editor.in_snippet().then(|| {
            (
                editor.windows.current().point,
                editor.current_buffer().len_chars(),
            )
        });

        let outcome = self.registry.execute(editor, name, &args);

        if let Some((point, length)) = snippet_before {
            let delta = editor.current_buffer().len_chars() as isize - length as isize;
            if delta != 0 {
                let at = point.min(editor.windows.current().point);
                editor.shift_snippet_fields(at, delta);
            }
        }

        // The same command again at every other cursor, from the end of the
        // buffer backwards so an edit at one cannot move the ones still to
        // come. A command that cannot be run that way puts them away instead.
        if outcome.is_ok() && !editor.cursors.is_empty() {
            match before {
                Some((point, length)) => {
                    // The primary run has already changed the text under
                    // every cursor after it.
                    let after = editor.current_buffer().len_chars();
                    let delta = after as isize - length as isize;
                    if delta != 0 {
                        let at = point.min(editor.windows.current().point);
                        editor.cursors.shift_by(at, delta);
                    }
                    self.run_at_other_cursors(editor, name, &args);
                }
                None if !KEEP_CURSORS_COMMANDS.contains(&name) => editor.cursors.clear(),
                None => {}
            }
        }

        // A command that asked for a character keeps its argument until the
        // character arrives.
        let waiting = editor.pending_char.is_some();
        // Only the prefix-argument commands carry an argument forward.
        if !PREFIX_COMMANDS.contains(&name) && !waiting {
            editor.prefix = Prefix::None;
        }
        editor.last_command = editor.this_command.take();

        if let Some(before) = beacon_before {
            editor.consider_beacon(&before);
        }

        let mut result = match outcome {
            Ok(()) => Dispatch::Executed {
                command: name.to_string(),
            },
            Err(error) => {
                let message = error.to_string();
                editor.error(message.clone());
                Dispatch::Failed {
                    command: name.to_string(),
                    message,
                }
            }
        };

        // The popups that follow the cursor: the doc box and the list of
        // suggestions. Here rather than in a front end because they respond
        // to the command, not to the drawing — a window that forgot to call
        // them would have a list that never narrowed, and there would be
        // nothing in the editor to notice.
        follow_the_cursor(editor);

        // A command may hand control straight to another — accepting a prompt
        // re-enters whatever opened it. The chain is bounded so a command that
        // defers to itself cannot spin.
        for _ in 0..MAX_DEFERRED_CHAIN {
            let Some((next, args)) = editor.deferred.take() else {
                return result;
            };
            result = self.execute_with(editor, &next, args);
        }
        editor.deferred = None;
        editor.error("Command deferral chain is too long");
        result
    }

    /// Feeds a whole description such as `C-x C-f`, for tests and macros.
    pub fn handle_keys(&mut self, editor: &mut Editor, keys: &str) -> Dispatch {
        let sequence = KeySequence::parse(keys).expect("a well-formed key description");
        let mut last = Dispatch::Prefix {
            echo: String::new(),
        };
        for key in sequence.keys() {
            last = self.handle_key(editor, *key);
        }
        last
    }
}

/// Keeps the transient popups in step with the buffer, after any command.
fn follow_the_cursor(editor: &mut Editor) {
    dismiss_the_doc(editor);
    narrow_the_suggestions(editor);
}

/// The doc box goes as soon as the hand moves.
///
/// It reappears by itself once the cursor has rested somewhere new, which is
/// what makes it a thing that follows the cursor rather than a window that
/// has to be closed.
#[cfg(feature = "full")]
fn dismiss_the_doc(editor: &mut Editor) {
    let here = (editor.current_buffer_id(), editor.windows.current().point);
    // Except when the command did not move it — the reply may still be on
    // its way, and a box that flickers off on every keystroke is worse than
    // one that stays.
    if editor.doc_asked_at != Some(here) {
        editor.doc = None;
    }
}

#[cfg(not(feature = "full"))]
fn dismiss_the_doc(_: &mut Editor) {}

/// Keeps the suggestion list in step with what has been typed.
///
/// The list stays up while a word is being written and narrows to it —
/// which is the whole behaviour, and the reason its keymap takes only the
/// keys a list needs and lets every letter through to the buffer. It goes
/// when the word does: point left the word, another buffer was selected, or
/// nothing the server offered survives.
#[cfg(feature = "full")]
fn narrow_the_suggestions(editor: &mut Editor) {
    let Some(list) = editor.autocomplete.as_ref() else {
        return;
    };
    let (buffer, start) = (list.buffer, list.start);
    if buffer != editor.current_buffer_id() {
        editor.close_autocomplete();
        return;
    }
    let point = editor.windows.current().point;
    // Behind where the word began, or off the end of a word that has since
    // been broken by a space.
    let text = editor.current_buffer().text();
    if point < start || crate::autocomplete::word_start(&text, point) != start {
        editor.close_autocomplete();
        return;
    }
    let prefix: String = text.chars().skip(start).take(point - start).collect();
    if let Some(list) = editor.autocomplete.as_mut() {
        list.narrow(&prefix);
    }
    if editor.autocomplete.as_ref().is_some_and(|l| l.is_empty()) {
        editor.close_autocomplete();
    }
}

#[cfg(not(feature = "full"))]
fn narrow_the_suggestions(_: &mut Editor) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::{Result, command};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn editor() -> Editor {
        Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        )
    }

    fn note(editor: &mut Editor, args: &Args) -> Result<()> {
        let name = editor.this_command.clone().unwrap_or_default();
        editor.message(format!("{name} x{}", args.count()));
        Ok(())
    }

    fn record_key(editor: &mut Editor, args: &Args) -> Result<()> {
        editor.message(args.key.map(|k| k.notation()).unwrap_or_default());
        Ok(())
    }

    fn boom(_: &mut Editor, _: &Args) -> Result<()> {
        Err(crate::CoreError::Message("it broke".into()))
    }

    fn universal(editor: &mut Editor, _: &Args) -> Result<()> {
        editor.prefix = editor.prefix.universal();
        Ok(())
    }

    fn digit(editor: &mut Editor, args: &Args) -> Result<()> {
        // The digit comes from the key code: `M-1` carries a Meta bit, so
        // `as_char` — which only answers for unmodified keys — would not do.
        let d = match args.key.map(|k| k.code) {
            Some(maxgus_keys::KeyCode::Char(c)) => c.to_digit(10).unwrap_or(0),
            _ => 0,
        };
        editor.prefix = editor.prefix.digit(d);
        Ok(())
    }

    fn dispatcher() -> Dispatcher {
        let mut r = Registry::new();
        for (name, handler) in [
            ("forward-char", note as fn(&mut Editor, &Args) -> Result<()>),
            ("find-file", note),
            ("save-buffer", note),
            ("kill-line", note),
            ("kill-word", note),
            ("yank", note),
            ("execute-extended-command", note),
        ] {
            r.register(Command {
                name,
                doc: "Test command.",
                handler,
                interactive: true,
            });
        }
        r.register(command!(
            "self-insert-command",
            "Insert the key.",
            record_key
        ));
        r.register(command!("keyboard-quit", "Quit.", note));
        r.register(command!("explode", "Fail.", boom));
        r.register(command!(
            "universal-argument",
            "Prefix.",
            universal,
            non_interactive
        ));
        r.register(command!("digit-argument", "Digit.", digit, non_interactive));
        Dispatcher::new(r)
    }

    #[test]
    fn a_single_key_binding_runs_immediately() {
        let (mut d, mut e) = (dispatcher(), editor());
        let out = d.handle_keys(&mut e, "C-f");
        assert_eq!(
            out,
            Dispatch::Executed {
                command: "forward-char".into()
            }
        );
        assert!(d.pending().is_empty());
    }

    #[test]
    fn a_prefix_key_waits_for_the_rest_of_the_sequence() {
        let (mut d, mut e) = (dispatcher(), editor());
        let out = d.handle_keys(&mut e, "C-x");
        assert!(out.is_prefix());
        assert_eq!(out, Dispatch::Prefix { echo: "C-x".into() });
        assert_eq!(d.pending().notation(), "C-x");

        let out = d.handle_keys(&mut e, "C-f");
        assert_eq!(
            out,
            Dispatch::Executed {
                command: "find-file".into()
            }
        );
        assert!(d.pending().is_empty(), "the sequence completed");
    }

    #[test]
    fn the_echo_grows_as_a_long_sequence_is_typed() {
        let (mut d, mut e) = (dispatcher(), editor());
        assert_eq!(
            d.handle_keys(&mut e, "C-x"),
            Dispatch::Prefix { echo: "C-x".into() }
        );
        // `C-x 4` is a prefix in the default map.
        assert_eq!(
            d.handle_keys(&mut e, "4"),
            Dispatch::Prefix {
                echo: "C-x 4".into()
            }
        );
    }

    #[test]
    fn an_unbound_sequence_is_reported_and_clears_the_pending_keys() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-x");
        let out = d.handle_keys(&mut e, "C-z");
        assert_eq!(
            out,
            Dispatch::Undefined {
                keys: "C-x C-z".into()
            }
        );
        assert!(d.pending().is_empty());
    }

    #[test]
    fn a_printable_key_falls_through_to_self_insert() {
        let (mut d, mut e) = (dispatcher(), editor());
        let out = d.handle_keys(&mut e, "q");
        assert_eq!(
            out,
            Dispatch::Executed {
                command: "self-insert-command".into()
            }
        );
        assert_eq!(
            e.minibuffer.display(),
            "q",
            "the command saw which key it was"
        );
    }

    #[test]
    fn the_escape_prefix_reaches_a_meta_binding() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "ESC");
        let out = d.handle_keys(&mut e, "x");
        assert_eq!(
            out,
            Dispatch::Executed {
                command: "execute-extended-command".into()
            }
        );
    }

    #[test]
    fn a_failing_command_reports_and_shows_its_message() {
        let (mut d, mut e) = (dispatcher(), editor());
        let out = d.execute(&mut e, "explode", None);
        assert_eq!(
            out,
            Dispatch::Failed {
                command: "explode".into(),
                message: "it broke".into()
            }
        );
        assert_eq!(e.minibuffer.display(), "it broke");
        assert!(e.minibuffer.message_is_error());
    }

    #[test]
    fn an_unregistered_but_bound_command_fails_rather_than_panicking() {
        let mut d = Dispatcher::new(Registry::new());
        let mut e = editor();
        let out = d.handle_keys(&mut e, "C-f");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn a_prefix_argument_reaches_the_command_and_is_then_cleared() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-u");
        assert_eq!(
            e.prefix,
            Prefix::Universal(1),
            "the argument survives its own command"
        );
        d.handle_keys(&mut e, "C-f");
        assert_eq!(e.minibuffer.display(), "forward-char x4");
        assert_eq!(e.prefix, Prefix::None, "cleared after an ordinary command");
    }

    #[test]
    fn digits_accumulate_across_several_keys() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "M-1");
        d.handle_keys(&mut e, "M-2");
        assert_eq!(e.prefix.count(), 12);
        d.handle_keys(&mut e, "C-f");
        assert_eq!(e.minibuffer.display(), "forward-char x12");
    }

    #[test]
    fn an_unbound_key_abandons_the_prefix_argument() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-u");
        d.handle_keys(&mut e, "C-x");
        d.handle_keys(&mut e, "C-z");
        assert_eq!(e.prefix, Prefix::None);
    }

    #[test]
    fn consecutive_kills_append_and_a_gap_breaks_the_run() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-k");
        assert!(!e.kill_appending, "the first kill starts a new entry");

        d.handle_keys(&mut e, "C-k");
        assert!(e.kill_appending, "the second appends to it");

        d.handle_keys(&mut e, "M-d");
        assert!(e.kill_appending, "a different kill command still appends");

        d.handle_keys(&mut e, "C-f");
        d.handle_keys(&mut e, "C-k");
        assert!(!e.kill_appending, "an intervening command breaks the run");
    }

    #[test]
    fn last_command_is_recorded_for_the_next_one_to_see() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-y");
        assert_eq!(e.last_command.as_deref(), Some("yank"));
        assert_eq!(e.this_command, None, "cleared once the command finished");
    }

    #[test]
    fn a_failing_command_is_still_recorded_as_the_last_one() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.execute(&mut e, "explode", None);
        assert_eq!(e.last_command.as_deref(), Some("explode"));
    }

    #[test]
    fn resetting_abandons_a_half_typed_sequence() {
        let (mut d, mut e) = (dispatcher(), editor());
        d.handle_keys(&mut e, "C-x");
        assert!(!d.pending().is_empty());
        d.reset();
        assert!(d.pending().is_empty());
        // The next key starts fresh.
        assert_eq!(
            d.handle_keys(&mut e, "C-f"),
            Dispatch::Executed {
                command: "forward-char".into()
            }
        );
    }

    #[test]
    fn the_minibuffer_map_takes_over_while_a_prompt_is_open() {
        let (mut d, mut e) = (dispatcher(), editor());
        // Outside the minibuffer, `C-f` moves point.
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));

        e.prompt(crate::MinibufferKind::Command, "M-x ");
        assert_eq!(
            d.handle_keys(&mut e, "C-f").command(),
            Some("minibuffer-forward-char"),
            "the prompt's own binding wins"
        );
        assert_eq!(
            d.handle_keys(&mut e, "a").command(),
            Some("minibuffer-self-insert"),
            "printable keys go to the prompt"
        );

        e.abort_prompt();
        assert_eq!(
            d.handle_keys(&mut e, "C-f").command(),
            Some("forward-char"),
            "and back again"
        );
    }

    #[test]
    fn opening_a_prompt_twice_does_not_stack_the_map() {
        let (mut d, mut e) = (dispatcher(), editor());
        e.prompt(crate::MinibufferKind::Command, "M-x ");
        e.prompt(crate::MinibufferKind::File, "Find file: ");
        e.abort_prompt();
        assert_eq!(
            d.handle_keys(&mut e, "C-f").command(),
            Some("forward-char"),
            "one abort was enough to restore the global map"
        );
    }

    #[test]
    fn a_minor_map_can_be_pushed_and_popped_directly() {
        let (mut d, mut e) = (dispatcher(), editor());
        let mut map = maxgus_keys::Keymap::new("test-mode");
        map.define_str("C-f", "save-buffer").unwrap();
        e.push_minor_map(map);
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("save-buffer"));
        assert!(e.remove_minor_map("test-mode"));
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));
        assert!(
            !e.remove_minor_map("test-mode"),
            "removing twice is a no-op"
        );
    }
}
