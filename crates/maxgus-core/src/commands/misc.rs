//! The commands that do not belong to any one family: prefix arguments,
//! `M-x`, keyboard macros, shell commands and leaving the editor.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    prefix::Prefix,
    task::Task,
};
use maxgus_keys::{Key, KeyCode};

/// The buffer shell output is collected into.
pub const SHELL_OUTPUT_NAME: &str = "*Shell Command Output*";

/// Registers the miscellaneous commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "toggle-line-numbers",
            "Show or hide the line-number column.",
            toggle_line_numbers
        ),
        command!(
            "toggle-fill-column-indicator",
            "Show or hide the rule at the fill column.",
            toggle_fill_column_indicator
        ),
        command!(
            "toggle-indent-style",
            "Indent with tabs instead of spaces, or the other way round.",
            toggle_indent_style
        ),
        command!(
            "toggle-truncate-lines",
            "Wrap long lines, or clip them at the edge.",
            toggle_truncate_lines
        ),
        command!(
            "text-scale-increase",
            "Draw the text a step larger, in a window.",
            text_scale_increase
        ),
        command!(
            "text-scale-decrease",
            "Draw the text a step smaller, in a window.",
            text_scale_decrease
        ),
        command!(
            "text-scale-reset",
            "Draw the text at its configured size again.",
            text_scale_reset
        ),
        command!(
            "toggle-frame-fullscreen",
            "Fill the screen with the window, or give it back.",
            toggle_frame_fullscreen
        ),
        command!(
            "universal-argument",
            "Begin a prefix argument.",
            universal_argument,
            non_interactive
        ),
        command!(
            "digit-argument",
            "Add a digit to the prefix argument.",
            digit_argument,
            non_interactive
        ),
        command!(
            "negative-argument",
            "Make the prefix argument negative.",
            negative_argument,
            non_interactive
        ),
        command!(
            "execute-extended-command",
            "Run a command by name.",
            execute_extended_command
        ),
        command!(
            "keyboard-escape-quit",
            "Abandon whatever is in progress.",
            escape_quit,
            non_interactive
        ),
        command!("suspend-maxgus", "Suspend the editor.", suspend),
        command!(
            "save-session",
            "Remember what is open, for the next time this project is opened.",
            save_session
        ),
        command!(
            "restore-session",
            "Open what was open the last time this project was.",
            restore_session
        ),
        command!(
            "startup-time",
            "Report how long the editor took to start.",
            startup_time
        ),
        command!(
            "edit-configuration",
            "Open the configuration file.",
            edit_configuration
        ),
        command!(
            "browse-files",
            "Open a file by looking: a box that narrows as you type.",
            browse_files
        ),
        command!(
            "browse-files-quit",
            "Close the file browser.",
            browse_quit,
            non_interactive
        ),
        command!(
            "browse-files-next",
            "Down one in the file browser.",
            browse_next,
            non_interactive
        ),
        command!(
            "browse-files-previous",
            "Up one in the file browser.",
            browse_previous,
            non_interactive
        ),
        command!(
            "browse-files-first",
            "To the top of the file browser.",
            browse_first,
            non_interactive
        ),
        command!(
            "browse-files-last",
            "To the bottom of the file browser.",
            browse_last,
            non_interactive
        ),
        command!(
            "browse-files-enter",
            "Go into the directory under the cursor.",
            browse_enter,
            non_interactive
        ),
        command!(
            "browse-files-up",
            "Go to the directory above this one.",
            browse_up,
            non_interactive
        ),
        command!(
            "browse-files-search",
            "Search every directory under the home directory.",
            browse_search,
            non_interactive
        ),
        command!(
            "browse-files-open",
            "Open what the cursor is on, or go into it.",
            browse_open,
            non_interactive
        ),
        command!(
            "browse-files-rub-out",
            "Rub out a character, or go up when there is none.",
            browse_rub_out,
            non_interactive
        ),
        command!(
            "browse-files-self-insert",
            "Narrow the browser by the typed character.",
            browse_self_insert,
            non_interactive
        ),
        command!("load-theme", "Switch to another theme.", load_theme),
        command!(
            "consult-theme",
            "Try each theme in turn and keep one. With a prefix argument, \
             also write it into the configuration file.",
            consult_theme
        ),
        command!(
            "save-theme",
            "Write the theme in use into the configuration file.",
            save_theme
        ),
        command!("repeat", "Run the last command again.", repeat),
        command!(
            "kmacro-start-macro",
            "Begin recording a keyboard macro.",
            start_macro
        ),
        command!(
            "kmacro-end-macro",
            "Stop recording a keyboard macro.",
            end_macro
        ),
        command!(
            "kmacro-end-and-call-macro",
            "Stop recording, or replay the last macro.",
            end_and_call_macro
        ),
        command!(
            "shell-command",
            "Run a shell command and show its output.",
            shell_command
        ),
        command!(
            "shell-command-on-region",
            "Pass the region through a shell command.",
            shell_command_on_region
        ),
    ]);
}

// ---- prefix arguments ---------------------------------------------------

fn universal_argument(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.prefix = editor.prefix.universal();
    let echo = editor.prefix.echo();
    editor.message(echo);
    Ok(())
}

fn digit_argument(editor: &mut Editor, args: &Args) -> Result<()> {
    // The digit comes from the key code: `M-1` carries a Meta bit, so the
    // "unmodified character" accessor would not answer for it.
    let digit = match args.key.map(|k| k.code) {
        Some(KeyCode::Char(c)) => c.to_digit(10),
        _ => None,
    };
    let Some(digit) = digit else {
        return Err(crate::CoreError::Message("That key is not a digit".into()));
    };
    editor.prefix = editor.prefix.digit(digit);
    let echo = editor.prefix.echo();
    editor.message(echo);
    Ok(())
}

