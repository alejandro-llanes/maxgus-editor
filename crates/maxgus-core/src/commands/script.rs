//! Running the commands a script defined.
//!
//! A script does not get the editor; it gets a description of what is on
//! screen and asks for a list of changes. This is where the description is
//! built and the changes are applied — and where a script that fails is made
//! to leave nothing behind.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::{CoreError, Result};
use maxgus_script::{Action, Context};

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "reload-scripts",
            "Read the scripts again, taking up any changes.",
            reload
        ),
        command!(
            "list-script-commands",
            "Show what the loaded scripts define.",
            list
        ),
    ]);
}

/// What a script is told about the editor.
pub fn context(editor: &Editor) -> Context {
    let buffer = editor.current_buffer();
    let point = editor.windows.current().point.min(buffer.len_chars());
    let line = buffer.line_of(point);
    Context {
        text: buffer.text(),
        point,
        line,
        column: point - buffer.line_start(line),
        buffer: buffer.name().to_string(),
        path: buffer.path().map(|p| p.display().to_string()),
        mode: editor.current_mode_name(),
        region: editor
            .current_buffer()
            .region()
            .map(|region| editor.current_buffer().slice(region)),
    }
}

/// Runs a script command and applies what it asked for.
pub fn run(editor: &mut Editor, name: &str) -> Result<()> {
    let context = context(editor);
    let actions = {
        let script = editor
            .script
            .as_ref()
            .ok_or_else(|| CoreError::UnknownCommand(name.to_string()))?;
        script
            .call(name, &context)
            .map_err(|error| CoreError::Message(format!("{name}: {error}")))?
    };
    // A `fail` anywhere means none of it happens: a script that noticed
    // something wrong should not have its earlier edits kept.
    if let Some(Action::Fail(why)) = actions.iter().find(|a| matches!(a, Action::Fail(_))) {
        return Err(CoreError::Message(why.clone()));
    }
    apply(editor, actions)
}

fn apply(editor: &mut Editor, actions: Vec<Action>) -> Result<()> {
    for action in actions {
        match action {
            Action::Insert(text) => {
                editor.with_current_buffer(move |b| b.insert_at_point(&text))?;
                editor.follow_point();
            }
            Action::Delete(count) => {
                let range = {
                    let buffer = editor.current_buffer();
                    let point = buffer.point();
                    maxgus_text::Range::new(point, (point + count).min(buffer.len_chars()))
                };
                if !range.is_empty() {
                    editor.with_current_buffer(move |b| b.delete(range))?;
                }
                editor.follow_point();
            }
            Action::Goto(offset) => editor.move_point_to(offset),
            Action::Message(text) => editor.message(text),
            // Handing control to another command is what `deferred` is for,
            // and it is how a script composes what the editor already does.
            Action::Run(command) => {
                editor.deferred = Some((command, Args::default()));
                return Ok(());
            }
            Action::Fail(why) => return Err(CoreError::Message(why)),
        }
    }
    Ok(())
}

fn reload(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = editor
        .script_path
        .clone()
        .ok_or_else(|| CoreError::Message("There is no script file".into()))?;
    editor.spawn(crate::task::Task::ReadScript { path });
    Ok(())
}

fn list(editor: &mut Editor, _: &Args) -> Result<()> {
    let script = editor
        .script
        .as_ref()
        .ok_or_else(|| CoreError::Message("No script is loaded".into()))?;
    if script.commands().is_empty() {
        return Err(CoreError::Message("The script defines no commands".into()));
    }
    let mut text = String::from("Commands defined by scripts\n\n");
    for command in script.commands() {
        text.push_str(&format!("{:<28}{}\n", command.name, command.doc));
    }
    crate::commands::help::show_help(editor, &text)
}
