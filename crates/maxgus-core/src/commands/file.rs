//! File commands.
//!
//! Nothing here touches the filesystem directly. Each command works out what
//! it wants done and queues a [`Task`]; the event loop runs it on tokio and
//! hands the answer back through `Editor::apply_task_result`. That is what
//! keeps a slow disk from freezing redisplay, and what makes these commands
//! testable without any I/O at all.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    task::{Task, WriteGuard},
};
use std::path::{Path, PathBuf};

/// Registers the file commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "delete-this-file",
            "Delete the file this buffer is visiting, and the buffer with it.",
            delete_this_file
        ),
        command!(
            "move-this-file",
            "Rename or move the file this buffer is visiting.",
            move_this_file
        ),
        command!(
            "copy-this-file",
            "Write a copy of this file somewhere else.",
            copy_this_file
        ),
        command!(
            "yank-buffer-path",
            "Put this file's path in the kill ring.",
            yank_buffer_path
        ),
        command!(
            "yank-buffer-path-relative-to-project",
            "Put this file's path within the project in the kill ring.",
            yank_relative_path
        ),
        command!(
            "open-externally",
            "Open this file with whatever the desktop opens it with.",
            open_externally
        ),
        command!("find-file", "Visit a file in this window.", find_file),
        command!(
            "view-image-at-point",
            "Open the picture the line refers to, in another window.",
            view_image_at_point
        ),
        command!(
            "find-file-other-window",
            "Visit a file in another window.",
            find_file_other_window
        ),
        command!(
            "find-alternate-file",
            "Visit another file in place of this one.",
            find_alternate_file
        ),
        command!("save-buffer", "Save this buffer to its file.", save_buffer),
        command!(
            "save-buffer-anyway",
            "Save over a file that has changed on disk.",
            save_buffer_anyway
        ),
        command!(
            "write-file",
            "Save this buffer under another name.",
            write_file
        ),
        command!(
            "save-some-buffers",
            "Save every buffer with unsaved changes.",
            save_some_buffers
        ),
        command!(
            "insert-file",
            "Insert a file's contents at point.",
            insert_file
        ),
        command!(
            "revert-buffer",
            "Re-read this buffer from its file.",
            revert_buffer
        ),
        command!(
            "set-buffer-file-coding-system",
            "Choose the line endings this buffer is saved with.",
            set_buffer_file_coding_system
        ),
        command!(
            "save-buffers-kill-terminal",
            "Leave the editor, refusing while a buffer has unsaved changes.",
            kill_terminal
        ),
    ]);
}

/// Expands `~` and makes the path absolute against the default directory, the
/// way a file prompt is expected to behave.
///
/// Kept free of the environment so it can be tested directly.
pub(crate) fn expand_against(directory: &Path, home: Option<&Path>, input: &str) -> PathBuf {
    let text = start_again(input.trim());
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(rest);
    }
    if text == "~"
        && let Some(home) = home
    {
        return home.to_path_buf();
    }
    let path = PathBuf::from(text);
    if path.is_absolute() {
        path
    } else {
        directory.join(path)
    }
}

/// What is left of a typed path once a new one has been started inside it.
///
/// A file prompt opens holding a directory, so a path typed straight in is
/// typed after it: `/home/me/project/~/notes.txt`, `/home/me/project//etc`.
/// Emacs reads `//` as the root starting again and `/~` as home starting
/// again, and so does this — taken literally, the first made a directory
/// called `~` inside the project on the next save.
fn start_again(text: &str) -> &str {
    let mut start = 0;
    let bytes = text.as_bytes();
    for at in 1..bytes.len() {
        if bytes[at - 1] != b'/' {
            continue;
        }
        match bytes[at] {
            b'/' => start = at,
            b'~' if bytes.get(at + 1).is_none_or(|next| *next == b'/') => start = at,
            _ => {}
        }
    }
    &text[start..]
}

/// A file prompt's input with a path started again inside it made the whole
/// of it, and `~` spelt out — what Emacs' `substitute-in-file-name` makes of
/// it — or `None` when there is nothing to substitute.
///
/// Done to the prompt as it is typed, so completion is over the directory
/// the input now names: `TAB` after `/project/~/no` looked for files in the
/// project called `~`.
pub(crate) fn substitute_input(input: &str, home: Option<&Path>) -> Option<String> {
    let restarted = start_again(input);
    let expanded = match restarted.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", home?.display())
        }
        _ => restarted.to_string(),
    };
    (expanded != input).then_some(expanded)
}

/// The same, against the editor's default directory and real `HOME`.
///
/// Shared with the tree, so `~/src` means there whether it was typed at a
/// file prompt or at the box that asks the tree which directory to add.
pub(crate) fn expand(editor: &Editor, input: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    expand_against(&editor.default_directory(), home.as_deref(), input)
}

/// Opens a file prompt pre-filled with the default directory, and asks for a
/// listing of it so TAB has something to complete against.
fn prompt_for_file(editor: &mut Editor, command: &str, verb: &str) {
    let directory = editor.default_directory();
    let mut initial = directory.to_string_lossy().into_owned();
    if !initial.ends_with('/') {
        initial.push('/');
    }
    // Opening the prompt asks for the listing of the directory it holds, and
    // for another whenever what is typed moves into one.
    editor.prompt_for(
        command,
        MinibufferKind::File,
        format!("{verb}: "),
        &initial,
        Vec::new(),
    );
}

/// Visits `path`, reusing an open buffer rather than re-reading from disk.
fn visit(editor: &mut Editor, path: PathBuf, other_window: bool) -> Result<()> {
    if let Some(id) = editor.buffers.find_by_path(&path) {
        if other_window {
            editor.select_other_editing_window()?;
        }
        return editor.switch_to_buffer(id);
    }
    editor.spawn(Task::ReadFile {
        path,
        reverting: None,
        other_window,
    });
    Ok(())
}

