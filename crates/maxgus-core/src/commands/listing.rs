//! The buffers that list places to go: `*Occur*` and `*xref*`.
//!
//! Both are a heading followed by one row per place, and the keys mean the
//! same in each — `n` and `p` walk the rows, `RET` goes where the row
//! points, `o` does so without leaving the list, `q` puts the list away.
//! The help buffer shares `q`, and nothing else: it has nowhere to go.

use std::path::PathBuf;

use crate::{
    CoreError, Result, command,
    command::{Args, Registry},
    editor::Editor,
    task::Task,
};
use maxgus_text::{BufferId, Range};

pub const OCCUR_MODE: &str = "occur-mode";
pub const XREF_MODE: &str = "xref-mode";
pub const HELP_MODE: &str = "help-mode";

/// Where a row of a listing points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub place: Place,
    /// Zero-based.
    pub line: usize,
    /// Zero-based, in characters; `None` means the start of the line.
    pub column: Option<usize>,
}

/// What a target is in: a buffer that is open, or a file that may not be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Buffer(BufferId),
    File(PathBuf),
}

/// One listing buffer's rows and what to mark in them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// One entry per line of the buffer; `None` for a heading or a blank.
    pub rows: Vec<Option<Target>>,
    /// The matches to highlight, as character ranges of the listing buffer.
    pub matches: Vec<Range>,
}

impl Listing {
    /// The target on `line`, if that row is one.
    pub fn target(&self, line: usize) -> Option<&Target> {
        self.rows.get(line).and_then(Option::as_ref)
    }

    /// The first row that points somewhere.
    pub fn first(&self) -> Option<usize> {
        self.rows.iter().position(Option::is_some)
    }

    /// The nearest row pointing somewhere after `line`, or before it.
    pub fn step(&self, line: usize, forward: bool) -> Option<usize> {
        match forward {
            true => (line + 1..self.rows.len()).find(|l| self.rows[*l].is_some()),
            false => (0..line.min(self.rows.len()))
                .rev()
                .find(|l| self.rows[*l].is_some()),
        }
    }
}

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "listing-visit",
            "Go where the row under point points.",
            visit,
            non_interactive
        ),
        command!(
            "listing-visit-other-window",
            "Go where the row under point points, staying in the list.",
            visit_other_window,
            non_interactive
        ),
        command!(
            "listing-next",
            "Move to the next row that points somewhere.",
            next,
            non_interactive
        ),
        command!(
            "listing-previous",
            "Move to the previous row that points somewhere.",
            previous,
            non_interactive
        ),
        command!(
            "listing-quit",
            "Close the listing, deleting its window when it was opened for it.",
            quit,
            non_interactive
        ),
        command!(
            "quit-window",
            "Put the buffer away, deleting its window when it was opened for it.",
            quit_window
        ),
    ]);
}

/// Shows `text` in the listing buffer called `name`, remembering what its
/// rows point at, with point on the first row that points anywhere.
pub fn show(editor: &mut Editor, name: &str, text: &str, listing: Listing) -> Result<()> {
    let id = match editor.buffers.find_by_name(name) {
        Some(id) => {
            editor.replace_buffer_contents(id, text)?;
            id
        }
        None => editor.buffers.create_with_text(name, text),
    };
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    let first = listing.first().unwrap_or(0);
    editor.listings.insert(name.to_string(), listing);
    editor.pop_to_buffer(id)?;
    editor.move_point_in(id, first);
    Ok(())
}

/// The listing the selected buffer is.
fn listing(editor: &Editor) -> Result<&Listing> {
    editor
        .listings
        .get(editor.current_buffer().name())
        .ok_or_else(|| CoreError::Message("Not in a listing".into()))
}

fn current_line(editor: &Editor) -> usize {
    editor
        .current_buffer()
        .line_of(editor.windows.current().point)
}

fn open(editor: &mut Editor, other_window: bool) -> Result<()> {
    let line = current_line(editor);
    let target = listing(editor)?
        .target(line)
        .cloned()
        .ok_or_else(|| CoreError::Message("Nothing here to go to".into()))?;
    let list = editor.windows.current_id();
    let id = match &target.place {
        Place::Buffer(id) => editor.buffers.get(*id).map(|_| *id),
        Place::File(path) => editor.buffers.find_by_path(path),
    };
    let Some(id) = id else {
        let Place::File(path) = target.place else {
            return Err(CoreError::Message("That buffer is gone".into()));
        };
        editor.pending_line = Some((path.clone(), target.line));
        editor.spawn(Task::ReadFile {
            path,
            reverting: None,
            other_window: true,
        });
        return Ok(());
    };
    // The place is shown in the editing window, which is the one the list
    // was popped up beside — never in the list's own window, which would
    // leave the list nowhere.
    if let Some(window) = editor
        .windows
        .ids()
        .into_iter()
        .find(|w| *w != list && editor.windows.get(*w).is_some_and(|w| w.buffer == id))
        .or_else(|| editor.editing_window().filter(|w| *w != list))
    {
        editor.select_window(window);
    }
    editor.switch_to_buffer(id)?;
    editor.go_to_line(target.line);
    if let Some(column) = target.column {
        let offset = {
            let buffer = editor.current_buffer();
            let start = buffer.line_start(target.line.min(buffer.len_lines().saturating_sub(1)));
            let end = maxgus_text::Motion::line_end(buffer.rope(), start);
            (start + column).min(end)
        };
        editor.windows.current_mut().point = offset;
        editor.with_current_buffer(move |b| b.set_point(offset));
        editor.follow_point();
    }
    if other_window {
        editor.select_window(list);
    }
    Ok(())
}