fn negative_argument(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.prefix = editor.prefix.minus();
    let echo = editor.prefix.echo();
    editor.message(echo);
    Ok(())
}

// ---- M-x ----------------------------------------------------------------

/// `M-x`: prompts for a command name and runs it.
///
/// The command is handed back through the deferred slot rather than called
/// here, so it goes through exactly the same path a key binding would — same
/// prefix argument, same `last-command` bookkeeping.
fn execute_extended_command(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "execute-extended-command",
            MinibufferKind::Command,
            "M-x ",
            "",
            editor.command_names.clone(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No command given".into()));
    }
    if !editor.command_names.contains(&name) {
        return Err(crate::CoreError::Message(format!(
            "No command named `{name}`"
        )));
    }
    editor.deferred = Some((name, Args::new(args.prefix, None)));
    Ok(())
}

/// `ESC ESC ESC`: the escape hatch that gets out of anything.
fn escape_quit(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.minibuffer.is_active() {
        editor.abort_prompt();
        return Ok(());
    }
    if editor.isearch.is_some() {
        editor.isearch = None;
        editor.remove_minor_map("isearch-mode");
        editor.message("Quit");
        return Ok(());
    }
    if editor.windows.len() > 1 {
        editor.windows.delete_others();
        return Ok(());
    }
    editor.prefix = Prefix::None;
    editor.with_current_buffer(|b| b.deactivate_mark());
    editor.message("Quit");
    Ok(())
}

/// `C-c e`: opens the configuration file, creating it if it is not there.
///
/// Creating it matters: the usual reason to reach for this key is that there
/// is no configuration yet, and being told the file does not exist is not
/// what anybody wanted from it.
/// Where this project's session is kept, when there is anywhere to keep it.
fn session_path(editor: &Editor) -> Result<std::path::PathBuf> {
    let state = editor
        .state_dir
        .clone()
        .ok_or_else(|| crate::CoreError::Message("There is nowhere to keep a session".into()))?;
    Ok(crate::session::path_for(&state, &editor.project_root()))
}

fn save_session(editor: &mut Editor, _: &Args) -> Result<()> {
    let session = editor.session();
    if session.is_empty() {
        return Err(crate::CoreError::Message("No files are open".into()));
    }
    let path = session_path(editor)?;
    editor.spawn(crate::task::Task::SaveSession {
        path,
        contents: session.to_kdl(),
    });
    Ok(())
}

fn restore_session(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = session_path(editor)?;
    editor.spawn(crate::task::Task::ReadSession { path });
    Ok(())
}

/// `M-x startup-time`, which is `emacs-init-time` by another name.
fn startup_time(editor: &mut Editor, _: &Args) -> Result<()> {
    match editor.startup_time {
        Some(elapsed) => {
            let text = format!("maxgus started in {}", crate::human_duration(elapsed));
            editor.message(text);
            Ok(())
        }
        // A session that was never started by the binary — a test, or an
        // embedding — has nothing to report and should say so.
        None => Err(crate::CoreError::Message(
            "The startup time is not known".into(),
        )),
    }
}

fn edit_configuration(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = editor
        .config_path
        .clone()
        .ok_or_else(|| crate::CoreError::Message("There is no configuration file".into()))?;
    match editor.buffers.find_by_path(&path) {
        Some(id) => editor.switch_to_buffer(id),
        None => {
            editor.spawn(crate::task::Task::ReadFile {
                path,
                reverting: None,
                other_window: false,
            });
            Ok(())
        }
    }
}

fn suspend(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.suspend = true;
    Ok(())
}

/// `C-x z`: runs whatever ran last, once more.
///
/// The dispatcher reads `last_command` out of `this_command` only *after* the
/// command body returns, so what is read here is genuinely the command before
/// this one, and deferring is enough — the deferred call sets `last_command`
/// back to the repeated command, so pressing the key again repeats it again.
fn repeat(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(last) = editor.last_command.clone() else {
        return Err(crate::CoreError::Message("Nothing to repeat".into()));
    };
    // Belt and braces: the ordering above means this cannot normally happen,
    // and a chain of repeats repeating itself is worth refusing outright.
    if last == "repeat" {
        return Err(crate::CoreError::Message("Nothing to repeat".into()));
    }
    editor.deferred = Some((last, Args::new(args.prefix, args.key)));
    Ok(())
}

// ---- the file browser ---------------------------------------------------

/// `M-x browse-files`: open a file by looking rather than by spelling.
///
/// `C-x C-f` is for when you know the path. This is for when you know
/// roughly where it is: a box over the frame, narrowing as you type, walked
/// with the arrows. Right goes into a directory and left comes back out.
fn browse_files(editor: &mut Editor, _: &Args) -> Result<()> {
    let start = editor.default_directory().to_path_buf();
    editor.browser = Some(crate::browser::Browser::opening(&start));
    editor.push_minor_map(
        crate::keymap::browse_keymap().expect("the built-in browse map is well formed"),
    );
    editor.spawn(Task::Browse { path: start });
    Ok(())
}

/// Closes it, and takes its keymap away with it.
///
/// The command waiting on an answer goes too. A box opened to ask which
/// directory and then abandoned has no answer to give, and a command left
/// pending would be run by whatever prompt came next.
pub fn browse_quit(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.browser.take().is_some() {
        editor.remove_minor_map("browse-files-mode");
        editor.pending_input = None;
    }
    Ok(())
}

