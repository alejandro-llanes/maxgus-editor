//! The panel's other two windows: the symbol outline and the buffer list.
//!
//! The file tree's commands are next door in `tree.rs`. These are for the two
//! windows stacked under it — each is an ordinary window with its own buffer,
//! its own point and its own keymap, which is what makes moving between them
//! ordinary window movement rather than a special case.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
    panel::PanelSection,
};

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "panel-toggle-symbol",
            "Expand or collapse the symbol here.",
            toggle_symbol,
            non_interactive
        ),
        command!(
            "panel-expand-symbol",
            "Expand the symbol here.",
            expand_symbol,
            non_interactive
        ),
        command!(
            "panel-collapse-symbol",
            "Collapse the symbol here.",
            collapse_symbol,
            non_interactive
        ),
        command!(
            "panel-goto-symbol",
            "Go to the symbol here in its buffer.",
            goto_symbol,
            non_interactive
        ),
        command!(
            "panel-switch-to-buffer",
            "Show the buffer here.",
            switch_to_buffer,
            non_interactive
        ),
        command!(
            "panel-kill-buffer",
            "Kill the buffer here.",
            kill_buffer,
            non_interactive
        ),
        command!(
            "panel-refresh-symbols",
            "Ask the language server for the outline again.",
            refresh_symbols
        ),
        command!("panel-quit", "Close the panel.", quit, non_interactive),
        command!(
            "panel-select-symbols",
            "Select the symbol outline window.",
            select_symbols
        ),
        command!(
            "panel-select-buffers",
            "Select the buffer list window.",
            select_buffers
        ),
        command!(
            "panel-toggle-tree-section",
            "Show or hide the file tree.",
            toggle_tree_section
        ),
        command!(
            "panel-toggle-symbols-section",
            "Show or hide the symbol outline.",
            toggle_symbols_section
        ),
        command!(
            "panel-toggle-buffers-section",
            "Show or hide the buffer list.",
            toggle_buffers_section
        ),
    ]);
}

// ---- which sections exist ----------------------------------------------

fn toggle_tree_section(editor: &mut Editor, _: &Args) -> Result<()> {
    set_section(editor, PanelSection::Tree)
}

fn toggle_symbols_section(editor: &mut Editor, _: &Args) -> Result<()> {
    set_section(editor, PanelSection::Symbols)
}

fn toggle_buffers_section(editor: &mut Editor, _: &Args) -> Result<()> {
    set_section(editor, PanelSection::Buffers)
}

/// Switching a section on or off rebuilds the column: which windows there are
/// is decided when it is built, so there is nothing to toggle in place.
fn set_section(editor: &mut Editor, section: PanelSection) -> Result<()> {
    let on = !editor.panel.is_enabled(section);
    // The last section cannot be switched off: an empty panel is a column of
    // nothing, which `C-x t t` already does better by closing it.
    //
    // Counted by what would actually appear, not by what is switched on: the
    // outline can be on and still have no window, when there is no language
    // server to ask.
    if !on && would_show(editor) <= 1 {
        return Err(crate::CoreError::Message(
            "The panel would have nothing left in it".into(),
        ));
    }
    editor.panel.set_enabled(section, on);
    if on && section == PanelSection::Symbols {
        editor.request_document_symbols();
    }
    crate::commands::tree::rebuild(editor)?;
    editor.message(format!(
        "{} {}",
        section.title(),
        if on { "shown" } else { "hidden" }
    ));
    Ok(())
}

/// How many windows the panel would have, as things stand.
fn would_show(editor: &Editor) -> usize {
    let mut count = 0;
    if editor.panel.is_enabled(PanelSection::Tree) {
        count += 1;
    }
    if editor.panel.is_enabled(PanelSection::Symbols) && editor.symbols_available() {
        count += 1;
    }
    if editor.panel.is_enabled(PanelSection::Buffers) {
        count += 1;
    }
    count
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    crate::commands::tree::close(editor);
    Ok(())
}

// ---- the outline --------------------------------------------------------

fn symbol_here(editor: &Editor) -> Result<usize> {
    editor
        .symbol_at_cursor()
        .ok_or_else(|| crate::CoreError::Message("No symbol here".into()))
}

pub fn toggle_symbol(editor: &mut Editor, _: &Args) -> Result<()> {
    let index = symbol_here(editor)?;
    if editor.panel.toggle_symbol(index) {
        // Point follows the symbol that was folded, which is the row acted
        // on: everything below it has just moved.
        let line = editor.panel.line_of_symbol(index);
        editor.render_symbols_buffer();
        if let Some(line) = line
            && let Some(id) = editor
                .buffers
                .find_by_name(crate::commands::tree::SYMBOLS_BUFFER_NAME)
        {
            editor.move_point_in(id, line);
        }
    }
    Ok(())
}

fn expand_symbol(editor: &mut Editor, _: &Args) -> Result<()> {
    let index = symbol_here(editor)?;
    if editor.panel.set_symbol_expanded(index, true) {
        editor.render_symbols_buffer();
    }
    Ok(())
}