fn visit(editor: &mut Editor, _: &Args) -> Result<()> {
    open(editor, false)
}

fn visit_other_window(editor: &mut Editor, _: &Args) -> Result<()> {
    open(editor, true)
}

fn step(editor: &mut Editor, forward: bool) -> Result<()> {
    let line = current_line(editor);
    let next = listing(editor)?
        .step(line, forward)
        .ok_or_else(|| CoreError::Message("No further rows".into()))?;
    let id = editor.current_buffer_id();
    editor.move_point_in(id, next);
    Ok(())
}

fn next(editor: &mut Editor, _: &Args) -> Result<()> {
    step(editor, true)
}

fn previous(editor: &mut Editor, _: &Args) -> Result<()> {
    step(editor, false)
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let name = editor.current_buffer().name().to_string();
    editor.listings.remove(&name);
    editor.quit_window(true);
    Ok(())
}

fn quit_window(editor: &mut Editor, _: &Args) -> Result<()> {
    editor.quit_window(false);
    Ok(())
}

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
        let id = editor
            .buffers
            .create_with_text("main.rs", "one\ntwo\nthree\nfour\n");
        editor.switch_to_buffer(id).unwrap();
        let registry = crate::commands::standard_registry();
        (Dispatcher::new(registry), editor)
    }

    fn listing_for(id: BufferId) -> Listing {
        let target = |line| {
            Some(Target {
                place: Place::Buffer(id),
                line,
                column: Some(1),
            })
        };
        Listing {
            rows: vec![None, target(1), target(3)],
            matches: vec![Range::new(20, 23)],
        }
    }

    #[test]
    fn a_listing_pops_up_beside_the_buffer_and_starts_on_its_first_row() {
        let (_, mut e) = setup();
        let id = e.current_buffer_id();
        show(
            &mut e,
            "*Occur*",
            "heading\n  2: two\n  4: four\n",
            listing_for(id),
        )
        .unwrap();
        assert_eq!(e.windows.len(), 2, "split off a window");
        assert_eq!(e.current_buffer().name(), "*Occur*");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 1);
        assert!(e.current_buffer().is_read_only());
    }

    #[test]
    fn the_keys_walk_the_rows_and_go_where_they_point() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        show(
            &mut e,
            "*Occur*",
            "heading\n  2: two\n  4: four\n",
            listing_for(id),
        )
        .unwrap();
        d.handle_keys(&mut e, "n");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 2);
        assert!(matches!(
            d.handle_keys(&mut e, "n"),
            Dispatch::Failed { .. }
        ));
        d.handle_keys(&mut e, "p");
        assert_eq!(e.current_buffer().line_of(e.windows.current().point), 1);

        d.handle_keys(&mut e, "o");
        assert_eq!(
            e.current_buffer().name(),
            "*Occur*",
            "`o` stays in the list"
        );
        let other = e.windows.iter().find(|w| w.buffer == id).unwrap();
        assert_eq!(other.point, 5, "line two, column one");

        d.handle_keys(&mut e, "RET");
        assert_eq!(e.current_buffer().name(), "main.rs", "`RET` goes there");
        assert_eq!(e.current_buffer().point(), 5);
        assert_eq!(e.windows.len(), 2, "the list is still there");
    }

    #[test]
    fn quitting_takes_the_popped_window_and_the_listing_away() {
        let (mut d, mut e) = setup();
        let id = e.current_buffer_id();
        show(&mut e, "*Occur*", "heading\n  2: two\n", listing_for(id)).unwrap();
        d.handle_keys(&mut e, "q");
        assert_eq!(e.windows.len(), 1);
        assert_eq!(e.current_buffer().name(), "main.rs");
        assert!(e.buffers.find_by_name("*Occur*").is_none());
        assert!(e.listings.is_empty());
    }

    #[test]
    fn a_second_editing_window_is_reused_rather_than_split() {
        let (_, mut e) = setup();
        let id = e.current_buffer_id();
        e.split_window(crate::window::Direction::Horizontal)
            .unwrap();
        show(&mut e, "*xref*", "heading\n  2: two\n", listing_for(id)).unwrap();
        assert_eq!(e.windows.len(), 2);
        assert_eq!(e.current_buffer().name(), "*xref*");
        // Quitting a window that was not split off buries the buffer instead.
        e.quit_window(true);
        assert_eq!(e.windows.len(), 2);
        assert_eq!(e.current_buffer().name(), "main.rs");
    }
}