/// Reads `path` into the browser, keeping what was typed.
fn browse_to(editor: &mut Editor, path: std::path::PathBuf) {
    if let Some(browser) = editor.browser.as_mut() {
        browser.pending = true;
        browser.clear_filter();
    }
    editor.spawn(Task::Browse { path });
}

fn browse_next(editor: &mut Editor, args: &Args) -> Result<()> {
    if let Some(browser) = editor.browser.as_mut() {
        browser.next(args.prefix.positive_count());
    }
    Ok(())
}

fn browse_previous(editor: &mut Editor, args: &Args) -> Result<()> {
    if let Some(browser) = editor.browser.as_mut() {
        browser.previous(args.prefix.positive_count());
    }
    Ok(())
}

fn browse_first(editor: &mut Editor, _: &Args) -> Result<()> {
    if let Some(browser) = editor.browser.as_mut() {
        browser.goto_first();
    }
    Ok(())
}

fn browse_last(editor: &mut Editor, _: &Args) -> Result<()> {
    if let Some(browser) = editor.browser.as_mut() {
        browser.goto_last();
    }
    Ok(())
}

/// Right: into the directory under the cursor, and nothing on a file —
/// there is nothing to go *into*, and opening it would be a surprise from
/// an arrow key.
fn browse_enter(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(browser) = editor.browser.as_ref() else {
        return Ok(());
    };
    if !browser.current_is_dir() {
        return Ok(());
    }
    let Some(path) = browser.current_path() else {
        return Ok(());
    };
    // `.` is where we already are. Reading it again would only throw away
    // the filter and the cursor to arrive back here.
    if path == browser.directory {
        return Ok(());
    }
    browse_to(editor, path);
    Ok(())
}

/// `C-s`: widen the search to every directory under the home directory.
///
/// Walking is the wrong way to reach somewhere that is not under where you
/// started — out to a common ancestor and back down the other side, one
/// press per level, when you already know the name. This hands the box the
/// whole tree at once, listed by path relative to home, and typing narrows
/// across all of it. `←` comes back out of the search.
fn browse_search(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(browser) = editor.browser.as_ref() else {
        return Ok(());
    };
    if !browser.is_choosing() {
        // The other box opens a file, and a walk for those would turn up
        // every file under a home directory. `C-x C-f` is the command that
        // takes a path.
        return Err(crate::CoreError::Message(
            "Searching wide is for choosing a directory".into(),
        ));
    }
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Err(crate::CoreError::Message(
            "There is no HOME to search".into(),
        ));
    };
    if let Some(browser) = editor.browser.as_mut() {
        browser.searching(&home);
    }
    editor.spawn(Task::FindDirectories { root: home });
    Ok(())
}

/// Left: out to the directory above, wherever the cursor happens to be.
fn browse_up(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(browser) = editor.browser.as_ref() else {
        return Ok(());
    };
    // Out of a search is back to the directory it was searching, not up out
    // of it: widening the search was not a move, so leaving it should not be
    // one either.
    let to = match browser.searched {
        true => Some(browser.directory.clone()),
        false => browser.parent(),
    };
    let Some(to) = to else {
        return Ok(());
    };
    browse_to(editor, to);
    Ok(())
}

/// `RET`: open a file, or go into a directory — or, when a directory is
/// what was asked for, answer with the one under the cursor.
///
/// The two meanings do not collide, because the two boxes do not list the
/// same rows. Asked for a file, every row but a directory is an answer and
/// `RET` on a directory can only sensibly descend. Asked for a directory,
/// every row is an answer and descending is what the right arrow is for.
fn browse_open(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(browser) = editor.browser.as_ref() else {
        return Ok(());
    };
    // A path typed in full answers before the listing does: somebody who
    // has pasted one is not asking about what is on the screen.
    let typed = browser.typed_path().map(str::to_string);
    let Some(path) = typed
        .clone()
        .map(std::path::PathBuf::from)
        .or_else(|| browser.current_path())
    else {
        return Err(crate::CoreError::Message("Nothing to open".into()));
    };
    if browser.is_choosing() {
        // `..` moves, it does not answer. It is the row you press to get
        // *out* of somewhere, and answering with the directory above is
        // never what pressing it meant — the way up is what it is for, and
        // the parent can still be chosen once you are standing in it, on
        // `.`. This is the one row where `RET` and the left arrow agree.
        if typed.is_none() && browser.current() == Some(crate::browser::Row::Parent) {
            return browse_up(editor, &Args::default());
        }
        let answer = typed.unwrap_or_else(|| path.to_string_lossy().into_owned());
        // Before the quit, which drops the command waiting on it.
        let waiting = editor.pending_input.take();
        browse_quit(editor, &Args::default())?;
        if let Some((command, prefix)) = waiting {
            editor.deferred = Some((command, Args::with_input(prefix, answer)));
        }
        return Ok(());
    }
    if browser.current_is_dir() {
        browse_to(editor, path);
        return Ok(());
    }
    browse_quit(editor, &Args::default())?;
    editor.spawn(Task::ReadFile {
        path,
        reverting: None,
        other_window: false,
    });
    Ok(())
}

/// Backspace: a character off the filter, or up a directory when there is
/// none — which is what backspace does in every file browser there is.
fn browse_rub_out(editor: &mut Editor, args: &Args) -> Result<()> {
    let rubbed = editor
        .browser
        .as_mut()
        .is_some_and(|browser| browser.rub_out());
    if rubbed {
        return Ok(());
    }
    browse_up(editor, args)
}

fn browse_self_insert(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(c) = args.key.and_then(|key| key.as_char()) else {
        return Ok(());
    };
    if let Some(browser) = editor.browser.as_mut() {
        browser.type_char(c);
    }
    Ok(())
}

