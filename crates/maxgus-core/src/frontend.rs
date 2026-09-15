//! The work a front end owes the editor, whatever it draws into.
//!
//! There are two of them — a terminal and a window — and everything in here
//! was once written in the terminal's loop alone. The window then quietly did
//! none of it: no macro replayed, the file tree never followed the file being
//! edited, the buffer was never re-highlighted after a change and the
//! language server was never told anything had changed, so hovering a symbol
//! described the file as it had been when it was opened.
//!
//! Sharing it is the only way the two stay in step. A front end decides
//! *when* — a terminal has tokio timers and a window has its event loop —
//! and this decides what.

use crate::{dispatch::Dispatcher, editor::Editor};

/// After a key has been handled: the things a command can ask for that only
/// the front end can carry out.
pub fn after_key(editor: &mut Editor, dispatcher: &mut Dispatcher) {
    replay_macro(editor, dispatcher);
    editor.forget_dead_windows();
    remember_the_editing_window(editor);
    follow_tree(editor);
    close_the_menu_on_the_way_out(editor);
}

/// Notes which editing window is in use, whichever way it was reached.
///
/// Selecting a window through the editor records it already; this catches
/// the layouts that change the selection by themselves — a window deleted
/// from under the cursor, a split.
fn remember_the_editing_window(editor: &mut Editor) {
    let current = editor.windows.current_id();
    if !editor.is_dedicated_window(current) {
        editor.last_editing_window = Some(current);
    }
}

/// Puts text a front end was handed — a bracketed paste in a terminal, the
/// middle button or a dropped string in a window — where the keys would
/// have put it.
///
/// Both front ends inserted it straight into the buffer, whatever was
/// taking the keys: a paste while searching went into the text rather than
/// the search, one into the terminal panel went nowhere near the shell, and
/// one into a buffer being typed in joined the typing's undo step.
pub fn paste_text(editor: &mut Editor, text: &str) {
    if text.is_empty() {
        return;
    }
    if editor.isearch.is_some() {
        if let Err(error) = crate::commands::search::extend_with_paste(editor, text) {
            editor.error(error.to_string());
        }
        return;
    }
    if editor.minibuffer.is_active() {
        editor.minibuffer.insert(&text.replace(['\r', '\n'], " "));
        editor.refresh_completions();
        return;
    }
    #[cfg(feature = "full")]
    if editor.terminal_pane() == Some(editor.windows.current_id())
        && let Some(terminal) = editor.terminals.current()
        && !terminal.in_copy_mode()
    {
        let bytes = maxgus_term::keys::paste(text, terminal.emulator.modes());
        let id = terminal.id;
        editor.spawn(crate::task::Task::TerminalInput {
            terminal: id,
            bytes,
        });
        return;
    }
    // Its own undo step, and on the kill ring as Emacs' `xterm-paste` leaves
    // it, so `M-y` can reach it again.
    let text = text.replace("\r\n", "\n");
    let inserted = editor.with_current_buffer(|b| {
        let at = b.point();
        b.insert(at, &text)?;
        b.set_point(at + text.chars().count());
        Ok::<(), maxgus_text::TextError>(())
    });
    match inserted {
        Ok(()) => editor.kill_ring.kill_new(text),
        Err(error) => editor.error(error.to_string()),
    }
    editor.follow_point();
}

/// Puts the file tree's `?` panel away once the tree is no longer where the
/// keys are going.
///
/// It stays up across the commands it describes — that is the point of it,
/// and treemacs' hydra does the same — so nothing else takes it down. But a
/// panel explaining the tree's keys, over a window that is not the tree, is
/// explaining keys that no longer do any of that.
fn close_the_menu_on_the_way_out(editor: &mut Editor) {
    if editor.key_menu.is_none() {
        return;
    }
    if editor.tree_window != Some(editor.windows.current_id()) {
        editor.key_menu = None;
    }
}

/// Replays the last keyboard macro, if a command asked for it.
fn replay_macro(editor: &mut Editor, dispatcher: &mut Dispatcher) {
    let repeats = std::mem::take(&mut editor.macro_repeats);
    if repeats == 0 {
        return;
    }
    let keys = editor.last_macro.clone();
    editor.replaying_macro = true;
    // A key that fails stops the macro, as it does in Emacs: a search that
    // finds nothing, or the end of the buffer, is where going on typing the
    // rest of the keys does damage.
    'replay: for _ in 0..repeats {
        for key in &keys {
            match dispatcher.handle_key(editor, *key) {
                crate::Dispatch::Failed { message, .. } => {
                    editor.error(format!("Keyboard macro stopped: {message}"));
                    break 'replay;
                }
                crate::Dispatch::Undefined { keys } => {
                    editor.error(format!("Keyboard macro stopped: {keys} is undefined"));
                    break 'replay;
                }
                _ => {}
            }
        }
    }
    editor.replaying_macro = false;
}