fn find_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        prompt_for_file(editor, "find-file", "Find file");
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No file name given".into()));
    }
    let path = expand(editor, &input);
    visit(editor, path, false)
}

fn find_file_other_window(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        prompt_for_file(
            editor,
            "find-file-other-window",
            "Find file in other window",
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No file name given".into()));
    }
    let path = expand(editor, &input);
    visit(editor, path, true)
}

/// `C-c o i`: the picture the line under point refers to — `![alt](path)`
/// in markdown, `<img src>` in HTML, or a bare path — in another window.
/// A relative path is taken from the file's own directory, which is where
/// the document's readers would resolve it from.
fn view_image_at_point(editor: &mut Editor, _args: &Args) -> Result<()> {
    let buffer = editor.current_buffer();
    let line = buffer.line_of(buffer.point());
    let column = buffer.point() - buffer.line_start(line);
    let text = buffer.line_text(line);
    let Some(target) = crate::picture::reference_at(&text, column) else {
        return Err(crate::CoreError::Message(
            "No picture is referred to on this line".into(),
        ));
    };
    let base = buffer
        .path()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| editor.default_directory());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = expand_against(&base, home.as_deref(), &target);
    visit(editor, path, true)
}

/// `C-x C-v`: replaces the current buffer with another file.
fn find_alternate_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let current = editor
            .current_buffer()
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        editor.prompt_for(
            "find-alternate-file",
            MinibufferKind::File,
            "Find alternate file: ",
            &current,
            Vec::new(),
        );
        return Ok(());
    };
    let id = editor.current_buffer_id();
    let unsaved = editor
        .buffers
        .get(id)
        .is_some_and(|b| b.is_modified() && b.path().is_some());
    if unsaved && !args.prefix.is_present() {
        return Err(crate::CoreError::Message(
            "Buffer has unsaved changes; C-u C-x C-v replaces it anyway".into(),
        ));
    }
    let path = expand(editor, &input);
    // `C-x C-v RET` on the file already visited is how Emacs reads it again
    // from the disk. Visiting it found this very buffer, and killing "the
    // old one" afterwards killed the file that had just been asked for.
    if editor.buffers.get(id).and_then(|b| b.path()) == Some(path.as_path()) {
        editor.spawn(Task::ReadFile {
            path,
            reverting: Some(id),
            other_window: false,
        });
        return Ok(());
    }
    visit(editor, path, false)?;
    // Only drop the old buffer once there is somewhere else to go.
    if editor.buffers.len() > 1 {
        editor.kill_buffer(id).ok();
    }
    Ok(())
}

/// Prepares a buffer's text for disk, applying the save-time settings.
fn contents_for_disk(editor: &mut Editor, id: maxgus_text::BufferId) -> Result<String> {
    if editor.trims_trailing_whitespace(id) {
        editor
            .with_buffer(id, |buffer| buffer.delete_trailing_whitespace())
            .ok_or(crate::CoreError::NoSuchBuffer)??;
    }
    if editor.requires_final_newline(id) {
        editor
            .with_buffer(id, |buffer| -> Result<()> {
                if !buffer.is_empty() && buffer.char_before(buffer.len_chars()) != Some('\n') {
                    let end = buffer.len_chars();
                    let point = buffer.point();
                    buffer.insert(end, "\n")?;
                    // The newline goes after the text, not in front of point.
                    buffer.set_point(point);
                }
                Ok(())
            })
            .ok_or(crate::CoreError::NoSuchBuffer)??;
    }
    let buffer = editor
        .buffers
        .get(id)
        .ok_or(crate::CoreError::NoSuchBuffer)?;
    Ok(buffer.to_disk_string())
}

/// Queues a write of `id` to `path`.
pub(crate) fn write(editor: &mut Editor, id: maxgus_text::BufferId, path: PathBuf) -> Result<()> {
    refuse_unsaveable(editor, id, &path)?;
    // A file that did not exist when it was visited must still not exist:
    // otherwise whatever somebody else put there since is gone.
    let guard = match editor.new_files.contains(&id) {
        true => WriteGuard::Absent,
        false => WriteGuard::Unchanged(editor.buffers.get(id).and_then(|b| b.disk_time())),
    };
    write_guarded(editor, id, path, guard)
}

/// Refuses to save what saving would damage.
///
/// A picture's buffer holds a caption, and `C-x C-q` then a save wrote the
/// caption over the image. A buffer read from bytes that are not UTF-8 holds
/// replacement characters where those bytes were; that one can be written
/// on purpose, with `save-buffer-anyway`, never by accident.
fn refuse_unsaveable(editor: &mut Editor, id: maxgus_text::BufferId, path: &Path) -> Result<()> {
    let name = editor
        .buffers
        .get(id)
        .map(|b| b.name().to_string())
        .unwrap_or_default();
    if editor.pictures.contains_key(&id) {
        return Err(crate::CoreError::Message(format!(
            "{name} shows a picture; there is no text in it to save"
        )));
    }
    if editor.lossy_buffers.contains(&id) {
        editor.pending_overwrite = Some((id, path.to_path_buf()));
        return Err(crate::CoreError::Message(format!(
            "{name} was read from bytes that are not text, and saving would replace them; \
             M-x save-buffer-anyway writes it regardless"
        )));
    }
    Ok(())
}

fn write_guarded(
    editor: &mut Editor,
    id: maxgus_text::BufferId,
    path: PathBuf,
    guard: WriteGuard,
) -> Result<()> {
    let contents = contents_for_disk(editor, id)?;
    let backup = editor.settings.backup_files;
    editor.spawn(Task::WriteFile {
        path,
        contents,
        buffer: id,
        backup,
        guard,
    });
    Ok(())
}

