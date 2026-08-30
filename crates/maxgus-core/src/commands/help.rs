//! Help commands: the `C-h` family.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_keys::KeySequence;

/// The buffer help is written into.
pub const HELP_BUFFER_NAME: &str = "*Help*";

/// Registers the help commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "describe-key",
            "Say what a key sequence runs.",
            describe_key
        ),
        command!(
            "describe-function",
            "Show a command's documentation.",
            describe_function
        ),
        command!(
            "describe-variable",
            "Show a setting's value.",
            describe_variable
        ),
        command!(
            "describe-bindings",
            "List every active key binding.",
            describe_bindings
        ),
        command!(
            "describe-mode",
            "Describe the current buffer's mode.",
            describe_mode
        ),
        command!("where-is", "Say which keys run a command.", where_is),
        command!(
            "help-with-tutorial",
            "Show a short guide to the editor.",
            tutorial
        ),
    ]);
}

/// Shows `text` in the help buffer, without disturbing the tree window.
pub fn show_help(editor: &mut Editor, text: &str) -> Result<()> {
    let id = match editor.buffers.find_by_name(HELP_BUFFER_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, text)?;
            id
        }
        None => editor.buffers.create_with_text(HELP_BUFFER_NAME, text),
    };
    editor
        .buffers
        .get_mut(id)
        .expect("just created")
        .set_read_only(true);
    editor.show_in_editing_window(id)
}

/// `C-h k`: reads a key sequence and reports what it runs.
fn describe_key(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(key) = args.read_char else {
        editor.read_char("describe-key", "Describe key: ");
        return Ok(());
    };
    let sequence = KeySequence::from(key).canonicalize_escape_prefix();
    let lookup = editor.keymaps.lookup(&sequence);
    let text = match lookup {
        maxgus_keys::Lookup::Command(name) => {
            let documentation = editor
                .command_docs
                .iter()
                .find(|(command, _)| *command == name)
                .map(|(_, doc)| doc.clone())
                .unwrap_or_else(|| "Undocumented.".to_string());
            format!(
                "{} runs the command `{name}`\n\n{documentation}\n",
                sequence.notation()
            )
        }
        // A prefix is answered rather than left hanging, so `C-h k C-x` says
        // something useful instead of waiting for a key that never comes.
        maxgus_keys::Lookup::Prefix => {
            format!("{} is a prefix key\n", sequence.notation())
        }
        maxgus_keys::Lookup::Undefined => {
            format!("{} is undefined\n", sequence.notation())
        }
    };
    editor.message(text.lines().next().unwrap_or_default().to_string());
    show_help(editor, &text)
}