/// Keeps the tree cursor on the file being edited, when follow mode is on.
fn follow_tree(editor: &mut Editor) {
    if !editor.tree_follow || editor.tree_window.is_none() {
        return;
    }
    // Only when the user is editing, not while they walk the tree itself.
    if Some(editor.windows.current_id()) == editor.tree_window {
        return;
    }
    let Some(path) = editor
        .current_buffer()
        .path()
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    if editor.tree.iter().any(|node| node.path == path) {
        editor.tree_follow_asked = None;
        editor.select_tree_path(&path);
        return;
    }
    // Once per file. This runs after every key *and after every answer the
    // executor sends*, and a file the tree cannot show — a dotfile while
    // dotfiles are hidden, an ignored directory — never turns up in the
    // answer: asking again each time kept a core spinning for as long as
    // the file stayed open.
    if editor.tree_follow_asked.as_ref() == Some(&path) {
        return;
    }
    // Nor anywhere the tree is not looking at all.
    let inside = editor
        .tree
        .iter()
        .any(|node| node.is_root && path.starts_with(&node.path));
    if !inside {
        return;
    }
    editor.tree_follow_asked = Some(path.clone());
    editor.spawn(crate::task::Task::Tree(crate::task::TreeAction::Reveal(
        path,
    )));
}

/// The work that waits for typing to stop: re-highlighting the buffer and
/// telling the language server what changed.
///
/// Both are expensive and neither is urgent, which is what the pause is for.
pub fn on_idle(editor: &mut Editor) {
    let id = editor.current_buffer_id();
    #[cfg(feature = "full")]
    if editor.highlights_are_stale(id) {
        editor.request_highlighting(id);
    }
    // The outline is of the text as the server last heard it; telling the
    // server about an edit is when to ask again.
    if editor.sync_language_server(id) && editor.panel.symbols_buffer == Some(id) {
        editor.request_document_symbols();
    }
    #[cfg(feature = "full")]
    ask_about_the_symbol_under_point(editor, id);
    #[cfg(feature = "full")]
    ask_what_could_follow(editor, id);
}

/// Asks the language server what could follow what is being typed.
///
/// Once per place, like the doc box: an idle pause where nothing has moved
/// has already been answered. The list that comes back is offered, never
/// inserted — a pause in typing must not put text in a buffer.
#[cfg(feature = "full")]
fn ask_what_could_follow(editor: &mut Editor, id: maxgus_text::BufferId) {
    if !editor.settings.autocomplete || !editor.settings.lsp_enabled {
        return;
    }
    if !server_running(editor) {
        return;
    }
    let point = editor.windows.current().point;
    if editor.completions_asked_at == Some((id, point)) {
        return;
    }
    // Only while a word is being typed. Moving onto the end of a word that
    // is already there — `M-f` along a line — is not asking what could
    // follow it, and a list appearing under every word passed was.
    if editor.autocomplete.is_none()
        && editor.last_command.as_deref() != Some("self-insert-command")
    {
        return;
    }
    // Enough of a word to be worth asking about. Without this every space
    // bar asks for the whole of what the server knows.
    let start = crate::autocomplete::word_start_in(editor.current_buffer(), point);
    if point.saturating_sub(start) < editor.settings.autocomplete_min_chars.max(1) {
        // And the list that was up is for a word that is no longer there.
        editor.close_autocomplete();
        return;
    }
    editor.completions_asked_at = Some((id, point));
    crate::commands::lsp::ask_for_completions(editor);
}

/// Whether a server has started for this buffer's language — it records the
/// encoding it negotiated — rather than merely being configured.
#[cfg(feature = "full")]
fn server_running(editor: &Editor) -> bool {
    editor.current_buffer().language().is_some_and(|language| {
        editor
            .lsp_encodings
            .iter()
            .any(|(name, _)| name == language)
    })
}

/// Asks the language server what the symbol under point is, once the cursor
/// has been sitting on it long enough to look like a question.
///
/// `lsp-ui-doc`. Only once per place: an idle pause where nothing has moved
/// has already been answered, and asking again would be a request per pause
/// for as long as the editor is left alone.
#[cfg(feature = "full")]
fn ask_about_the_symbol_under_point(editor: &mut Editor, id: maxgus_text::BufferId) {
    if !editor.settings.lsp_doc || !editor.settings.lsp_enabled {
        return;
    }
    // Only where a server has actually started, or every pause in a plain
    // text file queues a request for nobody to answer.
    if !server_running(editor) {
        return;
    }
    let point = editor.windows.current().point;
    if editor.doc_asked_at == Some((id, point)) {
        return;
    }
    editor.doc_asked_at = Some((id, point));
    crate::commands::lsp::ask_for_doc(editor);
}

#[cfg(test)]
mod tests {
    /// Every front end has to do all of it.
    ///
    /// Crude — it reads the source and looks for the calls — and it is the
    /// only check there is: a window cannot be driven from a test the way a
    /// pseudo-terminal can, so nothing else would notice the window quietly
    /// dropping the idle work again. It noticed once already.
    #[test]
    fn both_front_ends_do_the_work_this_module_holds() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("the workspace root");
        let front_ends = [
            ("the terminal", workspace.join("crates/maxgus/src/app.rs")),
            (
                "the window",
                workspace.join("crates/maxgus-gui/src/window.rs"),
            ),
        ];
        for (name, path) in front_ends {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("{} is at {}", name, path.display()));
            for call in ["frontend::after_key", "frontend::on_idle"] {
                assert!(
                    source.contains(call),
                    "{name} never calls `{call}`, so it is not doing that work"
                );
            }
        }
    }
}