fn save_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    let (path, modified) = {
        let buffer = editor
            .buffers
            .get(id)
            .ok_or(crate::CoreError::NoSuchBuffer)?;
        (buffer.path().map(Path::to_path_buf), buffer.is_modified())
    };
    let Some(path) = path else {
        // A buffer with no file needs a name before it can be saved.
        return write_file(editor, args);
    };
    if !modified {
        editor.message("(No changes need to be saved)");
        return Ok(());
    }
    write(editor, id, path)
}

/// The way past a refused save. Deliberately a separate command rather than a
/// second press of `C-x C-s`: overwriting somebody else's change is not
/// something to do by repeating a key that has just failed.
fn save_buffer_anyway(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some((id, path)) = editor.pending_overwrite.take() else {
        return Err(crate::CoreError::Message(
            "No save is waiting to be forced".into(),
        ));
    };
    if editor.buffers.get(id).is_none() {
        return Err(crate::CoreError::NoSuchBuffer);
    }
    if editor.pictures.contains_key(&id) {
        return Err(crate::CoreError::Message(
            "A picture's buffer has no text to save".into(),
        ));
    }
    write_guarded(editor, id, path, WriteGuard::Regardless)
}

fn write_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let directory = editor.default_directory();
        let mut initial = directory.to_string_lossy().into_owned();
        if !initial.ends_with('/') {
            initial.push('/');
        }
        editor.prompt_for(
            "write-file",
            MinibufferKind::File,
            "Write file: ",
            &initial,
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No file name given".into()));
    }
    let id = editor.current_buffer_id();
    let mut path = expand(editor, &input);
    // A directory — said with the slash — is where to put it, under the name
    // it already has, as Emacs does.
    if input.trim_end().ends_with('/') {
        let name = editor
            .buffers
            .get(id)
            .map(|b| match b.path().and_then(Path::file_name) {
                Some(name) => name.to_string_lossy().into_owned(),
                None => b.name().to_string(),
            })
            .unwrap_or_default();
        path = path.join(name);
    }
    refuse_unsaveable(editor, id, &path)?;
    // Writing back to the file this buffer already visits is an ordinary
    // save; writing to any other name must not destroy whatever is there.
    let same_file = editor.buffers.get(id).and_then(|b| b.path()) == Some(path.as_path());
    let guard = match same_file {
        true => WriteGuard::Unchanged(editor.buffers.get(id).and_then(|b| b.disk_time())),
        false => WriteGuard::Absent,
    };
    // The buffer takes on the new file, its name and its language when the
    // write has happened — see `FileWritten` — not before.
    write_guarded(editor, id, path, guard)
}

fn save_some_buffers(editor: &mut Editor, _: &Args) -> Result<()> {
    let modified = editor.buffers.modified();
    if modified.is_empty() {
        editor.message("(No files need saving)");
        return Ok(());
    }
    let mut saving = 0;
    let mut skipped = Vec::new();
    for id in modified {
        let Some(path) = editor
            .buffers
            .get(id)
            .and_then(|b| b.path())
            .map(Path::to_path_buf)
        else {
            continue;
        };
        // One that cannot be saved is said, and the rest are still saved.
        match write(editor, id, path) {
            Ok(()) => saving += 1,
            Err(_) => skipped.extend(editor.buffers.get(id).map(|b| b.name().to_string())),
        }
    }
    match skipped.is_empty() {
        true => editor.message(format!("Saving {}", crate::count(saving, "buffer"))),
        false => editor.error(format!(
            "Saving {}; not saved, because saving would damage them: {}",
            crate::count(saving, "buffer"),
            skipped.join(", ")
        )),
    }
    Ok(())
}

fn insert_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        prompt_for_file(editor, "insert-file", "Insert file");
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No file name given".into()));
    }
    let path = expand(editor, &input);
    if editor.current_buffer().is_read_only() {
        return Err(crate::CoreError::Text(maxgus_text::TextError::ReadOnly(
            editor.current_buffer().name().to_string(),
        )));
    }
    // An already-open file is inserted from the buffer, not re-read.
    let Some(id) = editor.buffers.find_by_path(&path) else {
        // Read to be inserted, not visited: this used to open the file in
        // the window instead of putting its text at point.
        let buffer = editor.current_buffer_id();
        editor.spawn(Task::InsertFile { path, buffer });
        return Ok(());
    };
    let text = editor.buffers.get(id).expect("just found").text();
    editor.with_current_buffer(|b| b.insert_at_point(&text))?;
    editor.follow_point();
    Ok(())
}

fn revert_buffer(editor: &mut Editor, args: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    let Some(path) = editor
        .buffers
        .get(id)
        .and_then(|b| b.path())
        .map(Path::to_path_buf)
    else {
        return Err(crate::CoreError::Message(
            "Buffer is not visiting a file".into(),
        ));
    };
    let modified = editor.buffers.get(id).is_some_and(|b| b.is_modified());
    if modified && !args.prefix.is_present() {
        return Err(crate::CoreError::Message(
            "Buffer has unsaved changes; C-u reverts anyway".into(),
        ));
    }
    editor.spawn(Task::ReadFile {
        path,
        reverting: Some(id),
        other_window: false,
    });
    Ok(())
}

/// `C-x C-c`: leaves, refusing while there is unsaved work.
fn kill_terminal(editor: &mut Editor, args: &Args) -> Result<()> {
    if editor.buffers.has_unsaved_changes() && !args.prefix.is_present() {
        let names: Vec<String> = editor
            .buffers
            .modified()
            .into_iter()
            .filter_map(|id| editor.buffers.get(id))
            .map(|b| b.name().to_string())
            .collect();
        return Err(crate::CoreError::Message(format!(
            "Unsaved: {}; C-x s saves them all, C-u C-x C-c leaves without saving",
            names.join(", ")
        )));
    }
    editor.quit = true;
    Ok(())
}

/// The coding-system names Emacs uses for the two line endings.
const CODING_UNIX: &str = "unix";
const CODING_DOS: &str = "dos";

