//! File tree commands.
//!
//! The tree is treemacs in miniature: a side window listing the project, with
//! treemacs' own keymap. Navigation happens immediately on the last snapshot,
//! so moving around never waits on the filesystem; anything structural —
//! expanding, refreshing, creating, deleting — is queued as a [`TreeAction`]
//! that the event loop performs on tokio and answers with a fresh snapshot.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    task::{Task, TreeAction},
    window::Direction,
};
use std::path::PathBuf;

/// The buffers the panel's three windows are drawn into.
pub const TREE_BUFFER_NAME: &str = "*treefile*";
pub const SYMBOLS_BUFFER_NAME: &str = "*symbols*";
pub const BUFFERS_BUFFER_NAME: &str = "*buffers*";

/// All three, for the places that need to know a buffer belongs to the panel.
pub const PANEL_BUFFERS: &[&str] = &[TREE_BUFFER_NAME, SYMBOLS_BUFFER_NAME, BUFFERS_BUFFER_NAME];

/// The mode names the outline and the buffer list go under.
pub const SYMBOLS_MODE: &str = "symbols-mode";
pub const BUFFERS_MODE: &str = "buffers-mode";

/// The mode name the tree's keymap goes under.
///
/// A *mode* map, not a minor one: it binds the arrow keys, and a minor map
/// applies in every buffer — so while the tree was open the arrows moved the
/// tree instead of the file being edited, wherever the cursor was.
pub const TREE_MODE: &str = "treefile-mode";

/// Registers the tree commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!("treefile-toggle", "Show or hide the file tree.", toggle),
        command!("treefile-select", "Select the file tree window.", select),
        command!(
            "treefile-select-directory",
            "Show the file tree rooted at a directory.",
            select_directory
        ),
        command!(
            "treefile-next-line",
            "Move down one line.",
            next_line,
            non_interactive
        ),
        command!(
            "treefile-previous-line",
            "Move up one line.",
            previous_line,
            non_interactive
        ),
        command!(
            "treefile-goto-first",
            "Move to the first line.",
            goto_first,
            non_interactive
        ),
        command!(
            "treefile-goto-last",
            "Move to the last line.",
            goto_last,
            non_interactive
        ),
        command!(
            "treefile-next-neighbour",
            "Move to the next node at this depth.",
            next_neighbour,
            non_interactive
        ),
        command!(
            "treefile-previous-neighbour",
            "Move to the previous node at this depth.",
            previous_neighbour,
            non_interactive
        ),
        command!(
            "treefile-goto-parent",
            "Move to the enclosing directory.",
            goto_parent,
            non_interactive
        ),
        command!(
            "treefile-toggle-node",
            "Expand or collapse the directory here.",
            toggle_node,
            non_interactive
        ),
        command!(
            "treefile-expand-node",
            "Expand the directory here.",
            expand_node,
            non_interactive
        ),
        command!(
            "treefile-collapse-node",
            "Collapse the directory here.",
            collapse_node,
            non_interactive
        ),
        command!(
            "treefile-collapse-parent",
            "Collapse the enclosing directory.",
            collapse_parent,
            non_interactive
        ),
        command!(
            "treefile-root-down",
            "Draw the tree from the selected directory.",
            root_down
        ),
        command!(
            "treefile-root-up",
            "Draw the tree from one directory further out.",
            root_up
        ),
        command!(
            "treefile-root-reset",
            "Draw the tree from where it opened.",
            root_reset
        ),
        command!(
            "treefile-expand-recursively",
            "Expand everything below here.",
            expand_recursively,
            non_interactive
        ),
        command!(
            "treefile-visit-node",
            "Open the file here, or expand the directory.",
            visit,
            non_interactive
        ),
        command!(
            "treefile-visit-node-vertical-split",
            "Open the file here in a window below.",
            visit_below,
            non_interactive
        ),
        command!(
            "treefile-visit-node-horizontal-split",
            "Open the file here in a window beside.",
            visit_beside,
            non_interactive
        ),
        command!(
            "treefile-visit-node-recent-window",
            "Open the file here in the other window.",
            visit_other,
            non_interactive
        ),
        command!(
            "treefile-visit-node-external",
            "Open the file here in an external program.",
            visit_external,
            non_interactive
        ),
        command!(
            "treefile-peek",
            "Show the file here without leaving the tree.",
            peek,
            non_interactive
        ),
        command!(
            "treefile-create-file",
            "Create a file here.",
            create_file,
            non_interactive
        ),
        command!(
            "treefile-create-dir",
            "Create a directory here.",
            create_dir,
            non_interactive
        ),
        command!(
            "treefile-rename-file",
            "Rename the file here.",
            rename,
            non_interactive
        ),
        command!(
            "treefile-delete-file",
            "Delete the file here.",
            delete,
            non_interactive
        ),
        command!(
            "treefile-move-file",
            "Move the file here somewhere else.",
            move_file,
            non_interactive
        ),
        command!(
            "treefile-run-shell-command",
            "Run a shell command in this directory.",
            shell_command,
            non_interactive
        ),
        command!(
            "treefile-copy-absolute-path",
            "Copy the full path here.",
            copy_absolute,
            non_interactive
        ),
        command!(
            "treefile-copy-relative-path",
            "Copy the path here, relative to the root.",
            copy_relative,
            non_interactive
        ),
        command!(
            "treefile-copy-project-path",
            "Copy the path here, relative to the project.",
            copy_relative,
            non_interactive
        ),
        command!(
            "treefile-copy-file",
            "Copy the file name here.",
            copy_name,
            non_interactive
        ),
        command!(
            "treefile-toggle-show-dotfiles",
            "Show or hide dotfiles.",
            toggle_hidden,
            non_interactive
        ),
        command!(
            "treefile-toggle-fixed-width",
            "Lock or unlock the tree width.",
            toggle_fixed_width,
            non_interactive
        ),
        command!(
            "treefile-toggle-follow-mode",
            "Follow the current buffer, or stop.",
            toggle_follow,
            non_interactive
        ),
        command!(
            "treefile-toggle-git-mode",
            "Show or hide git status.",
            toggle_git,
            non_interactive
        ),
        command!(
            "treefile-toggle-directories-first",
            "Sort directories first, or by name.",
            toggle_directories_first,
            non_interactive
        ),
        command!(
            "treefile-set-width",
            "Set the tree width.",
            set_width,
            non_interactive
        ),
        command!(
            "treefile-increase-width",
            "Widen the tree.",
            increase_width,
            non_interactive
        ),
        command!(
            "treefile-decrease-width",
            "Narrow the tree.",
            decrease_width,
            non_interactive
        ),
        command!(
            "treefile-refresh",
            "Re-read the tree from disk.",
            refresh,
            non_interactive
        ),
        command!(
            "treefile-resort",
            "Re-sort the tree.",
            refresh,
            non_interactive
        ),
        command!("treefile-quit", "Hide the tree.", quit, non_interactive),
        command!(
            "treefile-kill",
            "Hide the tree and forget it.",
            kill,
            non_interactive
        ),
        command!(
            "treefile-help",
            "Describe the tree keymap.",
            help,
            non_interactive
        ),
    ]);
}

