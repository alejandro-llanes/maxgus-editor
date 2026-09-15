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

/// How many script commands may be running inside one another before the
/// next is refused: a command that runs itself would otherwise run until
/// the stack ran out.
const MAX_SCRIPT_DEPTH: usize = 16;

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
        region: buffer.region().map(|region| buffer.slice(region)),
        region_start: buffer.region().map(|region| region.start),
        region_end: buffer.region().map(|region| region.end),
    }
}

/// Runs a script command and applies what it asked for.
///
/// In the order it asked: a `run` of one of the editor's commands happens
/// between the edits either side of it, and every `run` happens. Handing the
/// command to the dispatcher to run once this one had finished dropped
/// everything the script asked for after its first `run`, the second
/// command of a script that ran two included.
///
/// The edits are one step for `C-/`, as a built-in command's are.
pub fn run(editor: &mut Editor, name: &str, registry: &Registry) -> Result<()> {
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
    if editor.script_depth >= MAX_SCRIPT_DEPTH {
        return Err(CoreError::Message(format!(
            "{name}: scripts running scripts went {MAX_SCRIPT_DEPTH} deep, and were stopped"
        )));
    }
    let buffer = editor.current_buffer_id();
    editor.with_buffer(buffer, |b| b.begin_undo_group(false));
    editor.script_depth += 1;
    let outcome = apply(editor, actions, registry);
    editor.script_depth -= 1;
    editor.with_buffer(buffer, |b| b.commit_undo_group());
    outcome
}

fn apply(editor: &mut Editor, actions: Vec<Action>, registry: &Registry) -> Result<()> {
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
            // What a script composes: the editor's own commands, in turn.
            // One that fails stops the script there, as a failing step of a
            // keyboard macro stops the macro.
            Action::Run(command) => registry.execute(editor, &command, &Args::default())?,
            Action::Fail(why) => return Err(CoreError::Message(why)),
        }
    }
    Ok(())
}

/// Where the script is: where it was read from, or where it would be.
///
/// Asking only where it was read from meant a script written after the
/// editor started could not be loaded without starting it again —
/// `reload-scripts` said there was no script file.
pub fn script_path(editor: &Editor) -> Option<std::path::PathBuf> {
    editor.script_path.clone().or_else(|| {
        editor
            .config_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(|directory| directory.join("init.rhai"))
    })
}

fn reload(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = script_path(editor)
        .ok_or_else(|| CoreError::Message("There is no configuration directory".into()))?;
    // Remembered now, so that a script that is not there is said to be
    // missing rather than passed over as it is at startup.
    editor.script_path = Some(path.clone());
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