/// `M-x consult-theme`: walk the themes, seeing each one as you go.
///
/// The list is up from the moment it opens and every theme is applied as it
/// comes under the cursor, so choosing one is a matter of looking rather
/// than of remembering what the names mean. `RET` keeps what is showing and
/// that is the end of it; `C-g` puts back the one you started with.
///
/// It used to ask, on `RET`, whether to write the choice into the
/// configuration file — which put a yes-or-no question between someone and
/// a theme they had already chosen by looking at it. Trying themes on and
/// deciding to keep one for good are two different intentions, and only the
/// first of them is what this command is for. `save-theme` is the second,
/// and a prefix argument does both at once for anyone who knew all along.
///
/// Only a real theme is taken. A name that is not one leaves the theme
/// where it started rather than half-applying something that does not
/// exist, and an empty answer means the one already in use — so `RET`
/// straight away changes nothing.
fn consult_theme(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let candidates = editor.theme_names();
        let current = editor.settings.theme.clone();
        editor.theme_before_preview = Some(current.clone());
        editor.consult_theme_writes = args.prefix.is_present();
        editor.prompt_for(
            "consult-theme",
            MinibufferKind::Choice,
            format!("Consult theme (default {current}): "),
            "",
            candidates,
        );
        return Ok(());
    };
    let before = editor
        .theme_before_preview
        .clone()
        .unwrap_or_else(|| editor.settings.theme.clone());
    let writes = std::mem::take(&mut editor.consult_theme_writes);

    let name = match input.trim() {
        "" => before.clone(),
        typed => typed.to_string(),
    };
    // Checked before the preview is let go of, so a name that is not a theme
    // puts back the one that was showing instead of leaving it applied.
    if !editor.theme_names().contains(&name) {
        editor.end_theme_preview(true);
        return Err(crate::CoreError::Message(format!(
            "No theme named `{name}`"
        )));
    }
    // Accepted, so the preview stands rather than being undone.
    editor.end_theme_preview(false);
    editor.set_theme(&name)?;

    if writes {
        return persist_theme(editor, &name);
    }
    // Said only when there is something to say: a theme the configuration
    // already names will be there tomorrow whatever anyone does now.
    match editor.config_says_theme.as_deref() == Some(name.as_str()) {
        true => editor.message(format!("Theme {name}")),
        false => editor.message(format!("Theme {name} — `save-theme` keeps it")),
    }
    Ok(())
}

/// `M-x save-theme`: write the theme in use into the configuration file.
///
/// The other half of `consult-theme`, and useful on its own: a theme arrived
/// at by `load-theme`, or by editing the file and thinking better of it, is
/// kept by the same command.
///
/// Only the one setting is rewritten. The rest of the file — the comments,
/// the ordering, the keymaps — is left exactly as it was.
fn save_theme(editor: &mut Editor, _: &Args) -> Result<()> {
    let name = editor.settings.theme.clone();
    persist_theme(editor, &name)
}

fn persist_theme(editor: &mut Editor, name: &str) -> Result<()> {
    let Some(path) = editor.config_path.clone() else {
        return Err(crate::CoreError::Message(
            "There is no configuration file to write to".into(),
        ));
    };
    editor.spawn(Task::PersistTheme {
        path,
        theme: name.to_string(),
    });
    Ok(())
}

fn load_theme(editor: &mut Editor, args: &Args) -> Result<()> {
    let current = editor.settings.theme.clone();
    let Some(input) = args.input.clone() else {
        let candidates = editor.theme_names();
        // Empty, not pre-filled: this is a choice from a list rather than an
        // edit of a value, so anything already there would have to be erased
        // before a name could be typed. The current theme is the default the
        // prompt names instead.
        editor.prompt_for(
            "load-theme",
            MinibufferKind::Choice,
            format!("Load theme (default {current}): "),
            "",
            candidates,
        );
        return Ok(());
    };
    let name = match input.trim() {
        "" => current,
        typed => typed.to_string(),
    };
    editor.set_theme(&name)?;
    editor.message(format!("Theme {name}"));
    Ok(())
}

// ---- keyboard macros ----------------------------------------------------

fn start_macro(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.recording_macro.is_some() {
        return Err(crate::CoreError::Message(
            "Already recording a keyboard macro".into(),
        ));
    }
    editor.recording_macro = Some(Vec::new());
    editor.message("Defining keyboard macro...");
    Ok(())
}

/// Stops recording, dropping the keys that ended the recording.
fn finish_recording(editor: &mut Editor, trailing: usize) -> Result<usize> {
    let Some(mut keys) = editor.recording_macro.take() else {
        return Err(crate::CoreError::Message(
            "Not defining a keyboard macro".into(),
        ));
    };
    // The keys that invoked the stopping command are not part of the macro.
    keys.truncate(keys.len().saturating_sub(trailing));
    let length = keys.len();
    editor.last_macro = keys;
    Ok(length)
}

fn end_macro(editor: &mut Editor, _: &Args) -> Result<()> {
    // `C-x )` is two keys.
    let length = finish_recording(editor, 2)?;
    editor.message(format!("Keyboard macro defined ({length} keys)"));
    Ok(())
}

/// `C-x e`: ends a recording if one is running, then replays the macro.
fn end_and_call_macro(editor: &mut Editor, args: &Args) -> Result<()> {
    if editor.recording_macro.is_some() {
        finish_recording(editor, 2)?;
    }
    if editor.last_macro.is_empty() {
        return Err(crate::CoreError::Message(
            "No keyboard macro defined".into(),
        ));
    }
    // The loop replays it; doing so here would need the dispatcher, which the
    // command does not have.
    editor.macro_repeats = args.count();
    Ok(())
}