/// The node under the cursor, or an error naming what is missing.
fn selection(editor: &Editor) -> Result<maxgus_tree::VisibleNode> {
    editor
        .tree_selection()
        .cloned()
        .ok_or_else(|| crate::CoreError::Message("No node here".into()))
}

/// Queues a structural change.
fn act(editor: &mut Editor, action: TreeAction) {
    editor.spawn(Task::Tree(action));
}

// ---- showing and hiding -------------------------------------------------

/// Opens the panel: a column of windows down the left, one per section.
///
/// Three windows rather than one buffer with headings in it. Moving between
/// them is then ordinary window movement, each keeps its own point, and each
/// scrolls on its own.
pub fn open(editor: &mut Editor, root: PathBuf) -> Result<()> {
    use crate::panel::PanelSection;
    editor.tree_root = Some(root.clone());
    // Where `r r` comes back to, whatever `r d` does afterwards.
    editor.tree_home = Some(root.clone());

    // The configured heights, cut down to what the frame can actually give.
    // A twelve-row outline in a ten-row frame would leave the tree nothing at
    // all, and a panel whose first window is empty looks broken.
    let available = editor.frame.height.saturating_sub(1);
    let share = |wanted: u16| {
        wanted
            .min(available / 4)
            .max(3)
            .min(available.saturating_sub(3))
    };

    let mut entries = Vec::new();
    if editor.panel.is_enabled(PanelSection::Tree) {
        // No fixed height: the tree takes whatever the other two leave.
        entries.push((panel_buffer(editor, TREE_BUFFER_NAME), None));
    }
    if editor.panel.is_enabled(PanelSection::Symbols) && editor.symbols_available() {
        let height = share(editor.symbols_height);
        entries.push((panel_buffer(editor, SYMBOLS_BUFFER_NAME), Some(height)));
    }
    if editor.panel.is_enabled(PanelSection::Buffers) {
        let height = share(editor.buffers_height);
        entries.push((panel_buffer(editor, BUFFERS_BUFFER_NAME), Some(height)));
    }
    if entries.is_empty() {
        return Err(crate::CoreError::Message(
            "Every panel section is switched off".into(),
        ));
    }

    let windows = editor.windows.add_side_column(&entries, editor.tree_width);
    editor.tree_window = windows.first().copied();
    editor.panel_windows = windows;
    // Drawn straight away rather than when the directory walk answers: the
    // buffer list is already known, and an empty column while the filesystem
    // is read looks like a panel that failed to open.
    editor.render_panel_buffer();
    // The outline is asked for directly rather than through
    // `follow_panel_to_buffer`, which may rebuild the column — and rebuilding
    // comes back here. One cycle would terminate; relying on that is how a
    // later change turns into a stack overflow.
    editor.request_document_symbols();
    act(editor, TreeAction::Refresh);
    Ok(())
}

/// One of the panel's buffers, created read-only if this is the first time.
fn panel_buffer(editor: &mut Editor, name: &str) -> maxgus_text::BufferId {
    match editor.buffers.find_by_name(name) {
        Some(id) => id,
        None => {
            let id = editor.buffers.create_with_text(name, "");
            editor
                .buffers
                .get_mut(id)
                .expect("just created")
                .set_read_only(true);
            id
        }
    }
}

/// Closes every panel window, leaving their buffers alone.
pub fn close(editor: &mut Editor) {
    for window in std::mem::take(&mut editor.panel_windows) {
        editor.windows.delete(window).ok();
    }
    editor.tree_window = None;
    // The map goes with the buffer, so selecting whatever is left takes it
    // away; nothing to remove by hand.
    editor.activate_mode_keymap();
}

/// Closes the panel and opens it again, which is how a section is switched
/// on or off: the column's shape is decided when it is built.
pub fn rebuild(editor: &mut Editor) -> Result<()> {
    if editor.panel_windows.is_empty() {
        return Ok(());
    }
    let root = editor
        .tree_root
        .clone()
        .unwrap_or_else(|| editor.default_directory());
    let selected = editor.windows.current_id();
    let was_in_panel = editor.panel_windows.contains(&selected);
    close(editor);
    open(editor, root)?;
    if was_in_panel && let Some(window) = editor.tree_window {
        editor.select_window(window);
    }
    Ok(())
}

fn toggle(editor: &mut Editor, _: &Args) -> Result<()> {
    if !editor.panel_windows.is_empty() {
        close(editor);
        return Ok(());
    }
    let root = editor.default_directory();
    open(editor, root)
}

fn select(editor: &mut Editor, _: &Args) -> Result<()> {
    if editor.panel_windows.is_empty() {
        let root = editor.default_directory();
        open(editor, root)?;
    }
    let window = editor.tree_window.ok_or(crate::CoreError::NoSuchWindow)?;
    editor.select_window(window);
    Ok(())
}

