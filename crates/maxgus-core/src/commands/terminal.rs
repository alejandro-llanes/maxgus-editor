//! Terminal commands: the panel, its tabs, and reading what is in them.
//!
//! A terminal window is the one place in the editor where most keys are *not*
//! commands. The `terminal-mode` keymap binds only the handful that are, and
//! its default binding sends whatever is left to the shell — so `C-a` goes to
//! readline rather than to `move-beginning-of-line`, which is the whole point
//! of having a terminal rather than a shell buffer.
//!
//! Reading what the shell wrote is a second mode. `C-c C-t` stops keys going
//! to the shell and moves a cursor over the screen instead, so a selection can
//! be made and copied without a mouse; `C-g` gives the keyboard back. This is
//! the arrangement vterm settled on, and it is settled on for the same reason:
//! there is no key left over to mean "select" while the shell owns them all.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
    task::{Task, TerminalId},
};
use maxgus_term::selection::Mode as SelectionMode;

/// The buffer a terminal window shows.
pub const TERMINAL_BUFFER_NAME: &str = "*terminal*";

/// The mode name for typing at the shell.
pub const TERMINAL_MODE: &str = "terminal-mode";
/// The mode name for reading and selecting instead.
pub const TERMINAL_COPY_MODE: &str = "terminal-copy-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "terminal-toggle",
            "Show or hide the terminal panel.",
            toggle
        ),
        command!("terminal-select", "Select the terminal panel.", select),
        command!("terminal-new-tab", "Open another terminal tab.", new_tab),
        command!(
            "terminal-next-tab",
            "Show the next terminal tab.",
            next_tab,
            non_interactive
        ),
        command!(
            "terminal-previous-tab",
            "Show the previous terminal tab.",
            previous_tab,
            non_interactive
        ),
        command!(
            "terminal-select-tab",
            "Show the tab with this number.",
            select_tab,
            non_interactive
        ),
        command!(
            "terminal-close-tab",
            "Close this terminal tab.",
            close_tab,
            non_interactive
        ),
        command!(
            "terminal-send-key",
            "Send this key to the shell.",
            send_key,
            non_interactive
        ),
        command!(
            "terminal-send-control",
            "Send this key with control held to the shell.",
            send_control,
            non_interactive
        ),
        command!(
            "terminal-paste",
            "Paste the last kill into the shell.",
            paste,
            non_interactive
        ),
        command!(
            "terminal-copy-mode",
            "Read and select the terminal's output.",
            copy_mode,
            non_interactive
        ),
        command!(
            "terminal-copy-mode-quit",
            "Go back to typing at the shell.",
            copy_mode_quit,
            non_interactive
        ),
        command!(
            "terminal-set-mark",
            "Start a selection here.",
            set_mark,
            non_interactive
        ),
        command!(
            "terminal-set-line-mark",
            "Start a whole-line selection here.",
            set_line_mark,
            non_interactive
        ),
        command!(
            "terminal-set-block-mark",
            "Start a rectangular selection here.",
            set_block_mark,
            non_interactive
        ),
        command!(
            "terminal-copy",
            "Copy the selection and stop reading.",
            copy,
            non_interactive
        ),
        command!(
            "terminal-next-line",
            "Move down one line.",
            next_line,
            non_interactive
        ),
        command!(
            "terminal-previous-line",
            "Move up one line.",
            previous_line,
            non_interactive
        ),
        command!(
            "terminal-forward-char",
            "Move right one column.",
            forward_char,
            non_interactive
        ),
        command!(
            "terminal-backward-char",
            "Move left one column.",
            backward_char,
            non_interactive
        ),
        command!(
            "terminal-beginning-of-line",
            "Move to the start of the line.",
            beginning_of_line,
            non_interactive
        ),
        command!(
            "terminal-end-of-line",
            "Move to the end of the line.",
            end_of_line,
            non_interactive
        ),
        command!(
            "terminal-scroll-up",
            "Look further back through the output.",
            scroll_up,
            non_interactive
        ),
        command!(
            "terminal-scroll-down",
            "Look further forward through the output.",
            scroll_down,
            non_interactive
        ),
        command!(
            "terminal-goto-first",
            "Go to the oldest line kept.",
            goto_first,
            non_interactive
        ),
        command!(
            "terminal-goto-last",
            "Go to the newest line.",
            goto_last,
            non_interactive
        ),
    ]);
}

/// The terminal being shown, or an error saying there is none.
fn current(editor: &Editor) -> Result<TerminalId> {
    editor
        .terminals
        .current()
        .map(|terminal| terminal.id)
        .ok_or_else(|| crate::CoreError::Message("No terminal here".into()))
}

// ---- the panel ----------------------------------------------------------