// ---- shell --------------------------------------------------------------

fn shell_command(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(command) = args.input.clone() else {
        editor.prompt_for(
            "shell-command",
            MinibufferKind::Shell,
            "Shell command: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if command.trim().is_empty() {
        return Err(crate::CoreError::Message("No command given".into()));
    }
    let directory = editor.default_directory();
    // A prefix argument inserts the output at point instead of showing it.
    let insert_at = args
        .prefix
        .is_present()
        .then(|| (editor.current_buffer_id(), editor.current_buffer().point()));
    editor.spawn(Task::Shell {
        command,
        directory,
        insert_at,
    });
    Ok(())
}

fn shell_command_on_region(editor: &mut Editor, args: &Args) -> Result<()> {
    let range = editor.region()?;
    let Some(command) = args.input.clone() else {
        editor.prompt_for(
            "shell-command-on-region",
            MinibufferKind::Shell,
            "Shell command on region: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if command.trim().is_empty() {
        return Err(crate::CoreError::Message("No command given".into()));
    }
    // The region is passed on the command line, quoted, so no pipe plumbing is
    // needed for the common case of a filter such as `sort` or `tr`.
    let text = editor.current_buffer().slice(range);
    let directory = editor.default_directory();
    editor.spawn(Task::Shell {
        command: format!("printf %s {} | {command}", crate::shell_quote(&text)),
        directory,
        insert_at: None,
    });
    Ok(())
}

/// Puts shell output into its own buffer, as `M-!` does.
pub fn show_shell_output(editor: &mut Editor, command: &str, output: &str) -> Result<()> {
    // Short output goes in the echo area, as Emacs does.
    let lines = output.lines().count();
    if lines <= 1 {
        editor.message(output.trim_end().to_string());
        return Ok(());
    }
    let text = format!("$ {command}\n{output}");
    let id = match editor.buffers.find_by_name(SHELL_OUTPUT_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &text)?;
            id
        }
        None => editor.buffers.create_with_text(SHELL_OUTPUT_NAME, &text),
    };
    editor
        .buffers
        .get_mut(id)
        .expect("just created")
        .set_read_only(true);
    editor.switch_to_buffer(id)
}

/// The keys `C-x (` and `C-x )` are made of, for tests and documentation.
pub fn macro_delimiters() -> (Vec<Key>, Vec<Key>) {
    (
        vec![Key::ctrl('x'), Key::char('(')],
        vec![Key::ctrl('x'), Key::char(')')],
    )
}

// ---- the toggles Doom keeps under its leader -----------------------------

fn toggle_line_numbers(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.settings.line_numbers = !editor.settings.line_numbers;
    let state = on_or_off(editor.settings.line_numbers);
    editor.message(format!("Line numbers {state}"));
    Ok(())
}

fn toggle_fill_column_indicator(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.settings.fill_column_indicator = !editor.settings.fill_column_indicator;
    let state = on_or_off(editor.settings.fill_column_indicator);
    editor.message(format!("Fill-column indicator {state}"));
    Ok(())
}

/// Tabs or spaces, for every buffer at once — which is what
/// `doom/toggle-indent-style` means by it.
fn toggle_indent_style(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.settings.indent_with_tabs = !editor.settings.indent_with_tabs;
    editor.apply_settings_everywhere();
    let style = match editor.settings.indent_with_tabs {
        true => "tabs",
        false => "spaces",
    };
    editor.message(format!("Indenting with {style}"));
    Ok(())
}

fn toggle_truncate_lines(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.settings.truncate_lines = !editor.settings.truncate_lines;
    let how = match editor.settings.truncate_lines {
        true => "clipped at the edge",
        false => "wrapped",
    };
    editor.follow_point();
    editor.message(format!("Long lines {how}"));
    Ok(())
}

/// `C-x C-+`: a tenth larger, `C-u 3 C-x C-+` three tenths. The window
/// reads the new scale on its next frame and cuts the font again.
fn text_scale_increase(editor: &mut Editor, args: &Args) -> Result<()> {
    editor.adjust_text_scale(args.count().max(1) as i32)
}

fn text_scale_decrease(editor: &mut Editor, args: &Args) -> Result<()> {
    editor.adjust_text_scale(-(args.count().max(1) as i32))
}

fn text_scale_reset(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.adjust_text_scale(0)
}

/// `<f11>`: the window over the whole screen, or back in its frame. The
/// window reads the answer on its next frame and asks the compositor.
fn toggle_frame_fullscreen(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.toggle_fullscreen()
}