fn coding_system_name(ending: maxgus_text::LineEnding) -> &'static str {
    match ending {
        maxgus_text::LineEnding::Lf => CODING_UNIX,
        maxgus_text::LineEnding::Crlf => CODING_DOS,
    }
}

/// `C-x RET f`. The buffer already remembers what its file used, shows it in
/// the mode line and writes it back on save; this is what lets it be changed.
fn set_buffer_file_coding_system(editor: &mut Editor, args: &Args) -> Result<()> {
    let current = coding_system_name(editor.current_buffer().line_ending());
    let Some(input) = args.input.clone() else {
        // Empty rather than pre-filled, for the same reason `load-theme` is:
        // a name typed into a filled prompt joins onto what is already there.
        editor.prompt_for(
            "set-buffer-file-coding-system",
            MinibufferKind::Choice,
            format!("Coding system (default {current}): "),
            "",
            vec![CODING_UNIX.to_string(), CODING_DOS.to_string()],
        );
        return Ok(());
    };
    let name = match input.trim() {
        "" => current,
        typed => typed,
    };
    let ending = match name {
        CODING_UNIX => maxgus_text::LineEnding::Lf,
        CODING_DOS => maxgus_text::LineEnding::Crlf,
        other => {
            return Err(crate::CoreError::Message(format!(
                "Unknown coding system `{other}`"
            )));
        }
    };
    editor.with_current_buffer(|buffer| {
        buffer.set_line_ending(ending);
        // Nothing in the text changed, but the bytes on disk would, so the
        // file has to be worth writing again — otherwise `C-x C-s` would say
        // there was nothing to save and the choice would be silently lost.
        buffer.mark_modified();
    });
    editor.message(format!("Coding system set to {name}"));
    Ok(())
}

/// `C-c o`: hands the file being edited to the desktop.
///
/// The editor is not an image viewer or a PDF reader and has no business
/// becoming one. What it can do is ask the desktop, which already knows.
fn open_externally(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = editor
        .current_buffer()
        .path()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| crate::CoreError::Message("This buffer has no file".into()))?;
    let directory = editor.default_directory();
    editor.spawn(crate::task::Task::Shell {
        command: crate::desktop_open_command(&path),
        directory,
        insert_at: None,
    });
    editor.message(format!("Opening {path}"));
    Ok(())
}

// ---- what Doom keeps under its file leader -------------------------------

/// The file this buffer is visiting, or an explanation.
fn this_file(editor: &Editor) -> Result<std::path::PathBuf> {
    editor
        .current_buffer()
        .path()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| crate::CoreError::Message("This buffer has no file".into()))
}

fn delete_this_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let path = this_file(editor)?;
    let Some(answer) = args.input.clone() else {
        editor.prompt_for(
            "delete-this-file",
            MinibufferKind::Choice,
            format!("Delete {}? (yes or no) ", path.display()),
            "",
            vec!["yes".into(), "no".into()],
        );
        return Ok(());
    };
    if !answer.eq_ignore_ascii_case("yes") && !answer.eq_ignore_ascii_case("y") {
        editor.message("Nothing deleted".to_string());
        return Ok(());
    }
    // The buffer goes when the file has: a delete that fails leaves both.
    editor.spawn(crate::task::Task::DiredAct {
        action: crate::task::FileAction::Delete(vec![path]),
    });
    Ok(())
}

fn move_this_file(editor: &mut Editor, args: &Args) -> Result<()> {
    transfer_this_file(editor, args, false)
}

fn copy_this_file(editor: &mut Editor, args: &Args) -> Result<()> {
    transfer_this_file(editor, args, true)
}