fn select_directory(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let initial = editor.default_directory().to_string_lossy().into_owned();
        editor.prompt_for(
            "treefile-select-directory",
            MinibufferKind::File,
            "Tree root: ",
            &initial,
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(crate::CoreError::Message("No directory given".into()));
    }
    close(editor);
    open(editor, PathBuf::from(input.trim()))
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    close(editor);
    Ok(())
}

fn kill(editor: &mut Editor, _: &Args) -> Result<()> {
    close(editor);
    if let Some(id) = editor.buffers.find_by_name(TREE_BUFFER_NAME) {
        editor.kill_buffer(id).ok();
    }
    editor.tree.clear();
    editor.tree_root = None;
    editor.tree_home = None;
    Ok(())
}

// ---- navigation ---------------------------------------------------------

/// `n`: down one row of the *panel*, not of the tree.
///
/// The panel stacks the tree, the symbol outline and the buffer list in one
/// window. Moving by tree index — which is what this did — clamps at the last
/// file and can never reach a symbol or a buffer, so the sections below the
/// tree could be scrolled past but never entered.
fn next_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let line = editor.tree_cursor_line() + args.count();
    editor.move_tree_cursor_to_line(line);
    Ok(())
}

fn previous_line(editor: &mut Editor, args: &Args) -> Result<()> {
    let line = editor.tree_cursor_line().saturating_sub(args.count());
    editor.move_tree_cursor_to_line(line);
    Ok(())
}

fn goto_first(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.move_tree_cursor_to_line(0);
    Ok(())
}

fn goto_last(editor: &mut Editor, _: &Args) -> Result<()> {
    let last = editor.tree.len().saturating_sub(1);
    editor.move_tree_cursor_to_line(last);
    Ok(())
}

/// Walks to the next or previous node at the same depth, stopping when the
/// enclosing directory ends.
fn neighbour(editor: &mut Editor, forward: bool) -> Result<()> {
    let here = editor.tree_cursor_line();
    let Some(depth) = editor.tree.get(here).map(|n| n.depth) else {
        return Err(crate::CoreError::Message("No node here".into()));
    };
    let candidates: Vec<usize> = if forward {
        ((here + 1)..editor.tree.len()).collect()
    } else {
        (0..here).rev().collect()
    };
    for index in candidates {
        match editor.tree[index].depth.cmp(&depth) {
            std::cmp::Ordering::Equal => {
                editor.move_tree_cursor_to_line(index);
                return Ok(());
            }
            // A shallower node means the parent ended: no more siblings.
            std::cmp::Ordering::Less => break,
            std::cmp::Ordering::Greater => {}
        }
    }
    Err(crate::CoreError::Message(
        "No more nodes at this level".into(),
    ))
}

fn next_neighbour(editor: &mut Editor, _: &Args) -> Result<()> {
    neighbour(editor, true)
}

fn previous_neighbour(editor: &mut Editor, _: &Args) -> Result<()> {
    neighbour(editor, false)
}

/// The line of the enclosing directory, if there is one.
fn parent_line(editor: &Editor) -> Option<usize> {
    let here = editor.tree_cursor_line();
    let depth = editor.tree.get(here)?.depth;
    if depth == 0 {
        return None;
    }
    (0..here)
        .rev()
        .find(|index| editor.tree[*index].depth < depth)
}

fn goto_parent(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(line) = parent_line(editor) else {
        return Err(crate::CoreError::Message("Already at the root".into()));
    };
    editor.move_tree_cursor_to_line(line);
    Ok(())
}

// ---- expansion ----------------------------------------------------------

fn toggle_node(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    if !node.expandable {
        // `TAB` on a file visits it, as treemacs does.
        return visit(editor, &Args::default());
    }
    act(editor, TreeAction::Toggle(node.path));
    Ok(())
}

fn expand_node(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    if node.expandable {
        act(editor, TreeAction::Expand(node.path));
    }
    Ok(())
}

fn collapse_node(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    if node.expandable && node.expanded {
        act(editor, TreeAction::Collapse(node.path));
        return Ok(());
    }
    // Collapsing a leaf moves up to its directory, as treemacs does.
    collapse_parent(editor, &Args::default())
}

fn collapse_parent(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(line) = parent_line(editor) else {
        return Err(crate::CoreError::Message("Already at the root".into()));
    };
    let path = editor.tree[line].path.clone();
    editor.move_tree_cursor_to_line(line);
    act(editor, TreeAction::Collapse(path));
    Ok(())
}

// ---- the root ----------------------------------------------------------

/// `treefile-root-down`: draw the tree from the selected directory instead.
///
/// treemacs' `treemacs-root-down`. Only the tree moves — the project root
/// that a language server is told about and that a project search walks
/// stays where it was, because looking into a subdirectory is not the same
/// as working in a different project.
fn root_down(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    if !node.expandable {
        return Err(crate::CoreError::Message(
            "Only a directory can be the root".into(),
        ));
    }
    set_root(editor, node.path);
    Ok(())
}

/// `treefile-root-up`: one directory further out.
fn root_up(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(root) = editor.tree_root.clone() else {
        return Err(crate::CoreError::Message("There is no tree".into()));
    };
    let Some(parent) = root.parent().map(std::path::Path::to_path_buf) else {
        return Err(crate::CoreError::Message(
            "Already at the top of the filesystem".into(),
        ));
    };
    set_root(editor, parent);
    Ok(())
}

/// `treefile-root-reset`: back to where the tree opened.
fn root_reset(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(home) = editor.tree_home.clone() else {
        return Err(crate::CoreError::Message("There is no tree".into()));
    };
    if editor.tree_root.as_ref() == Some(&home) {
        return Err(crate::CoreError::Message(
            "Already at the project root".into(),
        ));
    }
    set_root(editor, home);
    Ok(())
}