fn describe_function(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "describe-function",
            MinibufferKind::Command,
            "Describe function: ",
            "",
            editor.command_names.clone(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    let Some((_, documentation)) = editor
        .command_docs
        .iter()
        .find(|(command, _)| *command == name)
        .cloned()
    else {
        return Err(crate::CoreError::Message(format!(
            "No command named `{name}`"
        )));
    };
    let bindings = editor.keymaps.where_is(&name);
    let keys = if bindings.is_empty() {
        "It is not bound to any key.".to_string()
    } else {
        format!(
            "It is bound to {}.",
            bindings
                .iter()
                .map(KeySequence::notation)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    show_help(editor, &format!("{name}\n\n{keys}\n\n{documentation}\n"))
}

/// `C-h v`: reports the value of a setting.
fn describe_variable(editor: &mut Editor, args: &Args) -> Result<()> {
    let names: Vec<String> = maxgus_config::settings::SETTING_NAMES
        .iter()
        .map(|n| n.to_string())
        .collect();
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "describe-variable",
            MinibufferKind::Command,
            "Describe variable: ",
            "",
            names,
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    let Some(value) = setting_value(editor, &name) else {
        return Err(crate::CoreError::Message(format!(
            "No variable named `{name}`"
        )));
    };
    show_help(editor, &format!("{name}\n\nIts value is {value}.\n"))
}

/// The current value of `name`, rendered for display.
fn setting_value(editor: &Editor, name: &str) -> Option<String> {
    let s = &editor.settings;
    Some(match name {
        "tab-width" => s.tab_width.to_string(),
        "indent-with-tabs" => s.indent_with_tabs.to_string(),
        "theme" => format!("\"{}\"", s.theme),
        "line-numbers" => s.line_numbers.to_string(),
        "truncate-lines" => s.truncate_lines.to_string(),
        "scroll-margin" => s.scroll_margin.to_string(),
        "fill-column" => s.fill_column.to_string(),
        "kill-ring-max" => s.kill_ring_max.to_string(),
        "case-fold-search" => match s.case_fold_search {
            Some(b) => b.to_string(),
            None => "smart (fold unless the search string has an uppercase letter)".into(),
        },
        "require-final-newline" => s.require_final_newline.to_string(),
        "delete-trailing-whitespace" => s.delete_trailing_whitespace.to_string(),
        "backup-files" => s.backup_files.to_string(),
        "syntax-highlighting" => s.syntax_highlighting.to_string(),
        "lsp-enabled" => s.lsp_enabled.to_string(),
        "idle-delay-ms" => s.idle_delay_ms.to_string(),
        "fill-column-indicator" => s.fill_column_indicator.to_string(),
        "blink-cursor" => s.blink_cursor.to_string(),
        "echo-keystrokes-ms" => s.echo_keystrokes_ms.to_string(),
        "nerd-font-icons" => s.nerd_font_icons.to_string(),
        "panel-tree" => s.panel_tree.to_string(),
        "panel-symbols" => s.panel_symbols.to_string(),
        "panel-buffers" => s.panel_buffers.to_string(),
        "panel-at-startup" => s.panel_at_startup.to_string(),
        "panel-symbols-height" => s.panel_symbols_height.to_string(),
        "panel-buffers-height" => s.panel_buffers_height.to_string(),
        "beacon" => s.beacon.to_string(),
        "beacon-size" => s.beacon_size.to_string(),
        "beacon-blink-delay-ms" => s.beacon_blink_delay_ms.to_string(),
        "beacon-blink-duration-ms" => s.beacon_blink_duration_ms.to_string(),
        "beacon-color" => s.beacon_color.clone(),
        "beacon-blink-when-buffer-changes" => s.beacon_blink_when_buffer_changes.to_string(),
        "beacon-blink-when-window-scrolls" => s.beacon_blink_when_window_scrolls.to_string(),
        "beacon-blink-when-window-changes" => s.beacon_blink_when_window_changes.to_string(),
        "beacon-blink-when-point-moves-vertically" => {
            s.beacon_blink_when_point_moves_vertically.to_string()
        }
        "session" => s.session.to_string(),
        "gui-font" => s.gui_font.clone(),
        "gui-font-size" => s.gui_font_size.to_string(),
        "lsp-doc" => s.lsp_doc.to_string(),
        "which-key" => s.which_key.to_string(),
        "which-key-delay-ms" => s.which_key_delay_ms.to_string(),
        "mouse-wheel-lines" => s.mouse_wheel_lines.to_string(),
        "smooth-scroll-ms" => s.smooth_scroll_ms.to_string(),
        "shell" => s.shell.clone().unwrap_or_else(|| "(from $SHELL)".into()),
        _ => return None,
    })
}

fn describe_bindings(editor: &mut Editor, _: &Args) -> Result<()> {
    let mut text = String::from("Key bindings\n\nkey                 binding\n");
    text.push_str(&"-".repeat(50));
    text.push('\n');
    for (sequence, command) in editor.keymaps.bindings() {
        text.push_str(&format!("{:<20}{command}\n", sequence.notation()));
    }
    show_help(editor, &text)
}

fn describe_mode(editor: &mut Editor, _: &Args) -> Result<()> {
    let (name, mode, path, encoding) = {
        let buffer = editor.current_buffer();
        (
            buffer.name().to_string(),
            buffer.language().unwrap_or("Fundamental").to_string(),
            buffer
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none".into()),
            match buffer.line_ending() {
                maxgus_text::LineEnding::Lf => "LF",
                maxgus_text::LineEnding::Crlf => "CRLF",
            },
        )
    };
    #[cfg(feature = "full")]
    let highlighting = if maxgus_syntax::is_supported(&mode) {
        "A tree-sitter grammar is available for this mode."
    } else {
        "No tree-sitter grammar is available for this mode."
    };
    #[cfg(not(feature = "full"))]
    let highlighting = "This build has no tree-sitter grammars in it.";
    let server = match editor.settings.lsp_enabled {
        true => "Language server support is on.",
        false => "Language server support is off.",
    };
    show_help(
        editor,
        &format!(
            "Buffer `{name}`\n\nMajor mode: {mode}\nFile: {path}\nLine endings: {encoding}\n\n\
             {highlighting}\n{server}\n"
        ),
    )
}

fn where_is(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "where-is",
            MinibufferKind::Command,
            "Where is command: ",
            "",
            editor.command_names.clone(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    let bindings = editor.keymaps.where_is(&name);
    if bindings.is_empty() {
        editor.message(format!("`{name}` is not on any key"));
        return Ok(());
    }
    let keys: Vec<String> = bindings.iter().map(KeySequence::notation).collect();
    editor.message(format!("`{name}` is on {}", keys.join(", ")));
    Ok(())
}

/// A short guide, so a first run is not a blank screen with no way out.
fn tutorial(editor: &mut Editor, _: &Args) -> Result<()> {
    show_help(editor, TUTORIAL)
}

const TUTORIAL: &str = "\
maxgus — a short guide

Keys are written as in Emacs: C-x means Control and x together, M-x means
Meta (Alt, or Escape first) and x.

Getting out
  C-g            abandon whatever is in progress
  C-x C-c        leave the editor

Moving
  C-f  C-b       forward and back one character
  C-n  C-p       down and up one line
  M-f  M-b       forward and back one word
  C-a  C-e       start and end of the line
  M-<  M->       start and end of the buffer
  C-v  M-v       forward and back one screenful

Editing
  DEL  C-d       delete backwards and forwards
  C-k            kill to the end of the line
  C-SPC          set the mark; move, then C-w cuts or M-w copies
  C-y            yank back what was killed; M-y cycles through earlier kills
  C-/            undo

Files and buffers
  C-x C-f        find a file
  C-x C-s        save
  C-x b          switch buffer
  C-x k          kill a buffer

Windows
  C-x 2  C-x 3   split below and beside
  C-x o          go to the other window
  C-x 0  C-x 1   delete this window, or every other one

Searching
  C-s  C-r       search forward and back; C-s again finds the next
  M-%            query-replace

The file tree
  C-x t t        show or hide it; press ? inside it for its own keys

Finding out more
  C-h k          say what a key does
  C-h b          list every binding
  M-x            run any command by name
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup() -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let registry = crate::commands::standard_registry();
        editor.command_names = registry.interactive_names();
        editor.command_docs = registry
            .iter()
            .map(|c| (c.name.to_string(), c.doc.to_string()))
            .collect();
        (Dispatcher::new(registry), editor)
    }

    fn run(d: &mut Dispatcher, e: &mut Editor, command: &str) {
        let out = d.execute(e, command, None);
        assert!(
            !matches!(out, Dispatch::Failed { .. }),
            "`{command}` failed: {out:?}"
        );
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
    fn describe_key_reads_a_key_and_names_its_command() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-key");
        assert!(e.pending_char.is_some());
        d.handle_keys(&mut e, "C-f");

        let text = e.current_buffer().text();
        assert!(
            text.contains("C-f runs the command `forward-char`"),
            "got `{text}`"
        );
        assert!(
            text.contains("Move point forward"),
            "the documentation, got `{text}`"
        );
    }

    #[test]
    fn describe_key_recognises_a_prefix_and_an_unbound_key() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-key");
        d.handle_keys(&mut e, "C-x");
        assert!(e.current_buffer().text().contains("is a prefix key"));

        run(&mut d, &mut e, "describe-key");
        d.handle_keys(&mut e, "<f12>");
        assert!(e.current_buffer().text().contains("is undefined"));
    }

    #[test]
    fn describe_function_shows_the_documentation_and_the_keys() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-function");
        answer(&mut d, &mut e, "save-buffer");
        let text = e.current_buffer().text();
        assert!(text.starts_with("save-buffer"), "got `{text}`");
        assert!(text.contains("C-x C-s"), "got `{text}`");
        assert!(text.contains("Save this buffer"), "got `{text}`");
    }

    #[test]
    fn describe_function_says_when_a_command_is_on_no_key() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-function");
        answer(&mut d, &mut e, "rename-buffer");
        assert!(e.current_buffer().text().contains("not bound to any key"));
    }

    #[test]
    fn describe_function_refuses_a_name_it_does_not_know() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-function");
        e.minibuffer.kill_whole();
        for c in "no-such-command".chars() {
            e.minibuffer.insert_char(c);
        }
        assert!(matches!(
            d.handle_keys(&mut e, "RET"),
            Dispatch::Failed { .. }
        ));
    }

    #[test]
    fn describe_variable_reports_the_current_value() {
        let (mut d, mut e) = setup();
        e.settings.tab_width = 8;
        run(&mut d, &mut e, "describe-variable");
        answer(&mut d, &mut e, "tab-width");
        assert!(e.current_buffer().text().contains("Its value is 8"));
    }

    #[test]
    fn every_setting_name_has_a_value_to_report() {
        let (_d, e) = setup();
        for name in maxgus_config::settings::SETTING_NAMES {
            assert!(setting_value(&e, name).is_some(), "`{name}` has no value");
        }
        assert!(setting_value(&e, "not-a-setting").is_none());
    }

    #[test]
    fn the_smart_case_setting_explains_itself() {
        let (_d, e) = setup();
        let value = setting_value(&e, "case-fold-search").unwrap();
        assert!(value.starts_with("smart"), "got `{value}`");
    }

    #[test]
    fn describe_bindings_lists_the_whole_keymap() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-bindings");
        let text = e.current_buffer().text();
        assert!(text.contains("C-x C-f"), "got a listing missing find-file");
        assert!(text.contains("save-buffer"));
        // Every global binding should appear.
        assert!(text.lines().count() > crate::keymap::GLOBAL_BINDINGS.len());
    }

    #[cfg(feature = "full")]
    #[test]
    fn describe_mode_reports_what_the_buffer_is() {
        let (mut d, mut e) = setup();
        let id = e.buffers.visit_file("/project/main.rs", "fn main() {}");
        e.switch_to_buffer(id).unwrap();
        run(&mut d, &mut e, "describe-mode");
        let text = e.current_buffer().text();
        assert!(text.contains("Major mode: rust"), "got `{text}`");
        assert!(text.contains("/project/main.rs"), "got `{text}`");
        assert!(
            text.contains("A tree-sitter grammar is available"),
            "got `{text}`"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn describe_mode_is_honest_about_a_mode_with_no_grammar() {
        let (mut d, mut e) = setup();
        let id = e.buffers.visit_file("/project/notes.md", "");
        e.switch_to_buffer(id).unwrap();
        run(&mut d, &mut e, "describe-mode");
        assert!(e.current_buffer().text().contains("No tree-sitter grammar"));
    }

    #[test]
    fn where_is_names_the_keys_a_command_is_on() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "where-is");
        answer(&mut d, &mut e, "undo");
        let message = e.minibuffer.display();
        assert!(message.contains("C-/"), "got `{message}`");
        assert!(message.contains("C-x u"), "got `{message}`");
    }

    #[test]
    fn where_is_says_when_a_command_is_on_nothing() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "where-is");
        answer(&mut d, &mut e, "rename-buffer");
        assert!(e.minibuffer.display().contains("not on any key"));
    }

    #[test]
    fn the_tutorial_covers_getting_in_and_out() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "help-with-tutorial");
        let text = e.current_buffer().text();
        for essential in ["C-g", "C-x C-c", "C-x C-f", "C-x C-s", "M-x", "C-h k"] {
            assert!(
                text.contains(essential),
                "the guide never mentions `{essential}`"
            );
        }
    }

    #[test]
    fn help_reuses_one_buffer() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "describe-bindings");
        run(&mut d, &mut e, "help-with-tutorial");
        assert_eq!(
            e.buffers
                .iter()
                .filter(|b| b.name() == HELP_BUFFER_NAME)
                .count(),
            1
        );
        assert!(e.current_buffer().is_read_only());
    }
}