fn transfer_this_file(editor: &mut Editor, args: &Args, copying: bool) -> Result<()> {
    let path = this_file(editor)?;
    let Some(input) = args.input.clone() else {
        let verb = match copying {
            true => "Copy",
            false => "Move",
        };
        editor.prompt_for(
            match copying {
                true => "copy-this-file",
                false => "move-this-file",
            },
            MinibufferKind::File,
            format!("{verb} to: "),
            &path.display().to_string(),
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No destination given".into()));
    }
    // `~` and a relative path, the way every file prompt reads them — and
    // relative to where the file is, which is what the prompt shows.
    let here = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let to = expand_against(&here, home.as_deref(), &input);
    if to == path {
        return Err(crate::CoreError::Message(
            "That is where it already is".into(),
        ));
    }
    let action = match copying {
        true => crate::task::FileAction::Copy {
            from: vec![path],
            to: to.clone(),
        },
        false => crate::task::FileAction::Rename {
            from: vec![path],
            to: to.clone(),
        },
    };
    // The buffer follows the file when it has moved, wherever it went —
    // into a directory it names, under its own name — which the executor
    // knows and says. Following it here, before the move, left the buffer
    // visiting a directory, or a file that a failed move never made.
    editor.spawn(crate::task::Task::DiredAct { action });
    Ok(())
}

fn yank_buffer_path(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = this_file(editor)?.display().to_string();
    editor.kill(&path, false);
    editor.message(format!("Copied {path}"));
    Ok(())
}

/// The path within the project, which is what goes in a message or a review.
fn yank_relative_path(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = this_file(editor)?;
    let root = editor.project_root();
    let shown = path
        .strip_prefix(&root)
        .unwrap_or(&path)
        .display()
        .to_string();
    editor.kill(&shown, false);
    editor.message(format!("Copied {shown}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, Prefix, task::TaskResult};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup() -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        // A predictable default directory, whatever the test runner's cwd is.
        let id = editor
            .buffers
            .visit_file("/project/main.rs", "fn main() {}\n");
        editor.switch_to_buffer(id).unwrap();
        editor.buffers.get_mut(id).unwrap().mark_saved();

        let mut registry = Registry::new();
        register(&mut registry);
        super::super::minibuffer::register(&mut registry);
        super::super::motion::register(&mut registry);
        super::super::edit::register(&mut registry);
        super::super::window::register(&mut registry);
        super::super::buffer::register(&mut registry);
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
        e.minibuffer.kill_whole();
        for c in text.chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(e, "RET");
    }

    #[test]
    fn the_coding_system_prompt_offers_both_endings_and_starts_at_the_current_one() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        assert!(e.minibuffer.is_active());
        assert_eq!(
            e.completion_candidates,
            vec!["unix".to_string(), "dos".to_string()]
        );
        assert_eq!(e.minibuffer.input(), "");
        // The file was read with LF, so that is the default offered.
        assert!(e.minibuffer.prompt().contains("default unix"));
    }

    #[test]
    fn choosing_dos_makes_the_next_save_write_crlf() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        answer(&mut d, &mut e, "dos");
        e.tasks.drain();

        run(&mut d, &mut e, "save-buffer");
        let queued = e.tasks.peek();
        let Some(Task::WriteFile { contents, .. }) = queued.first() else {
            panic!("nothing was queued to write: {queued:?}");
        };
        assert!(contents.contains("\r\n"), "got {contents:?}");
    }

    #[test]
    fn changing_the_coding_system_is_a_change_worth_saving() {
        // Nothing in the text moved, so the modified flag is the only thing
        // that can stop `save-buffer` from declining to write and losing the
        // choice silently.
        let (mut d, mut e) = setup();
        assert!(
            !e.current_buffer().is_modified(),
            "the fixture starts saved"
        );
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        answer(&mut d, &mut e, "dos");
        assert!(e.current_buffer().is_modified());
    }

    #[test]
    fn choosing_unix_again_goes_back_to_lf() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        answer(&mut d, &mut e, "dos");
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        assert!(
            e.minibuffer.prompt().contains("default dos"),
            "the default follows the buffer"
        );
        answer(&mut d, &mut e, "unix");
        e.tasks.drain();

        run(&mut d, &mut e, "save-buffer");
        let queued = e.tasks.peek();
        let Some(Task::WriteFile { contents, .. }) = queued.first() else {
            panic!("nothing was queued to write: {queued:?}");
        };
        assert!(!contents.contains("\r\n"), "got {contents:?}");
    }

    #[test]
    fn answering_the_coding_system_prompt_with_nothing_keeps_what_is_set() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        answer(&mut d, &mut e, "dos");
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        d.handle_keys(&mut e, "RET");
        assert_eq!(coding_system_name(e.current_buffer().line_ending()), "dos");
    }

    #[test]
    fn an_unknown_coding_system_is_refused() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "set-buffer-file-coding-system");
        e.minibuffer.kill_whole();
        for c in "ebcdic".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }), "got {out:?}");
        assert!(!e.current_buffer().is_modified(), "nothing was changed");
    }

    #[test]
    fn every_file_binding_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for name in [
            "find-file",
            "save-buffer",
            "write-file",
            "save-buffers-kill-terminal",
        ] {
            assert!(registry.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn find_file_prompts_from_the_current_directory() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        assert!(e.minibuffer.is_active());
        assert_eq!(e.minibuffer.input(), "/project/");
        // The prompt also asks for a listing so TAB has candidates.
        assert!(
            e.tasks
                .peek()
                .iter()
                .any(|t| matches!(t, Task::ListDirectory { .. })),
            "no listing was requested"
        );
    }

    #[test]
    fn find_file_queues_a_read_rather_than_touching_the_disk() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/other.rs");

        let tasks = e.tasks.drain();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0],
            Task::ReadFile {
                path: PathBuf::from("/project/other.rs"),
                reverting: None,
                other_window: false
            }
        );
    }

    #[test]
    fn a_relative_answer_resolves_against_the_default_directory() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "src/lib.rs");
        let tasks = e.tasks.drain();
        assert!(
            matches!(&tasks[0], Task::ReadFile { path, .. } if path == Path::new("/project/src/lib.rs")),
            "got {:?}",
            tasks[0]
        );
    }

    #[test]
    fn path_expansion_handles_absolute_relative_and_tilde_answers() {
        let directory = Path::new("/project");
        let home = Some(Path::new("/home/tester"));

        assert_eq!(
            expand_against(directory, home, "/etc/hosts"),
            Path::new("/etc/hosts"),
            "an absolute answer is taken as it stands"
        );
        assert_eq!(
            expand_against(directory, home, "src/lib.rs"),
            Path::new("/project/src/lib.rs"),
            "a relative answer resolves against the default directory"
        );
        assert_eq!(
            expand_against(directory, home, "~/notes.txt"),
            Path::new("/home/tester/notes.txt")
        );
        assert_eq!(
            expand_against(directory, home, "~"),
            Path::new("/home/tester")
        );
        assert_eq!(
            expand_against(directory, None, "~/notes.txt"),
            Path::new("/project/~/notes.txt"),
            "with no home directory there is nothing to expand to"
        );
        assert_eq!(
            expand_against(directory, home, "  spaced.rs  "),
            Path::new("/project/spaced.rs"),
            "surrounding whitespace is trimmed"
        );
    }

    #[test]
    fn visiting_an_already_open_file_switches_without_reading() {
        let (mut d, mut e) = setup();
        let other = e.buffers.visit_file("/project/other.rs", "contents");
        run(&mut d, &mut e, "find-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/other.rs");

        assert!(e.tasks.is_empty(), "no read was queued");
        assert_eq!(e.current_buffer_id(), other);
    }

    #[test]
    fn an_empty_file_name_is_refused() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        e.minibuffer.kill_whole();
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn a_picture_opens_as_a_read_only_caption_with_the_pixels_beside_it() {
        let (mut d, mut e) = setup();
        e.with_current_buffer(|b| {
            b.replace_all("Plan: ![the desk](shots/desk.png) is where it goes\n")
                .unwrap();
            b.set_point(10);
        });
        e.tasks.drain();
        run(&mut d, &mut e, "view-image-at-point");
        let tasks = e.tasks.drain();
        assert_eq!(
            tasks,
            vec![Task::ReadFile {
                path: PathBuf::from("/project/shots/desk.png"),
                reverting: None,
                other_window: true,
            }],
            "the path is taken from the document's own directory"
        );
        let picture = std::sync::Arc::new(crate::picture::Picture {
            width: 640,
            height: 480,
            format: "PNG".into(),
            bytes: 2048,
            pixels: crate::picture::Pixels {
                width: 640,
                height: 480,
                rgba: std::sync::Arc::from(vec![0u8; 4]),
            },
        });
        e.apply_task_result(TaskResult::PictureRead {
            path: PathBuf::from("/project/shots/desk.png"),
            picture: picture.clone(),
            reverting: None,
            other_window: true,
        })
        .unwrap();
        assert_eq!(e.windows.len(), 2, "shown beside the document");
        let id = e.current_buffer_id();
        assert_eq!(e.current_buffer().name(), "desk.png");
        assert!(e.current_buffer().is_read_only());
        assert!(
            e.current_buffer()
                .text()
                .starts_with("desk.png: 640×480 PNG, 2.0 KB\n"),
            "{:?}",
            e.current_buffer().text()
        );
        assert!(
            e.current_buffer().text().contains("terminal cannot"),
            "a terminal is told why there is no picture"
        );
        assert_eq!(e.pictures.get(&id), Some(&picture));
        // Killing the buffer lets the pixels go with it.
        e.kill_buffer(id).unwrap();
        assert!(e.pictures.is_empty());

        // A line with no picture on it says so.
        e.with_current_buffer(|b| {
            b.replace_all("nothing to see\n").unwrap();
            b.set_point(0);
        });
        let out = d.execute(&mut e, "view-image-at-point", None);
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn a_read_result_creates_the_buffer_and_selects_it() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/new.rs");

        e.apply_task_result(TaskResult::FileRead {
            path: PathBuf::from("/project/new.rs"),
            contents: "fn new() {}\n".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: Default::default(),
        })
        .unwrap();

        assert_eq!(e.current_buffer().name(), "new.rs");
        assert_eq!(e.current_buffer().text(), "fn new() {}\n");
        assert!(
            !e.current_buffer().is_modified(),
            "a freshly read file is not modified"
        );
        assert!(e.minibuffer.display().contains("new.rs"));
    }

    #[cfg(feature = "full")]
    #[test]
    fn opening_a_file_asks_for_highlighting_and_a_language_server() {
        let (_d, mut e) = setup();
        e.tasks.drain();
        e.apply_task_result(TaskResult::FileRead {
            path: PathBuf::from("/project/new.rs"),
            contents: "fn new() {}\n".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: Default::default(),
        })
        .unwrap();

        let tasks = e.tasks.drain();
        assert!(
            tasks.iter().any(|t| matches!(t, Task::Reparse { .. })),
            "no reparse queued"
        );
        assert!(
            tasks
                .iter()
                .any(|t| matches!(t, Task::StartLanguageServer { .. })),
            "no server start queued"
        );
        assert!(
            tasks.iter().any(|t| matches!(t, Task::LspDidOpen { .. })),
            "no didOpen queued"
        );
    }

    #[test]
    fn those_requests_are_skipped_when_the_features_are_off() {
        let (_d, mut e) = setup();
        e.settings.syntax_highlighting = false;
        e.settings.lsp_enabled = false;
        e.tasks.drain();
        e.apply_task_result(TaskResult::FileRead {
            path: PathBuf::from("/project/new.rs"),
            contents: "".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: Default::default(),
        })
        .unwrap();
        assert!(e.tasks.is_empty());
    }

    #[test]
    fn finding_a_file_in_another_window_splits_once() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file-other-window");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/other.rs");
        let tasks = e.tasks.drain();
        assert!(matches!(
            &tasks[0],
            Task::ReadFile {
                other_window: true,
                ..
            }
        ));

        e.apply_task_result(TaskResult::FileRead {
            path: PathBuf::from("/project/other.rs"),
            contents: "".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: true,
            editor_config: Default::default(),
        })
        .unwrap();
        assert_eq!(e.windows.len(), 2);
        assert_eq!(e.current_buffer().name(), "other.rs");
    }

    #[test]
    fn saving_queues_a_write_of_the_buffer_contents() {
        let (mut d, mut e) = setup();
        e.with_current_buffer(|b| b.insert_at_point("// edit\n").unwrap());
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer");

        let tasks = e.tasks.drain();
        assert_eq!(tasks.len(), 1);
        let Task::WriteFile { path, contents, .. } = &tasks[0] else {
            panic!("{:?}", tasks[0])
        };
        assert_eq!(path, Path::new("/project/main.rs"));
        assert!(contents.starts_with("// edit"));
    }

    #[test]
    fn saving_an_unmodified_buffer_says_there_is_nothing_to_do() {
        let (mut d, mut e) = setup();
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer");
        assert!(e.tasks.is_empty());
        assert_eq!(e.minibuffer.display(), "(No changes need to be saved)");
    }

    #[test]
    fn a_write_result_clears_the_modified_flag() {
        let (mut d, mut e) = setup();
        e.with_current_buffer(|b| b.insert_at_point("edit").unwrap());
        assert!(e.current_buffer().is_modified());
        run(&mut d, &mut e, "save-buffer");

        let id = e.current_buffer_id();
        e.apply_task_result(TaskResult::FileWritten {
            path: PathBuf::from("/project/main.rs"),
            buffer: id,
            bytes: 20,
            disk_time: None,
        })
        .unwrap();
        assert!(!e.current_buffer().is_modified());
        assert!(e.minibuffer.display().contains("Wrote"));
    }

    #[test]
    fn saving_a_buffer_with_no_file_prompts_for_a_name() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create("notes");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("some notes").unwrap());
        run(&mut d, &mut e, "save-buffer");
        assert!(e.minibuffer.is_active(), "it should have prompted");
        assert!(e.minibuffer.prompt().starts_with("Write file"));
    }

    #[test]
    fn writing_under_a_new_name_renames_the_buffer_and_takes_the_language() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create("notes");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        run(&mut d, &mut e, "write-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/notes.py");
        assert!(matches!(&e.tasks.peek()[0], Task::WriteFile { .. }));
        // Not yet: a write that is refused must leave the buffer as it was.
        assert_eq!(e.current_buffer().name(), "notes");

        e.apply_task_result(TaskResult::FileWritten {
            path: "/project/notes.py".into(),
            buffer: id,
            bytes: 1,
            disk_time: None,
        })
        .unwrap();
        assert_eq!(e.current_buffer().name(), "notes.py");
        assert_eq!(
            e.current_buffer().path().unwrap(),
            Path::new("/project/notes.py")
        );
        assert_eq!(e.current_buffer().language(), Some("python"));
    }

    #[test]
    fn a_refused_write_under_a_new_name_leaves_the_buffer_where_it_was() {
        let (mut d, mut e) = setup();
        let before = e.current_buffer().path().map(Path::to_path_buf);
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        run(&mut d, &mut e, "write-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/taken.rs");
        let id = e.current_buffer_id();
        e.apply_task_result(TaskResult::WriteRefused {
            path: "/project/taken.rs".into(),
            buffer: id,
            because: WriteGuard::Absent,
        })
        .unwrap();
        assert_eq!(e.current_buffer().path().map(Path::to_path_buf), before);
        // And the way past it writes where it was asked to.
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer-anyway");
        let Task::WriteFile { path, guard, .. } = &e.tasks.peek()[0] else {
            panic!("no write")
        };
        assert_eq!(path, Path::new("/project/taken.rs"));
        assert_eq!(*guard, WriteGuard::Regardless);
    }

    #[test]
    fn a_path_typed_after_the_prefilled_directory_starts_again() {
        let home = Some(Path::new("/home/me"));
        let here = Path::new("/project");
        assert_eq!(
            expand_against(here, home, "/project/src/~/notes.txt"),
            Path::new("/home/me/notes.txt")
        );
        assert_eq!(
            expand_against(here, home, "/project/src//etc/hosts"),
            Path::new("/etc/hosts")
        );
        assert_eq!(
            expand_against(here, home, "/project/src/~"),
            Path::new("/home/me")
        );
        assert_eq!(
            expand_against(here, home, "/project/a~b/c"),
            Path::new("/project/a~b/c"),
            "a tilde inside a name is just a letter"
        );
    }

    #[test]
    fn a_picture_is_never_saved_as_its_caption() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        e.pictures.insert(
            id,
            std::sync::Arc::new(crate::picture::Picture {
                width: 1,
                height: 1,
                format: "PNG".into(),
                bytes: 4,
                pixels: crate::picture::Pixels {
                    width: 1,
                    height: 1,
                    rgba: std::sync::Arc::from(vec![0u8; 4]),
                },
            }),
        );
        e.with_current_buffer(|b| b.insert_at_point("caption edited").unwrap());
        e.tasks.drain();
        let outcome = d.execute(&mut e, "save-buffer", None);
        assert!(
            matches!(outcome, crate::Dispatch::Failed { .. }),
            "got {outcome:?}"
        );
        assert!(e.tasks.drain().is_empty(), "a write was queued");
    }

    #[test]
    fn bytes_that_are_not_text_are_only_written_when_asked_twice() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        e.lossy_buffers.insert(id);
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        e.tasks.drain();
        let outcome = d.execute(&mut e, "save-buffer", None);
        assert!(matches!(outcome, crate::Dispatch::Failed { .. }));
        assert!(e.tasks.drain().is_empty());
        run(&mut d, &mut e, "save-buffer-anyway");
        assert!(matches!(&e.tasks.peek()[0], Task::WriteFile { .. }));
    }

    #[test]
    fn inserting_a_file_reads_it_for_insertion_rather_than_visiting_it() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        run(&mut d, &mut e, "insert-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/other.txt");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::InsertFile {
                path: "/project/other.txt".into(),
                buffer: id
            }]
        );
        e.with_current_buffer(|b| b.set_point(0));
        e.apply_task_result(TaskResult::FileInserted {
            path: "/project/other.txt".into(),
            buffer: id,
            contents: "inserted\n".into(),
        })
        .unwrap();
        assert!(e.current_buffer().text().starts_with("inserted\nfn main"));
        assert_eq!(e.current_buffer_id(), id, "nothing was visited");
    }

    #[test]
    fn find_alternate_file_on_the_same_file_reads_it_again() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        run(&mut d, &mut e, "find-alternate-file");
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/main.rs");
        assert!(e.buffers.get(id).is_some(), "the buffer was killed");
        assert!(matches!(
            &e.tasks.peek()[0],
            Task::ReadFile {
                reverting: Some(r),
                ..
            } if *r == id
        ));
    }

    #[test]
    fn a_final_newline_is_added_when_the_setting_asks_for_it() {
        let (mut d, mut e) = setup();
        e.settings.require_final_newline = true;
        e.with_current_buffer(|b| {
            b.replace_all("no trailing newline").unwrap();
        });
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer");
        let Task::WriteFile { contents, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn trailing_whitespace_is_stripped_when_the_setting_asks_for_it() {
        let (mut d, mut e) = setup();
        e.settings.delete_trailing_whitespace = true;
        e.with_current_buffer(|b| {
            b.replace_all("line one   \nline two\t\n").unwrap();
        });
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer");
        let Task::WriteFile { contents, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert_eq!(contents, "line one\nline two\n");
    }

    #[test]
    fn crlf_line_endings_are_restored_on_save() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("dos.txt", "a\r\nb\r\n");
        e.switch_to_buffer(id).unwrap();
        e.buffers.get_mut(id).unwrap().set_path("/project/dos.txt");
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        e.tasks.drain();
        run(&mut d, &mut e, "save-buffer");
        let Task::WriteFile { contents, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert!(contents.contains("\r\n"), "got `{contents:?}`");
    }

    #[test]
    fn save_some_buffers_writes_every_modified_file() {
        let (mut d, mut e) = setup();
        let a = e.buffers.visit_file("/project/a.rs", "");
        let b = e.buffers.visit_file("/project/b.rs", "");
        e.buffers
            .get_mut(a)
            .unwrap()
            .insert_at_point("edit")
            .unwrap();
        e.buffers
            .get_mut(b)
            .unwrap()
            .insert_at_point("edit")
            .unwrap();
        e.tasks.drain();

        run(&mut d, &mut e, "save-some-buffers");
        let writes = e.tasks.drain();
        assert_eq!(writes.len(), 2);
        assert!(writes.iter().all(|t| matches!(t, Task::WriteFile { .. })));
    }

    #[test]
    fn save_some_buffers_with_nothing_to_do_says_so() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "save-some-buffers");
        assert_eq!(e.minibuffer.display(), "(No files need saving)");
    }

    #[test]
    fn reverting_needs_a_file_and_confirmation_when_modified() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create("no-file");
        e.switch_to_buffer(id).unwrap();
        assert!(fails(&mut d, &mut e, "revert-buffer").contains("not visiting a file"));

        let file = e
            .buffers
            .find_by_path(Path::new("/project/main.rs"))
            .unwrap();
        e.switch_to_buffer(file).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("edit").unwrap());
        assert!(fails(&mut d, &mut e, "revert-buffer").contains("unsaved"));

        e.prefix = Prefix::Universal(1);
        d.execute(&mut e, "revert-buffer", None);
        assert!(matches!(
            &e.tasks.peek()[0],
            Task::ReadFile {
                reverting: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn a_revert_result_replaces_the_contents_without_undo_history() {
        let (_d, mut e) = setup();
        let id = e.current_buffer_id();
        e.with_current_buffer(|b| b.insert_at_point("edited ").unwrap());

        e.apply_task_result(TaskResult::FileRead {
            path: PathBuf::from("/project/main.rs"),
            contents: "fresh from disk\n".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: Some(id),
            other_window: false,
            editor_config: Default::default(),
        })
        .unwrap();

        assert_eq!(e.current_buffer().text(), "fresh from disk\n");
        assert!(!e.current_buffer().is_modified());
        assert!(!e.current_buffer().can_undo());
    }

    #[test]
    fn find_alternate_file_replaces_the_buffer() {
        let (mut d, mut e) = setup();
        let original = e.current_buffer_id();
        run(&mut d, &mut e, "find-alternate-file");
        assert_eq!(
            e.minibuffer.input(),
            "/project/main.rs",
            "pre-filled with this file"
        );
        e.tasks.drain();
        answer(&mut d, &mut e, "/project/other.rs");

        assert!(
            e.buffers.get(original).is_none(),
            "the old buffer was killed"
        );
        assert!(matches!(&e.tasks.peek()[0], Task::ReadFile { .. }));
    }

    #[test]
    fn find_alternate_file_refuses_to_discard_unsaved_work() {
        let (mut d, mut e) = setup();
        e.with_current_buffer(|b| b.insert_at_point("edit").unwrap());
        d.execute(&mut e, "find-alternate-file", None);
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn insert_file_pulls_in_an_open_buffer_directly() {
        let (mut d, mut e) = setup();
        e.buffers.visit_file("/project/snippet.rs", "// snippet\n");
        e.with_current_buffer(|b| b.set_point(0));
        e.tasks.drain();

        run(&mut d, &mut e, "insert-file");
        answer(&mut d, &mut e, "/project/snippet.rs");
        assert!(e.current_buffer().text().starts_with("// snippet\n"));
    }

    #[test]
    fn quitting_refuses_while_work_is_unsaved_and_names_the_buffers() {
        let (mut d, mut e) = setup();
        e.with_current_buffer(|b| b.insert_at_point("edit").unwrap());
        let message = fails(&mut d, &mut e, "save-buffers-kill-terminal");
        assert!(message.contains("main.rs"), "got `{message}`");
        assert!(!e.quit);

        e.prefix = Prefix::Universal(1);
        d.execute(&mut e, "save-buffers-kill-terminal", None);
        assert!(e.quit);
    }

    #[test]
    fn quitting_with_everything_saved_just_leaves() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "save-buffers-kill-terminal");
        assert!(e.quit);
    }

    #[test]
    fn a_directory_listing_result_fills_in_completion() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "find-file");
        e.apply_task_result(TaskResult::DirectoryListed {
            path: PathBuf::from("/project"),
            entries: vec!["/project/main.rs".into(), "/project/mod.rs".into()],
        })
        .unwrap();
        assert_eq!(e.completion_candidates.len(), 2);
    }

    #[test]
    fn a_failed_task_reports_its_context() {
        let (_d, mut e) = setup();
        e.apply_task_result(TaskResult::Failed {
            context: "find-file".into(),
            message: "No such file or directory".into(),
        })
        .unwrap();
        assert!(e.minibuffer.message_is_error());
        assert!(e.minibuffer.display().contains("No such file"));
    }

    #[cfg(feature = "full")]
    #[test]
    fn diagnostics_results_are_stored_against_their_document() {
        let (_d, mut e) = setup();
        let diagnostic = maxgus_lsp::Diagnostic::new(
            maxgus_lsp::LspRange::empty(maxgus_lsp::LspPosition::ZERO),
            maxgus_lsp::Severity::Error,
            "boom",
        );
        e.apply_task_result(TaskResult::Diagnostics {
            uri: "file:///project/main.rs".into(),
            diagnostics: vec![diagnostic],
        })
        .unwrap();
        assert_eq!(e.diagnostics.for_uri("file:///project/main.rs").len(), 1);
    }
}
