//! Window commands: the `C-x` family that splits, deletes and moves between
//! windows.

use crate::{
    Result, command,
    command::{Args, Registry},
    editor::Editor,
    window::{Direction, Towards},
};

/// Registers the window commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "split-window-below",
            "Split the selected window in two, stacked.",
            split_below
        ),
        command!(
            "split-window-right",
            "Split the selected window in two, side by side.",
            split_right
        ),
        command!(
            "delete-window",
            "Delete the selected window.",
            delete_window
        ),
        command!(
            "delete-other-windows",
            "Delete every window but the selected one.",
            delete_other_windows
        ),
        command!("other-window", "Select another window.", other_window),
        command!(
            "windmove-left",
            "Select the window to the left.",
            windmove_left
        ),
        command!(
            "windmove-right",
            "Select the window to the right.",
            windmove_right
        ),
        command!("windmove-up", "Select the window above.", windmove_up),
        command!("windmove-down", "Select the window below.", windmove_down),
        command!(
            "enlarge-window",
            "Make the selected window taller.",
            enlarge_window
        ),
        command!(
            "shrink-window",
            "Make the selected window shorter.",
            shrink_window
        ),
        command!(
            "enlarge-window-horizontally",
            "Make the selected window wider.",
            enlarge_horizontally
        ),
        command!(
            "shrink-window-horizontally",
            "Make the selected window narrower.",
            shrink_horizontally
        ),
        command!(
            "balance-windows",
            "Give every window the same size.",
            balance_windows
        ),
        command!(
            "scroll-other-window",
            "Scroll the next window down a page.",
            scroll_other_window
        ),
        command!(
            "move-to-window-line-top-bottom",
            "Move point to the middle, top or bottom line of the window.",
            move_to_window_line_top_bottom
        ),
        command!(
            "kill-buffer-and-window",
            "Kill this buffer and delete its window.",
            kill_buffer_and_window
        ),
    ]);
}

fn split_below(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.split_window(Direction::Vertical)?;
    Ok(())
}

fn split_right(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.split_window(Direction::Horizontal)?;
    Ok(())
}

fn delete_window(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.sync_to_buffer();
    let current = editor.windows.current_id();
    editor.windows.delete(current)?;
    // The tree window may have been the one deleted.
    if editor.tree_window == Some(current) {
        editor.tree_window = None;
    }
    editor.follow_point();
    Ok(())
}

fn delete_other_windows(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.sync_to_buffer();
    let kept = editor.windows.current_id();
    editor.windows.delete_others();
    if editor.tree_window.is_some_and(|tree| tree != kept) {
        editor.tree_window = None;
    }
    editor.follow_point();
    Ok(())
}

fn other_window(editor: &mut Editor, args: &Args) -> Result<()> {
    if editor.windows.len() < 2 {
        return Err(crate::CoreError::Message("No other window".into()));
    }
    editor.other_window(args.signed_count());
    Ok(())
}

/// `C-M-v`: scrolls the window `C-x o` would go to, without leaving this one.
fn scroll_other_window(editor: &mut Editor, args: &Args) -> Result<()> {
    if editor.windows.len() < 2 {
        return Err(crate::CoreError::Message("No other window".into()));
    }
    let here = editor.windows.current_id();
    // Selecting the other window, scrolling it and coming back is what keeps
    // this in step with `scroll-up-command`: the scroll has to drag that
    // window's point along, and that logic only exists for the selected one.
    editor.other_window(args.signed_count().signum().max(1));
    crate::commands::motion::scroll_selected_window_down(editor);
    editor.windows.select(here);
    Ok(())
}

