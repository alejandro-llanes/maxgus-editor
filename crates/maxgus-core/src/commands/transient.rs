//! Driving a transient menu.
//!
//! While a menu is up it takes every key, so that a menu offering `-f` and
//! `p` is not quietly competing with whatever `f` and `p` mean underneath.
//! That is done with a minor keymap whose default binding catches everything
//! — the same mechanism a terminal window uses, and for the same reason.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
    transient::Action,
};

/// The mode name the menu's keymap goes under.
pub const TRANSIENT_MODE: &str = "transient-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "transient-dispatch",
            "Act on a key pressed in a menu.",
            dispatch,
            non_interactive
        ),
        command!("transient-quit", "Close the menu.", quit, non_interactive),
    ]);
}

/// What a key does in whatever menu is showing.
fn dispatch(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(key) = args.key else { return Ok(()) };
    let notation = key.notation();

    // Leaving is always available, and goes back one menu at a time so that
    // stepping into a submenu by mistake costs one key rather than the lot.
    if notation == "C-g" || notation == "q" || notation == "ESC" {
        return quit(editor, args);
    }

    let Some(active) = editor.transient.as_mut() else {
        return Ok(());
    };
    match active.press(&notation) {
        crate::transient::Press::More => Ok(()),
        crate::transient::Press::Do(Action::Switch(flag)) => {
            active.toggle(flag);
            Ok(())
        }
        crate::transient::Press::Do(Action::Prefix(name)) => {
            active.push(name);
            Ok(())
        }
        crate::transient::Press::Do(Action::Command(name)) => {
            // The switches go with the command, not with the menu: a command
            // reads them and they are gone by the time the next menu opens.
            let arguments = active.arguments();
            close(editor);
            editor.transient_arguments = arguments;
            editor.deferred = Some((name.to_string(), Args::new(editor.prefix, None)));
            Ok(())
        }
        crate::transient::Press::Unknown => {
            editor.message(format!("`{notation}` is not one of these"));
            Ok(())
        }
    }
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let empty = editor.transient.as_mut().is_none_or(|active| !active.pop());
    if empty {
        close(editor);
    }
    Ok(())
}

/// Opens `name`, taking the keyboard until it is closed.
pub fn open(editor: &mut Editor, name: &'static str) -> Result<()> {
    if crate::transient::find(name).is_none() {
        return Err(crate::CoreError::Message(format!(
            "There is no {name} menu"
        )));
    }
    editor.transient = Some(crate::transient::Active::new(name));
    editor.transient_arguments.clear();
    let Ok(map) = crate::keymap::transient_keymap() else {
        return Err(crate::CoreError::Message(
            "The menu keymap will not build".into(),
        ));
    };
    editor.keymaps.remove_minor(TRANSIENT_MODE);
    editor.keymaps.push_minor(map);
    Ok(())
}

/// Takes the menu down and gives the keyboard back.
pub fn close(editor: &mut Editor) {
    editor.transient = None;
    editor.keymaps.remove_minor(TRANSIENT_MODE);
}