fn on_or_off(on: bool) -> &'static str {
    match on {
        true => "on",
        false => "off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
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

    fn answer(d: &mut Dispatcher, e: &mut Editor, text: &str) {
        assert!(e.minibuffer.is_active(), "expected a prompt");
        e.minibuffer.kill_whole();
        for c in text.chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(e, "RET");
    }

    #[test]
    fn the_window_fills_the_screen_and_gives_it_back_where_there_is_one() {
        let (mut d, mut e) = setup("");
        let refused = fails(&mut d, &mut e, "toggle-frame-fullscreen");
        assert!(refused.contains("terminal"), "{refused}");
        assert_eq!(e.fullscreen, None);

        e.fullscreen = Some(false);
        d.handle_keys(&mut e, "<f11>");
        assert_eq!(e.fullscreen, Some(true));
        assert!(
            e.minibuffer.message().is_some_and(|m| m.contains("<f11>")),
            "the way back is said, since the window's own buttons are gone"
        );
        run(&mut d, &mut e, "toggle-frame-fullscreen");
        assert_eq!(e.fullscreen, Some(false));
        assert_eq!(e.minibuffer.message(), Some("Back in a window"));
    }

    #[test]
    fn the_text_zooms_a_tenth_a_step_and_only_where_it_can() {
        let (mut d, mut e) = setup("");
        // A terminal: the size is not the editor's to change, and the
        // command says so rather than pretending.
        let refused = fails(&mut d, &mut e, "text-scale-increase");
        assert!(refused.contains("terminal"), "{refused}");
        assert_eq!(e.text_scale_factor(), 1.0);

        // A window said it can.
        e.text_scale = Some(0);
        run(&mut d, &mut e, "text-scale-increase");
        assert_eq!(e.text_scale, Some(1));
        assert!((e.text_scale_factor() - 1.1).abs() < 1e-6);
        assert_eq!(e.minibuffer.message(), Some("Text at 110%"));
        d.handle_keys(&mut e, "C-x C-+");
        d.handle_keys(&mut e, "C-x C-=");
        assert_eq!(e.text_scale, Some(3), "both keys go up");
        d.handle_keys(&mut e, "C-x C--");
        assert_eq!(e.text_scale, Some(2));
        d.handle_keys(&mut e, "C-x C-0");
        assert_eq!(e.text_scale, Some(0));
        assert_eq!(
            e.minibuffer.message(),
            Some("Text back to its configured size")
        );

        // It stops somewhere, and says so.
        for _ in 0..20 {
            run(&mut d, &mut e, "text-scale-decrease");
        }
        assert_eq!(e.text_scale, Some(-Editor::TEXT_SCALE_STEPS));
        assert_eq!(
            e.minibuffer.message(),
            Some("Text at 39%, as far as it goes")
        );
    }

    #[test]
    fn every_global_binding_resolves_to_a_registered_command() {
        let registry = crate::commands::standard_registry();
        for (keys, command) in crate::keymap::GLOBAL_BINDINGS {
            assert!(
                registry.contains(command),
                "`{keys}` runs unregistered `{command}`"
            );
        }
        for keys in crate::keymap::DIGIT_ARGUMENT_KEYS {
            assert!(
                registry.contains("digit-argument"),
                "`{keys}` has nothing to run"
            );
        }
        assert!(
            registry.contains("self-insert-command"),
            "the fallback binding"
        );
    }

    #[test]
    fn the_prefix_argument_builds_up_from_keys() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "C-u");
        assert_eq!(e.prefix, Prefix::Universal(1));
        assert_eq!(e.minibuffer.display(), "C-u ");

        d.handle_keys(&mut e, "C-u");
        assert_eq!(e.prefix.count(), 16);

        d.handle_keys(&mut e, "M-3");
        d.handle_keys(&mut e, "M-7");
        assert_eq!(e.prefix.count(), 37);
        assert_eq!(e.minibuffer.display(), "C-u 37 ");
    }

    #[test]
    fn a_negative_argument_is_available_from_its_own_key() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "M--");
        assert_eq!(e.prefix.count(), -1);
        d.handle_keys(&mut e, "M-5");
        assert_eq!(e.prefix.count(), -5);
    }

    #[test]
    fn the_prefix_argument_reaches_the_next_command() {
        let (mut d, mut e) = setup("abcdefghij");
        d.handle_keys(&mut e, "M-4");
        d.handle_keys(&mut e, "C-f");
        assert_eq!(e.windows.current().point, 4);
        assert_eq!(e.prefix, Prefix::None, "and is cleared afterwards");
    }

    #[test]
    fn m_x_prompts_and_runs_the_command_by_name() {
        let (mut d, mut e) = setup("hello world");
        d.execute(&mut e, "execute-extended-command", None);
        assert_eq!(e.minibuffer.prompt(), "M-x ");
        answer(&mut d, &mut e, "end-of-buffer");
        assert_eq!(e.windows.current().point, 11);
    }

    #[test]
    fn m_x_completes_against_the_command_list() {
        let (mut d, mut e) = setup("");
        d.execute(&mut e, "execute-extended-command", None);
        for c in "save-b".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "save-buffer");
    }

    #[test]
    fn m_x_carries_the_prefix_argument_through() {
        let (mut d, mut e) = setup("abcdefghij");
        e.prefix = Prefix::Numeric(5);
        d.execute(&mut e, "execute-extended-command", None);
        answer(&mut d, &mut e, "forward-char");
        assert_eq!(e.windows.current().point, 5);
    }

    #[test]
    fn m_x_refuses_a_name_it_does_not_know() {
        let (mut d, mut e) = setup("");
        d.execute(&mut e, "execute-extended-command", None);
        e.minibuffer.kill_whole();
        for c in "no-such-command".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
        assert!(e.minibuffer.display().contains("No command named"));
    }

    #[test]
    fn m_x_will_not_run_a_non_interactive_command() {
        let (mut d, mut e) = setup("");
        d.execute(&mut e, "execute-extended-command", None);
        e.minibuffer.kill_whole();
        for c in "minibuffer-self-insert".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(
            matches!(out, Dispatch::Failed { .. }),
            "it is not offered by M-x"
        );
    }

    #[test]
    fn escape_quit_unwinds_one_thing_at_a_time() {
        let (mut d, mut e) = setup("hello");
        // A prompt closes first.
        e.prompt(MinibufferKind::Command, "M-x ");
        run(&mut d, &mut e, "keyboard-escape-quit");
        assert!(!e.minibuffer.is_active());

        // Then extra windows.
        run(&mut d, &mut e, "split-window-below");
        run(&mut d, &mut e, "keyboard-escape-quit");
        assert_eq!(e.windows.len(), 1);

        // Then the region.
        e.with_current_buffer(|b| {
            b.set_mark(0);
            b.set_point(3);
        });
        run(&mut d, &mut e, "keyboard-escape-quit");
        assert!(e.region().is_err());
    }

    #[test]
    fn escape_quit_ends_a_search() {
        let (mut d, mut e) = setup("hello");
        run(&mut d, &mut e, "isearch-forward");
        run(&mut d, &mut e, "keyboard-escape-quit");
        assert!(e.isearch.is_none());
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));
    }

    #[test]
    fn load_theme_offers_every_theme_and_starts_on_the_current_one() {
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "load-theme");
        assert_eq!(
            e.completion_candidates,
            maxgus_faces::defaults::BUILTIN_THEMES
        );
        // Nothing to erase before a name can be typed.
        assert_eq!(e.minibuffer.input(), "");
        assert!(e.minibuffer.prompt().contains("default maxgus-dark"));
    }

    #[test]
    fn load_theme_switches_the_faces_the_editor_draws_with() {
        let (mut d, mut e) = setup("");
        let before = e.theme.resolve("default");
        run(&mut d, &mut e, "load-theme");
        answer(&mut d, &mut e, "maxgus-light");

        assert_eq!(e.theme.name(), "maxgus-light");
        assert_ne!(
            e.theme.resolve("default"),
            before,
            "a light theme draws differently"
        );
        // `describe-settings` reads the name back out of the settings.
        assert_eq!(e.settings.theme, "maxgus-light");
    }

    #[test]
    fn load_theme_keeps_the_faces_the_configuration_overrode() {
        // The whole reason the editor holds on to the theme blocks: switching
        // themes at runtime has to rebuild them the way startup did, or the
        // user's own colours vanish the first time they change theme.
        let (mut d, mut e) = setup("");
        let mut spec = maxgus_config::ThemeSpec::new("maxgus-light");
        spec.faces.push(maxgus_config::FaceSpec {
            name: "region".into(),
            foreground: Some("#ff0000".into()),
            ..Default::default()
        });
        e.theme_specs.push(spec);

        run(&mut d, &mut e, "load-theme");
        answer(&mut d, &mut e, "maxgus-light");
        assert_eq!(
            e.theme.resolve("region").foreground,
            Some(maxgus_faces::Color::Rgb(255, 0, 0))
        );
    }

    #[test]
    fn a_theme_the_editor_does_not_have_is_refused() {
        let (mut d, mut e) = setup("");
        let before = e.theme.name().to_string();
        run(&mut d, &mut e, "load-theme");
        e.minibuffer.kill_whole();
        for c in "solarized".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }), "got {out:?}");
        assert_eq!(e.theme.name(), before, "the theme in use did not change");
    }

    #[test]
    fn answering_the_theme_prompt_with_nothing_takes_the_default() {
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "load-theme");
        answer(&mut d, &mut e, "maxgus-light");
        run(&mut d, &mut e, "load-theme");
        // The prompt says what the default is; RET on an empty line must take
        // it rather than complain about a theme with no name.
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.theme.name(), "maxgus-light");
    }

    #[test]
    fn tab_completes_a_theme_name() {
        // The candidates being set is not the same as them being used: a kind
        // left out of `completes` collects a candidate list that TAB ignores.
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "load-theme");
        e.minibuffer.kill_whole();
        for c in "maxgus-l".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "TAB");
        assert_eq!(e.minibuffer.input(), "maxgus-light");
    }

    #[test]
    fn a_theme_named_only_by_the_configuration_can_be_loaded() {
        let (mut d, mut e) = setup("");
        e.theme_specs.push(maxgus_config::ThemeSpec::new("mine"));
        run(&mut d, &mut e, "load-theme");
        assert!(e.completion_candidates.contains(&"mine".to_string()));
        answer(&mut d, &mut e, "mine");
        assert_eq!(e.settings.theme, "mine");
    }

    #[test]
    fn repeat_runs_the_last_command_again() {
        let (mut d, mut e) = setup("abc def ghi\n");
        d.handle_keys(&mut e, "M-u");
        assert_eq!(e.current_buffer().text(), "ABC def ghi\n");
        d.handle_keys(&mut e, "C-x z");
        assert_eq!(e.current_buffer().text(), "ABC DEF ghi\n");
    }

    #[test]
    fn repeating_twice_runs_it_twice_more() {
        // The deferred call sets `last_command` back to the repeated command,
        // so the key keeps working rather than trying to repeat `repeat`.
        let (mut d, mut e) = setup("abc def ghi\n");
        d.handle_keys(&mut e, "M-u");
        d.handle_keys(&mut e, "C-x z");
        d.handle_keys(&mut e, "C-x z");
        assert_eq!(e.current_buffer().text(), "ABC DEF GHI\n");
    }

    #[test]
    fn repeat_with_nothing_before_it_says_so() {
        let (mut d, mut e) = setup("abc\n");
        e.last_command = None;
        assert_eq!(fails(&mut d, &mut e, "repeat"), "Nothing to repeat");
    }

    #[test]
    fn repeat_refuses_to_repeat_itself() {
        let (mut d, mut e) = setup("abc\n");
        e.last_command = Some("repeat".into());
        assert_eq!(fails(&mut d, &mut e, "repeat"), "Nothing to repeat");
    }

    #[test]
    fn suspending_raises_the_flag_the_loop_watches() {
        let (mut d, mut e) = setup("");
        assert!(!e.suspend);
        run(&mut d, &mut e, "suspend-maxgus");
        assert!(e.suspend);
    }

    #[test]
    fn a_keyboard_macro_records_the_keys_that_were_typed() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "C-x");
        d.handle_keys(&mut e, "(");
        assert!(e.recording_macro.is_some());
        assert_eq!(e.minibuffer.display(), "Defining keyboard macro...");

        for key in ["a", "b", "c"] {
            d.handle_keys(&mut e, key);
        }
        d.handle_keys(&mut e, "C-x");
        d.handle_keys(&mut e, ")");

        assert!(e.recording_macro.is_none());
        assert_eq!(e.last_macro.len(), 3, "the closing keys are not part of it");
        assert_eq!(e.last_macro[0], Key::char('a'));
        assert_eq!(e.current_buffer().text(), "abc");
    }

    #[test]
    fn starting_a_macro_twice_is_refused() {
        let (mut d, mut e) = setup("");
        run(&mut d, &mut e, "kmacro-start-macro");
        assert!(fails(&mut d, &mut e, "kmacro-start-macro").contains("Already recording"));
    }

    #[test]
    fn ending_a_macro_that_was_never_started_is_refused() {
        let (mut d, mut e) = setup("");
        assert!(fails(&mut d, &mut e, "kmacro-end-macro").contains("Not defining"));
    }

    #[test]
    fn calling_a_macro_asks_the_loop_to_replay_it() {
        let (mut d, mut e) = setup("");
        e.last_macro = vec![Key::char('x')];
        e.prefix = Prefix::Numeric(3);
        d.execute(&mut e, "kmacro-end-and-call-macro", None);
        assert_eq!(e.macro_repeats, 3, "the loop replays it three times");
    }

    #[test]
    fn calling_with_no_macro_defined_says_so() {
        let (mut d, mut e) = setup("");
        assert!(fails(&mut d, &mut e, "kmacro-end-and-call-macro").contains("No keyboard macro"));
    }

    #[test]
    fn calling_while_recording_closes_the_recording_first() {
        let (mut d, mut e) = setup("");
        d.handle_keys(&mut e, "C-x");
        d.handle_keys(&mut e, "(");
        d.handle_keys(&mut e, "z");
        d.handle_keys(&mut e, "C-x");
        d.handle_keys(&mut e, "e");
        assert!(e.recording_macro.is_none());
        assert_eq!(e.last_macro, vec![Key::char('z')]);
    }

    #[test]
    fn a_shell_command_is_queued_with_the_working_directory() {
        let (mut d, mut e) = setup("");
        let id = e.buffers.visit_file("/project/main.rs", "");
        e.switch_to_buffer(id).unwrap();
        d.execute(&mut e, "shell-command", None);
        e.tasks.drain();
        answer(&mut d, &mut e, "ls -l");

        let Task::Shell {
            command,
            directory,
            insert_at,
        } = &e.tasks.peek()[0]
        else {
            panic!()
        };
        assert_eq!(command, "ls -l");
        assert_eq!(directory, std::path::Path::new("/project"));
        assert!(insert_at.is_none());
    }

    #[test]
    fn a_prefix_argument_makes_the_output_go_into_the_buffer() {
        let (mut d, mut e) = setup("text");
        e.prefix = Prefix::Universal(1);
        d.execute(&mut e, "shell-command", None);
        answer(&mut d, &mut e, "date");
        let Task::Shell { insert_at, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert!(insert_at.is_some());
    }

    #[test]
    fn a_command_on_the_region_passes_the_text_through() {
        let (mut d, mut e) = setup("beta\nalpha\n");
        e.with_current_buffer(|b| {
            b.set_mark(0);
            b.set_point(11);
        });
        d.execute(&mut e, "shell-command-on-region", None);
        e.tasks.drain();
        answer(&mut d, &mut e, "sort");
        let Task::Shell { command, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert!(command.contains("| sort"), "got `{command}`");
        assert!(command.contains("beta"), "got `{command}`");
    }

    #[test]
    fn a_command_on_the_region_needs_a_region() {
        let (mut d, mut e) = setup("text");
        assert!(fails(&mut d, &mut e, "shell-command-on-region").contains("mark"));
    }

    #[test]
    fn shell_quoting_survives_an_apostrophe() {
        assert_eq!(crate::shell_quote("plain"), "'plain'");
        assert_eq!(crate::shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn short_output_goes_to_the_echo_area_and_long_output_to_a_buffer() {
        let (_d, mut e) = setup("");
        show_shell_output(&mut e, "date", "Tue 27 Aug\n").unwrap();
        assert_eq!(e.minibuffer.display(), "Tue 27 Aug");
        assert!(e.buffers.find_by_name(SHELL_OUTPUT_NAME).is_none());

        show_shell_output(&mut e, "ls", "one\ntwo\nthree\n").unwrap();
        assert_eq!(e.current_buffer().name(), SHELL_OUTPUT_NAME);
        assert!(e.current_buffer().is_read_only());
        assert!(e.current_buffer().text().starts_with("$ ls\n"));
    }

    #[test]
    fn shell_output_reuses_its_buffer() {
        let (_d, mut e) = setup("");
        show_shell_output(&mut e, "a", "1\n2\n").unwrap();
        show_shell_output(&mut e, "b", "3\n4\n").unwrap();
        assert_eq!(
            e.buffers
                .iter()
                .filter(|b| b.name() == SHELL_OUTPUT_NAME)
                .count(),
            1
        );
    }

    #[test]
    fn the_macro_delimiters_are_the_keys_the_keymap_binds() {
        let (start, end) = macro_delimiters();
        assert_eq!(start.len(), 2);
        assert_eq!(end.len(), 2);
        assert_eq!(start[0], Key::ctrl('x'));
    }
}
