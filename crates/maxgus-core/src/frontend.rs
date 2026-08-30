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
    follow_tree(editor);
}

/// Replays the last keyboard macro, if a command asked for it.
fn replay_macro(editor: &mut Editor, dispatcher: &mut Dispatcher) {
    let repeats = std::mem::take(&mut editor.macro_repeats);
    if repeats == 0 {
        return;
    }
    let keys = editor.last_macro.clone();
    editor.replaying_macro = true;
    for _ in 0..repeats {
        for key in &keys {
            dispatcher.handle_key(editor, *key);
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
        editor.select_tree_path(&path);
    } else {
        editor.spawn(crate::task::Task::Tree(crate::task::TreeAction::Reveal(
            path,
        )));
    }
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
    editor.sync_language_server(id);
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
    // Enough of a word to be worth asking about. Without this every space
    // bar asks for the whole of what the server knows.
    let text = editor.current_buffer().text();
    let start = crate::autocomplete::word_start(&text, point);
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