/// Opens the panel, starting a first tab if there is none.
fn open(editor: &mut Editor) -> Result<()> {
    let id = match editor.buffers.find_by_name(TERMINAL_BUFFER_NAME) {
        Some(id) => id,
        None => {
            let id = editor.buffers.create_with_text(TERMINAL_BUFFER_NAME, "");
            editor
                .buffers
                .get_mut(id)
                .expect("just created")
                .set_read_only(true);
            id
        }
    };
    let height = editor.terminal_height;
    let window = editor.windows.add_bottom_window(id, height);
    editor.terminal_window = Some(window);
    if editor.terminals.is_empty() {
        start_tab(editor);
    }
    editor.resize_terminals();
    Ok(())
}

fn close(editor: &mut Editor) {
    let Some(window) = editor.terminal_window.take() else {
        return;
    };
    editor.windows.delete(window).ok();
    editor.activate_mode_keymap();
}

/// Starts a shell and gives it a tab.
fn start_tab(editor: &mut Editor) {
    let (rows, columns) = editor.terminal_size();
    let shell = editor.settings.shell.clone();
    let title = shell
        .clone()
        .unwrap_or_else(|| "shell".to_string())
        .rsplit('/')
        .next()
        .unwrap_or("shell")
        .to_string();
    let id = editor.terminals.open(title, rows, columns);
    let directory = editor.default_directory();
    editor.spawn(Task::TerminalOpen {
        terminal: id,
        shell,
        directory,
        rows: rows as u16,
        columns: columns as u16,
    });
}

fn toggle(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.terminal_window.is_some() {
        close(editor);
        return Ok(());
    }
    open(editor)?;
    // Opening it means wanting to type in it, unlike the file tree which is
    // opened to look at.
    if let Some(window) = editor.terminal_window {
        editor.select_window(window);
    }
    Ok(())
}

fn select(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.terminal_window.is_none() {
        open(editor)?;
    }
    let window = editor
        .terminal_window
        .ok_or(crate::CoreError::NoSuchWindow)?;
    editor.select_window(window);
    Ok(())
}

// ---- tabs ---------------------------------------------------------------

fn new_tab(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.terminal_window.is_none() {
        open(editor)?;
    }
    start_tab(editor);
    if let Some(window) = editor.terminal_window {
        editor.select_window(window);
    }
    Ok(())
}

fn next_tab(editor: &mut Editor, args: &Args) -> Result<()> {
    editor
        .terminals
        .select_relative(args.signed_count().max(1) as isize);
    editor.activate_mode_keymap();
    Ok(())
}

fn previous_tab(editor: &mut Editor, args: &Args) -> Result<()> {
    editor
        .terminals
        .select_relative(-(args.signed_count().max(1) as isize));
    editor.activate_mode_keymap();
    Ok(())
}

/// `C-c 1` through `C-c 9`: the tab at that position.
fn select_tab(editor: &mut Editor, args: &Args) -> Result<()> {
    let wanted = args
        .key
        .and_then(|key| key.as_char())
        .and_then(|c| c.to_digit(10))
        .map(|digit| digit as usize)
        .unwrap_or_else(|| args.count());
    if wanted == 0 || !editor.terminals.select(wanted - 1) {
        return Err(crate::CoreError::Message(format!(
            "There is no tab {wanted}"
        )));
    }
    editor.activate_mode_keymap();
    Ok(())
}

fn close_tab(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = current(editor)?;
    editor.terminals.close(id);
    editor.spawn(Task::TerminalClose { terminal: id });
    // The last tab closing takes the panel with it: an empty terminal panel
    // is a band of nothing across the bottom of the frame.
    if editor.terminals.is_empty() {
        close(editor);
    } else {
        editor.activate_mode_keymap();
    }
    Ok(())
}

// ---- typing at the shell ------------------------------------------------

/// The default binding: whatever was pressed, encoded and sent.
fn send_key(editor: &mut Editor, args: &Args) -> Result<()> {
    let id = current(editor)?;
    let Some(key) = args.key else { return Ok(()) };
    let Some(terminal) = editor.terminals.current() else {
        return Ok(());
    };
    let Some(bytes) = maxgus_term::keys::encode(&key, terminal.emulator.modes()) else {
        return Ok(());
    };
    editor.spawn(Task::TerminalInput {
        terminal: id,
        bytes,
    });
    Ok(())
}

/// `C-c c`, `C-c x`, `C-c g`, `C-c h`: the control key the prefix took away.
///
/// The last key of the sequence says which, so one command covers all four
/// and adding another is a line in the keymap rather than a function here.
fn send_control(editor: &mut Editor, args: &Args) -> Result<()> {
    let id = current(editor)?;
    let byte = args
        .key
        .and_then(|key| key.as_char())
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| c.is_ascii_lowercase())
        .map(|c| c as u8 - b'a' + 1)
        .ok_or_else(|| crate::CoreError::Message("Not a control key".into()))?;
    editor.spawn(Task::TerminalInput {
        terminal: id,
        bytes: vec![byte],
    });
    Ok(())
}