fn collapse_symbol(editor: &mut Editor, _: &Args) -> Result<()> {
    let index = symbol_here(editor)?;
    if editor.panel.set_symbol_expanded(index, false) {
        editor.render_symbols_buffer();
    }
    Ok(())
}

/// Goes to the symbol point is on, in the window the outline belongs to.
///
/// The outline is scoped to one buffer, so this never has to open anything:
/// the file is already there, and the jump is a move within it.
pub fn goto_symbol(editor: &mut Editor, _: &Args) -> Result<()> {
    let index = symbol_here(editor)?;
    let Some(symbol) = editor.panel.symbols.get(index) else {
        return Err(crate::CoreError::Message("No symbol here".into()));
    };
    let (line, column) = (symbol.line, symbol.column);
    let buffer = editor
        .panel
        .symbols_buffer
        .ok_or_else(|| crate::CoreError::Message("The outline has no buffer".into()))?;

    let target = editing_window(editor)?;
    editor.select_window(target);
    if editor.windows.current().buffer != buffer {
        editor.switch_to_buffer(buffer)?;
    }
    let point = editor
        .buffers
        .get(buffer)
        .map(|b| b.offset_of(maxgus_text::Position::new(line, column)))
        .unwrap_or_default();
    editor.windows.current_mut().point = point;
    editor.with_current_buffer(move |b| b.set_point(point));
    editor.follow_point();
    Ok(())
}

fn refresh_symbols(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.request_document_symbols();
    Ok(())
}

// ---- the buffer list ----------------------------------------------------

fn buffer_here(editor: &Editor) -> Result<maxgus_text::BufferId> {
    editor
        .listed_buffer_at_cursor()
        .ok_or_else(|| crate::CoreError::Message("No buffer here".into()))
}

fn editing_window(editor: &Editor) -> Result<crate::window::WindowId> {
    editor
        .editing_window()
        .ok_or_else(|| crate::CoreError::Message("No window to show it in".into()))
}

/// Shows the buffer point is on, in the window beside the panel.
pub fn switch_to_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = buffer_here(editor)?;
    let target = editing_window(editor)?;
    editor.select_window(target);
    editor.switch_to_buffer(id)
}

fn kill_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = buffer_here(editor)?;
    let name = editor
        .buffers
        .get(id)
        .map(|b| b.name().to_string())
        .unwrap_or_default();
    editor.kill_buffer(id)?;
    editor.render_buffers_buffer();
    editor.message(format!("Killed {name}"));
    Ok(())
}

// ---- reaching a section straight off ------------------------------------

fn select_symbols(editor: &mut Editor, args: &Args) -> Result<()> {
    select_section(
        editor,
        args,
        crate::commands::tree::SYMBOLS_BUFFER_NAME,
        "outline",
    )
}

fn select_buffers(editor: &mut Editor, args: &Args) -> Result<()> {
    select_section(
        editor,
        args,
        crate::commands::tree::BUFFERS_BUFFER_NAME,
        "buffer list",
    )
}

/// Selects the panel window showing `name`, opening the panel if it is shut.
///
/// A section can be switched on and still have no window — the outline has
/// none while no language server is running — so a missing window is a thing
/// to say, not a thing to assert.
fn select_section(editor: &mut Editor, args: &Args, name: &str, what: &str) -> Result<()> {
    if editor.panel_windows.is_empty() {
        let root = editor.default_directory();
        crate::commands::tree::open(editor, root)?;
    }
    let target = editor
        .panel_windows
        .iter()
        .copied()
        .find(|id| {
            editor
                .windows
                .get(*id)
                .and_then(|w| editor.buffers.get(w.buffer))
                .is_some_and(|b| b.name() == name)
        })
        .ok_or_else(|| {
            let why = why_not_shown(editor, name);
            crate::CoreError::Message(format!("The {what} is not shown: {why}"))
        })?;
    editor.select_window(target);
    let _ = args;
    Ok(())
}

/// Why a section has no window, in the words of the setting to change or
/// the thing to start. "Not shown" alone sends the reader to the manual.
fn why_not_shown(editor: &Editor, name: &str) -> String {
    let (section, setting) = match name {
        crate::commands::tree::SYMBOLS_BUFFER_NAME => (PanelSection::Symbols, "panel-symbols"),
        _ => (PanelSection::Buffers, "panel-buffers"),
    };
    if !editor.panel.is_enabled(section) {
        return format!("`{setting}` is off");
    }
    if section != PanelSection::Symbols {
        return "it has no window".into();
    }
    if !cfg!(feature = "full") {
        return "this build has no language server support".into();
    }
    if !editor.settings.lsp_enabled {
        return "`lsp-enabled` is off, and the outline comes from the language server".into();
    }
    let language = editor
        .editing_buffer()
        .and_then(|id| editor.buffers.get(id))
        .and_then(|buffer| buffer.language().map(str::to_string));
    match language {
        Some(language) => format!("no language server is running for {language}"),
        None => "the buffer being edited has no language, so no server to ask".into(),
    }
}