fn set_root(editor: &mut Editor, path: PathBuf) {
    editor.message(format!("Tree root: {}", path.display()));
    editor.tree_root = Some(path.clone());
    act(editor, TreeAction::SetRoot(path));
}

fn expand_recursively(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    if node.expandable {
        act(editor, TreeAction::ExpandRecursively(node.path));
    }
    Ok(())
}

// ---- visiting -----------------------------------------------------------

/// Opens the file under the cursor. `split` says whether to make a window for
/// it first, and `stay` keeps the tree selected afterwards.
fn open_selection(editor: &mut Editor, split: Option<Direction>, stay: bool) -> Result<()> {
    let node = selection(editor)?;
    if node.expandable {
        act(editor, TreeAction::Toggle(node.path));
        return Ok(());
    }
    let tree_window = editor.tree_window;
    // Never open a file into one of the panel's own windows.
    let target = editor
        .editing_window()
        .ok_or_else(|| crate::CoreError::Message("No window to open into".into()))?;
    editor.select_window(target);
    if let Some(direction) = split {
        editor.split_window(direction)?;
        editor.other_window(1);
    }

    match editor.buffers.find_by_path(&node.path) {
        Some(id) => editor.switch_to_buffer(id)?,
        None => editor.spawn(Task::ReadFile {
            path: node.path.clone(),
            reverting: None,
            other_window: false,
        }),
    }
    if stay && let Some(window) = tree_window {
        editor.select_window(window);
    }
    Ok(())
}

fn visit(editor: &mut Editor, _: &Args) -> Result<()> {
    open_selection(editor, None, false)
}

fn visit_below(editor: &mut Editor, _: &Args) -> Result<()> {
    open_selection(editor, Some(Direction::Vertical), false)
}

fn visit_beside(editor: &mut Editor, _: &Args) -> Result<()> {
    open_selection(editor, Some(Direction::Horizontal), false)
}

fn visit_other(editor: &mut Editor, _: &Args) -> Result<()> {
    open_selection(editor, None, false)
}

/// Shows the file without leaving the tree, so the arrow keys can walk a
/// directory previewing as they go.
fn peek(editor: &mut Editor, _: &Args) -> Result<()> {
    open_selection(editor, None, true)
}

fn visit_external(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    let directory = editor
        .tree_root
        .clone()
        .unwrap_or_else(|| editor.default_directory());
    editor.spawn(Task::Shell {
        command: crate::desktop_open_command(&node.path.to_string_lossy()),
        directory,
        insert_at: None,
    });
    Ok(())
}

// ---- file operations ----------------------------------------------------

/// The directory a new entry goes in: the selection when it is a directory,
/// otherwise its parent.
fn target_directory(editor: &Editor) -> Result<PathBuf> {
    let node = selection(editor)?;
    if node.expandable {
        return Ok(node.path);
    }
    node.path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| crate::CoreError::Message("No directory here".into()))
}

fn create_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "treefile-create-file",
            MinibufferKind::Text,
            "Create file: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let parent = target_directory(editor)?;
    act(editor, TreeAction::CreateFile { parent, name });
    Ok(())
}

fn create_dir(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "treefile-create-dir",
            MinibufferKind::Text,
            "Create directory: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let parent = target_directory(editor)?;
    act(editor, TreeAction::CreateDirectory { parent, name });
    Ok(())
}

fn rename(editor: &mut Editor, args: &Args) -> Result<()> {
    let node = selection(editor)?;
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "treefile-rename-file",
            MinibufferKind::Text,
            format!("Rename `{}` to: ", node.name),
            &node.name,
            Vec::new(),
        );
        return Ok(());
    };
    act(
        editor,
        TreeAction::Rename {
            path: node.path,
            name,
        },
    );
    Ok(())
}

fn delete(editor: &mut Editor, args: &Args) -> Result<()> {
    let node = selection(editor)?;
    let Some(answer) = args.input.clone() else {
        editor.prompt_for(
            "treefile-delete-file",
            MinibufferKind::YesNo,
            format!("Delete `{}`? (yes or no) ", node.name),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if !answer.eq_ignore_ascii_case("yes") && !answer.eq_ignore_ascii_case("y") {
        editor.message("Not deleted");
        return Ok(());
    }
    act(editor, TreeAction::Delete(node.path));
    Ok(())
}

fn move_file(editor: &mut Editor, args: &Args) -> Result<()> {
    let node = selection(editor)?;
    let Some(destination) = args.input.clone() else {
        let initial = editor
            .tree_root
            .clone()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        editor.prompt_for(
            "treefile-move-file",
            MinibufferKind::File,
            format!("Move `{}` to directory: ", node.name),
            &initial,
            Vec::new(),
        );
        return Ok(());
    };
    act(
        editor,
        TreeAction::Move {
            path: node.path,
            destination: PathBuf::from(destination.trim()),
        },
    );
    Ok(())
}

fn shell_command(editor: &mut Editor, args: &Args) -> Result<()> {
    let directory = target_directory(editor)?;
    let Some(command) = args.input.clone() else {
        editor.prompt_for(
            "treefile-run-shell-command",
            MinibufferKind::Shell,
            format!("Shell command in {}: ", directory.display()),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if command.trim().is_empty() {
        return Err(crate::CoreError::Message("No command given".into()));
    }
    editor.spawn(Task::Shell {
        command,
        directory,
        insert_at: None,
    });
    Ok(())
}

// ---- copying paths ------------------------------------------------------

/// Puts `text` on the kill ring and says so.
fn copy(editor: &mut Editor, text: String) -> Result<()> {
    editor.kill_ring.kill_new(text.clone());
    editor.message(format!("Copied `{text}`"));
    Ok(())
}

fn copy_absolute(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    copy(editor, node.path.to_string_lossy().into_owned())
}

fn copy_relative(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    let root = editor.tree_root.clone().unwrap_or_default();
    let relative = node.path.strip_prefix(&root).unwrap_or(&node.path);
    copy(editor, relative.to_string_lossy().into_owned())
}

fn copy_name(editor: &mut Editor, _: &Args) -> Result<()> {
    let node = selection(editor)?;
    copy(editor, node.name)
}

// ---- toggles and width --------------------------------------------------

fn toggle_hidden(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, TreeAction::ToggleHidden);
    Ok(())
}

fn toggle_directories_first(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, TreeAction::ToggleDirectoriesFirst);
    Ok(())
}

fn toggle_git(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, TreeAction::ToggleGitStatus);
    Ok(())
}

fn toggle_follow(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.tree_follow = !editor.tree_follow;
    editor.message(if editor.tree_follow {
        "Follow mode on"
    } else {
        "Follow mode off"
    });
    Ok(())
}

fn toggle_fixed_width(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.tree_width_locked = !editor.tree_width_locked;
    editor.message(if editor.tree_width_locked {
        "Width locked"
    } else {
        "Width unlocked"
    });
    Ok(())
}

/// Changes the tree width by `delta`, refusing when the width is locked.
fn resize(editor: &mut Editor, delta: i32) -> Result<()> {
    if editor.tree_width_locked {
        return Err(crate::CoreError::Message("Tree width is locked".into()));
    }
    let width = (editor.tree_width as i32 + delta).clamp(8, 200) as u16;
    editor.tree_width = width;
    if let Some(window) = editor.tree_window {
        editor.windows.set_fixed_width(window, width);
    }
    Ok(())
}

fn increase_width(editor: &mut Editor, args: &Args) -> Result<()> {
    resize(editor, args.signed_count().max(1))
}

fn decrease_width(editor: &mut Editor, args: &Args) -> Result<()> {
    resize(editor, -args.signed_count().max(1))
}

fn set_width(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let current = editor.tree_width.to_string();
        editor.prompt_for(
            "treefile-set-width",
            MinibufferKind::Text,
            "Width: ",
            &current,
            Vec::new(),
        );
        return Ok(());
    };
    let width: i32 = input
        .trim()
        .parse()
        .map_err(|_| crate::CoreError::Message(format!("`{input}` is not a number")))?;
    let delta = width - editor.tree_width as i32;
    resize(editor, delta)
}