/// Pastes the most recent kill, bracketed when the program asked for that.
fn paste(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = current(editor)?;
    let text = editor
        .kill_ring
        .front()
        .map(str::to_string)
        .ok_or_else(|| crate::CoreError::Message("Kill ring is empty".into()))?;
    let Some(terminal) = editor.terminals.current() else {
        return Ok(());
    };
    let bytes = maxgus_term::keys::paste(&text, terminal.emulator.modes());
    editor.spawn(Task::TerminalInput {
        terminal: id,
        bytes,
    });
    Ok(())
}

// ---- reading and selecting ----------------------------------------------

fn copy_mode(editor: &mut Editor, _: &Args) -> Result<()> {
    current(editor)?;
    if let Some(terminal) = editor.terminals.current_mut() {
        terminal.begin_copy_mode();
    }
    editor.activate_mode_keymap();
    editor.message("Reading the terminal; C-SPC to mark, M-w to copy, C-g to stop".to_string());
    Ok(())
}

fn copy_mode_quit(editor: &mut Editor, _: &Args) -> Result<()> {
    if let Some(terminal) = editor.terminals.current_mut() {
        terminal.end_copy_mode();
        terminal.scroll = 0;
    }
    editor.activate_mode_keymap();
    Ok(())
}

fn set_mark(editor: &mut Editor, _: &Args) -> Result<()> {
    mark(editor, SelectionMode::Character)
}

fn set_line_mark(editor: &mut Editor, _: &Args) -> Result<()> {
    mark(editor, SelectionMode::Line)
}

fn set_block_mark(editor: &mut Editor, _: &Args) -> Result<()> {
    mark(editor, SelectionMode::Block)
}

fn mark(editor: &mut Editor, mode: SelectionMode) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Err(crate::CoreError::Message("No terminal here".into()));
    };
    if !terminal.in_copy_mode() {
        return Err(crate::CoreError::Message("Not reading the terminal".into()));
    }
    terminal.set_mark(mode);
    editor.message("Mark set".to_string());
    Ok(())
}

/// Copies the selection to the kill ring and goes back to typing.
fn copy(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = editor
        .terminals
        .current()
        .and_then(|terminal| terminal.selected_text())
        .ok_or_else(|| crate::CoreError::Message("Nothing selected".into()))?;
    let length = text.chars().count();
    editor.kill_ring.kill_new(text);
    if let Some(terminal) = editor.terminals.current_mut() {
        terminal.end_copy_mode();
        terminal.scroll = 0;
    }
    editor.activate_mode_keymap();
    editor.message(format!("Copied {length} characters"));
    Ok(())
}

fn moving(editor: &mut Editor, lines: isize, columns: isize) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Err(crate::CoreError::Message("No terminal here".into()));
    };
    terminal.move_copy_cursor(lines, columns);
    Ok(())
}

fn next_line(editor: &mut Editor, args: &Args) -> Result<()> {
    moving(editor, args.count() as isize, 0)
}

fn previous_line(editor: &mut Editor, args: &Args) -> Result<()> {
    moving(editor, -(args.count() as isize), 0)
}

fn forward_char(editor: &mut Editor, args: &Args) -> Result<()> {
    moving(editor, 0, args.count() as isize)
}

fn backward_char(editor: &mut Editor, args: &Args) -> Result<()> {
    moving(editor, 0, -(args.count() as isize))
}

fn beginning_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    if let Some(cursor) = terminal.copy_cursor {
        terminal.move_copy_cursor_to(cursor.line, 0);
    }
    Ok(())
}

fn end_of_line(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    if let Some(cursor) = terminal.copy_cursor {
        // The end of the *text*, not of the padding, which is where a reader
        // expects `C-e` to land.
        let width = terminal
            .emulator
            .grid()
            .all_lines()
            .nth(cursor.line)
            .map(|line| line.text().chars().count())
            .unwrap_or(0);
        terminal.move_copy_cursor_to(cursor.line, width.saturating_sub(1));
    }
    Ok(())
}

fn goto_first(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    terminal.move_copy_cursor_to(0, 0);
    Ok(())
}

fn goto_last(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    let last = terminal.emulator.grid().total_lines().saturating_sub(1);
    terminal.move_copy_cursor_to(last, 0);
    Ok(())
}

fn scroll_up(editor: &mut Editor, args: &Args) -> Result<()> {
    let page = editor.terminal_size().0.saturating_sub(2).max(1) * args.count();
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    if terminal.in_copy_mode() {
        terminal.move_copy_cursor(-(page as isize), 0);
    } else {
        terminal.scroll_back(page);
    }
    Ok(())
}

fn scroll_down(editor: &mut Editor, args: &Args) -> Result<()> {
    let page = editor.terminal_size().0.saturating_sub(2).max(1) * args.count();
    let Some(terminal) = editor.terminals.current_mut() else {
        return Ok(());
    };
    if terminal.in_copy_mode() {
        terminal.move_copy_cursor(page as isize, 0);
    } else {
        terminal.scroll_forward(page);
    }
    Ok(())
}