/// `M-r`: middle line, then top, then bottom, cycling while the key repeats.
/// A prefix argument names a line from the top, or from the bottom when
/// negative, and does not cycle.
fn move_to_window_line_top_bottom(editor: &mut Editor, args: &Args) -> Result<()> {
    let (top, height) = {
        let window = editor.windows.current();
        (window.top_line, window.text_height().max(1))
    };
    let last = editor.current_buffer().len_lines().saturating_sub(1);

    let target = if args.prefix.is_present() {
        let n = args.signed_count();
        match n < 0 {
            true => top + height.saturating_sub(n.unsigned_abs() as usize),
            false => top + n as usize,
        }
    } else {
        // Derived from where point already sits, so the cycle needs no state
        // of its own — the same trick `recenter-top-bottom` uses.
        let repeating = matches!(
            editor.last_command.as_deref(),
            Some("move-to-window-line-top-bottom")
        );
        let current = editor
            .current_buffer()
            .line_of(editor.windows.current().point);
        let middle = top + height / 2;
        let bottom = top + height.saturating_sub(1);
        match repeating {
            false => middle,
            true if current == middle => top,
            true if current == top => bottom,
            true => middle,
        }
    };
    let line = target.min(last);
    editor.with_current_buffer(|b| {
        let at = b.line_start(line);
        b.set_point(maxgus_text::Motion::back_to_indentation(b.rope(), at));
    });
    Ok(())
}

/// `C-x 4 0`: the pair of actions people reach for together often enough that
/// Emacs gives them one key.
fn kill_buffer_and_window(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    editor.kill_buffer(id)?;
    // Only after the buffer is gone, and only when there is more than one
    // window: deleting the last one would leave nothing to draw into.
    if editor.windows.len() > 1 {
        delete_window(editor, &Args::new(crate::Prefix::None, None))?;
    }
    Ok(())
}

/// `C-<left>`, `C-<right>`, `C-<up>`, `C-<down>`: go to the window that way.
///
/// `C-x o` cycles in storage order, which with a file tree open means guessing
/// where you will end up. This says where to go and goes there.
fn windmove(editor: &mut Editor, towards: Towards) -> Result<()> {
    let here = editor.windows.current_id();
    let Some(target) = editor.windows.neighbour(here, towards) else {
        return Err(crate::CoreError::Message(format!(
            "No window {}",
            match towards {
                Towards::Left => "to the left",
                Towards::Right => "to the right",
                Towards::Up => "above",
                Towards::Down => "below",
            }
        )));
    };
    editor.select_window(target);
    Ok(())
}

fn windmove_left(editor: &mut Editor, _: &Args) -> Result<()> {
    windmove(editor, Towards::Left)
}

fn windmove_right(editor: &mut Editor, _: &Args) -> Result<()> {
    windmove(editor, Towards::Right)
}

fn windmove_up(editor: &mut Editor, _: &Args) -> Result<()> {
    windmove(editor, Towards::Up)
}

fn windmove_down(editor: &mut Editor, _: &Args) -> Result<()> {
    windmove(editor, Towards::Down)
}

/// The three resize commands all adjust a side window's fixed width; an
/// ordinary window's size is decided by the layout, which splits evenly.
fn resize(editor: &mut Editor, delta: i32) -> Result<()> {
    let id = editor.windows.current_id();
    let Some(window) = editor.windows.get(id) else {
        return Err(crate::CoreError::NoSuchWindow);
    };
    if !window.side {
        return Err(crate::CoreError::Message(
            "Only side windows have an adjustable size".into(),
        ));
    }
    let width = window.rect.width as i32 + delta;
    editor.windows.set_fixed_width(id, width.max(8) as u16);
    Ok(())
}

fn enlarge_window(editor: &mut Editor, args: &Args) -> Result<()> {
    resize_vertically(editor, args.signed_count())
}

fn shrink_window(editor: &mut Editor, args: &Args) -> Result<()> {
    resize_vertically(editor, -args.signed_count())
}