fn refresh(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, TreeAction::Refresh);
    Ok(())
}

/// `?`: lists the tree's own bindings.
/// `?`: the keymap, in the panel `C-x` and `C-c` already draw.
///
/// It was a `*Help*` buffer of fifty lines, opened in the window beside the
/// tree — which is to say it took the file being edited off the screen to
/// tell you which key moves down one line. treemacs does not do that. Its
/// `?` summons a hydra: a panel of named columns over the bottom of the
/// frame, with the keys still live underneath, so the thing being explained
/// can be done while the explanation is up.
///
/// Pressing it again puts it away, which is the other half of a key that
/// costs nothing to press.
fn help(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.key_menu = match editor.key_menu.is_some() {
        true => None,
        false => Some(crate::which_key::Menu::tree()),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tree::{NodeKind, VisibleNode};
    use maxgus_tui::Rect;

    fn node(path: &str, name: &str, depth: usize, directory: bool, expanded: bool) -> VisibleNode {
        VisibleNode {
            path: PathBuf::from(path),
            name: name.into(),
            kind: if directory {
                NodeKind::Directory
            } else {
                NodeKind::File
            },
            depth,
            expanded,
            expandable: directory,
            git: None,
            is_root: depth == 0,
        }
    }

    /// A tree resembling a small project, already expanded one level.
    fn snapshot() -> Vec<VisibleNode> {
        vec![
            node("/project", "project", 0, true, true),
            node("/project/src", "src", 1, true, true),
            node("/project/src/main.rs", "main.rs", 2, false, false),
            node("/project/src/lib.rs", "lib.rs", 2, false, false),
            node("/project/docs", "docs", 1, true, false),
            node("/project/Cargo.toml", "Cargo.toml", 1, false, false),
        ]
    }

    fn setup() -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 100, 24),
        );
        let id = editor.buffers.visit_file("/project/start.rs", "");
        editor.switch_to_buffer(id).unwrap();

        let mut registry = Registry::new();
        register(&mut registry);
        super::super::minibuffer::register(&mut registry);
        super::super::motion::register(&mut registry);
        super::super::edit::register(&mut registry);
        super::super::window::register(&mut registry);
        super::super::buffer::register(&mut registry);
        super::super::file::register(&mut registry);
        (Dispatcher::new(registry), editor)
    }

    /// Opens the tree and installs a snapshot, as a refresh result would.
    fn with_tree(d: &mut Dispatcher, e: &mut Editor) {
        d.execute(e, "treefile-toggle", None);
        e.tasks.drain();
        e.apply_task_result(crate::TaskResult::TreeUpdated {
            nodes: snapshot(),
            select: None,
            show_hidden: false,
        })
        .unwrap();
        let window = e.tree_window.expect("the tree window is open");
        e.select_window(window);
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

    fn selected(e: &Editor) -> String {
        e.tree_selection()
            .map(|n| n.name.clone())
            .unwrap_or_default()
    }

    #[test]
    fn every_treemacs_binding_is_registered() {
        // The map covers the whole panel, not only the tree, so the panel's
        // own commands have to be registered alongside it.
        let mut registry = Registry::new();
        register(&mut registry);
        crate::commands::panel::register(&mut registry);
        for (keys, command) in maxgus_tree::TREEMACS_BINDINGS {
            assert!(
                registry.contains(command),
                "`{keys}` runs unregistered `{command}`"
            );
        }
    }

    #[test]
    fn toggling_opens_and_closes_the_side_window() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "treefile-toggle");
        assert!(e.tree_window.is_some());
        // The panel is a column: the tree and the buffer list, with no
        // outline because no language server is running. Plus the buffer.
        assert_eq!(e.panel_windows.len(), 2);
        assert_eq!(e.windows.len(), 3);
        let window = e.tree_window.unwrap();
        assert_eq!(
            e.windows.get(window).unwrap().rect.x,
            0,
            "the tree sits on the left"
        );
        assert!(e.windows.get(window).unwrap().side);
        assert!(
            e.tasks
                .peek()
                .iter()
                .any(|t| matches!(t, Task::Tree(TreeAction::Refresh))),
            "a refresh was queued"
        );

        run(&mut d, &mut e, "treefile-toggle");
        assert!(e.tree_window.is_none());
        assert_eq!(e.windows.len(), 1);
    }

    #[test]
    fn the_tree_keymap_takes_over_while_the_tree_is_selected() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        assert_eq!(
            d.handle_keys(&mut e, "n").command(),
            Some("treefile-next-line")
        );
        assert_eq!(
            d.handle_keys(&mut e, "u").command(),
            Some("treefile-goto-parent")
        );

        run(&mut d, &mut e, "treefile-quit");
        assert_eq!(
            d.handle_keys(&mut e, "n").command(),
            Some("self-insert-command")
        );
    }

    #[test]
    fn a_snapshot_is_rendered_into_the_tree_buffer() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        let id = e.buffers.find_by_name(TREE_BUFFER_NAME).unwrap();
        let text = e.buffers.get(id).unwrap().text();
        assert!(text.starts_with("v project\n"), "got `{text}`");
        assert!(text.contains("  v src"), "got `{text}`");
        assert!(text.contains("      main.rs"), "got `{text}`");
        assert!(text.contains("  > docs"), "collapsed, got `{text}`");
        assert!(e.buffers.get(id).unwrap().is_read_only());
    }

    #[test]
    fn navigation_moves_through_the_snapshot() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        assert_eq!(selected(&e), "project");
        run(&mut d, &mut e, "treefile-next-line");
        assert_eq!(selected(&e), "src");
        run(&mut d, &mut e, "treefile-next-line");
        assert_eq!(selected(&e), "main.rs");
        run(&mut d, &mut e, "treefile-previous-line");
        assert_eq!(selected(&e), "src");
        run(&mut d, &mut e, "treefile-goto-last");
        assert_eq!(selected(&e), "Cargo.toml");
        run(&mut d, &mut e, "treefile-goto-first");
        assert_eq!(selected(&e), "project");
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        run(&mut d, &mut e, "treefile-previous-line");
        assert_eq!(selected(&e), "project", "it walked off the top");
        e.prefix = crate::Prefix::Numeric(100);
        d.execute(&mut e, "treefile-next-line", None);
        assert_eq!(selected(&e), "Cargo.toml", "it walked off the bottom");
    }

    #[test]
    fn neighbour_navigation_skips_over_children() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(1);
        assert_eq!(selected(&e), "src");
        run(&mut d, &mut e, "treefile-next-neighbour");
        assert_eq!(selected(&e), "docs", "skipped src's children");
        run(&mut d, &mut e, "treefile-next-neighbour");
        assert_eq!(selected(&e), "Cargo.toml");
        assert!(fails(&mut d, &mut e, "treefile-next-neighbour").contains("No more nodes"));
        run(&mut d, &mut e, "treefile-previous-neighbour");
        assert_eq!(selected(&e), "docs");
    }

    #[test]
    fn goto_parent_walks_up_a_level() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        assert_eq!(selected(&e), "main.rs");
        run(&mut d, &mut e, "treefile-goto-parent");
        assert_eq!(selected(&e), "src");
        run(&mut d, &mut e, "treefile-goto-parent");
        assert_eq!(selected(&e), "project");
        assert!(fails(&mut d, &mut e, "treefile-goto-parent").contains("root"));
    }

    #[test]
    fn toggling_a_directory_queues_the_work_rather_than_blocking() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(4);
        assert_eq!(selected(&e), "docs");
        run(&mut d, &mut e, "treefile-toggle-node");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::Toggle(PathBuf::from(
                "/project/docs"
            )))]
        );
    }

    #[test]
    fn collapsing_a_leaf_collapses_its_directory_instead() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        run(&mut d, &mut e, "treefile-collapse-node");
        assert_eq!(selected(&e), "src", "moved up to the directory");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::Collapse(PathBuf::from(
                "/project/src"
            )))]
        );
    }

    #[test]
    fn expanding_recursively_queues_one_action() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(1);
        run(&mut d, &mut e, "treefile-expand-recursively");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::ExpandRecursively(PathBuf::from(
                "/project/src"
            )))]
        );
    }

    #[test]
    fn visiting_a_file_opens_it_outside_the_tree_window() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        e.tasks.drain();
        run(&mut d, &mut e, "treefile-visit-node");

        assert_ne!(
            e.windows.current_id(),
            e.tree_window.unwrap(),
            "not into the tree window"
        );
        assert_eq!(
            e.tasks.drain(),
            vec![Task::ReadFile {
                path: PathBuf::from("/project/src/main.rs"),
                reverting: None,
                other_window: false
            }]
        );
    }

    #[test]
    fn visiting_an_already_open_file_switches_to_its_buffer() {
        let (mut d, mut e) = setup();
        e.buffers.visit_file("/project/src/main.rs", "fn main() {}");
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        e.tasks.drain();
        run(&mut d, &mut e, "treefile-visit-node");
        assert!(e.tasks.is_empty(), "nothing needed reading");
        assert_eq!(e.current_buffer().name(), "main.rs");
    }

    #[test]
    fn visiting_in_a_split_makes_a_window_for_it() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        run(&mut d, &mut e, "treefile-visit-node-vertical-split");
        assert_eq!(
            e.windows.len(),
            e.panel_windows.len() + 2,
            "the panel plus two"
        );
    }

    #[test]
    fn visiting_a_directory_expands_it_instead() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(4);
        e.tasks.drain();
        run(&mut d, &mut e, "treefile-visit-node");
        assert!(matches!(
            &e.tasks.peek()[0],
            Task::Tree(TreeAction::Toggle(_))
        ));
    }

    #[test]
    fn peeking_leaves_the_tree_selected() {
        let (mut d, mut e) = setup();
        e.buffers.visit_file("/project/src/main.rs", "");
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        run(&mut d, &mut e, "treefile-peek");
        assert_eq!(e.windows.current_id(), e.tree_window.unwrap());
    }

    #[test]
    fn creating_prompts_and_targets_the_right_directory() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        // On a file, the new entry goes beside it.
        e.move_tree_cursor_to_line(2);
        d.execute(&mut e, "treefile-create-file", None);
        assert!(e.minibuffer.is_active());
        for c in "new.rs".chars() {
            e.minibuffer.insert_char(c);
        }
        e.tasks.drain();
        d.handle_keys(&mut e, "RET");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::CreateFile {
                parent: PathBuf::from("/project/src"),
                name: "new.rs".into()
            })]
        );
    }

    #[test]
    fn creating_on_a_directory_puts_the_entry_inside_it() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(4);
        d.execute(&mut e, "treefile-create-dir", None);
        for c in "images".chars() {
            e.minibuffer.insert_char(c);
        }
        e.tasks.drain();
        d.handle_keys(&mut e, "RET");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::CreateDirectory {
                parent: PathBuf::from("/project/docs"),
                name: "images".into()
            })]
        );
    }

    #[test]
    fn renaming_starts_from_the_current_name() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        d.execute(&mut e, "treefile-rename-file", None);
        assert_eq!(e.minibuffer.input(), "main.rs");
        e.minibuffer.kill_whole();
        for c in "app.rs".chars() {
            e.minibuffer.insert_char(c);
        }
        e.tasks.drain();
        d.handle_keys(&mut e, "RET");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::Rename {
                path: PathBuf::from("/project/src/main.rs"),
                name: "app.rs".into()
            })]
        );
    }

    #[test]
    fn deleting_asks_first_and_a_no_cancels_it() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);

        d.execute(&mut e, "treefile-delete-file", None);
        assert!(e.minibuffer.prompt().contains("Delete `main.rs`?"));
        for c in "no".chars() {
            e.minibuffer.insert_char(c);
        }
        e.tasks.drain();
        d.handle_keys(&mut e, "RET");
        assert!(e.tasks.is_empty(), "nothing was queued");
        assert_eq!(e.minibuffer.display(), "Not deleted");

        d.execute(&mut e, "treefile-delete-file", None);
        for c in "yes".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        assert_eq!(
            e.tasks.drain(),
            vec![Task::Tree(TreeAction::Delete(PathBuf::from(
                "/project/src/main.rs"
            )))]
        );
    }

    #[test]
    fn copying_paths_puts_them_on_the_kill_ring() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);

        run(&mut d, &mut e, "treefile-copy-absolute-path");
        assert_eq!(e.kill_ring.front(), Some("/project/src/main.rs"));
        run(&mut d, &mut e, "treefile-copy-relative-path");
        assert_eq!(e.kill_ring.front(), Some("src/main.rs"));
        run(&mut d, &mut e, "treefile-copy-file");
        assert_eq!(e.kill_ring.front(), Some("main.rs"));
        assert!(e.minibuffer.display().starts_with("Copied"));
    }

    #[test]
    fn the_toggles_queue_their_actions() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.tasks.drain();
        run(&mut d, &mut e, "treefile-toggle-show-dotfiles");
        run(&mut d, &mut e, "treefile-toggle-git-mode");
        run(&mut d, &mut e, "treefile-toggle-directories-first");
        assert_eq!(
            e.tasks.drain(),
            vec![
                Task::Tree(TreeAction::ToggleHidden),
                Task::Tree(TreeAction::ToggleGitStatus),
                Task::Tree(TreeAction::ToggleDirectoriesFirst),
            ]
        );
    }

    #[test]
    fn follow_mode_toggles_locally() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        let before = e.tree_follow;
        run(&mut d, &mut e, "treefile-toggle-follow-mode");
        assert_ne!(e.tree_follow, before);
        assert!(e.minibuffer.display().starts_with("Follow mode"));
    }

    #[test]
    fn the_width_can_be_changed_and_locked() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        let window = e.tree_window.unwrap();
        let before = e.tree_width;

        e.prefix = crate::Prefix::Numeric(6);
        d.execute(&mut e, "treefile-increase-width", None);
        assert_eq!(e.tree_width, before + 6);
        assert_eq!(e.windows.get(window).unwrap().rect.width, before + 6);

        e.prefix = crate::Prefix::Numeric(6);
        d.execute(&mut e, "treefile-decrease-width", None);
        assert_eq!(e.tree_width, before);

        run(&mut d, &mut e, "treefile-toggle-fixed-width");
        assert!(fails(&mut d, &mut e, "treefile-increase-width").contains("locked"));
    }

    #[test]
    fn the_width_can_be_set_outright_and_has_a_floor() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        d.execute(&mut e, "treefile-set-width", None);
        e.minibuffer.kill_whole();
        for c in "50".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.tree_width, 50);

        d.execute(&mut e, "treefile-set-width", None);
        e.minibuffer.kill_whole();
        for c in "2".chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(&mut e, "RET");
        assert_eq!(e.tree_width, 8, "a floor keeps the tree usable");
    }

    #[test]
    fn a_width_that_is_not_a_number_is_refused() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        d.execute(&mut e, "treefile-set-width", None);
        e.minibuffer.kill_whole();
        for c in "wide".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn a_shell_command_runs_in_the_selected_directory() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        d.execute(&mut e, "treefile-run-shell-command", None);
        assert!(e.minibuffer.prompt().contains("/project/src"));
        for c in "ls".chars() {
            e.minibuffer.insert_char(c);
        }
        e.tasks.drain();
        d.handle_keys(&mut e, "RET");
        let Task::Shell {
            command, directory, ..
        } = &e.tasks.peek()[0]
        else {
            panic!()
        };
        assert_eq!(command, "ls");
        assert_eq!(directory, &PathBuf::from("/project/src"));
    }

    #[test]
    fn opening_externally_quotes_the_path() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(2);
        e.tasks.drain();
        run(&mut d, &mut e, "treefile-visit-node-external");
        let Task::Shell { command, .. } = &e.tasks.peek()[0] else {
            panic!()
        };
        assert_eq!(command, "xdg-open '/project/src/main.rs'");
        assert_eq!(crate::shell_quote("it's here"), r"'it'\''s here'");
    }

    #[test]
    fn the_help_opens_a_panel_rather_than_taking_a_window() {
        // It used to put a fifty-line `*Help*` buffer in the window beside
        // the tree, which is to say it took the file being edited off the
        // screen to say which key moves down one line.
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        let editing = e.current_buffer_id();
        run(&mut d, &mut e, "treefile-help");

        let menu = e.key_menu.as_ref().expect("no panel opened");
        assert_eq!(menu.title, "File tree");
        let entries: Vec<&str> = menu
            .sections
            .iter()
            .flat_map(|s| s.entries.iter().map(|(key, _)| key.as_str()))
            .collect();
        assert!(entries.contains(&"RET"), "got {entries:?}");
        assert!(entries.contains(&"c f"), "got {entries:?}");
        assert!(
            menu.sections.iter().any(|s| s.title == "Navigation"),
            "no headings: {:?}",
            menu.sections.iter().map(|s| &s.title).collect::<Vec<_>>()
        );
        assert_eq!(
            e.current_buffer_id(),
            editing,
            "the panel took a window from the buffer being edited"
        );
        assert!(
            e.buffers.find_by_name("*Help*").is_none(),
            "the help buffer is still being made"
        );
    }

    #[test]
    fn the_panel_stays_up_while_the_tree_is_walked() {
        // treemacs' hydra leaves the keys live, which is the point of it:
        // reading what `n` does and pressing it should not take two goes.
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        run(&mut d, &mut e, "treefile-help");
        run(&mut d, &mut e, "treefile-next-line");
        assert!(e.key_menu.is_some(), "walking the tree closed the panel");
    }

    #[test]
    fn asking_again_puts_the_panel_away() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        run(&mut d, &mut e, "treefile-help");
        run(&mut d, &mut e, "treefile-help");
        assert!(e.key_menu.is_none(), "`?` would not close it");
    }

    #[test]
    fn quitting_puts_the_panel_away() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        run(&mut d, &mut e, "treefile-help");
        run(&mut d, &mut e, "keyboard-quit");
        assert!(e.key_menu.is_none(), "`C-g` would not close it");
    }

    #[test]
    fn a_refresh_result_keeps_the_cursor_where_it_was() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(3);
        assert_eq!(selected(&e), "lib.rs");

        e.apply_task_result(crate::TaskResult::TreeUpdated {
            nodes: snapshot(),
            select: None,
            show_hidden: false,
        })
        .unwrap();
        assert_eq!(
            selected(&e),
            "lib.rs",
            "the cursor line survived the redraw"
        );
    }

    #[test]
    fn the_cursor_stays_on_its_node_when_the_tree_around_it_changes() {
        // Keeping the *line* is not enough: expanding a directory above the
        // cursor pushes everything below it down, and the line the cursor
        // was on is then a different node.
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(4);
        assert_eq!(selected(&e), "docs");

        // `src` grows two more children, above where the cursor is.
        let mut grown = snapshot();
        grown.insert(4, node("/project/src/a.rs", "a.rs", 2, false, false));
        grown.insert(5, node("/project/src/b.rs", "b.rs", 2, false, false));
        e.apply_task_result(crate::TaskResult::TreeUpdated {
            nodes: grown,
            select: None,
            show_hidden: false,
        })
        .unwrap();
        assert_eq!(
            selected(&e),
            "docs",
            "the cursor followed the line rather than the node"
        );
    }

    #[test]
    fn a_cursor_on_a_node_that_is_gone_keeps_its_place_in_the_list() {
        // Deleting what the cursor was on has to leave it somewhere, and
        // the line it was on is the nearest thing to where it was.
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.move_tree_cursor_to_line(3);
        assert_eq!(selected(&e), "lib.rs");

        let mut without = snapshot();
        without.remove(3);
        e.apply_task_result(crate::TaskResult::TreeUpdated {
            nodes: without,
            select: None,
            show_hidden: false,
        })
        .unwrap();
        assert_eq!(
            selected(&e),
            "docs",
            "it should not have jumped to the root"
        );
    }

    #[test]
    fn a_result_can_ask_for_a_particular_node_to_be_selected() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        e.apply_task_result(crate::TaskResult::TreeUpdated {
            nodes: snapshot(),
            select: Some(PathBuf::from("/project/Cargo.toml")),
            show_hidden: true,
        })
        .unwrap();
        assert_eq!(selected(&e), "Cargo.toml");
        assert!(e.tree_shows_hidden);
    }

    #[test]
    fn killing_the_tree_forgets_it_entirely() {
        let (mut d, mut e) = setup();
        with_tree(&mut d, &mut e);
        run(&mut d, &mut e, "treefile-kill");
        assert!(e.tree_window.is_none());
        assert!(e.tree.is_empty());
        assert!(e.buffers.find_by_name(TREE_BUFFER_NAME).is_none());
    }

    #[test]
    fn commands_on_an_empty_tree_say_there_is_nothing_here() {
        let (mut d, mut e) = setup();
        d.execute(&mut e, "treefile-toggle", None);
        assert!(fails(&mut d, &mut e, "treefile-toggle-node").contains("No node here"));
        assert!(fails(&mut d, &mut e, "treefile-copy-absolute-path").contains("No node here"));
        assert!(fails(&mut d, &mut e, "treefile-delete-file").contains("No node here"));
        assert!(fails(&mut d, &mut e, "treefile-rename-file").contains("No node here"));
    }
}