/// Changes the selected window's height by `delta` rows.
///
/// Pins the height and looks at what the layout actually did with it: a
/// window that is not one the layout gives a height to — the sole window, or
/// one whose height comes from a split above it — is reported rather than
/// silently doing nothing.
fn resize_vertically(editor: &mut Editor, delta: i32) -> Result<()> {
    let id = editor.windows.current_id();
    let Some(window) = editor.windows.get(id) else {
        return Err(crate::CoreError::NoSuchWindow);
    };
    let before = window.rect.height;
    let wanted = (before as i32 + delta).max(2) as u16;
    editor.windows.set_fixed_height(id, wanted);
    let after = editor
        .windows
        .get(id)
        .map(|window| window.rect.height)
        .unwrap_or(before);
    if after == before {
        editor.windows.clear_fixed_height(id);
        return Err(crate::CoreError::Message(
            "The layout decides this window's height".into(),
        ));
    }
    editor.follow_point();
    Ok(())
}

fn enlarge_horizontally(editor: &mut Editor, args: &Args) -> Result<()> {
    resize(editor, args.signed_count())
}

fn shrink_horizontally(editor: &mut Editor, args: &Args) -> Result<()> {
    resize(editor, -args.signed_count())
}

fn balance_windows(editor: &mut Editor, _: &Args) -> Result<()> {
    let frame = editor.windows.frame();
    editor.windows.layout(frame);
    editor.follow_point();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher, Prefix};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup() -> (Dispatcher, Editor) {
        let editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let mut registry = Registry::new();
        register(&mut registry);
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

    #[test]
    fn every_window_binding_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for name in [
            "split-window-below",
            "split-window-right",
            "delete-window",
            "delete-other-windows",
            "other-window",
            "balance-windows",
        ] {
            assert!(registry.contains(name), "`{name}` is missing");
        }
    }

    #[test]
    fn splitting_below_makes_two_stacked_windows() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-below");
        assert_eq!(e.windows.len(), 2);
        // The frame is 24 rows; the echo area takes one, leaving 23 to split.
        let heights: Vec<u16> = e.windows.iter().map(|w| w.rect.height).collect();
        assert_eq!(heights, vec![12, 11]);
    }

    #[test]
    fn splitting_right_makes_two_side_by_side_windows() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-right");
        let widths: Vec<u16> = e.windows.iter().map(|w| w.rect.width).collect();
        assert_eq!(widths, vec![40, 40]);
    }

    #[test]
    fn a_split_keeps_the_original_window_selected() {
        let (mut d, mut e) = setup();
        let before = e.windows.current_id();
        run(&mut d, &mut e, "split-window-below");
        assert_eq!(e.windows.current_id(), before);
    }

    #[test]
    fn a_window_too_small_to_split_refuses() {
        let mut e = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 3),
        );
        let mut registry = Registry::new();
        register(&mut registry);
        let mut d = Dispatcher::new(registry);
        assert!(fails(&mut d, &mut e, "split-window-below").contains("too small"));
    }

    #[test]
    fn scrolling_the_other_window_leaves_this_one_selected() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("long", &"line\n".repeat(200));
        e.switch_to_buffer(id).unwrap();
        run(&mut d, &mut e, "split-window-below");
        let here = e.windows.current_id();
        let before = e.windows.current().top_line;

        run(&mut d, &mut e, "scroll-other-window");

        assert_eq!(e.windows.current_id(), here, "selection came back");
        assert_eq!(
            e.windows.current().top_line,
            before,
            "this window did not move"
        );
        let other = e
            .windows
            .ids()
            .into_iter()
            .find(|w| *w != here)
            .expect("two windows");
        assert!(
            e.windows.get(other).unwrap().top_line > before,
            "the other one scrolled"
        );
    }

    #[test]
    fn scrolling_the_other_window_needs_one() {
        let (mut d, mut e) = setup();
        assert_eq!(
            fails(&mut d, &mut e, "scroll-other-window"),
            "No other window"
        );
    }

    #[test]
    fn move_to_window_line_cycles_middle_top_bottom() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("long", &"line\n".repeat(200));
        e.switch_to_buffer(id).unwrap();
        let top = e.windows.current().top_line;
        let height = e.windows.current().text_height();

        let line_of_point = |e: &Editor| e.current_buffer().line_of(e.current_buffer().point());

        run(&mut d, &mut e, "move-to-window-line-top-bottom");
        assert_eq!(line_of_point(&e), top + height / 2, "the middle first");
        e.last_command = Some("move-to-window-line-top-bottom".into());
        run(&mut d, &mut e, "move-to-window-line-top-bottom");
        assert_eq!(line_of_point(&e), top, "then the top");
        e.last_command = Some("move-to-window-line-top-bottom".into());
        run(&mut d, &mut e, "move-to-window-line-top-bottom");
        assert_eq!(line_of_point(&e), top + height - 1, "then the bottom");
    }

    #[test]
    fn move_to_window_line_with_an_argument_counts_from_the_top() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("long", &"line\n".repeat(200));
        e.switch_to_buffer(id).unwrap();
        let top = e.windows.current().top_line;
        e.prefix = crate::Prefix::Numeric(3);
        run(&mut d, &mut e, "move-to-window-line-top-bottom");
        assert_eq!(
            e.current_buffer().line_of(e.current_buffer().point()),
            top + 3
        );
    }

    #[test]
    fn killing_a_buffer_and_its_window_does_both() {
        let (mut d, mut e) = setup();
        let keep = e.buffers.create_with_text("keep", "keep\n");
        e.switch_to_buffer(keep).unwrap();
        let doomed = e.buffers.create_with_text("doomed", "gone\n");
        e.switch_to_buffer(doomed).unwrap();
        run(&mut d, &mut e, "split-window-below");
        assert_eq!(e.windows.len(), 2);

        run(&mut d, &mut e, "kill-buffer-and-window");

        assert_eq!(e.windows.len(), 1, "the window went too");
        assert!(e.buffers.get(doomed).is_none(), "the buffer was killed");
    }

    #[test]
    fn killing_the_buffer_of_the_only_window_keeps_the_window() {
        // There would be nothing left to draw into otherwise.
        let (mut d, mut e) = setup();
        let keep = e.buffers.create_with_text("keep", "keep\n");
        e.switch_to_buffer(keep).unwrap();
        let doomed = e.buffers.create_with_text("doomed", "gone\n");
        e.switch_to_buffer(doomed).unwrap();
        assert_eq!(e.windows.len(), 1);

        run(&mut d, &mut e, "kill-buffer-and-window");

        assert_eq!(e.windows.len(), 1);
        assert!(e.buffers.get(doomed).is_none());
    }

    #[test]
    fn other_window_cycles_and_wraps() {
        let (mut d, mut e) = setup();
        let first = e.windows.current_id();
        run(&mut d, &mut e, "split-window-below");
        run(&mut d, &mut e, "other-window");
        assert_ne!(e.windows.current_id(), first);
        run(&mut d, &mut e, "other-window");
        assert_eq!(e.windows.current_id(), first, "wrapped around");
    }

    #[test]
    fn other_window_with_one_window_says_there_is_none() {
        let (mut d, mut e) = setup();
        assert_eq!(fails(&mut d, &mut e, "other-window"), "No other window");
    }

    #[test]
    fn other_window_honours_a_negative_argument() {
        let (mut d, mut e) = setup();
        let first = e.windows.current_id();
        run(&mut d, &mut e, "split-window-below");
        e.prefix = Prefix::Numeric(-1);
        d.execute(&mut e, "other-window", None);
        assert_ne!(e.windows.current_id(), first, "moved the other way");
    }

    #[test]
    fn deleting_a_window_gives_its_space_back() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-below");
        run(&mut d, &mut e, "delete-window");
        assert_eq!(e.windows.len(), 1);
        assert_eq!(e.windows.current().rect, Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn the_last_window_cannot_be_deleted() {
        let (mut d, mut e) = setup();
        assert!(fails(&mut d, &mut e, "delete-window").contains("only window"));
    }

    #[test]
    fn delete_other_windows_keeps_just_the_selected_one() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-below");
        run(&mut d, &mut e, "split-window-right");
        assert_eq!(e.windows.len(), 3);
        let kept = e.windows.current_id();
        run(&mut d, &mut e, "delete-other-windows");
        assert_eq!(e.windows.len(), 1);
        assert_eq!(e.windows.current_id(), kept);
    }

    #[test]
    fn deleting_windows_forgets_a_tree_window_that_went_with_them() {
        let (mut d, mut e) = setup();
        let buffer = e.buffers.create("*treefile*");
        let tree = e.windows.add_side_window(buffer, 32);
        e.tree_window = Some(tree);

        run(&mut d, &mut e, "delete-other-windows");
        assert!(
            e.tree_window.is_none(),
            "the tree window is gone, so the record is too"
        );
    }

    #[test]
    fn deleting_the_tree_window_itself_clears_the_record() {
        let (mut d, mut e) = setup();
        let buffer = e.buffers.create("*treefile*");
        let tree = e.windows.add_side_window(buffer, 32);
        e.tree_window = Some(tree);
        e.select_window(tree);

        run(&mut d, &mut e, "delete-window");
        assert!(e.tree_window.is_none());
    }

    #[test]
    fn a_side_window_can_be_widened_and_narrowed() {
        let (mut d, mut e) = setup();
        let buffer = e.buffers.create("*treefile*");
        let tree = e.windows.add_side_window(buffer, 32);
        e.select_window(tree);

        e.prefix = Prefix::Numeric(8);
        d.execute(&mut e, "enlarge-window-horizontally", None);
        assert_eq!(e.windows.get(tree).unwrap().rect.width, 40);

        e.prefix = Prefix::Numeric(8);
        d.execute(&mut e, "shrink-window-horizontally", None);
        assert_eq!(e.windows.get(tree).unwrap().rect.width, 32);
    }

    #[test]
    fn a_side_window_has_a_minimum_width() {
        let (mut d, mut e) = setup();
        let buffer = e.buffers.create("*treefile*");
        let tree = e.windows.add_side_window(buffer, 32);
        e.select_window(tree);
        e.prefix = Prefix::Numeric(100);
        d.execute(&mut e, "shrink-window-horizontally", None);
        assert_eq!(e.windows.get(tree).unwrap().rect.width, 8);
    }

    #[test]
    fn resizing_an_ordinary_window_explains_why_it_cannot() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-right");
        assert!(fails(&mut d, &mut e, "enlarge-window-horizontally").contains("side windows"));
        assert!(fails(&mut d, &mut e, "enlarge-window").contains("layout"));
    }

    #[test]
    fn balancing_restores_an_even_layout() {
        let (mut d, mut e) = setup();
        run(&mut d, &mut e, "split-window-right");
        run(&mut d, &mut e, "balance-windows");
        let widths: Vec<u16> = e.windows.iter().map(|w| w.rect.width).collect();
        assert_eq!(widths, vec![40, 40]);
    }

    #[test]
    fn a_split_window_shows_the_same_buffer_at_the_same_place() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("test", "0123456789");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.set_point(6));

        run(&mut d, &mut e, "split-window-below");
        let other = e
            .windows
            .ids()
            .into_iter()
            .find(|w| *w != e.windows.current_id())
            .unwrap();
        let window = e.windows.get(other).unwrap();
        assert_eq!(window.buffer, id);
        assert_eq!(window.point, 6);
    }

    #[test]
    fn each_window_keeps_its_own_point_after_a_split() {
        let (mut d, mut e) = setup();
        let id = e.buffers.create_with_text("test", "0123456789");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.set_point(2));
        run(&mut d, &mut e, "split-window-below");

        run(&mut d, &mut e, "other-window");
        e.with_current_buffer(|b| b.set_point(8));

        run(&mut d, &mut e, "other-window");
        e.sync_to_buffer();
        assert_eq!(
            e.current_buffer().point(),
            2,
            "the first window's point survived"
        );
    }
}
