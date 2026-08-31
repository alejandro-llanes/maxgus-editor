//! End-to-end sessions.
//!
//! These drive the editor the way a person does — key sequences in, rendered
//! screen out — so they cover the whole path from the keymap through dispatch
//! and the commands to redisplay. A unit test can confirm `kill-line` removes
//! the right characters; only this level confirms that pressing `C-k` does.

use maxgus_config::Settings;
use maxgus_core::{Dispatcher, Editor, MinibufferKind, Task};
use maxgus_faces::defaults;
use maxgus_tui::{Rect, Size, Surface};

/// A terminal-sized editor with the real keymap and the real command set.
struct Session {
    editor: Editor,
    dispatcher: Dispatcher,
    surface: Surface,
}

impl Session {
    fn new(width: u16, height: u16) -> Session {
        Session::configured(Settings::default(), width, height)
    }

    /// A session started from settings the test chose, which is the only way
    /// to cover anything read out of the configuration at startup.
    fn configured(settings: Settings, width: u16, height: u16) -> Session {
        let frame = Rect::new(0, 0, width, height);
        let mut editor = Editor::new(settings, defaults::builtin("maxgus-dark").unwrap(), frame);
        let registry = maxgus_core::standard_registry();
        editor.command_names = registry.interactive_names();
        editor.command_docs = registry
            .iter()
            .map(|c| (c.name.to_string(), c.doc.to_string()))
            .collect();
        Session {
            editor,
            dispatcher: Dispatcher::new(registry),
            surface: Surface::new(Size::new(width, height)),
        }
    }

    /// A session editing `text` in a buffer visiting `path`.
    fn editing(path: &str, text: &str) -> Session {
        let mut session = Session::new(60, 10);
        let id = session.editor.buffers.visit_file(path, text);
        session.editor.switch_to_buffer(id).unwrap();
        session.editor.with_current_buffer(|b| b.set_point(0));
        session.editor.tasks.drain();
        session
    }

    /// Presses each key in a whitespace-separated description.
    fn keys(&mut self, description: &str) -> &mut Session {
        for key in description.split_whitespace() {
            self.dispatcher.handle_keys(&mut self.editor, key);
        }
        self
    }

    /// Types `text` one self-inserting character at a time.
    fn type_text(&mut self, text: &str) -> &mut Session {
        for c in text.chars() {
            let key = if c == ' ' {
                "SPC".to_string()
            } else {
                c.to_string()
            };
            self.dispatcher.handle_keys(&mut self.editor, &key);
        }
        self
    }

    fn text(&self) -> String {
        self.editor.current_buffer().text()
    }

    fn point(&self) -> usize {
        self.editor.windows.current().point
    }

    /// The screen as the user would see it, with trailing blanks trimmed.
    fn screen(&mut self) -> Vec<String> {
        maxgus_core::draw(&self.editor, &mut self.surface);
        self.surface
            .to_lines()
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect()
    }

    fn echo(&mut self) -> String {
        self.screen().last().cloned().unwrap_or_default()
    }

    /// The face one cell is drawn in, after a fresh redisplay.
    fn face_at(&mut self, x: u16, y: u16) -> maxgus_faces::Face {
        maxgus_core::draw(&self.editor, &mut self.surface);
        self.surface.get(x, y).map(|c| c.face).unwrap_or_default()
    }

    fn mode_line(&mut self) -> String {
        let screen = self.screen();
        screen[screen.len() - 2].clone()
    }
}

#[test]
fn typing_and_moving_around_behaves_as_emacs_does() {
    let mut s = Session::editing("/project/notes.txt", "");
    s.type_text("hello world");
    assert_eq!(s.text(), "hello world");

    // `C-a` to the start, `M-f` forward a word, `C-e` to the end.
    s.keys("C-a");
    assert_eq!(s.point(), 0);
    s.keys("M-f");
    assert_eq!(s.point(), 5);
    s.keys("C-e");
    assert_eq!(s.point(), 11);

    // The text is on screen where it was typed.
    assert_eq!(s.screen()[0], "hello world");
}

#[test]
fn a_kill_and_a_yank_move_text_around() {
    let mut s = Session::editing("/project/notes.txt", "first line\nsecond line\n");

    // Kill the first line and its newline, then yank it back at the end.
    s.keys("C-k C-k");
    assert_eq!(s.text(), "second line\n");

    s.keys("M->").keys("C-y");
    assert_eq!(s.text(), "second line\nfirst line\n");
    // Both kills collected into one ring entry.
    assert_eq!(s.editor.kill_ring.len(), 1);
}

#[test]
fn the_region_can_be_marked_cut_and_pasted() {
    let mut s = Session::editing("/project/notes.txt", "alpha beta gamma");

    // Mark the first word, cut it, move to the end, paste it back.
    s.keys("C-SPC M-f C-w");
    assert_eq!(s.text(), " beta gamma");
    s.keys("M-> C-y");
    assert_eq!(s.text(), " beta gammaalpha");
}

#[test]
fn undo_walks_back_through_a_session() {
    let mut s = Session::editing("/project/notes.txt", "");
    s.type_text("one");
    s.keys("RET");
    s.type_text("two");
    assert_eq!(s.text(), "one\ntwo");

    s.keys("C-/");
    assert_eq!(s.text(), "one\n", "the second word undid as one step");
    s.keys("C-/");
    assert_eq!(s.text(), "one");
    s.keys("C-/");
    assert_eq!(s.text(), "");
}

#[test]
fn a_prefix_argument_repeats_a_command() {
    let mut s = Session::editing("/project/notes.txt", "");
    // `C-u 5 -` inserts five hyphens.
    s.keys("C-u M-5").type_text("-");
    assert_eq!(s.text(), "-----");

    // `M-3 C-b` moves back three.
    s.keys("M-3 C-b");
    assert_eq!(s.point(), 2);
}

#[test]
fn an_incremental_search_finds_and_leaves_point_at_the_match() {
    let mut s = Session::editing("/project/notes.txt", "alpha\nbeta\ngamma\n");

    s.keys("C-s");
    s.type_text("gam");
    assert!(s.echo().starts_with("I-search: gam"), "got `{}`", s.echo());
    s.keys("RET");
    assert_eq!(s.point(), 14, "just past `gam`");

    // The search string is remembered for the next one.
    s.keys("M-<").keys("C-s C-s");
    assert_eq!(s.point(), 14, "found `gam` again from the top");
}

#[test]
fn a_failing_search_says_so_and_going_back_recovers() {
    let mut s = Session::editing("/project/notes.txt", "alpha beta");
    s.keys("C-s");
    s.type_text("alz");
    assert!(
        s.echo().starts_with("failing I-search"),
        "got `{}`",
        s.echo()
    );
    s.keys("DEL");
    assert!(s.echo().starts_with("I-search: al"), "got `{}`", s.echo());
    s.keys("C-g");
    assert_eq!(s.point(), 0, "point went back where the search began");
}

#[test]
fn m_x_runs_a_command_by_name() {
    let mut s = Session::editing("/project/notes.txt", "hello world");
    s.keys("M-x");
    s.type_text("upcase-region");
    // Without a region it refuses, and says why.
    s.editor.with_current_buffer(|b| {
        b.set_point(0);
        b.set_mark(0);
        b.set_point(5);
    });
    s.keys("RET");
    assert_eq!(s.text(), "HELLO world");
}

#[test]
fn a_key_sequence_in_progress_is_echoed() {
    let mut s = Session::editing("/project/notes.txt", "text");
    // The loop echoes the pending sequence; here it is set the same way.
    s.keys("C-x");
    s.editor.pending_keys = Some("C-x".into());
    assert_eq!(s.echo(), "C-x");
    s.keys("C-s");
    s.editor.pending_keys = None;
    assert!(s.echo().is_empty() || !s.echo().starts_with("C-x"));
}

#[test]
fn splitting_and_moving_between_windows_works_from_the_keyboard() {
    let mut s = Session::editing("/project/notes.txt", "one\ntwo\nthree\n");
    let first = s.editor.windows.current_id();

    s.keys("C-x 2");
    assert_eq!(s.editor.windows.len(), 2);
    s.keys("C-x o");
    assert_ne!(s.editor.windows.current_id(), first);

    // Each window keeps its own position.
    s.keys("M->");
    s.keys("C-x o");
    assert_eq!(s.point(), 0, "the first window did not move");

    s.keys("C-x 1");
    assert_eq!(s.editor.windows.len(), 1);
}

#[test]
fn the_mode_line_tracks_the_buffer_state() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor.with_current_buffer(|b| b.mark_saved());
    assert!(
        s.mode_line().contains(maxgus_core::icons::SAVED),
        "unmodified, got `{}`",
        s.mode_line()
    );
    assert!(s.mode_line().contains("main.rs"));
    assert!(s.mode_line().contains("rust"));

    s.type_text("x");
    assert!(
        s.mode_line().contains(maxgus_core::icons::MODIFIED),
        "modified, got `{}`",
        s.mode_line()
    );
}

#[test]
fn saving_queues_a_write_and_the_result_clears_the_flag() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor.with_current_buffer(|b| b.mark_saved());
    s.type_text("// note\n");
    assert!(s.editor.current_buffer().is_modified());

    s.keys("C-x C-s");
    let tasks = s.editor.tasks.drain();
    let Some(Task::WriteFile {
        path,
        contents,
        buffer,
        ..
    }) = tasks.into_iter().next()
    else {
        panic!("no write was queued");
    };
    assert_eq!(path, std::path::Path::new("/project/main.rs"));
    assert!(contents.starts_with("// note"));

    s.editor
        .apply_task_result(maxgus_core::TaskResult::FileWritten {
            path,
            buffer,
            bytes: 21,
            disk_time: None,
        })
        .unwrap();
    assert!(!s.editor.current_buffer().is_modified());
    assert!(s.echo().contains("Wrote"), "got `{}`", s.echo());
}

#[test]
fn find_file_prompts_and_queues_a_read() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("C-x C-f");
    assert_eq!(s.editor.minibuffer.kind(), Some(MinibufferKind::File));
    assert_eq!(s.editor.minibuffer.input(), "/project/");
    assert!(
        s.echo().starts_with("Find file: /project/"),
        "got `{}`",
        s.echo()
    );

    s.editor.tasks.drain();
    s.type_text("other.rs");
    s.keys("RET");
    let tasks = s.editor.tasks.drain();
    assert!(
        tasks
            .iter()
            .any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("other.rs"))),
        "got {tasks:?}"
    );
}

#[test]
fn switching_buffers_from_the_keyboard_completes_on_tab() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor.buffers.create("scratch-notes");

    s.keys("C-x b");
    s.type_text("scr");
    s.keys("TAB");
    assert_eq!(s.editor.minibuffer.input(), "scratch-notes");
    s.keys("RET");
    assert_eq!(s.editor.current_buffer().name(), "scratch-notes");
}

#[test]
fn quitting_with_unsaved_work_refuses_and_says_which_buffer() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor.with_current_buffer(|b| b.mark_saved());
    s.type_text("x");

    s.keys("C-x C-c");
    assert!(!s.editor.quit);
    assert!(s.echo().contains("main.rs"), "got `{}`", s.echo());

    // `C-u C-x C-c` leaves anyway.
    s.keys("C-u C-x C-c");
    assert!(s.editor.quit);
}

#[test]
fn the_file_tree_opens_beside_the_buffer_and_takes_the_keyboard() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("C-x t t");
    let tree = s.editor.tree_window.expect("the tree opened");
    // The panel is a column: the tree and the buffer list, since no language
    // server is running to give an outline. Plus the window being edited.
    assert_eq!(s.editor.panel_windows.len(), 2);
    assert_eq!(s.editor.windows.len(), 3);

    // A snapshot arrives from the executor.
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TreeUpdated {
            nodes: vec![
                maxgus_tree::VisibleNode {
                    path: "/project".into(),
                    name: "project".into(),
                    kind: maxgus_tree::NodeKind::Directory,
                    depth: 0,
                    expanded: true,
                    expandable: true,
                    git: None,
                    is_root: true,
                },
                maxgus_tree::VisibleNode {
                    path: "/project/main.rs".into(),
                    name: "main.rs".into(),
                    kind: maxgus_tree::NodeKind::File,
                    depth: 1,
                    expanded: false,
                    expandable: false,
                    git: None,
                    is_root: false,
                },
            ],
            select: None,
            show_hidden: false,
        })
        .unwrap();

    // The tree is drawn on the left, the buffer beside it. Each panel window
    // holds one section and nothing else, so the tree's first row is a node.
    let screen = s.screen();
    // A column at the left is kept for the selection mark, then the open
    // chevron, then the open-directory glyph, then the name — in that
    // order, which is what makes the row read left to right.
    let chevron = screen[0]
        .find(maxgus_core::icons::CHEVRON_DOWN)
        .unwrap_or_else(|| panic!("no open chevron in `{}`", screen[0]));
    let folder = screen[0]
        .find(maxgus_core::icons::DIRECTORY_OPEN)
        .unwrap_or_else(|| panic!("no open-directory glyph in `{}`", screen[0]));
    assert!(
        chevron < folder,
        "the chevron comes after the glyph: `{}`",
        screen[0]
    );
    assert!(screen[0].contains("project"), "got `{}`", screen[0]);
    assert!(screen[0].contains("fn main()"), "got `{}`", screen[0]);

    // With the tree selected, `n` moves down it rather than inserting.
    s.editor.select_window(tree);
    s.keys("n");
    assert_eq!(s.editor.tree_selection().unwrap().name, "main.rs");
    // The tree's buffer is the tree and nothing else: one line per node,
    // which is what lets its commands index straight into the snapshot.
    // Depth one indents by two and a file has no arrow, so four spaces.
    assert_eq!(s.editor.current_buffer().text(), "v project\n    main.rs\n");

    s.keys("q");
    assert!(s.editor.tree_window.is_none());
    assert!(s.editor.panel_windows.is_empty());
    assert_eq!(s.editor.windows.len(), 1);
}

#[cfg(feature = "full")]
#[test]
fn syntax_highlighting_reaches_the_screen() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    // Opening the file asked for highlighting; the answer comes back.
    let id = s.editor.current_buffer_id();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::Reparsed {
            buffer: id,
            revision: s.editor.current_buffer().revision(),
            range: 0..usize::MAX,
            highlights: vec![maxgus_syntax::Highlight::new(0, 2, "font-lock-keyword")],
        })
        .unwrap();

    maxgus_core::draw(&s.editor, &mut s.surface);
    let keyword = s.editor.theme.resolve("font-lock-keyword");
    assert_eq!(
        s.surface.get(0, 0).unwrap().face,
        keyword,
        "`fn` is a keyword"
    );
    assert_ne!(s.surface.get(3, 0).unwrap().face, keyword, "`main` is not");
}

#[cfg(feature = "full")]
#[test]
fn a_diagnostic_is_underlined_and_counted_in_the_mode_line() {
    let mut s = Session::editing("/project/main.rs", "let unused = 1;\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::Diagnostics {
            uri: maxgus_lsp::client::path_to_uri(std::path::Path::new("/project/main.rs")),
            diagnostics: vec![maxgus_lsp::Diagnostic::new(
                maxgus_lsp::LspRange::new(
                    maxgus_lsp::LspPosition::new(0, 4),
                    maxgus_lsp::LspPosition::new(0, 10),
                ),
                maxgus_lsp::Severity::Warning,
                "unused variable",
            )],
        })
        .unwrap();

    maxgus_core::draw(&s.editor, &mut s.surface);
    assert_eq!(
        s.surface.get(4, 0).unwrap().face.attributes.underline,
        Some(true)
    );
    assert!(
        s.mode_line().contains(maxgus_core::icons::WARNING),
        "got `{}`",
        s.mode_line()
    );

    // `M-g n` jumps to it and reports what it says.
    s.keys("M-g n");
    assert_eq!(s.point(), 4);
    assert!(s.echo().contains("unused variable"), "got `{}`", s.echo());
}

#[cfg(feature = "full")]
#[test]
fn editing_after_opening_a_file_is_reported_to_the_language_server() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    let id = s.editor.current_buffer_id();
    s.editor.request_language_server(id);
    s.editor.tasks.drain();

    s.type_text("x");
    assert!(
        s.editor.sync_language_server(id),
        "the change is worth reporting"
    );
    let tasks = s.editor.tasks.drain();
    assert!(
        tasks.iter().any(|t| matches!(t, Task::LspDidChange { .. })),
        "the server was not told, got {tasks:?}"
    );
}

#[test]
fn a_query_replace_runs_to_completion_from_the_keyboard() {
    let mut s = Session::editing("/project/notes.txt", "one two one two one");
    s.keys("M-%");
    s.type_text("one");
    s.keys("RET");
    s.type_text("1");
    s.keys("RET");

    // Answer the first, skip the second, take the rest.
    s.keys("y");
    s.keys("n");
    s.keys("!");
    assert_eq!(s.text(), "1 two one two 1");
    assert!(s.echo().contains("Replaced"), "got `{}`", s.echo());
}

#[test]
fn help_describes_a_key_the_user_pressed() {
    let mut s = Session::editing("/project/notes.txt", "text");
    s.keys("C-h k");
    s.keys("C-e");
    assert_eq!(s.editor.current_buffer().name(), "*Help*");
    assert!(s.text().contains("move-end-of-line"), "got `{}`", s.text());
}

#[test]
fn a_keyboard_macro_records_and_replays() {
    let mut s = Session::editing("/project/notes.txt", "a\nb\nc\n");

    // Record: go to the start of the line, insert `- `, go down.
    s.keys("C-x (");
    s.keys("C-a");
    s.type_text("- ");
    s.keys("C-n");
    s.keys("C-x )");
    assert_eq!(s.text(), "- a\nb\nc\n");

    // Replay it by hand the way the loop does.
    let keys = s.editor.last_macro.clone();
    assert!(!keys.is_empty());
    for _ in 0..2 {
        for key in &keys {
            s.dispatcher.handle_key(&mut s.editor, *key);
        }
    }
    assert_eq!(s.text(), "- a\n- b\n- c\n");
}

#[test]
fn an_unbound_key_is_reported_rather_than_ignored() {
    let mut s = Session::editing("/project/notes.txt", "text");
    let outcome = s.dispatcher.handle_keys(&mut s.editor, "<f12>");
    assert!(
        matches!(outcome, maxgus_core::Dispatch::Undefined { .. }),
        "got {outcome:?}"
    );
    assert_eq!(s.text(), "text", "and it did not insert anything");
}

#[test]
fn a_narrow_terminal_still_renders_without_panicking() {
    let mut s = Session::new(20, 3);
    let id = s.editor.buffers.create_with_text("test", &"x".repeat(200));
    s.editor.switch_to_buffer(id).unwrap();
    let screen = s.screen();
    assert_eq!(screen.len(), 3);
    assert_eq!(screen[0], "x".repeat(20));
}

#[cfg(feature = "full")]
/// The README's key table is documentation people rely on; a binding that is
/// renamed or removed should break this rather than quietly mislead.
#[test]
fn every_key_the_readme_documents_is_really_bound() {
    let readme = include_str!("../../../README.md");
    let section = readme
        .split("## Keys worth knowing")
        .nth(1)
        .expect("the README has a key table");
    // Only that section: everything after it would sweep in the crate table
    // further down, whose first column is backticked crate names.
    let table = section.split("\n## ").next().expect("a section body");

    let session = Session::new(80, 24);
    let mut checked = 0usize;

    for row in table.lines().filter(|l| l.starts_with('|')) {
        let Some(keys_cell) = row.split('|').nth(1) else {
            continue;
        };
        // Each cell holds one or more key sequences in backticks.
        for description in keys_cell.split('`').skip(1).step_by(2) {
            let description = description.trim();
            if description.is_empty() || description == "Key" {
                continue;
            }
            // The tree's own keys are documented in prose, not this table.
            if description == "?" {
                continue;
            }
            let sequence = maxgus_keys::KeySequence::parse(description).unwrap_or_else(|e| {
                panic!("the README documents `{description}`, which is not a key: {e}")
            });
            let lookup = session.editor.keymaps.lookup(&sequence);
            assert!(
                !lookup.is_undefined(),
                "the README documents `{description}`, which is bound to nothing"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 15,
        "only {checked} keys were checked; is the table still there?"
    );
}

/// The commands the README names in its feature list must exist.
///
/// The README describes the whole editor, so this is a claim about the full
/// build: a minimal one does not have `lsp-format-buffer` and does not
/// pretend to.
#[cfg(feature = "full")]
#[test]
fn every_command_the_readme_names_is_registered() {
    let readme = include_str!("../../../README.md");
    let registry = maxgus_core::standard_registry();
    let mut checked = 0;
    let mut check = |word: &str| {
        if word.starts_with("lsp-") || word.starts_with("treefile-") {
            assert!(
                registry.contains(word),
                "the README names `{word}`, which is not a command"
            );
            checked += 1;
        }
    };
    for line in readme.lines() {
        // Names in the configuration examples, which are quoted because that
        // is how a `bind` names a command.
        for quoted in line.split('"').skip(1).step_by(2) {
            check(quoted);
        }
        // And names written in prose, in backticks. A console transcript is
        // neither of those, which is why it is not scanned: a line of one
        // can begin with `lsp-` without naming a command.
        for span in line.split('`').skip(1).step_by(2) {
            check(span);
        }
    }
    // The README names two, both `lsp-format-buffer`: one in the scripting
    // example and one in the configuration example. The floor is there to
    // catch the scan breaking, not to require a number of mentions.
    assert!(
        checked >= 2,
        "only {checked} were checked; the scan has stopped finding them"
    );
}

#[test]
fn a_jump_to_the_end_of_the_buffer_can_be_undone_with_c_u_c_spc() {
    let mut s = Session::editing("/project/notes.txt", "alpha\nbeta\ngamma\ndelta\n");
    s.keys("C-n C-n");
    let before = s.point();
    assert_eq!(before, 11, "on the third line");

    s.keys("M->");
    assert_eq!(s.point(), 23, "at the end");

    // `C-u C-SPC` goes back to where the jump started.
    s.keys("C-u C-SPC");
    assert_eq!(s.point(), before, "the jump was undone");
}

#[test]
fn leaving_a_search_leaves_a_way_back_to_where_it_started() {
    let mut s = Session::editing("/project/notes.txt", "alpha\nbeta\ngamma\n");
    s.keys("C-n");
    let before = s.point();

    s.keys("C-s");
    s.type_text("gam");
    s.keys("RET");
    assert_ne!(s.point(), before, "the search moved point");

    s.keys("C-u C-SPC");
    assert_eq!(s.point(), before, "and it can be undone");
}

#[test]
fn jumping_to_a_register_can_be_undone_too() {
    let mut s = Session::editing("/project/notes.txt", "0123456789");
    s.keys("C-u M-6 C-f");
    assert_eq!(s.point(), 6);
    // `C-x r SPC a` records point in register `a`.
    s.keys("C-x r SPC").keys("a");

    s.keys("M-<");
    assert_eq!(s.point(), 0);
    // `C-x r j a` jumps there.
    s.keys("C-x r j").keys("a");
    assert_eq!(s.point(), 6);

    s.keys("C-u C-SPC");
    assert_eq!(s.point(), 0, "back to where the jump started");
}

/// Every setting the configuration accepts must actually do something.
///
/// A setting that parses and is stored but never read is worse than one that
/// does not exist: the documentation promises behaviour the editor does not
/// have. Two of them were exactly that before this test was written.
#[test]
fn no_setting_is_merely_decorative() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the workspace root");

    // Every source file, with its test modules taken out: a setting read only
    // by a test is still doing nothing for the user.
    let mut production = String::new();
    let mut stack = vec![workspace.join("crates")];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "settings.rs")
                && let Ok(source) = std::fs::read_to_string(&path)
            {
                production.push_str(&strip_test_modules(&source));
            }
        }
    }

    let mut decorative = Vec::new();
    for name in maxgus_config::settings::SETTING_NAMES {
        // The field is the setting name with hyphens turned into underscores.
        let field = name.replace('-', "_");
        if !production.contains(&format!("settings.{field}")) {
            decorative.push(name.to_string());
        }
    }
    // Only the `set` fields are checked here. The same trick was tried for
    // the tree block and for mode keymaps and does not work: `.width` matches
    // any rectangle, and a call site can be gutted while the name it is
    // spelled with survives. Those are covered by tests that press the keys
    // and look at the result instead, which is the only check that holds.
    assert!(
        decorative.is_empty(),
        "these are parsed but never read, so they do nothing: {decorative:?}"
    );
}

/// Source with `#[cfg(test)]` modules removed.
fn strip_test_modules(source: &str) -> String {
    let mut out = String::new();
    let mut in_test = false;
    let mut depth = 0i32;
    for line in source.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
            depth = 0;
            continue;
        }
        if in_test {
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if depth <= 0 && line.contains('}') {
                in_test = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Everything that must be true of the editor after any command.
fn assert_consistent(session: &Session, after: &str) {
    let editor = &session.editor;
    assert!(
        editor.windows.is_consistent(),
        "after `{after}` the window tree disagrees with the windows that exist"
    );
    assert!(
        !editor.buffers.is_empty(),
        "after `{after}` there are no buffers left"
    );
    assert!(
        editor.buffers.get(editor.current_buffer_id()).is_some(),
        "after `{after}` the selected window shows a buffer that is gone"
    );
    for window in editor.windows.iter() {
        let Some(buffer) = editor.buffers.get(window.buffer) else {
            panic!("after `{after}` a window shows a buffer that is gone");
        };
        assert!(
            window.point <= buffer.len_chars(),
            "after `{after}` point {} is past the end of `{}` ({} characters)",
            window.point,
            buffer.name(),
            buffer.len_chars()
        );
        assert!(
            window.top_line < buffer.len_lines().max(1),
            "after `{after}` the window is scrolled past the end of `{}`",
            buffer.name()
        );
    }
    for buffer in editor.buffers.iter() {
        let length = buffer.len_chars();
        if let Some(mark) = buffer.mark() {
            assert!(
                mark <= length,
                "after `{after}` the mark in `{}` is at {mark}, past its {length} characters",
                buffer.name()
            );
        }
        // The accessible portion has to be a range, inside the buffer.
        assert!(
            buffer.point_min() <= buffer.point_max(),
            "after `{after}` the accessible portion of `{}` runs backwards",
            buffer.name()
        );
        assert!(
            buffer.point_max() <= length,
            "after `{after}` the accessible portion of `{}` ends past its text",
            buffer.name()
        );
        // The buffer's own point obeys narrowing, which is what commands that
        // compare the two rely on.
        assert!(
            buffer.point() >= buffer.point_min() && buffer.point() <= buffer.point_max(),
            "after `{after}` point {} in `{}` is outside {}..{}",
            buffer.point(),
            buffer.name(),
            buffer.point_min(),
            buffer.point_max()
        );
    }
    // An empty kill ring entry would be yanked as nothing at all.
    assert!(
        editor.kill_ring.front().is_none_or(|text| !text.is_empty()),
        "after `{after}` the kill ring holds an empty entry"
    );
    // Highlight spans are drawn by byte offset; one past the end would be read
    // out of the buffer it describes.
    #[cfg(feature = "full")]
    for (buffer, (_, _, spans)) in &editor.highlights {
        let Some(length) = editor.buffers.get(*buffer).map(|b| b.text().len()) else {
            continue;
        };
        for span in spans {
            assert!(
                span.start <= span.end && span.end <= length,
                "after `{after}` a highlight span {}..{} does not fit its {length} bytes",
                span.start,
                span.end
            );
        }
    }
    // The tree cursor addresses a node that exists.
    if !editor.tree.is_empty() {
        assert!(
            editor.tree_cursor_line() < editor.tree.len(),
            "after `{after}` the tree cursor is past the last node"
        );
    }
}

/// Answers whatever prompt is open, so a command that asks a question is
/// actually carried out rather than left waiting.
///
/// Pressing the keys alone is not enough: `C-x k` only opens a prompt, and a
/// sweep that stops there never reaches the code that kills the buffer.
fn answer_any_prompt(session: &mut Session) {
    for _ in 0..4 {
        let Some(kind) = session.editor.minibuffer.kind() else {
            return;
        };
        match kind {
            // A single-key prompt takes the first plausible answer.
            MinibufferKind::Char => {
                session.keys("y");
            }
            MinibufferKind::YesNo => {
                session.type_text("yes");
                session.keys("RET");
            }
            // Everything else accepts its default, which is what a person
            // pressing RET would get.
            _ => {
                session.keys("RET");
            }
        }
    }
}

/// A session with something to work on: several lines, a region, a kill ring
/// entry and a mark, so commands have material rather than empty buffers.
fn furnished() -> Session {
    let mut session = Session::editing(
        "/project/main.rs",
        "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
    );
    session.editor.kill_ring.kill_new("something to yank");
    session.editor.with_current_buffer(|b| {
        b.set_point(0);
        b.set_mark(0);
        b.set_point(11);
    });
    session
}

/// A session on a file far taller than the window, so the scrolling paths are
/// actually reached: on a buffer that fits on screen they never run at all.
fn furnished_long() -> Session {
    let text: String = (0..500)
        .map(|n| format!("line {n} of a long file\n"))
        .collect();
    let mut session = Session::editing("/project/long.rs", &text);
    session.editor.kill_ring.kill_new("something to yank");
    session.editor.with_current_buffer(|b| {
        b.set_point(b.line_start(250));
        b.set_mark(b.line_start(250));
        b.set_point(b.line_start(260));
    });
    session.editor.follow_point();
    session
}

#[test]
fn pressing_any_binding_on_a_long_file_leaves_the_editor_consistent() {
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished_long();
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("on a long file: {keys} ({command})"));
    }
}

#[test]
fn scrolling_around_a_long_file_leaves_the_editor_consistent() {
    // Paging and recentring in combination are where a scroll position walks
    // off the end of the buffer.
    let sequences = [
        "C-v C-v C-v C-v C-v C-v C-v C-v",
        "M-v M-v M-v",
        "M-> C-v C-v",
        "M-< M-v M-v",
        "M-> C-l C-l C-l",
        "M-< C-l C-v M-v C-l",
        "C-u M-> C-v",
    ];
    for sequence in sequences {
        let mut session = furnished_long();
        session.keys(sequence);
        assert_consistent(&session, sequence);
        // And the window must still be showing the line point is on.
        let line = {
            let buffer = session.editor.current_buffer();
            buffer.line_of(session.point().min(buffer.len_chars()))
        };
        let window = session.editor.windows.current();
        assert!(
            window.shows_line(line) || window.text_height() == 0,
            "after `{sequence}` point is on line {line} but the window shows {}..{}",
            window.top_line,
            window.bottom_line()
        );
    }
}

#[test]
fn pressing_any_binding_leaves_the_editor_consistent() {
    // A command that leaves point past the end of a buffer, or a window
    // showing a buffer that has been killed, corrupts everything that comes
    // after it. Every binding is pressed here to make sure none of them can.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished();
        session.keys(keys);
        assert_consistent(&session, &format!("{keys} ({command})"));
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("{keys} ({command}), prompt answered"));
    }
}

#[test]
fn pressing_any_binding_twice_leaves_the_editor_consistent() {
    // Repeating is where the interesting failures are: the second `C-x C-x`
    // acts on what the first one left behind.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished();
        session.keys(keys);
        answer_any_prompt(&mut session);
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("{keys} {keys} ({command})"));
    }
}

#[test]
fn pressing_any_binding_at_the_end_of_the_buffer_leaves_it_consistent() {
    // The end of the buffer is where an off-by-one shows up.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished();
        session.keys("M->");
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("M-> {keys} ({command})"));
    }
}

#[test]
fn pressing_any_binding_in_an_empty_buffer_leaves_it_consistent() {
    // An empty buffer has no character to look at, no line to measure and no
    // word to move over: every assumption a command might make is false.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = Session::editing("/project/empty.rs", "");
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("in an empty buffer: {keys} ({command})"));
    }
}

#[test]
fn every_binding_can_be_pressed_with_a_prefix_argument() {
    // A prefix argument changes what most commands do, and a count is exactly
    // the kind of thing that walks off the end of a buffer.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished();
        session.keys("C-u");
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("C-u {keys} ({command})"));

        let mut session = furnished();
        session.keys("M--");
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("M-- {keys} ({command})"));
    }
}

#[test]
fn every_binding_can_be_pressed_while_the_tree_is_open() {
    // The tree takes a window and a keymap; commands that move between
    // windows or kill buffers have to cope with it being there.
    for (keys, command) in maxgus_core::keymap::GLOBAL_BINDINGS {
        let mut session = furnished();
        session.keys("C-x t t");
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("with the tree open: {keys} ({command})"));
    }
}

#[test]
fn every_tree_binding_leaves_the_editor_consistent() {
    for (keys, command) in maxgus_tree::TREEMACS_BINDINGS {
        let mut session = furnished();
        session.keys("C-x t t");
        // The tree has no snapshot yet, which is itself worth exercising.
        session.keys(keys);
        answer_any_prompt(&mut session);
        assert_consistent(&session, &format!("in the tree: {keys} ({command})"));
    }
}

/// Draws the editor and checks the result is fit to put on a terminal.
///
/// Redisplay is reached from every state the editor can be in, and a panic
/// there takes the whole program down. Drawing during the walk is the only way
/// to reach the states a hand-written render test never thinks of.
fn assert_draws_sanely(session: &mut Session, after: &str) {
    let lines = session.screen();
    let size = session.surface.size();
    assert_eq!(
        lines.len(),
        size.height as usize,
        "after `{after}` the frame changed height"
    );

    let (x, y) = session.editor.cursor_position();
    assert!(
        x < size.width && y < size.height,
        "after `{after}` the cursor is at ({x}, {y}), outside a {}x{} frame",
        size.width,
        size.height
    );
    // A row wider than the frame would wrap and shift everything below it.
    for (row, line) in lines.iter().enumerate() {
        assert!(
            line.chars().count() <= size.width as usize,
            "after `{after}` row {row} is {} columns wide in a {}-column frame",
            line.chars().count(),
            size.width
        );
    }
}

/// A reproducible pseudo-random source.
///
/// A fixed seed matters more than randomness here: a failure has to be
/// reproducible from the message alone, and a suite that fails on a different
/// run each time is worse than one that never fails.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*, which is plenty for choosing keys.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

/// Keys that are worth pressing between bindings: text, motion and answers.
const FILLER: &[&str] = &[
    "a", "b", "SPC", "RET", "x", "1", "y", "n", "q", "DEL", "TAB", "C-f", "C-b", "C-n", "C-p",
    "C-a", "C-e", "C-g", "M-f", "M-b",
];

/// Runs a pseudo-random walk over the keymap, checking after every step.
///
/// The single-key sweep cannot reach what happens when a search is left half
/// typed and a window is then deleted, or when a prompt is open and the buffer
/// it was about is killed. This can.
fn random_walk(seed: u64, steps: usize, session: &mut Session) {
    let bindings = maxgus_core::keymap::GLOBAL_BINDINGS;
    let mut rng = Rng(seed);
    let mut history: Vec<String> = Vec::new();

    for _ in 0..steps {
        // Mostly bindings, sometimes ordinary typing.
        let keys = if rng.below(4) == 0 {
            FILLER[rng.below(FILLER.len())].to_string()
        } else {
            bindings[rng.below(bindings.len())].0.to_string()
        };
        history.push(keys.clone());
        session.keys(&keys);

        // `C-x C-c` would end the walk early; carry on instead.
        session.editor.quit = false;

        let context = format!("seed {seed}, keys: {}", history.join(" "));
        assert_consistent(session, &context);
        assert_draws_sanely(session, &context);
    }
}

#[test]
#[ignore = "exploratory: `cargo test -- --ignored` when hunting for new ones"]
fn a_long_random_exploration() {
    // Deliberately larger than the suite should pay for on every run. This is
    // the net that caught the stale window points, the mark that undo left
    // behind, and point stepping outside a narrowed region.
    for seed in 1..120u64 {
        let mut session = if seed % 3 == 0 {
            furnished_long()
        } else {
            furnished()
        };
        if seed % 4 == 0 {
            session.keys("C-x t t");
        }
        random_walk(seed.wrapping_mul(0x9e37_79b9), 600, &mut session);
    }
}

#[test]
fn a_random_walk_over_the_keymap_never_corrupts_the_editor() {
    for seed in [1, 2, 3, 5, 8, 13, 21, 34] {
        let mut session = furnished();
        random_walk(seed, 200, &mut session);
    }
}

#[test]
fn a_random_walk_on_a_long_file_never_corrupts_the_editor() {
    for seed in [7, 11, 42, 99] {
        let mut session = furnished_long();
        random_walk(seed, 200, &mut session);
    }
}

#[test]
fn a_random_walk_with_the_tree_open_never_corrupts_the_editor() {
    for seed in [4, 6, 9, 12] {
        let mut session = furnished();
        session.keys("C-x t t");
        random_walk(seed, 200, &mut session);
    }
}

/// Keys that change the buffer without leaving it or opening a prompt.
const EDITS: &[&str] = &[
    "a", "b", "SPC", "RET", "x", "1", "TAB", "C-o", "C-d", "DEL", "C-k", "C-w", "M-w", "C-y",
    "M-y", "C-j", "C-t", "M-t", "M-u", "M-l", "M-c", "M-d", "M-DEL", "C-M-k", "M-\\", "M-SPC",
    "M-^", "C-x C-o", "M-;", "M-q", "C-S-DEL", "C-x C-u", "C-x C-l", "C-x C-t",
];

/// Keys that move around without changing anything.
const MOVES: &[&str] = &[
    "C-f", "C-b", "C-n", "C-p", "C-a", "C-e", "M-f", "M-b", "M-<", "M->", "C-SPC", "C-x C-x",
    "M-m", "C-M-f", "C-M-b",
];

#[test]
#[ignore = "exploratory: `cargo test -- --ignored` when hunting for new ones"]
fn a_long_undo_exploration() {
    for seed in 1..300u64 {
        let mut session = Session::editing(
            "/project/main.rs",
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
        );
        session.editor.kill_ring.kill_new("yanked text");
        let original = session.text();
        let mut rng = Rng(seed.wrapping_mul(0xff51_afd7_ed55_8ccd) | 1);
        let mut history: Vec<String> = Vec::new();
        for _ in 0..120 {
            let keys = if rng.below(3) == 0 {
                MOVES[rng.below(MOVES.len())]
            } else {
                EDITS[rng.below(EDITS.len())]
            };
            history.push(keys.to_string());
            session.keys(keys);
        }
        for _ in 0..2000 {
            if !session.editor.current_buffer().can_undo() {
                break;
            }
            session.keys("C-/");
        }
        assert_eq!(
            session.text(),
            original,
            "seed {seed}: undo did not restore the buffer\n  keys: {}",
            history.join(" ")
        );
    }
}

#[test]
fn undoing_everything_restores_the_buffer_exactly() {
    // Undo's whole promise. A command that edits without recording what it
    // did, or records it wrongly, shows up here and almost nowhere else.
    for seed in 1..40u64 {
        let mut session = Session::editing(
            "/project/main.rs",
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
        );
        session.editor.kill_ring.kill_new("yanked text");
        let original = session.text();
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9) | 1);
        let mut history: Vec<String> = Vec::new();

        for _ in 0..60 {
            let keys = if rng.below(3) == 0 {
                MOVES[rng.below(MOVES.len())]
            } else {
                EDITS[rng.below(EDITS.len())]
            };
            history.push(keys.to_string());
            session.keys(keys);
            assert_consistent(&session, &format!("seed {seed}: {}", history.join(" ")));
        }

        // Undo until there is nothing left to undo.
        for _ in 0..500 {
            if !session.editor.current_buffer().can_undo() {
                break;
            }
            session.keys("C-/");
        }
        assert!(
            !session.editor.current_buffer().can_undo(),
            "seed {seed}: undo never ran out after {} edits",
            history.len()
        );
        assert_eq!(
            session.text(),
            original,
            "seed {seed}: undoing everything did not restore the buffer\n  keys: {}",
            history.join(" ")
        );
    }
}

#[test]
fn redoing_everything_puts_it_back() {
    for seed in 1..25u64 {
        let mut session = Session::editing("/project/main.rs", "one\ntwo\nthree\n");
        session.editor.kill_ring.kill_new("yanked");
        let mut rng = Rng(seed.wrapping_mul(0x517c_c1b7) | 1);

        for _ in 0..40 {
            let keys = if rng.below(3) == 0 {
                MOVES[rng.below(MOVES.len())]
            } else {
                EDITS[rng.below(EDITS.len())]
            };
            session.keys(keys);
        }
        let edited = session.text();

        for _ in 0..500 {
            if !session.editor.current_buffer().can_undo() {
                break;
            }
            session.keys("C-/");
        }
        for _ in 0..500 {
            if !session.editor.current_buffer().can_redo() {
                break;
            }
            session.keys("C-M-/");
        }
        assert_eq!(
            session.text(),
            edited,
            "seed {seed}: undoing and redoing everything did not come back"
        );
    }
}

#[test]
fn the_buffer_arrows_walk_the_buffer_list_from_the_keyboard() {
    // `next-buffer` and `previous-buffer` were implemented and registered but
    // had no key, so they could only be run through `M-x`. Pressing the keys
    // Emacs uses is the only thing that shows they are reachable now.
    let mut session = Session::editing("/project/first.rs", "one\n");
    let second = session
        .editor
        .buffers
        .visit_file("/project/second.rs", "two\n");
    session.editor.switch_to_buffer(second).unwrap();
    session.editor.tasks.drain();
    assert!(session.mode_line().contains("second.rs"));

    session.keys("C-x <right>");
    let after_next = session.mode_line();
    assert!(
        !after_next.contains("second.rs"),
        "the buffer did not change: `{after_next}`"
    );

    session.keys("C-x <left>");
    assert!(
        session.mode_line().contains("second.rs"),
        "going back did not return: `{}`",
        session.mode_line()
    );

    // Emacs binds the control-modified arrows to the same pair.
    session.keys("C-x C-<right>");
    assert_eq!(
        session.mode_line(),
        after_next,
        "`C-x C-<right>` is the same command"
    );
}

#[test]
fn comment_line_can_be_reached_from_its_key() {
    // Registered, tested, and unreachable without `M-x` until `C-x C-;` was
    // bound to it.
    let mut session = Session::editing("/project/main.rs", "let x = 1;\n");
    session.keys("C-x C-;");
    assert!(
        session.text().starts_with("//"),
        "the line was not commented: {:?}",
        session.text()
    );
}

// ---- prompts that list what is on offer ---------------------------------

#[test]
fn m_x_lists_the_commands_before_anything_is_typed() {
    let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
    session.keys("M-x");
    let shown = &session.editor.minibuffer.completion();
    assert!(shown.visible, "the list is not up");
    assert!(shown.len() > 100, "only {} commands offered", shown.len());
    assert!(shown.candidates.iter().any(|c| c == "save-buffer"));
}

#[test]
fn typing_narrows_the_command_list_and_deleting_widens_it_again() {
    let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
    session.keys("M-x");
    let everything = session.editor.minibuffer.completion().len();

    session.type_text("save-");
    let narrowed = session.editor.minibuffer.completion().candidates.clone();
    assert!(
        narrowed.len() < everything,
        "typing did not narrow anything"
    );
    assert!(
        narrowed.iter().all(|c| c.starts_with("save-")),
        "got {narrowed:?}"
    );
    assert!(narrowed.iter().any(|c| c == "save-buffer"));

    // Cleared rather than one DEL: every command beginning `save` also begins
    // `save-`, so dropping the hyphen alone narrows to the very same set.
    session.keys("C-a C-k");
    assert_eq!(session.editor.minibuffer.input(), "");
    assert_eq!(
        session.editor.minibuffer.completion().len(),
        everything,
        "clearing the input did not bring the whole list back"
    );
}

#[test]
fn c_x_b_lists_the_buffers_that_exist() {
    let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
    let other = session.editor.buffers.visit_file("/project/other.rs", "");
    session.editor.switch_to_buffer(other).unwrap();
    session.editor.tasks.drain();

    session.keys("C-x b");
    let shown = session.editor.minibuffer.completion();
    assert!(shown.visible, "the buffer list is not up");
    assert!(
        shown.candidates.iter().any(|c| c == "main.rs"),
        "got {:?}",
        shown.candidates
    );
    assert!(
        shown.candidates.iter().any(|c| c == "other.rs"),
        "got {:?}",
        shown.candidates
    );
}

#[test]
fn the_first_tab_still_completes_rather_than_cycling() {
    // The list is up from the moment the prompt opens, but TAB has always
    // grown the input to the common prefix first and only cycled afterwards.
    // Showing the list must not quietly change what TAB does.
    let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
    // `sa` is deliberate: the commands it matches share the prefix `save-`,
    // while the first of them is `save-buffer`. Completing and cycling give
    // different answers here, which is what makes the test able to tell them
    // apart — from `save-b` both would produce `save-buffer` and it would
    // pass either way.
    session.keys("M-x");
    session.type_text("sa");
    session.keys("TAB");
    assert_eq!(
        session.editor.minibuffer.input(),
        "save-",
        "the first TAB cycled to a candidate instead of completing"
    );
}

#[test]
fn every_prompt_edit_keeps_the_list_in_step_with_the_input() {
    // The re-filter is called from each editing command by hand, so a new one
    // could be added without it. Every key that changes the input is pressed
    // here, and the list has to match what is actually typed afterwards.
    let edits = [
        "a", "DEL", "C-k", "C-a", "C-e", "M-DEL", "C-d", "M-p", "M-n",
    ];
    for keys in edits {
        let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
        session.keys("M-x");
        session.type_text("save");
        session.keys(keys);

        let input = session.editor.minibuffer.input().to_string();
        let shown = session.editor.minibuffer.completion().candidates.clone();
        let expected = maxgus_core::fuzzy::matches(&input, session.editor.command_names.iter());
        assert_eq!(
            shown, expected,
            "after `{keys}` the list does not match `{input}`"
        );
    }
}

fn node(path: &str, name: &str, dir: bool, depth: usize, root: bool) -> maxgus_tree::VisibleNode {
    maxgus_tree::VisibleNode {
        path: path.into(),
        name: name.into(),
        kind: if dir {
            maxgus_tree::NodeKind::Directory
        } else {
            maxgus_tree::NodeKind::File
        },
        depth,
        expanded: dir,
        expandable: dir,
        git: None,
        is_root: root,
    }
}

/// A session with the tree open beside the file, and a snapshot delivered.
fn with_tree() -> Session {
    let mut s = Session::editing("/project/main.rs", "one\ntwo\nthree\nfour\nfive\n");
    s.keys("C-x t t");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TreeUpdated {
            nodes: vec![
                node("/project", "project", true, 0, true),
                node("/project/main.rs", "main.rs", false, 1, false),
                node("/project/other.rs", "other.rs", false, 1, false),
            ],
            select: None,
            show_hidden: false,
        })
        .unwrap();
    s.editor.tasks.drain();
    s
}

/// What `app.rs` does after every event when `tree-follow` is on.
fn follow_tree(s: &mut Session) {
    if !s.editor.tree_follow || s.editor.tree_window.is_none() {
        return;
    }
    if Some(s.editor.windows.current_id()) == s.editor.tree_window {
        return;
    }
    let Some(path) = s
        .editor
        .current_buffer()
        .path()
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    if s.editor.tree.iter().any(|n| n.path == path) {
        s.editor.select_tree_path(&path);
    }
}

#[test]
fn control_arrows_move_between_the_tree_and_the_code() {
    // The reason for them: `C-x o` cycles in whatever order the windows are
    // stored, so with a tree open you have to guess where you will land.
    let mut s = with_tree();
    let tree = s.editor.tree_window.expect("the tree opened");
    let code = s.editor.windows.current_id();
    assert_ne!(tree, code, "the tree opened beside the file, not over it");

    s.keys("C-<left>");
    assert!(
        s.editor
            .panel_windows
            .contains(&s.editor.windows.current_id()),
        "C-<left> did not reach the panel"
    );
    s.editor.select_window(tree);

    s.keys("C-<right>");
    assert_eq!(
        s.editor.windows.current_id(),
        code,
        "C-<right> did not come back"
    );

    // And repeating it does not wander off somewhere else.
    s.keys("C-<right>");
    assert_eq!(
        s.editor.windows.current_id(),
        code,
        "there is nothing to the right"
    );
}

#[test]
fn control_arrows_move_up_and_down_between_stacked_windows() {
    let mut s = Session::editing("/project/main.rs", "one\ntwo\nthree\n");
    s.keys("C-x 2");
    let top = s.editor.windows.current_id();

    s.keys("C-<down>");
    let bottom = s.editor.windows.current_id();
    assert_ne!(bottom, top, "C-<down> did not move");

    s.keys("C-<up>");
    assert_eq!(
        s.editor.windows.current_id(),
        top,
        "C-<up> did not come back"
    );
}

#[test]
fn moving_where_there_is_no_window_says_so_and_stays_put() {
    let mut s = Session::editing("/project/main.rs", "one\n");
    let only = s.editor.windows.current_id();
    for keys in ["C-<left>", "C-<right>", "C-<up>", "C-<down>"] {
        s.keys(keys);
        assert_eq!(
            s.editor.windows.current_id(),
            only,
            "`{keys}` moved somewhere"
        );
        assert!(
            s.editor.minibuffer.message_is_error(),
            "`{keys}` said nothing"
        );
    }
}

#[test]
fn moving_between_windows_keeps_each_one_where_it_was() {
    // The point of a window is that it remembers its own place.
    let mut s = with_tree();
    s.keys("C-n C-n");
    let code_line = s
        .editor
        .current_buffer()
        .line_of(s.editor.windows.current().point);

    s.keys("C-<left>");
    assert_eq!(s.editor.current_buffer().name(), "*treefile*");
    s.keys("C-<right>");
    assert_eq!(
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point),
        code_line,
        "coming back landed somewhere else"
    );
}

#[test]
fn tree_follow_mode_does_not_disturb_the_window_being_edited() {
    // `app.rs` runs this after every event when `tree-follow` is on, so it
    // runs between every keystroke a person types. If it moved the editing
    // window's point — or left it behind — the cursor would appear to stick,
    // which is exactly what a tree open is reported to feel like.
    let mut s = with_tree();
    assert!(s.editor.tree_follow, "follow mode is on by default");

    let mut points = Vec::new();
    for _ in 0..4 {
        s.keys("C-n");
        follow_tree(&mut s);
        points.push(s.editor.windows.current().point);
    }
    assert!(
        points.windows(2).all(|w| w[1] > w[0]),
        "point stopped moving while the tree was following: {points:?}"
    );
    assert_eq!(
        s.editor.current_buffer().name(),
        "main.rs",
        "focus left the file"
    );

    // And the tree did follow: its cursor sits on the file being edited.
    // Asked through `tree_selection`, because a panel line is no longer a
    // tree index — the section headings sit between them.
    assert_eq!(
        s.editor.tree_selection().map(|n| n.name.as_str()),
        Some("main.rs"),
        "the tree did not follow the file being edited"
    );
}

#[test]
fn the_arrow_keys_move_in_a_file_while_the_tree_is_open() {
    // The tree binds the arrows to its own commands. Applied everywhere they
    // take the arrows away from every other buffer — reported as "opening a
    // file from the tree, Left/Right/Up/Down do not work, but PgUp/PgDown do",
    // because the tree map has the arrows and not the paging keys.
    let mut s = with_tree();
    assert_eq!(
        s.editor.current_buffer().name(),
        "main.rs",
        "focus is in the file"
    );

    let start = s.editor.windows.current().point;
    s.keys("<down>");
    let down = s.editor.windows.current().point;
    assert!(down > start, "<down> did not move point in the file");

    s.keys("<right>");
    assert!(
        s.editor.windows.current().point > down,
        "<right> did not move point"
    );

    s.keys("<up>");
    assert!(
        s.editor.windows.current().point < down + 1,
        "<up> did not move point back"
    );
}

#[test]
fn the_arrow_keys_still_drive_the_tree_when_the_tree_is_selected() {
    // The other half: moving the tree's map out of every buffer must not take
    // it away from the tree.
    //
    // Asserted on `<right>`, not `<down>`: the global map binds `<down>` to
    // `next-line`, which moves point in the tree buffer too, so it cannot tell
    // the tree's binding from the fallback. `<right>` expands a directory
    // where the global one would move a character — only the tree map does
    // that, and only it queues the task.
    let mut s = with_tree();
    s.keys("C-<left>");
    assert_eq!(s.editor.current_buffer().name(), "*treefile*");
    s.editor.tasks.drain();

    // Onto the root, which is a directory and can be expanded.
    s.keys("M-<");
    s.editor.tasks.drain();
    s.keys("<right>");

    let queued = s.editor.tasks.drain();
    assert!(
        queued.iter().any(|t| matches!(
            t,
            maxgus_core::Task::Tree(maxgus_core::TreeAction::Expand(_))
        )),
        "`<right>` did not reach the tree's own binding: {queued:?}"
    );
}

#[test]
fn a_prefix_in_the_trees_map_is_not_stolen_by_self_insert() {
    // `o o`, `c f`, `y a`, `t h`, `g r` — most of the treemacs keymap is
    // multi-key. The global map binds any printable key to
    // `self-insert-command` as a fallback, and that fallback was winning over
    // a *prefix* held by a higher-precedence map, so `o` typed itself instead
    // of starting a sequence. In a read-only tree buffer that reads as the
    // editor refusing to edit rather than as a lost binding.
    let mut s = with_tree();
    s.keys("C-<left>");
    assert_eq!(s.editor.current_buffer().name(), "*treefile*");

    let out = s.dispatcher.handle_keys(&mut s.editor, "o");
    assert!(
        matches!(out, maxgus_core::Dispatch::Prefix { .. }),
        "`o` did not start a sequence in the tree: {out:?}"
    );
}

#[test]
fn the_trees_two_key_bindings_reach_their_commands() {
    let mut s = with_tree();
    s.keys("C-<left>");
    s.editor.tasks.drain();

    // `o o` visits the selected node in the other window.
    s.keys("M-<");
    s.editor.tasks.drain();
    let out = s.dispatcher.handle_keys(&mut s.editor, "o o");
    assert!(
        !matches!(out, maxgus_core::Dispatch::Undefined { .. }),
        "`o o` is undefined in the tree: {out:?}"
    );
    assert!(
        !s.editor.current_buffer().text().contains("oo"),
        "`o o` was typed into the tree buffer instead of being a binding"
    );
}

#[test]
fn the_cursor_sits_on_the_text_when_line_numbers_are_shown() {
    // The line-number column shifts the text right; the cursor has to move
    // with it or it sits in the gutter, three columns adrift of the character
    // it is meant to be on.
    let mut s = Session::editing("/project/main.rs", "one\ntwo\nthree\n");
    let without = s.editor.cursor_position();
    assert_eq!(without, (0, 0));

    s.editor.settings.line_numbers = true;
    let with = s.editor.cursor_position();
    assert!(
        with.0 > without.0,
        "the cursor did not move over for the line-number column: {with:?}"
    );

    // And it tracks the column from there.
    s.keys("C-f C-f");
    assert_eq!(
        s.editor.cursor_position(),
        (with.0 + 2, with.1),
        "the cursor lost the gutter offset once point moved"
    );
}

// ---- consulting themes --------------------------------------------------

fn with_two_themes() -> Session {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    let mut light = maxgus_config::ThemeSpec::new("daylight");
    light.base = Some("maxgus-light".into());
    light.faces.push(maxgus_config::FaceSpec {
        name: "region".into(),
        background: Some("#cceeff".into()),
        ..Default::default()
    });
    s.editor.theme_specs.push(light);
    s.editor.config_says_theme = Some("maxgus-dark".into());
    s.editor.config_path = Some("/project/config.kdl".into());
    s
}

#[test]
fn consulting_themes_shows_each_one_as_it_comes_under_the_cursor() {
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();

    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    assert!(s.editor.minibuffer.is_active(), "the prompt did not open");
    assert!(
        s.editor.minibuffer.completion().visible,
        "the themes are not listed"
    );

    // Typing a name shows it without anything being accepted yet.
    s.type_text("daylight");
    assert_eq!(
        s.editor.theme.name(),
        "daylight",
        "the theme was not previewed"
    );
    assert!(s.editor.minibuffer.is_active(), "the prompt closed early");
    assert_ne!(
        started_as, "daylight",
        "the fixture proves nothing otherwise"
    );
}

#[test]
fn abandoning_the_prompt_puts_the_old_theme_back() {
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();

    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    assert_eq!(s.editor.theme.name(), "daylight");

    s.keys("C-g");
    assert_eq!(s.editor.theme.name(), started_as, "C-g did not put it back");
    assert!(!s.editor.minibuffer.is_active());
}

#[test]
fn choosing_a_theme_is_the_end_of_it() {
    // It used to ask, here, whether to write the choice into the config —
    // a yes-or-no question between someone and a theme they had already
    // chosen by looking at it. Trying themes on and keeping one for good
    // are two intentions, and this command is the first.
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");

    assert_eq!(
        s.editor.theme.name(),
        "daylight",
        "the choice did not stick"
    );
    assert!(!s.editor.minibuffer.is_active(), "it asked something");
    assert!(
        s.editor.tasks.drain().is_empty(),
        "it wrote to the configuration file without being asked to"
    );
}

#[test]
fn a_theme_the_config_does_not_name_says_how_to_keep_it() {
    // The question that used to be asked, turned into an answer nobody has
    // to give: the way to make it stick is named where it comes up.
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");
    assert!(
        s.editor.minibuffer.display().contains("save-theme"),
        "got `{}`",
        s.editor.minibuffer.display()
    );
}

#[test]
fn the_theme_the_config_already_names_is_not_offered_a_way_to_keep_it() {
    // It will be there tomorrow whatever anyone does now, so saying how to
    // keep it would be noise.
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("maxgus-dark");
    s.keys("RET");

    assert!(!s.editor.minibuffer.is_active(), "it asked anyway");
    assert_eq!(s.editor.theme.name(), "maxgus-dark");
    assert!(
        !s.editor.minibuffer.display().contains("save-theme"),
        "got `{}`",
        s.editor.minibuffer.display()
    );
}

#[test]
fn save_theme_writes_the_one_in_use() {
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");
    s.editor.tasks.drain();

    s.keys("M-x");
    s.type_text("save-theme");
    s.keys("RET");

    let queued = s.editor.tasks.drain();
    let wrote = queued.iter().any(
        |task| matches!(task, maxgus_core::Task::PersistTheme { theme, .. } if theme == "daylight"),
    );
    assert!(wrote, "nothing was queued to write the theme: {queued:?}");
}

#[test]
fn a_prefix_argument_visits_and_keeps_in_one_go() {
    // For someone who knew before they started. The argument is given when
    // the prompt opens and wanted when it closes, which is the only reason
    // it has to be remembered at all.
    let mut s = with_two_themes();
    s.keys("C-u");
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");

    assert_eq!(s.editor.theme.name(), "daylight");
    let queued = s.editor.tasks.drain();
    let wrote = queued.iter().any(
        |task| matches!(task, maxgus_core::Task::PersistTheme { theme, .. } if theme == "daylight"),
    );
    assert!(wrote, "the prefix argument did not write it: {queued:?}");
}

#[test]
fn a_name_that_is_not_a_theme_leaves_the_one_that_was_showing() {
    // Half-applying something that does not exist is the failure here: the
    // preview has already changed the screen by the time `RET` is pressed.
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.type_text("daylight");
    assert_eq!(s.editor.theme.name(), "daylight", "no preview to undo");
    // Carry on typing until it is no longer any theme's name.
    s.type_text("-and-a-half");
    s.keys("RET");

    assert_eq!(
        s.editor.theme.name(),
        started_as,
        "a name that is not a theme was left applied"
    );
    assert!(
        s.editor.minibuffer.message_is_error(),
        "it was not reported: `{}`",
        s.editor.minibuffer.display()
    );
}

#[test]
fn an_empty_answer_keeps_the_theme_that_was_already_in_use() {
    // `RET` straight away is how someone leaves without choosing, and it
    // has to be as harmless as `C-g`.
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();
    s.keys("M-x");
    s.type_text("consult-theme");
    s.keys("RET");
    s.keys("RET");

    assert_eq!(s.editor.theme.name(), started_as);
    assert!(!s.editor.minibuffer.is_active());
}

#[cfg(feature = "full")]
#[test]
fn the_mode_line_shows_the_branch_once_it_is_known() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    assert!(
        !s.mode_line().contains(maxgus_core::icons::BRANCH),
        "there is no branch to show yet: `{}`",
        s.mode_line()
    );

    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitBranch {
            branch: Some("feature/icons".into()),
        })
        .unwrap();

    let line = s.mode_line();
    assert!(line.contains("feature/icons"), "got `{line}`");
    assert!(
        line.contains(maxgus_core::icons::BRANCH),
        "without its glyph: `{line}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_directory_that_is_not_a_repository_shows_no_branch() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitBranch { branch: None })
        .unwrap();
    assert!(
        !s.mode_line().contains(maxgus_core::icons::BRANCH),
        "got `{}`",
        s.mode_line()
    );
}

#[test]
fn m_x_opens_a_popup_at_the_top_of_the_frame() {
    // The list is where the eye already is rather than a strip above the
    // bottom line, and it says where in the list the highlight sits.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    let total = s.editor.command_names.len();
    let screen = s.screen();

    // The box is centred, so every one of its rows starts at the same column
    // rather than at the frame's edge.
    let left = popup_left(&mut s);
    let at = |row: &str, column: usize| row.chars().nth(column);
    assert_eq!(
        at(&screen[0], left),
        Some('\u{256d}'),
        "no popup border: `{}`",
        screen[0]
    );
    let right = screen[0]
        .chars()
        .position(|c| c == '\u{256e}')
        .expect("the box has a right corner");
    assert!(
        left.abs_diff(60 - 1 - right) <= 1,
        "the popup is not centred: `{}`",
        screen[0]
    );
    // Where the highlight is, out of how many match: the first of all of them.
    assert!(
        screen[1].contains(&format!("1/{total} M-x")),
        "the prompt line does not count the list: `{}`",
        screen[1]
    );
    assert_eq!(
        at(&screen[2], left),
        Some('\u{2502}'),
        "no candidates under it: `{}`",
        screen[2]
    );
    // The prompt went up with the list instead of being drawn in both places.
    assert!(
        screen[9].is_empty(),
        "the echo area still prompts: `{}`",
        screen[9]
    );

    // Narrowing changes the right half of the count and walking the list
    // changes the left. Telling them apart needs a case where the two differ
    // from each other and from the size of the whole set.
    s.type_text("buffer");
    let matched = s.editor.minibuffer.completion().len();
    assert!(matched < total, "`buffer` matched the whole command set");
    let row = s.screen()[1].clone();
    assert!(
        row.contains(&format!("1/{matched} M-x buffer")),
        "count line is `{row}`"
    );

    s.keys("<down>");
    let row = s.screen()[1].clone();
    assert!(
        row.contains(&format!("2/{matched} M-x buffer")),
        "count line is `{row}`"
    );
}

#[test]
fn the_popup_says_what_each_command_does_and_which_key_runs_it() {
    // A bare list of names is the least useful thing `M-x` could show. The
    // binding is what stops the user needing `M-x` next time.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("save-buffer");
    // From the third line down: the first two are the border and the prompt,
    // and the prompt holds what was typed, which is the same text.
    let row = s
        .screen()
        .into_iter()
        .skip(2)
        .find(|l| l.contains("save-buffer"))
        .unwrap();
    assert!(
        row.contains("C-x C-s"),
        "no key binding beside the name: `{row}`"
    );
    // Clipped at the right edge of a sixty-column frame, which is what the
    // column is for: the name and the key stay put and the prose gives way.
    assert!(row.contains("Save this"), "no summary: `{row}`");
}

#[test]
fn a_wide_frame_gives_the_summary_room_to_finish() {
    // The other side of the popup no longer taking the whole frame: what it
    // does take has to be enough for the documentation column to say
    // something. A terminal with room shows the whole line.
    let mut s = Session::new(140, 20);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("M-x");
    s.type_text("save-buffer");
    let row = s
        .screen()
        .into_iter()
        .skip(2)
        .find(|l| l.contains("save-buffer"))
        .expect("the candidate row");
    assert!(
        row.contains("Save this buffer to its file."),
        "the summary is still clipped in a wide frame: `{row}`"
    );
}

#[test]
fn the_command_list_is_matched_fuzzily() {
    // `sbfr` is not a prefix of anything, and the letters are not adjacent in
    // what it should find: only a subsequence match gets there.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("sbfr");
    let found = s.editor.minibuffer.completion().candidates.clone();
    assert!(
        found.iter().any(|c| c == "save-buffer"),
        "fuzzy matching did not reach `save-buffer`: {found:?}"
    );
    assert!(
        s.screen().iter().any(|l| l.contains("save-buffer")),
        "it is not on screen"
    );
}

#[test]
fn the_arrow_keys_walk_the_candidate_list() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("buffer");
    let first = s
        .editor
        .minibuffer
        .completion()
        .current()
        .unwrap()
        .to_string();

    s.keys("<down>");
    let second = s
        .editor
        .minibuffer
        .completion()
        .current()
        .unwrap()
        .to_string();
    assert_ne!(first, second, "`<down>` did not move the highlight");
    // The highlight is on the row it moved to, not the one it left.
    let chosen = s.editor.theme.resolve("completion-selected").background;
    let inside = popup_left(&mut s) as u16 + 1;
    assert_eq!(
        s.face_at(inside, 3).background,
        chosen,
        "the second row is not marked"
    );
    assert_ne!(
        s.face_at(inside, 2).background,
        chosen,
        "the first row is still marked"
    );

    s.keys("<up>");
    assert_eq!(
        s.editor.minibuffer.completion().current(),
        Some(first.as_str())
    );
}

#[test]
fn the_page_keys_move_a_screenful_of_candidates() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    let page = s.editor.completion_rows();
    assert!(
        page > 1,
        "a page of {page} rows cannot tell paging from stepping"
    );

    s.keys("<next>");
    assert_eq!(s.editor.minibuffer.completion().selected, Some(page));
    s.keys("<prior>");
    assert_eq!(s.editor.minibuffer.completion().selected, Some(0));
}

#[test]
fn return_runs_the_highlighted_candidate() {
    // Not what was typed: `mwb` is not the name of anything.
    let mut s = Session::editing("/project/main.rs", "one\ntwo\nthree\n");
    s.keys("M-x");
    s.type_text("mwb");
    let mut steps = 0;
    while s.editor.minibuffer.completion().current() != Some("mark-whole-buffer") {
        s.keys("<down>");
        steps += 1;
        assert!(steps < 40, "`mark-whole-buffer` is not in the list at all");
    }
    s.keys("RET");
    let whole = maxgus_text::Range::new(0, s.editor.current_buffer().len_chars());
    assert_eq!(
        s.editor.region().unwrap(),
        whole,
        "`mark-whole-buffer` did not run"
    );
}

#[test]
fn the_cursor_follows_the_prompt_into_the_popup() {
    // Leaving it behind in the echo area is what makes a prompt feel frozen:
    // the text appears in one place and the caret blinks in another.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("save");

    let (x, y) = s.editor.cursor_position();
    assert_eq!(y, 1, "the cursor is not on the popup's prompt line");
    let row: Vec<char> = s.screen()[1].chars().collect();
    let before: String = row[1..x as usize].iter().collect();
    assert!(
        before.ends_with("M-x save"),
        "the cursor is not after what was typed: `{before}`"
    );
    assert_eq!(row[x as usize], ' ', "the cursor sits on top of drawn text");
}

#[test]
fn a_file_prompt_answers_with_what_was_typed_not_what_it_matched() {
    // `C-x C-f` has to be able to name a file that is not there yet, and a
    // new name is very often a subsequence of one that is: `notes` of
    // `notes-2024.md`. Answering with the match would visit the old file and
    // never create the new one.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("C-x C-f");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::DirectoryListed {
            path: "/project".into(),
            entries: vec!["/project/notes-2024.md".to_string()],
        })
        .unwrap();
    s.editor.tasks.drain();

    s.type_text("notes");
    assert!(
        s.editor
            .minibuffer
            .completion()
            .candidates
            .iter()
            .any(|c| c.ends_with("2024.md")),
        "the older file should still be offered"
    );
    s.keys("RET");

    let tasks = s.editor.tasks.drain();
    assert!(
        tasks
            .iter()
            .any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("notes"))),
        "the prompt visited something other than what was typed: {tasks:?}"
    );
}

#[test]
fn a_file_prompt_answers_with_the_candidate_once_the_arrows_pick_one() {
    // The other half of the rule: choosing from the list is still choosing.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("C-x C-f");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::DirectoryListed {
            path: "/project".into(),
            entries: vec!["/project/notes-2024.md".to_string()],
        })
        .unwrap();
    s.editor.tasks.drain();

    s.type_text("notes");
    s.keys("<down>");
    s.keys("RET");
    let tasks = s.editor.tasks.drain();
    assert!(
        tasks
            .iter()
            .any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("notes-2024.md"))),
        "the highlighted file was not the one visited: {tasks:?}"
    );
}

#[test]
fn return_on_an_untouched_command_prompt_runs_the_highlighted_one() {
    // `M-x` is the one completing prompt with no default to fall back on: an
    // empty command name answers nothing, so what is highlighted is the only
    // useful reading of `RET`. The prompts that do name a default keep it.
    // Which command sorts first is not the point and used to be written
    // down here, so adding one broke this test rather than anything real.
    // What is asked is that `RET` runs *whatever* is highlighted.
    let mut s = Session::editing("/project/main.rs", "    indented\n");
    s.keys("M-x");
    let highlighted = s
        .editor
        .minibuffer
        .completion()
        .current()
        .map(str::to_string)
        .expect("something is highlighted on an empty prompt");

    s.keys("RET");
    assert!(
        s.editor.minibuffer.completion().is_empty(),
        "the prompt is still open"
    );
    assert_eq!(
        s.editor.last_command.as_deref(),
        Some(highlighted.as_str()),
        "`RET` ran something other than what was highlighted"
    );
}

#[test]
fn a_query_that_matches_nothing_keeps_the_popup_where_it_is() {
    // Dropping the box when the last match goes would throw the prompt to the
    // bottom of the screen on the keystroke that stops matching and back up
    // on the one that deletes it. It stays, and says nothing matched.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("zzzq");
    assert!(
        s.editor.minibuffer.completion().is_empty(),
        "`zzzq` matched something"
    );

    let screen = s.screen();
    assert!(
        screen[0].contains('\u{256d}'),
        "the popup left: {screen:#?}"
    );
    assert!(
        screen[1].contains("0/0 M-x zzzq"),
        "prompt line is `{}`",
        screen[1]
    );
    assert!(
        screen[9].is_empty(),
        "the prompt fell back to the echo area: `{}`",
        screen[9]
    );
}

// ---- the side panel -----------------------------------------------------

/// A session with room for all three sections at once. The default ten rows
/// fit the tree and the outline and push the buffer list off the bottom.
fn tall_session(path: &str, text: &str) -> Session {
    let mut session = Session::new(90, 30);
    let id = session.editor.buffers.visit_file(path, text);
    session.editor.switch_to_buffer(id).unwrap();
    session.editor.with_current_buffer(|b| b.set_point(0));
    session.editor.tasks.drain();
    session
}

#[cfg(feature = "full")]
/// A session with the panel open: three windows down the left, the tree
/// filled, a language server up, and an outline delivered.
fn with_panel() -> Session {
    let mut s = tall_session("/project/main.rs", "fn one() {}\nfn two() {}\nstruct S;\n");
    s.editor
        .buffers
        .visit_file("/project/other.rs", "fn other() {}\n");
    // The server first: whether the outline window exists is decided when the
    // column is built.
    start_server(&mut s);
    s.keys("C-x t t");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TreeUpdated {
            nodes: vec![
                node("/project", "project", true, 0, true),
                node("/project/main.rs", "main.rs", false, 1, false),
                node("/project/other.rs", "other.rs", false, 1, false),
            ],
            select: None,
            show_hidden: false,
        })
        .unwrap();
    deliver_symbols(&mut s);
    s.editor.tasks.drain();
    s
}

#[cfg(feature = "full")]
fn start_server(s: &mut Session) {
    s.editor
        .apply_task_result(maxgus_core::TaskResult::LanguageServerStarted {
            language: "rust".into(),
            encoding: maxgus_lsp::PositionEncoding::Utf16,
        })
        .unwrap();
}

#[cfg(feature = "full")]
/// `one` and `two` are plain functions; `S` is a struct with a field, so the
/// outline has something to fold.
fn deliver_symbols(s: &mut Session) {
    let symbols = serde_json::json!([
        {"name": "one", "kind": 12, "selectionRange": {"start": {"line": 0, "character": 3}}},
        {"name": "two", "kind": 12, "selectionRange": {"start": {"line": 1, "character": 3}}},
        {"name": "S", "kind": 23, "selectionRange": {"start": {"line": 2, "character": 7}},
         "children": [
            {"name": "field", "kind": 8, "selectionRange": {"start": {"line": 2, "character": 9}}}
         ]}
    ]);
    s.editor
        .apply_lsp_response(maxgus_core::TaskResult::LspResponse {
            language: "rust".into(),
            uri: "file:///project/main.rs".into(),
            query: maxgus_core::LspQuery::DocumentSymbols { for_panel: true },
            result: symbols,
        });
}

/// Selects the panel window showing `name`, and says which it was.
fn select_panel_window(s: &mut Session, name: &str) -> maxgus_core::window::WindowId {
    let id = s
        .editor
        .buffers
        .find_by_name(name)
        .unwrap_or_else(|| panic!("no {name} buffer"));
    let window = *s
        .editor
        .windows
        .showing(id)
        .first()
        .unwrap_or_else(|| panic!("{name} is not in a window"));
    s.editor.select_window(window);
    window
}

#[cfg(feature = "full")]
#[test]
fn the_panel_is_three_windows_stacked_down_the_left() {
    // One window per section rather than one buffer with headings in it, so
    // that moving between them is ordinary window movement and each keeps its
    // own point.
    let s = with_panel();
    assert_eq!(s.editor.panel_windows.len(), 3, "not three windows");
    let rects: Vec<_> = s
        .editor
        .panel_windows
        .iter()
        .filter_map(|id| s.editor.windows.get(*id))
        .collect();
    for window in &rects {
        assert_eq!(window.rect.x, 0, "a panel window is not on the left");
        assert_eq!(
            window.rect.width, s.editor.tree_width,
            "a panel window is the wrong width"
        );
    }
    // Stacked, in order, without overlapping.
    assert!(rects[0].rect.bottom() <= rects[1].rect.y);
    assert!(rects[1].rect.bottom() <= rects[2].rect.y);
}

#[cfg(feature = "full")]
#[test]
fn the_control_arrows_move_between_the_panels_windows() {
    // The reason for making them windows: this is ordinary window movement,
    // with nothing special about the panel at all.
    let mut s = with_panel();
    let names = ["*treefile*", "*symbols*", "*buffers*"];
    select_panel_window(&mut s, names[0]);

    for expected in &names[1..] {
        s.keys("C-<down>");
        assert_eq!(
            s.editor.current_buffer().name(),
            *expected,
            "`C-<down>` did not reach {expected}"
        );
    }
    for expected in names[..2].iter().rev() {
        s.keys("C-<up>");
        assert_eq!(s.editor.current_buffer().name(), *expected);
    }
    // And out of the column to the file being edited.
    s.keys("C-<right>");
    assert_eq!(s.editor.current_buffer().name(), "main.rs");
}

#[cfg(feature = "full")]
#[test]
fn each_panel_window_keeps_its_own_point() {
    // The other reason: a single buffer could not do this.
    let mut s = with_panel();
    select_panel_window(&mut s, "*symbols*");
    s.keys("n");
    s.keys("n");
    let symbol = s.editor.symbol_at_cursor();
    assert_eq!(symbol, Some(2), "the outline did not move");

    select_panel_window(&mut s, "*treefile*");
    s.keys("n");
    assert_eq!(s.editor.tree_cursor_line(), 1);

    // Back to the outline, still where it was.
    select_panel_window(&mut s, "*symbols*");
    assert_eq!(
        s.editor.symbol_at_cursor(),
        symbol,
        "the outline lost its place"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_symbol_is_selected_and_gone_to() {
    let mut s = with_panel();
    select_panel_window(&mut s, "*symbols*");
    s.keys("n");
    s.keys("n");
    assert_eq!(s.editor.symbol_at_cursor(), Some(2), "not on the struct");

    s.keys("RET");
    // In the window beside the panel, not in one of the panel's own: showing
    // the file inside the outline window would leave both useless.
    assert!(
        !s.editor
            .panel_windows
            .contains(&s.editor.windows.current_id()),
        "the file was opened inside the panel"
    );
    assert_eq!(
        s.editor.current_buffer().name(),
        "main.rs",
        "focus did not leave the panel"
    );
    let position = s.editor.current_buffer().position_of(s.point());
    assert_eq!(
        (position.line, position.column),
        (2, 7),
        "point is not on the struct"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_buffer_is_selected_and_shown() {
    let mut s = with_panel();
    select_panel_window(&mut s, "*buffers*");
    let expected = s
        .editor
        .listed_buffer_at_cursor()
        .expect("a buffer under point");
    s.keys("RET");
    assert_eq!(s.editor.windows.current().buffer, expected);
    // And the panel's own buffers are not in the list.
    let listed: Vec<String> = s
        .editor
        .panel_buffers()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    assert!(
        !listed
            .iter()
            .any(|name| name.starts_with('*') && name.ends_with('*') && name != "*scratch*"),
        "a panel buffer is listed: {listed:?}"
    );
}

#[cfg(feature = "full")]
#[test]
fn tab_folds_a_symbol_and_hides_what_is_inside_it() {
    let mut s = with_panel();
    select_panel_window(&mut s, "*symbols*");
    let shown = |s: &Session| s.editor.panel.visible_symbols().len();
    assert_eq!(shown(&s), 4);

    s.keys("n");
    s.keys("n");
    s.keys("TAB");
    assert_eq!(shown(&s), 3, "the struct's field is still shown");
    // Point stays on the symbol that was folded.
    assert_eq!(s.editor.symbol_at_cursor(), Some(2));
    s.keys("TAB");
    assert_eq!(shown(&s), 4);
}

#[test]
fn the_outline_window_is_absent_when_there_is_no_server() {
    // Not an empty window — absent. A window over nothing is worse than one
    // section fewer.
    let mut s = tall_session("/project/main.rs", "fn one() {}\n");
    s.keys("C-x t t");
    assert_eq!(
        s.editor.panel_windows.len(),
        2,
        "the outline window should not be there"
    );
    assert!(s.editor.buffers.find_by_name("*symbols*").is_none());
}

#[test]
fn a_section_switched_off_in_configuration_is_not_in_the_panel() {
    let settings = Settings {
        panel_buffers: false,
        ..Settings::default()
    };
    let mut s = Session::configured(settings, 90, 30);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn one() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.tasks.drain();
    s.keys("C-x t t");
    assert!(
        s.editor.buffers.find_by_name("*buffers*").is_none(),
        "the list is still there"
    );
    assert!(s.editor.buffers.find_by_name("*treefile*").is_some());
}

#[cfg(feature = "full")]
#[test]
fn switching_a_section_on_rebuilds_the_column() {
    let mut s = with_panel();
    assert_eq!(s.editor.panel_windows.len(), 3);
    select_panel_window(&mut s, "*treefile*");
    s.keys("t b");
    assert_eq!(
        s.editor.panel_windows.len(),
        2,
        "the buffer list did not go"
    );
    s.keys("t b");
    assert_eq!(s.editor.panel_windows.len(), 3, "it did not come back");
}

#[test]
fn the_last_section_cannot_be_switched_off() {
    let mut s = tall_session("/project/main.rs", "fn one() {}\n");
    s.keys("C-x t t");
    select_panel_window(&mut s, "*treefile*");
    s.keys("t b");
    s.keys("t r");
    assert!(s.echo().contains("nothing left"), "got `{}`", s.echo());
    assert!(
        s.editor.buffers.find_by_name("*treefile*").is_some(),
        "the tree went away"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_outline_belongs_to_the_buffer_being_edited() {
    // Symbols for one file shown against another is worse than no symbols.
    let mut s = with_panel();
    assert_eq!(s.editor.panel.symbols.len(), 4);

    let other = s
        .editor
        .buffers
        .find_by_path(std::path::Path::new("/project/other.rs"))
        .unwrap();
    let editing = s
        .editor
        .windows
        .ids()
        .into_iter()
        .find(|id| !s.editor.panel_windows.contains(id))
        .expect("a window to edit in");
    s.editor.select_window(editing);
    s.editor.switch_to_buffer(other).unwrap();
    assert!(
        s.editor.panel.symbols.is_empty(),
        "the old file's outline is still up"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_tree_command_typed_in_the_outline_does_nothing_to_the_tree() {
    // Each window has its own keymap, so `d` in the outline is not the
    // tree's delete at all.
    let mut s = with_panel();
    select_panel_window(&mut s, "*symbols*");
    s.editor.tasks.drain();
    s.keys("d");
    assert!(
        s.editor.tasks.drain().is_empty(),
        "a tree task was queued from the outline"
    );
}

// ---- the terminal panel -------------------------------------------------

#[cfg(feature = "full")]
/// A session with the terminal open and its shell "started".
fn with_terminal() -> Session {
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.keys("C-x t v");
    s.editor.tasks.drain();
    s
}

#[cfg(feature = "full")]
/// The bytes sent to the shell since the last drain.
fn sent(s: &mut Session) -> Vec<u8> {
    s.editor
        .tasks
        .drain()
        .into_iter()
        .filter_map(|task| match task {
            Task::TerminalInput { bytes, .. } => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect()
}

#[cfg(feature = "full")]
/// Feeds output from the program to the terminal showing.
fn output(s: &mut Session, bytes: &str) {
    let terminal = s.editor.terminals.current().expect("a terminal").id;
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TerminalOutput {
            terminal,
            bytes: bytes.as_bytes().to_vec(),
        })
        .unwrap();
}

#[cfg(feature = "full")]
#[test]
fn the_terminal_opens_along_the_bottom_and_starts_a_shell() {
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.keys("C-x t v");

    let window = s.editor.terminal_window.expect("the terminal window");
    let rect = s.editor.windows.get(window).unwrap().rect;
    assert_eq!(rect.width, 90, "the panel does not span the frame");
    assert!(rect.y > 0, "it is not along the bottom");

    let tasks = s.editor.tasks.drain();
    assert!(
        tasks.iter().any(|t| matches!(t, Task::TerminalOpen { .. })),
        "no shell was started: {tasks:?}"
    );
    // Opening it means wanting to type in it.
    assert_eq!(s.editor.windows.current_id(), window);
}

#[cfg(feature = "full")]
#[test]
fn typing_in_a_terminal_reaches_the_shell_rather_than_the_editor() {
    // This is the whole point of a terminal window: `C-a` is readline's, not
    // `move-beginning-of-line`, and `l` is a keystroke, not `self-insert`.
    let mut s = with_terminal();
    let before = s
        .editor
        .buffers
        .get(s.editor.buffers.ids()[1])
        .map(|b| b.text());

    s.type_text("ls");
    s.keys("C-a");
    s.keys("RET");
    assert_eq!(sent(&mut s), b"ls\x01\r");

    let after = s
        .editor
        .buffers
        .get(s.editor.buffers.ids()[1])
        .map(|b| b.text());
    assert_eq!(before, after, "a keystroke was inserted into a buffer");
}

#[cfg(feature = "full")]
#[test]
fn what_the_shell_writes_is_drawn_in_the_panel() {
    let mut s = with_terminal();
    output(&mut s, "hello from the shell\r\n$ ");
    assert!(
        s.screen()
            .iter()
            .any(|line| line.contains("hello from the shell")),
        "the output is not on screen:\n{:#?}",
        s.screen()
    );
}

#[cfg(feature = "full")]
#[test]
fn tabs_are_opened_and_walked_between() {
    let mut s = with_terminal();
    assert_eq!(s.editor.terminals.len(), 1);

    s.keys("C-c t");
    assert_eq!(s.editor.terminals.len(), 2, "`C-c t` did not open a tab");
    assert_eq!(
        s.editor.terminals.current_index(),
        1,
        "the new tab is not the one showing"
    );

    s.keys("C-c p");
    assert_eq!(s.editor.terminals.current_index(), 0);
    s.keys("C-c n");
    assert_eq!(s.editor.terminals.current_index(), 1);
    // Two tabs is few enough that walking off the end should come round.
    s.keys("C-c n");
    assert_eq!(s.editor.terminals.current_index(), 0);

    // And the bar says which is which, by the number `C-c 1` refers to.
    let bar = s
        .screen()
        .into_iter()
        .find(|line| line.contains(" 1 "))
        .unwrap_or_default();
    assert!(
        bar.contains(" 2 "),
        "the tab bar does not list both: `{bar}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_tab_is_chosen_by_its_number() {
    let mut s = with_terminal();
    s.keys("C-c t");
    s.keys("C-c t");
    assert_eq!(s.editor.terminals.len(), 3);

    s.keys("C-c 1");
    assert_eq!(s.editor.terminals.current_index(), 0);
    s.keys("C-c 3");
    assert_eq!(s.editor.terminals.current_index(), 2);
    s.keys("C-c 9");
    assert!(s.echo().contains("no tab 9"), "got `{}`", s.echo());
}

#[cfg(feature = "full")]
#[test]
fn closing_the_last_tab_closes_the_panel_with_it() {
    // A terminal panel with no terminal in it is a band of nothing across the
    // bottom of the frame.
    let mut s = with_terminal();
    s.keys("C-c t");
    s.keys("C-c k");
    assert_eq!(s.editor.terminals.len(), 1);
    assert!(
        s.editor.terminal_window.is_some(),
        "the panel closed too early"
    );

    s.keys("C-c k");
    assert!(s.editor.terminals.is_empty());
    assert!(
        s.editor.terminal_window.is_none(),
        "the panel outlived its last tab"
    );
}

#[cfg(feature = "full")]
#[test]
fn reading_mode_stops_the_keys_reaching_the_shell() {
    let mut s = with_terminal();
    output(&mut s, "one\r\ntwo\r\nthree\r\n");
    s.editor.tasks.drain();

    s.keys("C-c C-t");
    assert!(s.editor.terminals.current().unwrap().in_copy_mode());
    // `n` moves the reading cursor; it must not be typed at the shell.
    s.keys("n");
    s.keys("p");
    assert!(
        sent(&mut s).is_empty(),
        "keys leaked to the shell while reading"
    );

    s.keys("C-g");
    assert!(!s.editor.terminals.current().unwrap().in_copy_mode());
    s.type_text("x");
    assert_eq!(sent(&mut s), b"x", "the keyboard did not come back");
}

#[cfg(feature = "full")]
#[test]
fn a_selection_made_while_reading_goes_to_the_kill_ring() {
    let mut s = with_terminal();
    output(&mut s, "hello world\r\n");
    s.keys("C-c C-t");

    // To the start of the first line, mark, then five characters right.
    s.keys("M-<");
    s.keys("C-SPC");
    for _ in 0..4 {
        s.keys("C-f");
    }
    s.keys("M-w");

    assert_eq!(s.editor.kill_ring.front(), Some("hello"));
    assert!(
        !s.editor.terminals.current().unwrap().in_copy_mode(),
        "copying should end reading"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_paste_goes_in_bracketed_when_the_shell_asked_for_that() {
    let mut s = with_terminal();
    s.editor.kill_ring.kill_new("echo one\necho two");
    s.editor.tasks.drain();

    s.keys("C-c C-y");
    assert_eq!(sent(&mut s), b"echo one\recho two", "a plain paste");

    // Once the shell turns bracketed paste on, the same paste is wrapped so
    // it cannot run half of itself.
    output(&mut s, "\x1b[?2004h");
    s.editor.tasks.drain();
    s.keys("C-c C-y");
    assert_eq!(
        sent(&mut s),
        b"\x1b[200~echo one\recho two\x1b[201~".to_vec()
    );
}

#[cfg(feature = "full")]
#[test]
fn the_interrupt_key_is_given_back_by_the_prefix_that_took_it() {
    // `C-c` is the prefix, so `C-c c` has to send a real interrupt or there
    // would be no way to stop a program.
    let mut s = with_terminal();
    s.keys("C-c c");
    assert_eq!(sent(&mut s), [3]);
}

#[cfg(feature = "full")]
#[test]
fn resizing_the_frame_tells_the_programs_inside() {
    // Without this, `vim` in a tab goes on drawing to the shape it started
    // with, which looks like a corrupt screen rather than a stale size.
    let mut s = with_terminal();
    s.editor.tasks.drain();
    s.editor.set_frame(maxgus_tui::Rect::new(0, 0, 100, 40));

    let tasks = s.editor.tasks.drain();
    assert!(
        tasks
            .iter()
            .any(|t| matches!(t, Task::TerminalResize { columns: 100, .. })),
        "no resize was sent: {tasks:?}"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_terminal_spans_the_frame_with_the_side_panel_open() {
    let mut s = with_terminal();
    s.keys("C-x t t");
    let terminal = s.editor.terminal_window.expect("the terminal");
    let panel = s.editor.tree_window.expect("the panel");
    assert_eq!(s.editor.windows.get(terminal).unwrap().rect.width, 90);
    assert!(
        s.editor.windows.get(panel).unwrap().rect.bottom()
            <= s.editor.windows.get(terminal).unwrap().rect.y,
        "the side panel overlaps the terminal"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_arrows_change_shape_when_the_shell_asks() {
    // `DECCKM`. Sending the wrong spelling makes the arrows print `^[[A` at a
    // prompt instead of walking the history.
    let mut s = with_terminal();
    s.editor.tasks.drain();
    s.keys("<up>");
    assert_eq!(sent(&mut s), b"\x1b[A");

    output(&mut s, "\x1b[?1h");
    s.editor.tasks.drain();
    s.keys("<up>");
    assert_eq!(sent(&mut s), b"\x1bOA");
}

#[cfg(feature = "full")]
#[test]
fn the_editors_own_prefix_still_works_from_inside_a_terminal() {
    // Without this the terminal swallows `C-x` along with everything else and
    // there is no way out of it — no other window, no saving, no quitting.
    let mut s = with_terminal();
    s.editor.tasks.drain();

    s.keys("C-x t v");
    assert!(
        s.editor.terminal_window.is_none(),
        "`C-x t v` did not reach the editor"
    );
    assert!(sent(&mut s).is_empty(), "the prefix leaked to the shell");
}

#[cfg(feature = "full")]
#[test]
fn the_keys_the_editor_keeps_are_given_back_under_the_prefix() {
    // Four keys stay the editor's, so each has a spelling that sends it for
    // real: a program that wants `C-x` or `C-g` must still be able to have it.
    let mut s = with_terminal();
    s.editor.tasks.drain();

    s.keys("C-c x");
    assert_eq!(sent(&mut s), [0x18], "C-c x should send a real C-x");
    s.keys("C-c g");
    assert_eq!(sent(&mut s), [0x07], "C-c g should send a real C-g");
    s.keys("C-c c");
    assert_eq!(sent(&mut s), [0x03], "C-c c should send a real interrupt");
}

#[cfg(feature = "full")]
#[test]
fn help_and_m_x_still_reach_the_editor_from_a_terminal() {
    let mut s = with_terminal();
    s.editor.tasks.drain();
    s.keys("M-x");
    assert_eq!(
        s.editor.minibuffer.kind(),
        Some(MinibufferKind::Command),
        "M-x was swallowed"
    );
    s.keys("C-g");
    assert!(
        sent(&mut s).is_empty(),
        "the editor's keys leaked to the shell"
    );
}

// ---- the git status view ------------------------------------------------

#[cfg(feature = "full")]
const UNSTAGED_DIFF: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
-old first
+new first
 tail
@@ -20,2 +20,2 @@
-old second
+new second
 tail
";

#[cfg(feature = "full")]
const STAGED_DIFF: &str = "\
diff --git a/src/b.rs b/src/b.rs
index 333..444 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -5,2 +5,2 @@
-was
+is
 tail
";

#[cfg(feature = "full")]
/// A session showing a repository with one unstaged file, one staged file,
/// an untracked file and a stash.
fn with_git() -> Session {
    // Thirty lines, so a jump to the second hunk at line twenty lands
    // somewhere rather than being clamped to the end of a short file.
    let text: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    let mut s = tall_session("/project/src/a.rs", &text);
    s.keys("C-x g");
    s.editor.tasks.drain();
    refresh_git(&mut s);
    s
}

#[cfg(feature = "full")]
fn refresh_git(s: &mut Session) {
    let status = maxgus_git::status::parse(
        b"# branch.oid 5958f5e13418d8b5\0\
          # branch.head main\0\
          # branch.upstream origin/main\0\
          # branch.ab +1 -0\0\
          1 .M N... 100644 100644 100644 aaa bbb src/a.rs\0\
          1 M. N... 100644 100644 100644 aaa bbb src/b.rs\0\
          ? notes.txt\0",
    );
    let snapshot = maxgus_core::task::GitSnapshot {
        root: "/project".into(),
        status,
        unstaged: maxgus_git::diff::parse(UNSTAGED_DIFF),
        staged: maxgus_git::diff::parse(STAGED_DIFF),
        stashes: maxgus_git::log::parse_stashes("stash@{0}\u{1f}WIP on main\u{1e}\n"),
        unpushed: maxgus_git::log::parse_log(
            "h1\u{1f}abc1234\u{1f}Someone\u{1f}an hour ago\u{1f}\u{1f}not pushed yet\u{1e}\n",
        ),
        unpulled: Vec::new(),
        recent: maxgus_git::log::parse_log(
            "h2\u{1f}def5678\u{1f}Someone\u{1f}a day ago\u{1f}HEAD -> main, tag: v1\u{1f}the last one\u{1e}\n",
        ),
        head_subject: "the last one".into(),
        branches: vec!["main".into(), "feature/x".into()],
        references: vec![
            maxgus_git::Reference {
                name: "main".into(),
                kind: maxgus_git::RefKind::Local,
            },
            maxgus_git::Reference {
                name: "feature/x".into(),
                kind: maxgus_git::RefKind::Local,
            },
            maxgus_git::Reference {
                name: "origin/main".into(),
                kind: maxgus_git::RefKind::Remote,
            },
            maxgus_git::Reference {
                name: "v1.0".into(),
                kind: maxgus_git::RefKind::Tag,
            },
        ],
    };
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitRefreshed(Box::new(snapshot)))
        .unwrap();
    s.editor.tasks.drain();
}

#[cfg(feature = "full")]
/// Moves point to the first row matching, and says which line it was.
fn go_to_git(s: &mut Session, matching: impl Fn(&maxgus_core::git::Row) -> bool) -> usize {
    let line = s
        .editor
        .git
        .rows()
        .iter()
        .position(&matching)
        .expect("no such row");
    s.editor.move_git_cursor_to_line(line);
    line
}

#[cfg(feature = "full")]
/// The git tasks queued since the last drain.
fn git_tasks(s: &mut Session) -> Vec<maxgus_core::task::GitAction> {
    s.editor
        .tasks
        .drain()
        .into_iter()
        .filter_map(|task| match task {
            Task::Git { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

#[cfg(feature = "full")]
#[test]
fn the_status_view_shows_the_whole_state_of_the_repository() {
    let mut s = with_git();
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));

    assert!(has("Head:"), "no head line:\n{screen:#?}");
    assert!(has("main"), "no branch");
    assert!(
        has("Untracked files (1)"),
        "no untracked section:\n{screen:#?}"
    );
    assert!(has("Unstaged changes (1)"), "no unstaged section");
    assert!(has("Staged changes (1)"), "no staged section");
    assert!(has("Stashes (1)"), "no stashes");
    assert!(has("Unpushed to upstream (1)"), "no unpushed section");
    assert!(has("Recent commits (1)"), "no recent commits");
    // Nothing is unpulled, so that heading should not be there at all.
    assert!(
        !has("Unpulled"),
        "an empty section was headed:\n{screen:#?}"
    );
}

#[cfg(feature = "full")]
#[test]
fn tab_folds_a_section_a_file_and_a_hunk_in_turn() {
    let mut s = with_git();
    let rows = |s: &Session| s.editor.git.rows().len();

    // A file first: its hunks appear.
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    let before = rows(&s);
    s.keys("TAB");
    assert!(rows(&s) > before, "opening the file showed nothing");
    assert!(
        s.editor
            .git
            .rows()
            .iter()
            .any(|r| matches!(r, maxgus_core::git::Row::Hunk { .. })),
        "no hunks appeared"
    );

    // Then one of its hunks.
    let opened = rows(&s);
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { .. })
    });
    s.keys("TAB");
    assert!(rows(&s) < opened, "folding the hunk hid nothing");

    // Then the whole section.
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Section(section)
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("TAB");
    assert!(
        !s.editor
            .git
            .rows()
            .iter()
            .any(|r| matches!(r, maxgus_core::git::Row::File { section, .. }
            if *section == maxgus_core::git::Section::Unstaged)),
        "the section folded but its files are still listed"
    );
}

#[cfg(feature = "full")]
#[test]
fn point_stays_on_the_row_that_was_folded() {
    // Everything below a fold moves. Coming back to the same line number
    // would leave point somewhere unrelated to what was just acted on.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Section(section)
        if *section == maxgus_core::git::Section::Untracked)
    });
    s.keys("TAB");
    assert!(
        matches!(
            s.editor.git_row_at_cursor(),
            Some(maxgus_core::git::Row::Section(
                maxgus_core::git::Section::Untracked
            ))
        ),
        "point left the row it folded: {:?}",
        s.editor.git_row_at_cursor()
    );
}

#[cfg(feature = "full")]
#[test]
fn staging_a_file_stages_that_file() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("s");
    let actions = git_tasks(&mut s);
    assert_eq!(actions.len(), 1, "got {actions:?}");
    match &actions[0] {
        maxgus_core::task::GitAction::Stage(paths) => {
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].to_string_lossy(), "src/a.rs");
        }
        other => panic!("expected a stage, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn staging_one_hunk_sends_a_patch_of_that_hunk_alone() {
    // The signature magit operation, and the one where getting it wrong
    // stages something the user did not look at.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("TAB");
    // The second hunk, so taking the whole file would be visibly wrong.
    let line = go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { hunk: 1, .. })
    });
    assert!(line > 0);
    s.editor.tasks.drain();

    s.keys("s");
    let actions = git_tasks(&mut s);
    match &actions[..] {
        [
            maxgus_core::task::GitAction::ApplyPatch {
                patch, arguments, ..
            },
        ] => {
            assert_eq!(arguments, &["--cached".to_string()]);
            assert!(patch.contains("new second"), "the wrong hunk:\n{patch}");
            assert!(
                !patch.contains("new first"),
                "the other hunk came too:\n{patch}"
            );
            assert!(patch.starts_with("diff --git"), "no header:\n{patch}");
        }
        other => panic!("expected one patch, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn unstaging_a_hunk_reverses_the_same_patch() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.keys("TAB");
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.editor.tasks.drain();

    s.keys("u");
    match &git_tasks(&mut s)[..] {
        [
            maxgus_core::task::GitAction::ApplyPatch {
                arguments, patch, ..
            },
        ] => {
            assert_eq!(
                arguments,
                &["--cached".to_string(), "--reverse".to_string()]
            );
            assert!(
                patch.contains("+is"),
                "the staged change is not in the patch:\n{patch}"
            );
        }
        other => panic!("expected a reversed patch, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn staging_something_already_staged_says_so_rather_than_doing_it_twice() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.keys("s");
    assert!(s.echo().contains("Already staged"), "got `{}`", s.echo());
    assert!(git_tasks(&mut s).is_empty(), "it went ahead anyway");
}

#[cfg(feature = "full")]
#[test]
fn discarding_asks_first_and_says_what_it_will_lose() {
    // The one irreversible thing here.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("k");
    assert_eq!(
        s.editor.minibuffer.kind(),
        Some(MinibufferKind::YesNo),
        "it did not ask"
    );
    assert!(
        s.editor.minibuffer.prompt().contains("src/a.rs"),
        "the question does not say what: `{}`",
        s.editor.minibuffer.prompt()
    );
    assert!(git_tasks(&mut s).is_empty(), "it discarded before asking");

    s.type_text("no");
    s.keys("RET");
    assert!(git_tasks(&mut s).is_empty(), "`no` discarded it anyway");

    s.keys("k");
    s.type_text("yes");
    s.keys("RET");
    assert!(
        matches!(
            git_tasks(&mut s).first(),
            Some(maxgus_core::task::GitAction::Discard(_))
        ),
        "`yes` did not discard"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_commit_is_written_in_a_buffer_and_finished_with_two_keys() {
    let mut s = with_git();
    s.keys("c c");
    assert_eq!(
        s.editor.current_buffer().name(),
        "COMMIT_EDITMSG",
        "no message buffer"
    );

    s.type_text("a good commit message");
    s.keys("C-c C-c");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Commit { message, amend, .. }] => {
            assert_eq!(message.trim(), "a good commit message");
            assert!(!amend);
        }
        other => panic!("expected a commit, got {other:?}"),
    }
    assert_ne!(
        s.editor.current_buffer().name(),
        "COMMIT_EDITMSG",
        "the buffer stayed up"
    );
}

#[cfg(feature = "full")]
#[test]
fn an_empty_commit_message_is_refused() {
    let mut s = with_git();
    s.keys("c c");
    s.keys("C-c C-c");
    assert!(
        s.echo().contains("empty commit message"),
        "got `{}`",
        s.echo()
    );
    assert!(git_tasks(&mut s).is_empty(), "it committed nothing");
}

#[cfg(feature = "full")]
#[test]
fn comment_lines_are_stripped_from_a_commit_message() {
    // As git strips them, so a template can explain itself without ending up
    // in the history.
    let mut s = with_git();
    s.keys("c c");
    s.type_text("the subject");
    s.keys("RET");
    s.type_text("# this is a comment");
    s.keys("C-c C-c");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Commit { message, .. }] => {
            assert!(
                !message.contains("comment"),
                "the comment was committed: `{message}`"
            );
            assert!(message.contains("the subject"));
        }
        other => panic!("expected a commit, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn return_on_a_hunk_opens_the_file_at_that_hunk() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("TAB");
    // The second hunk starts at line 20 of the new file.
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { hunk: 1, .. })
    });
    s.editor.tasks.drain();
    s.keys("RET");

    // The file is already open, so it is shown rather than read again; either
    // way what matters is where point ends up.
    assert_eq!(
        s.editor.current_buffer().name(),
        "a.rs",
        "the file was not shown"
    );
    let position = s.editor.current_buffer().position_of(s.point());
    assert_eq!(position.line, 19, "point is not at the hunk");
}

#[cfg(feature = "full")]
#[test]
fn return_on_a_hunk_of_a_file_not_open_yet_reads_it_and_then_jumps() {
    let mut s = with_git();
    // The staged change is to a file this session has never opened.
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.keys("TAB");
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.editor.tasks.drain();
    s.keys("RET");

    let tasks = s.editor.tasks.drain();
    assert!(
        tasks
            .iter()
            .any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("src/b.rs"))),
        "the file was not read: {tasks:?}"
    );
    // The jump waits for the read, which is what `pending_line` is for.
    assert_eq!(
        s.editor.pending_line.as_ref().map(|(_, line)| *line),
        Some(4),
        "it did not aim at the hunk"
    );
}

#[cfg(feature = "full")]
#[test]
fn return_on_a_diff_line_counts_only_the_lines_that_exist_in_the_file() {
    // The hunk is `-was`, `+is`, ` tail` starting at line five. A removed
    // line is not in the file being opened, so counting it would land a line
    // further down every time a hunk removes anything.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.keys("TAB");
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Line { section, line: 2, .. }
        if *section == maxgus_core::git::Section::Staged)
    });
    s.editor.tasks.drain();
    s.keys("RET");

    // ` tail` is the second line of the new file's hunk: line six, which is
    // five zero-based.
    assert_eq!(
        s.editor.pending_line.as_ref().map(|(_, line)| *line),
        Some(5),
        "the removed line was counted as if it were in the file"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_section_keys_walk_between_headings() {
    let mut s = with_git();
    let section = |s: &Session| s.editor.git_row_at_cursor().and_then(|row| row.section());
    s.editor.move_git_cursor_to_line(0);
    s.keys("M-n");
    assert_eq!(section(&s), Some(maxgus_core::git::Section::Untracked));
    s.keys("M-n");
    assert_eq!(section(&s), Some(maxgus_core::git::Section::Unstaged));
    s.keys("M-p");
    assert_eq!(section(&s), Some(maxgus_core::git::Section::Untracked));
}

#[cfg(feature = "full")]
#[test]
fn a_stash_is_acted_on_by_the_name_git_knows_it_by() {
    let mut s = with_git();
    go_to_git(&mut s, |row| matches!(row, maxgus_core::git::Row::Stash(_)));
    s.keys("z p");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::StashPop(name)] => assert_eq!(name, "stash@{0}"),
        other => panic!("expected a pop, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn the_branch_prompt_offers_the_branches_there_are() {
    let mut s = with_git();
    s.keys("b b");
    assert_eq!(s.editor.minibuffer.kind(), Some(MinibufferKind::Choice));
    let offered = s.editor.completion_candidates.clone();
    assert!(
        offered.contains(&"feature/x".to_string()),
        "got {offered:?}"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_mode_line_branch_comes_from_the_same_reading_as_the_view() {
    // Two sources would eventually disagree, and the one on screen all the
    // time is the one that would be wrong.
    let s = with_git();
    assert_eq!(s.editor.git_branch.as_deref(), Some("main"));
}

// ---- the transient menus ------------------------------------------------

#[cfg(feature = "full")]
#[test]
fn the_dispatch_menu_shows_what_git_can_do_here() {
    let mut s = with_git();
    s.keys("?");
    assert!(s.editor.transient.is_some(), "no menu opened");
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("Git"), "no title:\n{screen:#?}");
    assert!(has("Commit"), "no committing entry");
    assert!(has("Push"), "no pushing entry");
    assert!(has("Inspect"), "no group headings");
}

#[cfg(feature = "full")]
#[test]
fn a_key_in_a_menu_opens_the_menu_underneath_it() {
    let mut s = with_git();
    s.keys("?");
    s.keys("P");
    let screen = s.screen();
    assert!(
        screen.iter().any(|line| line.contains("Force with lease")),
        "the push menu did not open:\n{screen:#?}"
    );
    // And going back returns to the one it came from.
    s.keys("C-g");
    assert!(
        s.screen().iter().any(|line| line.contains("Inspect")),
        "C-g did not go back to the top menu"
    );
    s.keys("C-g");
    assert!(s.editor.transient.is_none(), "the menu would not close");
}

#[cfg(feature = "full")]
#[test]
fn a_switch_stays_on_and_is_given_to_the_command() {
    // The whole point of a menu: `--force-with-lease` is visible before it
    // happens rather than remembered afterwards.
    let mut s = with_git();
    s.keys("P");
    s.keys("- f");
    assert!(
        s.screen()
            .iter()
            .any(|line| line.contains("Force with lease") && line.contains('\u{2713}')),
        "the switch is not shown as on:\n{:#?}",
        s.screen()
    );
    s.editor.tasks.drain();

    s.keys("p");
    assert!(
        s.editor.transient.is_none(),
        "running a command should close the menu"
    );
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Push { arguments }] => {
            assert_eq!(arguments, &["--force-with-lease".to_string()]);
        }
        other => panic!("expected a push, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn a_key_that_is_not_in_the_menu_says_so_and_leaves_it_up() {
    let mut s = with_git();
    s.keys("?");
    s.keys("Z");
    assert!(
        s.editor.transient.is_some(),
        "an unknown key closed the menu"
    );
    assert!(s.echo().contains("not one of these"), "got `{}`", s.echo());
}

#[cfg(feature = "full")]
#[test]
fn the_menu_takes_every_key_while_it_is_up() {
    // A menu that let some keys through would be competing with whatever
    // they mean underneath. `TAB` is the case that matters: it inserts
    // nothing, so a menu that only caught self-inserting keys would let it
    // through and fold a section behind the menu.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Section(_))
    });
    let before = s.editor.git.rows().len();
    s.editor.tasks.drain();

    s.keys("c");
    s.keys("TAB");
    assert_eq!(
        s.editor.git.rows().len(),
        before,
        "`TAB` folded a section behind the menu"
    );
    s.keys("s");
    assert!(
        git_tasks(&mut s).is_empty(),
        "`s` staged something behind the menu"
    );
    assert!(s.editor.transient.is_some(), "the menu should still be up");
}

// ---- the other views ----------------------------------------------------

#[cfg(feature = "full")]
/// Feeds a commit as the executor would answer a `Show`.
fn deliver_revision(s: &mut Session) {
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitDiff {
            title: "commit abc1234".into(),
            preamble: vec![
                "Author:     Someone <s@example.invalid>".into(),
                "AuthorDate: 2026-08-29 10:00".into(),
                String::new(),
                "    the commit message".into(),
            ],
            files: maxgus_git::diff::parse(&format!("{UNSTAGED_DIFF}{STAGED_DIFF}")),
        })
        .unwrap();
}

#[cfg(feature = "full")]
#[test]
fn return_on_a_commit_shows_it_in_full() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Commit { .. })
    });
    s.editor.tasks.drain();
    s.keys("RET");

    // It asks for the commit, by hash.
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Show { revision }] => assert_eq!(revision, "h1"),
        other => panic!("expected a show, got {other:?}"),
    }

    deliver_revision(&mut s);
    assert_eq!(s.editor.current_buffer().name(), "magit: revision");
    assert_eq!(
        s.editor.windows.current().point,
        0,
        "a commit should open at its first line"
    );
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("commit abc1234"), "no title:\n{screen:#?}");
    assert!(has("Author:"), "no author line");
    assert!(has("the commit message"), "no message");
    assert!(has("src/b.rs"), "no diff");
    assert!(has("+is"), "the diff has no lines");
}

#[cfg(feature = "full")]
#[test]
fn a_file_in_a_revision_folds_on_its_own() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Commit { .. })
    });
    s.keys("RET");
    deliver_revision(&mut s);
    let shows = |s: &mut Session, needle: &str| s.screen().iter().any(|l| l.contains(needle));
    assert!(
        shows(&mut s, "+new first"),
        "the first file's diff is missing"
    );
    assert!(shows(&mut s, "+is"), "the second file's diff is missing");

    // Down to the first file's heading and fold it.
    let line = s
        .editor
        .git_diff_view()
        .expect("a diff view")
        .rows()
        .iter()
        .position(|row| matches!(row, maxgus_core::git::DiffRow::File(_)))
        .expect("a file row");
    let offset = s.editor.current_buffer().line_start(line);
    s.editor.windows.current_mut().point = offset;
    s.keys("TAB");
    assert!(
        !shows(&mut s, "+new first"),
        "the file folded but its lines are still drawn:\n{:#?}",
        s.screen()
    );
    // On its own: the other file is untouched.
    assert!(
        shows(&mut s, "+is"),
        "folding one file took the other's lines with it:\n{:#?}",
        s.screen()
    );
}

#[cfg(feature = "full")]
#[test]
fn folding_a_file_in_a_revision_leaves_point_on_it() {
    // What magit does: `TAB` folds the section point is on and point stays on
    // its heading. Sent back to line 1, a reader loses their place in a long
    // commit every time they collapse a file.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Commit { .. })
    });
    s.keys("RET");
    deliver_revision(&mut s);

    let file_line = |s: &Session| -> usize {
        s.editor
            .git_diff_view()
            .expect("a diff view")
            .rows()
            .iter()
            .position(|row| matches!(row, maxgus_core::git::DiffRow::File(_)))
            .expect("a file row")
    };
    let line = file_line(&s);
    assert!(line > 0, "the test needs a file below the first line");
    let offset = s.editor.current_buffer().line_start(line);
    s.editor.windows.current_mut().point = offset;

    s.keys("TAB"); // fold
    assert_eq!(
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point),
        file_line(&s),
        "folding moved point off the file"
    );

    s.keys("TAB"); // and open it again
    assert_eq!(
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point),
        file_line(&s),
        "expanding moved point off the file"
    );
}

#[cfg(feature = "full")]
#[test]
fn folding_the_second_file_keeps_point_on_the_second_file() {
    // The line a file's heading is on changes when a file above it folds, so
    // point has to be put back by which row it was on, not by its old line.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Commit { .. })
    });
    s.keys("RET");
    deliver_revision(&mut s);

    let files: Vec<usize> = s
        .editor
        .git_diff_view()
        .expect("a diff view")
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, maxgus_core::git::DiffRow::File(_)))
        .map(|(line, _)| line)
        .collect();
    assert_eq!(files.len(), 2, "the fixture should have two files");
    // Fold the first file: every heading below it moves up.
    let offset = s.editor.current_buffer().line_start(files[0]);
    s.editor.windows.current_mut().point = offset;
    s.keys("TAB");
    // Now fold the second, from its new line.
    let moved = s
        .editor
        .git_diff_view()
        .expect("a diff view")
        .rows()
        .iter()
        .position(|row| matches!(row, maxgus_core::git::DiffRow::File(1)))
        .expect("the second file");
    assert!(moved < files[1], "the first file did not fold");
    let offset = s.editor.current_buffer().line_start(moved);
    s.editor.windows.current_mut().point = offset;
    s.keys("TAB");

    let now = s
        .editor
        .current_buffer()
        .line_of(s.editor.windows.current().point);
    let row = s
        .editor
        .git_diff_view()
        .expect("a diff view")
        .row(now)
        .cloned();
    assert_eq!(
        row,
        Some(maxgus_core::git::DiffRow::File(1)),
        "point left the second file"
    );
}

#[cfg(feature = "full")]
#[test]
fn q_kills_the_magit_buffer_rather_than_leaving_it_behind() {
    // Magit's views are scratch views. Buried, they pile up in `C-x b` and
    // have to be killed by hand, which is what a reader ends up doing.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Commit { .. })
    });
    s.keys("RET");
    deliver_revision(&mut s);
    assert_eq!(s.editor.current_buffer().name(), "magit: revision");

    s.keys("q");
    assert!(
        s.editor.buffers.find_by_name("magit: revision").is_none(),
        "`q` left the revision buffer behind: {:?}",
        buffer_names(&s)
    );
    // And it goes back to where the revision was opened from.
    assert_eq!(s.editor.current_buffer().name(), "magit: status");

    s.keys("q");
    assert!(
        s.editor.buffers.find_by_name("magit: status").is_none(),
        "`q` left the status buffer behind: {:?}",
        buffer_names(&s)
    );
    assert_eq!(
        s.editor.current_buffer().name(),
        "a.rs",
        "it did not go back to the file"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_prefix_argument_keeps_the_magit_buffer() {
    // The way back for anyone who wants the view kept: `C-u q` buries it,
    // which is what `q` used to do on its own.
    let mut s = with_git();
    s.keys("C-u q");
    assert!(
        s.editor.buffers.find_by_name("magit: status").is_some(),
        "`C-u q` killed the buffer: {:?}",
        buffer_names(&s)
    );
    assert_ne!(
        s.editor.current_buffer().name(),
        "magit: status",
        "it stayed"
    );
}

#[cfg(feature = "full")]
#[test]
fn quitting_magit_twice_over_does_not_run_out_of_buffers() {
    // `q` on the last buffer standing has nothing to fall back to. It has to
    // report that rather than kill the only buffer there is.
    let mut s = with_git();
    while s.editor.buffers.ids().len() > 1 {
        let id = s.editor.current_buffer_id();
        if s.editor.kill_buffer(id).is_err() {
            break;
        }
    }
    s.keys("C-x g");
    refresh_git(&mut s);
    while s.editor.buffers.find_by_name("magit: status").is_some() {
        let before = s.editor.buffers.ids().len();
        s.keys("q");
        if s.editor.buffers.ids().len() == before {
            break; // it refused, which is the point
        }
    }
    assert!(
        !s.editor.buffers.ids().is_empty(),
        "every buffer was killed"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_commit_message_buffer_goes_when_the_commit_is_made() {
    // Left behind it is a stale message sitting in `C-x b`, and the next
    // commit opens on top of it.
    let mut s = with_git();
    s.keys("c");
    s.keys("c");
    assert_eq!(s.editor.current_buffer().name(), "COMMIT_EDITMSG");
    s.type_text("a message");
    s.keys("C-c C-c");
    assert!(
        s.editor.buffers.find_by_name("COMMIT_EDITMSG").is_none(),
        "the message buffer outlived the commit: {:?}",
        buffer_names(&s)
    );
}

#[cfg(feature = "full")]
#[test]
fn the_commit_message_buffer_goes_when_the_commit_is_abandoned() {
    let mut s = with_git();
    s.keys("c");
    s.keys("c");
    s.type_text("half a thought");
    s.keys("C-c C-k");
    assert!(
        s.editor.buffers.find_by_name("COMMIT_EDITMSG").is_none(),
        "abandoning kept the message: {:?}",
        buffer_names(&s)
    );
    // And the next commit starts empty rather than on top of it.
    s.keys("c");
    s.keys("c");
    assert_eq!(
        s.editor.current_buffer().text().trim(),
        "",
        "the old message came back"
    );
}

#[cfg(feature = "full")]
#[test]
fn killing_a_buffer_takes_the_keymap_of_the_one_left_showing() {
    // The window shows a different buffer afterwards, which is as much a
    // change of buffer as switching to it. Without it the dead buffer's map
    // is still in effect and `q` in the magit buffer underneath types a `q`.
    let mut s = with_git();
    s.editor
        .buffers
        .visit_file("/project/notes.txt", "a note\n");
    let id = s
        .editor
        .buffers
        .find_by_name("notes.txt")
        .expect("the note");
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.kill_buffer(id).unwrap();

    assert_eq!(s.editor.current_buffer().name(), "magit: status");
    s.keys("q");
    assert!(
        s.editor.buffers.find_by_name("magit: status").is_none(),
        "`q` did not reach the magit map: {:?}",
        buffer_names(&s)
    );
}

#[cfg(feature = "full")]
#[test]
fn the_log_menu_opens_a_log_buffer() {
    let mut s = with_git();
    s.editor.tasks.drain();
    s.keys("l");
    s.keys("l");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Log { arguments, title }] => {
            assert!(title.contains("main"), "got `{title}`");
            assert!(arguments.contains(&"main".to_string()));
        }
        other => panic!("expected a log, got {other:?}"),
    }

    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitLog {
            title: "Log main".into(),
            commits: maxgus_git::log::parse_log(
                "abc\u{1f}abc1234\u{1f}Someone\u{1f}an hour ago\u{1f}HEAD -> main\u{1f}a change\u{1e}\n",
            ),
        })
        .unwrap();
    assert_eq!(s.editor.current_buffer().name(), "magit: log");
    assert!(
        s.screen()
            .iter()
            .any(|line| line.contains("abc1234") && line.contains("a change"))
    );
}

#[cfg(feature = "full")]
#[test]
fn return_in_a_log_shows_the_commit_that_line_is() {
    let mut s = with_git();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitLog {
            title: "Log".into(),
            commits: maxgus_git::log::parse_log(
                "thehash\u{1f}abc1234\u{1f}Someone\u{1f}an hour ago\u{1f}\u{1f}a change\u{1e}\n",
            ),
        })
        .unwrap();
    s.editor.tasks.drain();
    s.keys("RET");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Show { revision }] => assert_eq!(revision, "thehash"),
        other => panic!("expected a show, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn the_references_view_lists_branches_and_tags_apart() {
    let mut s = with_git();
    s.keys("y");
    assert_eq!(s.editor.current_buffer().name(), "magit: refs");
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("Branches"), "no branches heading:\n{screen:#?}");
    assert!(has("Remotes"), "no remotes heading");
    assert!(has("Tags"), "no tags heading");
    assert!(has("* main"), "the current branch is not marked");
    // `feature/x` is a local branch with a slash; it must not be filed as a
    // remote.
    let branches = screen
        .iter()
        .position(|line| line.contains("Branches"))
        .unwrap();
    let remotes = screen
        .iter()
        .position(|line| line.contains("Remotes"))
        .unwrap();
    let feature = screen
        .iter()
        .position(|line| line.contains("feature/x"))
        .unwrap();
    assert!(
        feature > branches && feature < remotes,
        "a local branch was listed as a remote"
    );
}

#[cfg(feature = "full")]
#[test]
fn return_in_the_references_view_checks_that_branch_out() {
    let mut s = with_git();
    s.keys("y");
    // Down to the first branch.
    while !s.editor.current_buffer().text().is_empty() && s.editor.git_list_target().is_none() {
        s.keys("C-n");
    }
    s.editor.tasks.drain();
    s.keys("RET");
    match &git_tasks(&mut s)[..] {
        [maxgus_core::task::GitAction::Checkout(name)] => assert_eq!(name, "main"),
        other => panic!("expected a checkout, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn the_process_buffer_shows_what_git_was_asked_to_do() {
    let mut s = with_git();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GitDone {
            action: "Stage".into(),
            command: "git add -- src/a.rs".into(),
            output: "".into(),
        })
        .unwrap();
    s.keys("$");
    assert_eq!(s.editor.current_buffer().name(), "magit: process");
    assert!(
        s.screen()
            .iter()
            .any(|line| line.contains("git add -- src/a.rs")),
        "the command is not listed:\n{:#?}",
        s.screen()
    );
}

#[cfg(feature = "full")]
#[test]
fn n_and_p_move_by_section_rather_than_by_line() {
    // Stepping through the lines of a hunk one at a time is what `C-n` is
    // for; `n` is for getting about.
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("TAB");
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { hunk: 0, .. })
    });

    s.keys("n");
    assert!(
        matches!(
            s.editor.git_row_at_cursor(),
            Some(maxgus_core::git::Row::Hunk { hunk: 1, .. })
        ),
        "`n` stopped inside the hunk: {:?}",
        s.editor.git_row_at_cursor()
    );

    // `C-n` does step into it.
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { hunk: 0, .. })
    });
    s.keys("C-n");
    assert!(
        matches!(
            s.editor.git_row_at_cursor(),
            Some(maxgus_core::git::Row::Line { .. })
        ),
        "`C-n` did not move by line"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_caret_goes_out_to_what_contains_this() {
    let mut s = with_git();
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::File { section, .. }
        if *section == maxgus_core::git::Section::Unstaged)
    });
    s.keys("TAB");
    go_to_git(&mut s, |row| {
        matches!(row, maxgus_core::git::Row::Hunk { hunk: 1, .. })
    });

    s.keys("^");
    assert!(
        matches!(
            s.editor.git_row_at_cursor(),
            Some(maxgus_core::git::Row::File { .. })
        ),
        "`^` did not go out to the file: {:?}",
        s.editor.git_row_at_cursor()
    );
    s.keys("^");
    assert!(
        matches!(
            s.editor.git_row_at_cursor(),
            Some(maxgus_core::git::Row::Section(_))
        ),
        "`^` did not go out to the section"
    );
}

#[cfg(feature = "full")]
#[test]
fn magit_is_reachable_by_the_name_a_person_types() {
    // In Emacs `magit` is an alias for `magit-status`. Someone reaching for
    // it should not have to discover that this one spells it differently.
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("magit");
    s.keys("RET");
    let said = s.echo();
    assert_eq!(
        s.editor.current_buffer().name(),
        "magit: status",
        "`M-x magit` did not open the status view; got `{said}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn magit_and_magit_status_are_the_same_command() {
    let registry = maxgus_core::standard_registry();
    for name in ["magit", "magit-status", "magit-dispatch"] {
        assert!(registry.contains(name), "`M-x {name}` is not a command");
    }
}

// ---- the mode line ------------------------------------------------------

#[cfg(feature = "full")]
#[test]
fn the_mode_line_says_how_big_the_buffer_is_and_where_the_file_is() {
    // Three buffers called `mod.rs` are told apart by where they are, which
    // is exactly what a bare file name leaves out.
    let mut s = tall_session("/project/src/deep/mod.rs", "fn main() {}\n");
    s.editor.git_root = Some("/project".into());
    let bar = s.mode_line();
    assert!(
        bar.contains("project/src/deep/mod.rs"),
        "no project path: `{bar}`"
    );
    assert!(bar.contains("13"), "no size: `{bar}`");
}

#[cfg(feature = "full")]
#[test]
fn a_file_outside_the_project_keeps_its_bare_name() {
    // An absolute path is usually longer than the bar and tells the reader
    // nothing they wanted.
    let mut s = tall_session("/elsewhere/notes.txt", "hello\n");
    s.editor.git_root = Some("/project".into());
    let bar = s.mode_line();
    assert!(bar.contains("notes.txt"), "got `{bar}`");
    assert!(
        !bar.contains("/elsewhere"),
        "the absolute path leaked in: `{bar}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_branch_and_the_language_sit_against_the_right_edge() {
    // What is being edited is on the left, where the eye starts; what the
    // editor knows about it is on the right, to be glanced at.
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.editor.git_branch = Some("main".into());
    let bar = s.mode_line();
    let trimmed = bar.trim_end();
    assert!(
        trimmed.ends_with("rust"),
        "the language is not at the edge: `{bar}`"
    );
    let branch = trimmed.rfind("main").expect("the branch");
    let name = trimmed.find("main.rs").expect("the file name");
    assert!(
        branch > name,
        "the branch is not to the right of the file: `{bar}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_narrow_bar_keeps_the_file_and_drops_the_rest() {
    // The file being edited is what must survive a narrow window; the two
    // halves overlapping would leave neither readable.
    let mut s = Session::new(28, 10);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.git_branch = Some("a-very-long-branch-name".into());
    let bar = s.mode_line();
    assert!(bar.contains("main.rs"), "the file was pushed out: `{bar}`");
    assert!(
        !bar.contains("a-very-long-branch-name"),
        "the two halves overlapped: `{bar}`"
    );
}

#[cfg(feature = "full")]
#[test]
fn the_outline_window_appears_when_a_server_starts_later() {
    // Which windows the column has is decided when it is built, so a server
    // that starts after the panel was opened would otherwise never get one.
    let mut s = tall_session("/project/main.rs", "fn one() {}\n");
    s.keys("C-x t t");
    assert!(
        s.editor.buffers.find_by_name("*symbols*").is_none(),
        "an outline with no server"
    );
    let before = s.editor.panel_windows.len();

    start_server(&mut s);
    assert_eq!(
        s.editor.panel_windows.len(),
        before + 1,
        "no outline window appeared"
    );
    let id = s
        .editor
        .buffers
        .find_by_name("*symbols*")
        .expect("the outline buffer");
    assert!(!s.editor.windows.showing(id).is_empty(), "it has no window");
}

#[cfg(feature = "full")]
#[test]
fn the_outline_window_goes_when_its_server_does() {
    let mut s = with_panel();
    assert_eq!(s.editor.panel_windows.len(), 3);
    s.editor
        .apply_task_result(maxgus_core::TaskResult::LanguageServerStopped {
            language: "rust".into(),
        })
        .unwrap();
    assert_eq!(
        s.editor.panel_windows.len(),
        2,
        "the outline window outlived its server"
    );
}

#[test]
fn the_configuration_file_is_opened_by_a_key() {
    // The usual reason to reach for this is that there is no configuration
    // yet, so it opens the file rather than complaining about it.
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.editor.config_path = Some("/home/someone/.config/maxgus/config.kdl".into());
    s.editor.tasks.drain();

    s.keys("C-c f p");
    let tasks = s.editor.tasks.drain();
    assert!(
        tasks.iter().any(|t| matches!(t, Task::ReadFile { path, .. }
            if path.ends_with("maxgus/config.kdl"))),
        "the configuration was not opened: {tasks:?}"
    );
}

#[test]
fn opening_the_configuration_twice_shows_the_buffer_it_already_has() {
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    let path = "/home/someone/.config/maxgus/config.kdl";
    s.editor.config_path = Some(path.into());
    s.editor.buffers.visit_file(path, "set tab-width=4\n");
    s.editor.tasks.drain();

    s.keys("C-c f p");
    assert_eq!(s.editor.current_buffer().name(), "config.kdl");
    assert!(s.editor.tasks.drain().is_empty(), "it read the file again");
}

#[cfg(feature = "full")]
#[test]
fn each_panel_window_is_reached_by_its_own_key() {
    let mut s = with_panel();
    s.keys("C-x t 2");
    assert_eq!(s.editor.current_buffer().name(), "*symbols*");
    s.keys("C-x t 3");
    assert_eq!(s.editor.current_buffer().name(), "*buffers*");
    s.keys("C-x t 1");
    assert_eq!(s.editor.current_buffer().name(), "*treefile*");
}

#[test]
fn asking_for_the_outline_with_no_server_says_so() {
    // Not a panic and not a silent no-op: the section is on, but with no
    // server there is no window to go to.
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.keys("C-x t t");
    s.keys("C-x t 2");
    assert!(
        s.echo().contains("outline is not shown"),
        "no explanation: `{}`",
        s.echo()
    );
    // And it left the selection alone rather than dropping it somewhere else.
    assert_eq!(s.editor.current_buffer().name(), "main.rs");
}

#[cfg(feature = "full")]
#[test]
fn a_short_frame_still_leaves_the_tree_something_to_show() {
    // Configured heights are what the user wants, not what the frame has. A
    // 12-row outline and an 8-row list in a 16-row frame would take every row
    // and leave the tree — the window the panel exists for — nothing.
    let mut s = Session::new(90, 16);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn one() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.symbols_height = 12;
    s.editor.buffers_height = 8;
    start_server(&mut s);
    s.keys("C-x t t");
    deliver_symbols(&mut s);

    let rects: Vec<_> = s
        .editor
        .panel_windows
        .iter()
        .map(|id| s.editor.windows.get(*id).expect("a panel window").rect)
        .collect();
    assert_eq!(rects.len(), 3, "not every section got a window");
    for rect in &rects {
        assert!(rect.height >= 3, "a panel window got {} rows", rect.height);
    }
    // And they still fit: the column does not run off the bottom.
    let bottom = rects.last().expect("three windows").bottom();
    assert!(
        bottom <= s.editor.frame.height,
        "the column overflows the frame"
    );
}

#[cfg(feature = "full")]
/// The two numbers the README quotes, counted the way a reader would check
/// them: what `C-h b` lists, and what `M-x` offers.
///
/// A count of the whole editor, so it is a claim about the full build.
#[cfg(feature = "full")]
#[test]
fn the_readme_quotes_the_right_totals() {
    let s = tall_session("/project/main.rs", "fn main() {}\n");
    let mut bindings = s.editor.keymaps.bindings().len();
    // Plus the maps that only a particular buffer has.
    bindings += maxgus_tree::keymap::treemacs_keymap()
        .expect("the tree map")
        .bindings()
        .len();
    for map in [
        maxgus_core::keymap::symbols_keymap(),
        maxgus_core::keymap::buffers_keymap(),
        maxgus_core::keymap::magit_keymap(),
        maxgus_core::keymap::terminal_keymap(),
    ] {
        bindings += map.expect("a keymap").bindings().len();
    }
    let commands = s.dispatcher.registry.len();
    assert_eq!(
        (bindings, commands),
        (README_BINDINGS, README_COMMANDS),
        "the README says {README_BINDINGS} bindings and {README_COMMANDS} commands"
    );
}

#[cfg(feature = "full")]
const README_BINDINGS: usize = 401;
#[cfg(feature = "full")]
const README_COMMANDS: usize = 459;

#[cfg(feature = "full")]
#[test]
fn a_second_outline_answer_does_not_open_a_listing_over_the_file() {
    // Two requests can be in flight at once — the panel refreshing itself
    // after a rebuild, and the panel refreshing itself again on a buffer
    // switch. Both answers are for the panel; neither is a person asking for
    // `*xref*`, and one arriving after the other must not become one.
    let mut s = with_panel();
    s.keys("C-<right>"); // out of the panel, into the file
    let before = s.editor.current_buffer().name().to_string();

    deliver_symbols(&mut s);
    deliver_symbols(&mut s);

    assert_eq!(
        s.editor.current_buffer().name(),
        before,
        "an answer replaced the file being edited"
    );
    assert!(
        s.editor.buffers.find_by_name("*xref*").is_none(),
        "the panel's own answer opened a listing"
    );
}

#[cfg(feature = "full")]
#[test]
fn asking_for_the_symbol_listing_still_opens_it() {
    // The other half: `M-x lsp-document-symbols` is a person asking, and it
    // still gets the listing even while the panel is refreshing itself.
    let mut s = with_panel();
    s.keys("C-<right>"); // out of the panel, into the file
    s.editor.tasks.drain();
    s.dispatcher
        .execute(&mut s.editor, "lsp-document-symbols", None);
    let query = s
        .editor
        .tasks
        .drain()
        .into_iter()
        .find_map(|t| match t {
            Task::LspRequest { query, .. } => Some(query),
            _ => None,
        })
        .expect("a request went out");
    assert!(
        matches!(
            query,
            maxgus_core::task::LspQuery::DocumentSymbols { for_panel: false }
        ),
        "the command asked as if it were the panel: {query:?}"
    );

    s.editor.apply_lsp_response(maxgus_core::TaskResult::LspResponse {
        language: "rust".into(),
        uri: "file:///project/main.rs".into(),
        query,
        result: serde_json::json!([{
            "name": "one", "kind": 12,
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 10}},
            "selectionRange": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}},
        }]),
    });
    assert_eq!(
        s.editor.current_buffer().name(),
        "*xref*",
        "no listing appeared"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_listing_never_takes_over_a_panel_window() {
    // Point is in the buffer list when the answer arrives. The listing has to
    // go to the file's window: shown here it would replace the list, and the
    // next rebuild would find a window in the column that is not the panel's.
    let mut s = with_panel();
    select_panel_window(&mut s, "*buffers*");

    s.editor.apply_lsp_response(maxgus_core::TaskResult::LspResponse {
        language: "rust".into(),
        uri: "file:///project/main.rs".into(),
        query: maxgus_core::task::LspQuery::DocumentSymbols { for_panel: false },
        result: serde_json::json!([{
            "name": "one", "kind": 12,
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 10}},
            "selectionRange": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}},
        }]),
    });

    assert_eq!(s.editor.current_buffer().name(), "*xref*");
    assert!(
        !s.editor
            .panel_windows
            .contains(&s.editor.windows.current_id()),
        "the listing took over a panel window"
    );
    // Read the column as it stands: showing the listing rebuilds it, so the
    // windows are not the ones it had a moment ago.
    for window in s.editor.panel_windows.clone() {
        let buffer = s.editor.windows.get(window).expect("a panel window").buffer;
        let name = s
            .editor
            .buffers
            .get(buffer)
            .expect("its buffer")
            .name()
            .to_string();
        assert!(
            maxgus_core::commands::tree::PANEL_BUFFERS.contains(&name.as_str()),
            "a panel window is showing `{name}`"
        );
    }
}

#[cfg(feature = "full")]
fn buffer_names(s: &Session) -> Vec<String> {
    s.editor
        .buffers
        .iter()
        .map(|b| b.name().to_string())
        .collect()
}

#[test]
fn holding_down_walks_the_whole_list_and_comes_back_round() {
    // The list as a person drives it: `M-x`, then `<down>` past the bottom of
    // the box, on to the end of the list, and round to the top again — with
    // the highlight on screen the whole way.
    let mut s = Session::new(100, 16);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("M-x");
    s.type_text("buffer");
    let total = s.editor.minibuffer.completion().len();
    assert!(
        total > s.editor.completion_rows(),
        "the list should not fit"
    );

    // `total` presses from the first row walks every row and wraps back to it.
    for step in 0..total {
        let selected = s
            .editor
            .minibuffer
            .completion()
            .current()
            .expect("something is highlighted")
            .to_string();
        let screen = s.screen();
        // The name column is narrow, so a long name is drawn abbreviated;
        // what has to be on screen is the row, not every character of it.
        let shown: String = selected.chars().take(18).collect();
        assert!(
            screen.iter().skip(2).any(|line| line.contains(&shown)),
            "step {step}: `{selected}` is highlighted but not drawn:\n{screen:#?}"
        );
        s.keys("<down>");
    }
    // All the way round: back where it started.
    assert_eq!(s.editor.minibuffer.completion().selected, Some(0));

    // And the same walking up.
    for step in 0..total {
        s.keys("<up>");
        let selected = s
            .editor
            .minibuffer
            .completion()
            .current()
            .expect("something is highlighted")
            .to_string();
        let screen = s.screen();
        let shown: String = selected.chars().take(18).collect();
        assert!(
            screen.iter().skip(2).any(|line| line.contains(&shown)),
            "step {step} going up: `{selected}` is not drawn:\n{screen:#?}"
        );
    }
    assert_eq!(s.editor.minibuffer.completion().selected, Some(0));
}

#[test]
fn a_page_at_a_time_keeps_the_highlight_on_screen_too() {
    let mut s = Session::new(100, 16);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("M-x");
    s.type_text("buffer");
    for _ in 0..4 {
        s.keys("<next>");
        let selected = s
            .editor
            .minibuffer
            .completion()
            .current()
            .expect("something is highlighted")
            .to_string();
        let screen = s.screen();
        assert!(
            screen.iter().skip(2).any(|line| line.contains(&selected)),
            "`{selected}` is highlighted but not drawn:\n{screen:#?}"
        );
    }
}

/// The column the completion popup's left border is on. The box is centred,
/// so this is where the rows inside it start rather than column zero.
fn popup_left(s: &mut Session) -> usize {
    let screen = s.screen();
    screen[0]
        .chars()
        .position(|c| c == '\u{256d}')
        .unwrap_or_else(|| panic!("no popup on screen:\n{screen:#?}"))
}

// ---- what a pointer means ------------------------------------------------

#[test]
fn a_click_puts_point_on_the_character_under_it() {
    let mut s = Session::new(60, 10);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "first line\nsecond line\nthird line\n");
    s.editor.switch_to_buffer(id).unwrap();

    // Row 1, column 3: the `o` of `second`.
    assert!(s.editor.point_at_cell(3, 1), "the cell is in a window");
    let point = s.editor.windows.current().point;
    assert_eq!(
        s.editor.current_buffer().text()[point..point + 1].to_string(),
        "o"
    );
}

#[test]
fn a_click_past_the_end_of_a_line_lands_at_its_end() {
    let mut s = Session::new(60, 10);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "ab\nlonger line\n");
    s.editor.switch_to_buffer(id).unwrap();

    s.editor.point_at_cell(40, 0);
    let point = s.editor.windows.current().point;
    assert_eq!(point, 2, "point should be at the end of `ab`, not past it");
}

#[test]
fn a_click_below_the_text_lands_at_the_end_of_the_buffer() {
    let mut s = Session::new(60, 10);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "one\ntwo\n");
    s.editor.switch_to_buffer(id).unwrap();

    s.editor.point_at_cell(0, 6);
    assert_eq!(
        s.editor.windows.current().point,
        s.editor.current_buffer().len_chars()
    );
}

#[test]
fn a_click_on_a_mode_line_or_the_echo_area_is_not_a_click_on_text() {
    let mut s = Session::new(60, 10);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "one\ntwo\n");
    s.editor.switch_to_buffer(id).unwrap();
    let before = s.editor.windows.current().point;

    // The window's last row is its mode line; the frame's is the echo area.
    let mode_line = s.editor.windows.current().rect.bottom() - 1;
    assert!(
        !s.editor.point_at_cell(0, mode_line),
        "the mode line is not text"
    );
    assert!(!s.editor.point_at_cell(0, 9), "the echo area is not text");
    assert_eq!(
        s.editor.windows.current().point,
        before,
        "point moved anyway"
    );
}

#[test]
fn a_click_in_another_window_selects_it() {
    let mut s = Session::new(60, 20);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "one\ntwo\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("C-x 2"); // split
    let first = s.editor.windows.current_id();
    let other = s
        .editor
        .windows
        .ids()
        .into_iter()
        .find(|w| *w != first)
        .expect("two windows");
    let rect = s.editor.windows.get(other).expect("the other").rect;

    assert!(s.editor.point_at_cell(rect.x, rect.y));
    assert_eq!(s.editor.windows.current_id(), other, "it stayed put");
}

#[test]
fn a_drag_stays_in_the_window_it_started_in() {
    let mut s = Session::new(60, 20);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "one\ntwo\nthree\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("C-x 2");
    let first = s.editor.windows.current_id();
    let other = s
        .editor
        .windows
        .ids()
        .into_iter()
        .find(|w| *w != first)
        .expect("two windows");
    let elsewhere = s.editor.windows.get(other).expect("the other").rect;

    s.editor.point_at_cell(0, 0);
    s.editor.set_mark_here();
    assert!(
        !s.editor.extend_to_cell(elsewhere.x, elsewhere.y),
        "the drag reached into another window"
    );
    assert_eq!(s.editor.windows.current_id(), first, "it changed window");
}

#[test]
fn a_drag_selects_what_it_covers() {
    let mut s = Session::new(60, 10);
    let id = s.editor.buffers.visit_file("/project/main.rs", "abcdef\n");
    s.editor.switch_to_buffer(id).unwrap();

    s.editor.point_at_cell(1, 0);
    s.editor.set_mark_here();
    s.editor.extend_to_cell(4, 0);
    assert_eq!(s.editor.region_text().as_deref(), Some("bcd"));
}

#[test]
fn the_wheel_moves_the_view_and_drags_point_along_when_it_has_to() {
    let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(60, 12);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.with_current_buffer(|b| b.set_point(0));

    s.editor.scroll_lines(30);
    assert_eq!(
        s.editor.windows.current().top_line,
        30,
        "the view did not move"
    );
    let line = {
        let point = s.editor.windows.current().point;
        s.editor.current_buffer().line_of(point)
    };
    assert!(
        (30..30 + 12).contains(&line),
        "point was left off screen at line {line}"
    );
}

#[test]
fn the_wheel_scrolls_the_window_it_is_over_rather_than_the_one_being_typed_in() {
    // What a window with a mouse in it has to do: a turn of the wheel over
    // the file tree moves the file tree, not the code beside it.
    let long: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(80, 20);
    let one = s.editor.buffers.visit_file("/project/one.rs", &long);
    s.editor.switch_to_buffer(one).unwrap();
    s.keys("C-x 2");
    let other = s.editor.windows.current_id();
    let two = s.editor.buffers.visit_file("/project/two.rs", &long);
    s.editor.switch_to_buffer(two).unwrap();
    s.keys("C-x o");
    let selected = s.editor.windows.current_id();
    assert_ne!(selected, other, "the split did not leave two windows");

    s.editor.scroll_window_lines(other, 20);
    assert_eq!(
        s.editor.windows.get(other).unwrap().top_line,
        20,
        "the window under the pointer did not move"
    );
    assert_eq!(
        s.editor.windows.get(selected).unwrap().top_line,
        0,
        "it scrolled the selected window instead"
    );
    assert_eq!(
        s.editor.windows.current_id(),
        selected,
        "scrolling a window selected it"
    );
}

#[test]
fn the_wheel_stops_at_the_ends_rather_than_scrolling_into_nothing() {
    let mut s = Session::new(60, 12);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "one\ntwo\n");
    s.editor.switch_to_buffer(id).unwrap();

    s.editor.scroll_lines(-100);
    assert_eq!(s.editor.windows.current().top_line, 0);
    s.editor.scroll_lines(1000);
    assert!(
        s.editor.windows.current().top_line < s.editor.current_buffer().len_lines(),
        "it scrolled past the end of the buffer"
    );
}

#[test]
fn a_file_is_handed_to_the_desktop_to_open() {
    let mut s = tall_session("/project/diagram.png", "");
    s.editor.tasks.drain();
    s.keys("C-c o b");
    let tasks = s.editor.tasks.drain();
    match &tasks[..] {
        [Task::Shell { command, .. }] => {
            assert!(
                command.contains("'/project/diagram.png'"),
                "the path is not quoted into the command: `{command}`"
            );
        }
        other => panic!("expected one shell command, got {other:?}"),
    }
}

#[test]
fn a_buffer_with_no_file_says_so_rather_than_opening_nothing() {
    let mut s = Session::new(60, 10);
    s.editor.tasks.drain();
    s.keys("C-c o b");
    assert!(s.echo().contains("no file"), "got `{}`", s.echo());
    assert!(s.editor.tasks.drain().is_empty(), "it ran something anyway");
}

// ---- project-wide search -------------------------------------------------

#[cfg(feature = "full")]
fn found(hits: &[(&str, usize, &str)]) -> maxgus_grep::Found {
    maxgus_grep::Found {
        hits: hits
            .iter()
            .map(|(path, line, text)| maxgus_grep::Hit {
                path: std::path::PathBuf::from(path),
                line: *line,
                column: 0,
                length: 1,
                text: text.to_string(),
            })
            .collect(),
        files_searched: 12,
        truncated: false,
    }
}

#[cfg(feature = "full")]
fn with_grep() -> Session {
    let mut s = tall_session("/project/src/a.rs", "fn alpha() {}\nfn beta() {}\n");
    s.editor.tree_root = Some("/project".into());
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GrepFinished {
            pattern: "alpha".into(),
            found: found(&[
                ("/project/src/a.rs", 0, "fn alpha() {}"),
                ("/project/src/b.rs", 3, "// alpha again"),
            ]),
        })
        .unwrap();
    s.editor.tasks.drain();
    s
}

#[cfg(feature = "full")]
#[test]
fn a_search_asks_for_a_pattern_and_offers_the_word_at_point() {
    let mut s = tall_session("/project/src/a.rs", "fn alpha() {}\n");
    s.editor.with_current_buffer(|b| b.set_point(5));
    s.keys("M-s g");
    assert!(s.editor.minibuffer.is_active(), "no prompt");
    assert!(
        s.editor.minibuffer.prompt().contains("default alpha"),
        "the word at point was not offered: `{}`",
        s.editor.minibuffer.prompt()
    );
    // Offered, not typed in: a different search does not have to be cleared
    // out of the prompt first.
    assert_eq!(s.editor.minibuffer.input(), "");

    // And an empty answer takes it.
    s.editor.tree_root = Some("/project".into());
    s.editor.tasks.drain();
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::Grep { search, .. }] => assert_eq!(search.pattern, "alpha"),
        other => panic!("expected a search for the default, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn a_pattern_becomes_a_search_of_the_project() {
    let mut s = tall_session("/project/src/a.rs", "fn alpha() {}\n");
    s.editor.tree_root = Some("/project".into());
    s.editor.tasks.drain();
    s.keys("M-s g");
    s.type_text("beta");
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::Grep { root, search }] => {
            assert_eq!(root, std::path::Path::new("/project"));
            assert_eq!(search.pattern, "beta");
            assert!(search.regexp, "`M-s g` searches by regexp");
        }
        other => panic!("expected a search, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn the_results_are_a_buffer_of_files_and_lines() {
    let mut s = with_grep();
    assert_eq!(s.editor.current_buffer().name(), "*grep*");
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("2 matches for `alpha`"), "no summary:\n{screen:#?}");
    assert!(has("/project/src/a.rs"), "no first file");
    assert!(has("fn alpha() {}"), "no first line");
    assert!(has("/project/src/b.rs"), "no second file");
}

#[cfg(feature = "full")]
#[test]
fn point_starts_on_a_result_and_n_walks_them() {
    let mut s = with_grep();
    let line_now = |s: &Session| {
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point)
    };
    let first = line_now(&s);
    assert!(
        matches!(
            s.editor.grep.as_ref().unwrap().row(first),
            Some(maxgus_core::grep::Row::Hit(_, _))
        ),
        "it opened on line {first}, which is not a result"
    );
    s.keys("n");
    let second = line_now(&s);
    assert!(second > first, "`n` did not move forward");
    assert!(
        matches!(
            s.editor.grep.as_ref().unwrap().row(second),
            Some(maxgus_core::grep::Row::Hit(_, _))
        ),
        "`n` stopped on a heading"
    );
    s.keys("p");
    assert_eq!(line_now(&s), first, "`p` did not come back");
}

#[cfg(feature = "full")]
#[test]
fn return_opens_the_file_at_the_line_that_matched() {
    let mut s = with_grep();
    s.keys("n"); // the hit in b.rs, which is not open
    s.keys("n");
    s.editor.tasks.drain();
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::ReadFile { path, .. }] => {
            assert!(path.ends_with("b.rs"), "wrong file: {path:?}");
        }
        other => panic!("expected a read, got {other:?}"),
    }
    assert_eq!(
        s.editor.pending_line.as_ref().map(|(_, line)| *line),
        Some(3),
        "it did not aim at the matching line"
    );
}

#[cfg(feature = "full")]
#[test]
fn return_on_a_file_already_open_goes_straight_to_it() {
    let mut s = with_grep();
    s.editor.tasks.drain();
    s.keys("RET"); // the hit in a.rs, which is open
    assert_eq!(s.editor.current_buffer().name(), "a.rs");
    assert!(s.editor.tasks.drain().is_empty(), "it read a file it had");
}

#[cfg(feature = "full")]
#[test]
fn the_results_are_read_only_until_they_are_made_editable() {
    let mut s = with_grep();
    s.type_text("x");
    assert!(
        s.echo().contains("read-only"),
        "the results took a keystroke: `{}`",
        s.echo()
    );

    s.keys("C-c C-p");
    s.type_text("x");
    assert!(
        s.editor.current_buffer().text().contains('x'),
        "`C-c C-p` did not make the buffer writable"
    );
}

#[cfg(feature = "full")]
#[test]
fn an_edited_line_is_written_back_to_the_file_it_came_from() {
    let mut s = with_grep();
    s.keys("C-c C-p");
    let edited = s
        .editor
        .current_buffer()
        .text()
        .replace("fn alpha() {}", "fn renamed() {}");
    let id = s.editor.current_buffer_id();
    s.editor.replace_buffer_contents(id, &edited).unwrap();
    s.editor.tasks.drain();

    s.keys("C-c C-c");
    match &s.editor.tasks.drain()[..] {
        [Task::ApplyGrep { replacements }] => {
            assert_eq!(replacements.len(), 1);
            assert_eq!(replacements[0].now, "fn renamed() {}");
            assert_eq!(replacements[0].line, 0);
            assert!(replacements[0].path.ends_with("a.rs"));
        }
        other => panic!("expected the edits, got {other:?}"),
    }
}

#[cfg(feature = "full")]
#[test]
fn applying_with_nothing_changed_says_so_rather_than_writing() {
    let mut s = with_grep();
    s.keys("C-c C-p");
    s.editor.tasks.drain();
    s.keys("C-c C-c");
    assert!(
        s.echo().contains("Nothing was changed"),
        "got `{}`",
        s.echo()
    );
    assert!(s.editor.tasks.drain().is_empty(), "it wrote anyway");
}

#[cfg(feature = "full")]
#[test]
fn applying_before_editing_says_which_key_to_press() {
    let mut s = with_grep();
    s.editor.tasks.drain();
    s.keys("C-c C-c");
    assert!(s.echo().contains("C-c C-p"), "got `{}`", s.echo());
    assert!(s.editor.tasks.drain().is_empty());
}

#[cfg(feature = "full")]
#[test]
fn abandoning_puts_the_results_back_as_they_were() {
    let mut s = with_grep();
    let before = s.editor.current_buffer().text();
    s.keys("C-c C-p");
    s.type_text("nonsense");
    s.keys("C-c C-k");
    assert_eq!(s.editor.current_buffer().text(), before);
    s.type_text("x");
    assert!(s.echo().contains("read-only"), "it stayed writable");
}

#[cfg(feature = "full")]
#[test]
fn the_navigation_keys_become_letters_while_the_results_are_edited() {
    // `n`, `p`, `o`, `g` and `q` move around the results while they are being
    // read. A buffer being typed into needs them to be themselves, or a
    // rename to `renamed` runs five commands and inserts two letters.
    let mut s = with_grep();
    s.keys("C-c C-p");
    let before = s.editor.current_buffer().text();
    s.type_text("nqgop");
    let after = s.editor.current_buffer().text();
    assert!(
        after.contains("nqgop"),
        "the letters ran commands instead of being typed:\n{after}"
    );
    assert_ne!(after, before);

    // And they go back to being commands when the editing stops.
    s.keys("C-c C-k");
    let line = |s: &Session| {
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point)
    };
    let start = line(&s);
    s.type_text("n");
    assert!(line(&s) > start, "`n` did not go back to being a command");
}

#[cfg(feature = "full")]
#[test]
fn a_search_that_found_nothing_says_so_and_opens_no_buffer() {
    let mut s = tall_session("/project/src/a.rs", "fn alpha() {}\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GrepFinished {
            pattern: "zzz".into(),
            found: maxgus_grep::Found::default(),
        })
        .unwrap();
    assert!(s.echo().contains("No matches"), "got `{}`", s.echo());
    assert!(s.editor.buffers.find_by_name("*grep*").is_none());
}

#[cfg(feature = "full")]
#[test]
fn writing_the_files_re_reads_the_buffers_that_were_showing_them() {
    // A buffer left showing the old text of a file that was just rewritten
    // is how an edit gets undone by the next save.
    let mut s = with_grep();
    let id = s
        .editor
        .buffers
        .find_by_path(std::path::Path::new("/project/src/a.rs"));
    assert!(id.is_some(), "the fixture should have a.rs open");
    s.editor.tasks.drain();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::GrepApplied {
            applied: maxgus_grep::Applied { files: 1, lines: 1 },
            paths: vec!["/project/src/a.rs".into()],
        })
        .unwrap();
    match &s.editor.tasks.drain()[..] {
        [
            Task::ReadFile {
                path, reverting, ..
            },
        ] => {
            assert!(path.ends_with("a.rs"));
            assert_eq!(*reverting, id, "it did not revert the buffer");
        }
        other => panic!("expected a revert, got {other:?}"),
    }
}

// ---- the undo tree -------------------------------------------------------

/// Reports the write the editor just asked for as having happened, which is
/// what marks the buffer saved.
fn deliver_write(s: &mut Session) {
    let buffer = s.editor.current_buffer_id();
    let path = s
        .editor
        .current_buffer()
        .path()
        .expect("a file")
        .to_path_buf();
    let bytes = s.editor.current_buffer().text().len();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::FileWritten {
            path,
            buffer,
            bytes,
            disk_time: None,
        })
        .unwrap();
    s.editor.tasks.drain();
}

/// A buffer with two versions of its second word, one abandoned by an undo.
fn with_two_versions() -> Session {
    let mut s = tall_session("/project/main.rs", "");
    s.type_text("alpha ");
    s.keys("C-x C-s"); // a boundary, and the state on disk
    deliver_write(&mut s);
    s.type_text("first");
    s.keys("C-/"); // undo, abandoning `first`
    s.type_text("second");
    s
}

#[test]
fn typing_after_an_undo_keeps_the_version_it_replaced() {
    // Linear undo throws `first` away here; a tree keeps it.
    let mut s = with_two_versions();
    assert!(s.editor.current_buffer().text().contains("second"));

    s.keys("C-x U");
    assert_eq!(s.editor.current_buffer().name(), "*undo-tree*");
    let screen = s.screen();
    assert!(
        screen.iter().any(|line| line.contains("branches")),
        "the tree does not show a fork:\n{screen:#?}"
    );
}

#[test]
fn the_visualiser_opens_beside_the_buffer_rather_than_over_it() {
    let mut s = with_two_versions();
    s.keys("C-x U");
    assert!(s.editor.windows.len() >= 2, "it took the only window");
    let showing: Vec<String> = s
        .editor
        .windows
        .iter()
        .filter_map(|w| s.editor.buffers.get(w.buffer))
        .map(|b| b.name().to_string())
        .collect();
    assert!(showing.contains(&"main.rs".to_string()), "the file is gone");
    assert!(showing.contains(&"*undo-tree*".to_string()));
}

#[test]
fn moving_in_the_visualiser_moves_the_buffer_under_it() {
    let mut s = with_two_versions();
    let subject = s.editor.buffers.find_by_name("main.rs").expect("the file");
    let text = |s: &Session| {
        s.editor
            .buffers
            .get(subject)
            .expect("the file")
            .text()
            .to_string()
    };
    s.keys("C-x U");
    assert!(text(&s).contains("second"));

    s.keys("p"); // undo
    assert!(
        !text(&s).contains("second"),
        "the buffer did not move: {:?}",
        text(&s)
    );
    s.keys("n"); // redo
    assert!(text(&s).contains("second"), "it did not come back");
}

#[test]
fn the_other_branch_is_reachable_from_the_visualiser() {
    // The whole reason for a tree: `first` was undone past and typed over,
    // and it is still here.
    let mut s = with_two_versions();
    let subject = s.editor.buffers.find_by_name("main.rs").expect("the file");
    s.keys("C-x U");
    s.keys("p"); // back to the fork
    s.keys("b"); // the other way forward
    s.keys("n"); // and along it
    let text = s
        .editor
        .buffers
        .get(subject)
        .expect("the file")
        .text()
        .to_string();
    assert!(
        text.contains("first"),
        "the abandoned version is gone: {text:?}"
    );
}

#[test]
fn the_visualiser_marks_where_the_buffer_is_and_what_is_on_disk() {
    let mut s = with_two_versions();
    s.keys("C-x U");
    let screen = s.screen();
    let text: String = screen.join("\n");
    assert!(text.contains("← here"), "no current marker:\n{screen:#?}");
    assert!(text.contains("(on disk)"), "no saved marker:\n{screen:#?}");
}

#[test]
fn closing_the_visualiser_goes_back_to_the_buffer() {
    let mut s = with_two_versions();
    s.keys("C-x U");
    s.keys("q");
    assert_eq!(s.editor.current_buffer().name(), "main.rs");
    assert!(
        s.editor.buffers.find_by_name("*undo-tree*").is_none(),
        "the visualiser was left behind"
    );
}

#[test]
fn undoing_back_to_the_saved_state_stops_calling_the_buffer_modified() {
    let mut s = tall_session("/project/main.rs", "");
    s.type_text("alpha");
    s.keys("C-x C-s");
    deliver_write(&mut s);
    assert!(
        !s.editor.current_buffer().is_modified(),
        "the save did not take"
    );
    s.type_text(" beta");
    assert!(s.editor.current_buffer().is_modified());
    s.keys("C-/");
    assert!(
        !s.editor.current_buffer().is_modified(),
        "it is back to what is on disk and still says otherwise"
    );
}

#[test]
fn the_visualiser_on_a_buffer_with_no_history_still_opens() {
    let mut s = tall_session("/project/main.rs", "unchanged\n");
    s.keys("C-x U");
    assert_eq!(s.editor.current_buffer().name(), "*undo-tree*");
    assert!(
        s.screen().iter().any(|l| l.contains("0 change(s)")),
        "it did not say the history is empty:\n{:#?}",
        s.screen()
    );
}

// ---- several cursors -----------------------------------------------------

/// A buffer with the same name in three places.
fn with_three_names() -> Session {
    let mut s = tall_session(
        "/project/main.rs",
        "let alpha = 1;\nlet beta = alpha + 1;\nprintln!(\"{alpha}\");\n",
    );
    s.editor.with_current_buffer(|b| b.set_point(4));
    s
}

#[test]
fn marking_the_next_occurrence_makes_a_second_cursor() {
    let mut s = with_three_names();
    s.keys("C->");
    assert_eq!(s.editor.cursors.len(), 1, "no cursor was added");
    assert!(s.echo().contains("2 cursors"), "got `{}`", s.echo());
}

#[test]
fn typing_with_several_cursors_types_at_all_of_them() {
    let mut s = with_three_names();
    s.keys("C->");
    s.keys("C->");
    assert_eq!(s.editor.cursors.len(), 2, "three cursors expected");

    s.type_text("_x");
    let text = s.editor.current_buffer().text();
    assert_eq!(
        text.matches("alpha_x").count(),
        3,
        "not every cursor typed: {text:?}"
    );
}

#[test]
fn marking_them_all_at_once_reaches_every_occurrence() {
    let mut s = with_three_names();
    s.keys("C-c C-<");
    s.type_text("!");
    let text = s.editor.current_buffer().text();
    assert_eq!(text.matches("alpha!").count(), 3, "got {text:?}");
}

#[test]
fn deleting_with_several_cursors_deletes_at_all_of_them() {
    let mut s = with_three_names();
    s.keys("C-c C-<");
    // Point is at the end of each `alpha`; a backspace takes the `a`.
    s.keys("DEL");
    let text = s.editor.current_buffer().text();
    assert_eq!(text.matches("alph").count(), 3, "got {text:?}");
    assert!(
        !text.contains("alpha"),
        "an occurrence was left alone: {text:?}"
    );
}

#[test]
fn moving_with_several_cursors_moves_all_of_them() {
    let mut s = with_three_names();
    s.keys("C-c C-<");
    let before = s.editor.cursors.offsets().to_vec();
    s.keys("C-f");
    let after = s.editor.cursors.offsets().to_vec();
    assert_eq!(after.len(), before.len(), "a cursor was lost");
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(*b, a + 1, "a cursor did not move");
    }
}

#[test]
fn a_cursor_can_be_put_on_the_line_below() {
    let mut s = with_three_names();
    s.keys("C-c m <down>");
    assert_eq!(s.editor.cursors.len(), 1);
    s.keys("C-c m <down>");
    assert_eq!(
        s.editor.cursors.len(),
        2,
        "the second went to the same line"
    );
    // The buffer ends with a newline, so there is an empty last line to
    // reach; the one after that does not exist.
    s.keys("C-c m <down>");
    assert_eq!(s.editor.cursors.len(), 3);
    s.keys("C-c m <down>");
    assert!(s.echo().contains("No line below"), "got `{}`", s.echo());
}

#[test]
fn c_g_goes_back_to_one_cursor() {
    let mut s = with_three_names();
    s.keys("C-c C-<");
    assert!(!s.editor.cursors.is_empty());
    s.keys("C-g");
    assert!(s.editor.cursors.is_empty(), "the cursors stayed");
    assert!(s.echo().contains("One cursor"), "got `{}`", s.echo());
}

#[test]
fn a_command_that_cannot_be_run_everywhere_puts_the_cursors_away() {
    // Splitting a window five times is not what several cursors mean, and
    // silently doing it would be worse than stopping.
    let mut s = with_three_names();
    s.keys("C-c C-<");
    s.keys("C-x 2");
    assert!(
        s.editor.cursors.is_empty(),
        "the cursors survived a command that cannot use them"
    );
}

#[test]
fn the_extra_cursors_are_drawn() {
    let mut s = with_three_names();
    s.keys("C-c C-<");
    let cursor = s.editor.theme.resolve("cursor").background;
    let painted = (0..3)
        .flat_map(|y| (0..40).map(move |x| (x, y)))
        .filter(|(x, y)| s.face_at(*x, *y).background == cursor)
        .count();
    assert!(
        painted >= 2,
        "the extra cursors are not on screen: {painted} cells painted"
    );
}

#[test]
fn undoing_a_multi_cursor_edit_takes_back_every_part_of_it() {
    let mut s = with_three_names();
    let before = s.editor.current_buffer().text();
    s.keys("C-c C-<");
    s.type_text("!");
    assert_ne!(s.editor.current_buffer().text(), before);
    s.keys("C-g");
    for _ in 0..5 {
        s.keys("C-/");
    }
    assert_eq!(
        s.editor.current_buffer().text(),
        before,
        "the edit could not be taken back"
    );
}

#[test]
fn typing_twice_with_several_cursors_stays_lined_up() {
    // The second round is where the cursors have to have been kept up to
    // date: each edit moves every cursor after it, and the ones that have
    // already run are all after the one running now.
    let mut s = with_three_names();
    s.keys("C-c C-<");
    s.type_text("A");
    s.type_text("B");
    let text = s.editor.current_buffer().text();
    assert_eq!(
        text.matches("alphaAB").count(),
        3,
        "the cursors drifted between rounds: {text:?}"
    );
}

#[test]
fn deleting_twice_with_several_cursors_stays_lined_up() {
    // The same the other way: a deletion moves everything after it back.
    let mut s = with_three_names();
    s.keys("C-c C-<");
    s.keys("DEL");
    s.keys("DEL");
    let text = s.editor.current_buffer().text();
    assert_eq!(
        text.matches("alp").count(),
        3,
        "the cursors drifted between rounds: {text:?}"
    );
    assert!(!text.contains("alph"), "one was left behind: {text:?}");
}

// ---- what a project asks of a file --------------------------------------

/// Delivers a file with the properties its `.editorconfig` would have given.
fn read_with_config(
    s: &mut Session,
    path: &str,
    text: &str,
    asked: maxgus_core::task::EditorConfig,
) {
    s.editor
        .apply_task_result(maxgus_core::TaskResult::FileRead {
            path: path.into(),
            contents: text.into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: asked,
        })
        .unwrap();
    s.editor.tasks.drain();
}

#[test]
fn a_projects_indent_settings_win_over_the_configuration() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.tab_width = 8;
    s.editor.settings.indent_with_tabs = true;
    s.editor.apply_settings_everywhere();

    read_with_config(
        &mut s,
        "/project/src/app.js",
        "let x;\n",
        maxgus_core::task::EditorConfig {
            tab_width: Some(2),
            indent_with_tabs: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(s.editor.current_buffer().name(), "app.js");
    assert_eq!(s.editor.current_buffer().tab_width(), 2);
    assert!(!s.editor.current_buffer().indent_with_tabs());
}

#[test]
fn a_file_with_nothing_to_say_keeps_the_configurations_settings() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.tab_width = 8;
    s.editor.apply_settings_everywhere();
    read_with_config(
        &mut s,
        "/project/plain.txt",
        "text\n",
        maxgus_core::task::EditorConfig::default(),
    );
    assert_eq!(s.editor.current_buffer().tab_width(), 8);
}

#[test]
fn the_projects_settings_survive_the_configuration_being_reapplied() {
    // `load-theme` and friends re-apply the settings to every buffer; the
    // file's own rules must not be flattened by that.
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.tab_width = 8;
    read_with_config(
        &mut s,
        "/project/src/app.js",
        "let x;\n",
        maxgus_core::task::EditorConfig {
            tab_width: Some(2),
            ..Default::default()
        },
    );
    s.editor.apply_settings_everywhere();
    assert_eq!(s.editor.current_buffer().tab_width(), 2);
}

#[test]
fn a_project_can_ask_for_crlf_line_endings() {
    let mut s = tall_session("/project/main.rs", "");
    read_with_config(
        &mut s,
        "/project/win.bat",
        "echo hi\n",
        maxgus_core::task::EditorConfig {
            crlf: Some(true),
            ..Default::default()
        },
    );
    assert_eq!(
        s.editor.current_buffer().line_ending(),
        maxgus_text::LineEnding::Crlf
    );
}

#[test]
fn a_project_can_turn_off_trimming_for_its_own_files() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.delete_trailing_whitespace = true;
    read_with_config(
        &mut s,
        "/project/keep.md",
        "a line   \n",
        maxgus_core::task::EditorConfig {
            trim_trailing_whitespace: Some(false),
            ..Default::default()
        },
    );
    // A buffer with no changes is not written at all.
    s.type_text("x");
    s.editor.tasks.drain();
    s.keys("C-x C-s");
    match &s.editor.tasks.drain()[..] {
        [Task::WriteFile { contents, .. }] => assert!(
            contents.contains("a line   "),
            "the trailing spaces were trimmed anyway: {contents:?}"
        ),
        other => panic!("expected a write, got {other:?}"),
    }
}

#[test]
fn a_project_can_ask_for_a_final_newline_the_configuration_does_not() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.require_final_newline = false;
    read_with_config(
        &mut s,
        "/project/needs.txt",
        "no newline",
        maxgus_core::task::EditorConfig {
            final_newline: Some(true),
            ..Default::default()
        },
    );
    s.keys("M->");
    s.type_text("!");
    s.editor.tasks.drain();
    s.keys("C-x C-s");
    match &s.editor.tasks.drain()[..] {
        [Task::WriteFile { contents, .. }] => {
            assert!(contents.ends_with('\n'), "no final newline: {contents:?}")
        }
        other => panic!("expected a write, got {other:?}"),
    }
}

#[test]
fn a_projects_line_length_is_what_fill_uses() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.settings.fill_column = 70;
    read_with_config(
        &mut s,
        "/project/narrow.md",
        &format!("{}\n", "word ".repeat(20)),
        maxgus_core::task::EditorConfig {
            fill_column: Some(20),
            ..Default::default()
        },
    );
    s.keys("M-q");
    let text = s.editor.current_buffer().text();
    for line in text.lines() {
        assert!(
            line.chars().count() <= 20,
            "a line is longer than the project asked for: {line:?}"
        );
    }
}

// ---- sessions ------------------------------------------------------------

#[test]
fn a_restored_session_puts_point_back_where_it_was() {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.restore_session(maxgus_core::session::Session {
        root: Some("/project".into()),
        files: vec![maxgus_core::session::OpenFile {
            path: "/project/notes.txt".into(),
            point: 9,
            top_line: 2,
        }],
        current: None,
        panel_open: false,
    });
    // The file arrives as any other read does.
    s.editor
        .apply_task_result(maxgus_core::TaskResult::FileRead {
            path: "/project/notes.txt".into(),
            contents: "one\ntwo\nthree\nfour\n".into(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: Default::default(),
        })
        .unwrap();

    assert_eq!(s.editor.current_buffer().name(), "notes.txt");
    assert_eq!(
        s.editor.windows.current().point,
        9,
        "point is not where the session left it"
    );
    assert_eq!(
        s.editor.windows.current().top_line,
        2,
        "the window is not scrolled where it was"
    );
}

#[test]
fn a_session_describes_what_is_open() {
    let mut s = tall_session(
        "/project/main.rs",
        "fn main() {}
",
    );
    s.editor.tree_root = Some("/project".into());
    s.editor
        .buffers
        .visit_file("/project/notes.txt", "a note\n");
    s.editor.with_current_buffer(|b| b.set_point(4));
    s.editor.windows.current_mut().point = 4;

    let session = s.editor.session();
    let paths: Vec<String> = session
        .files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();
    assert!(paths.iter().any(|p| p.ends_with("main.rs")));
    assert!(paths.iter().any(|p| p.ends_with("notes.txt")));
    assert_eq!(
        session.current.as_deref(),
        Some(std::path::Path::new("/project/main.rs"))
    );
    let main = session
        .files
        .iter()
        .find(|f| f.path.ends_with("main.rs"))
        .expect("the file");
    assert_eq!(main.point, 4, "point was not remembered");
}

#[test]
fn a_session_leaves_out_the_buffers_that_are_not_files() {
    // `*scratch*` restored from a previous run would be a surprise, and the
    // editor's own buffers are made again when they are needed.
    let mut s = tall_session(
        "/project/main.rs",
        "fn main() {}
",
    );
    s.keys("C-x t t"); // the panel, whose buffers have no files
    let session = s.editor.session();
    for file in &session.files {
        assert!(
            file.path.extension().is_some(),
            "a buffer with no file got into the session: {:?}",
            file.path
        );
    }
    assert!(
        session.panel_open,
        "the panel being open was not remembered"
    );
}

// ---- snippets ------------------------------------------------------------

fn with_snippets() -> Session {
    let mut s = tall_session("/project/main.rs", "");
    s.editor.snippets = vec![
        maxgus_core::snippet::Snippet {
            key: "for".into(),
            name: "a for loop".into(),
            mode: None,
            body: "for ${1:item} in ${2:items} {\n    $0\n}".into(),
        },
        maxgus_core::snippet::Snippet {
            key: "pr".into(),
            name: "println".into(),
            mode: None,
            body: "println!(\"$1\");".into(),
        },
    ];
    s
}

#[test]
fn tab_after_a_snippet_key_expands_it() {
    let mut s = with_snippets();
    s.type_text("pr");
    s.keys("TAB");
    assert_eq!(
        s.editor.current_buffer().text(),
        "println!(\"\");",
        "the key was not expanded"
    );
}

#[test]
fn the_first_field_is_selected_so_typing_replaces_its_default() {
    let mut s = with_snippets();
    s.type_text("for");
    s.keys("TAB");
    assert!(
        s.editor
            .current_buffer()
            .text()
            .starts_with("for item in items"),
        "got {:?}",
        s.editor.current_buffer().text()
    );
    // `item` is the region, so typing takes its place.
    s.type_text("line");
    assert!(
        s.editor
            .current_buffer()
            .text()
            .starts_with("for line in items"),
        "the default was not replaced: {:?}",
        s.editor.current_buffer().text()
    );
}

#[test]
fn tab_moves_to_the_next_field_and_backtab_comes_back() {
    let mut s = with_snippets();
    s.type_text("for");
    s.keys("TAB");
    // Longer than the default it replaces, so the field after it has to have
    // moved: a replacement of the same length would not notice.
    s.type_text("each_line");
    s.keys("TAB"); // to `items`
    s.type_text("lines");
    assert!(
        s.editor
            .current_buffer()
            .text()
            .starts_with("for each_line in lines"),
        "the second field was not reached: {:?}",
        s.editor.current_buffer().text()
    );
    s.keys("S-TAB");
    assert!(
        s.echo().contains("Field 1"),
        "S-TAB did not go back: `{}`",
        s.echo()
    );
}

#[test]
fn the_last_tab_finishes_the_snippet_and_gives_tab_back() {
    let mut s = with_snippets();
    s.type_text("pr");
    s.keys("TAB"); // expand, on field 1
    s.keys("TAB"); // the last field, which finishes it
    assert!(!s.editor.in_snippet(), "the snippet is still going");
    // And `TAB` indents again.
    let before = s.editor.current_buffer().text();
    s.keys("TAB");
    assert_ne!(
        s.editor.current_buffer().text(),
        before,
        "`TAB` did not go back to indenting"
    );
}

#[test]
fn a_word_that_is_not_a_snippet_key_still_indents() {
    let mut s = with_snippets();
    s.type_text("notasnippet");
    s.keys("TAB");
    assert!(
        s.editor.current_buffer().text().contains("notasnippet"),
        "the word was eaten: {:?}",
        s.editor.current_buffer().text()
    );
    assert!(!s.editor.in_snippet());
}

#[test]
fn c_g_abandons_a_snippet_being_filled_in() {
    let mut s = with_snippets();
    s.type_text("for");
    s.keys("TAB");
    s.keys("C-g");
    assert!(!s.editor.in_snippet());
    assert!(s.echo().contains("abandoned"), "got `{}`", s.echo());
    // The text stays: giving up on the fields is not undoing the expansion.
    assert!(s.editor.current_buffer().text().starts_with("for item in"));
}

#[test]
fn a_snippet_can_be_chosen_by_name() {
    let mut s = with_snippets();
    s.dispatcher.execute(&mut s.editor, "insert-snippet", None);
    assert!(s.editor.minibuffer.is_active(), "no prompt");
    s.type_text("println");
    s.keys("RET");
    assert!(
        s.editor.current_buffer().text().contains("println!"),
        "got {:?}",
        s.editor.current_buffer().text()
    );
}

#[test]
fn a_snippet_for_another_mode_is_not_offered() {
    let mut s = with_snippets();
    s.editor.snippets.push(maxgus_core::snippet::Snippet {
        key: "el".into(),
        name: "an elisp thing".into(),
        mode: Some("emacs-lisp-mode".into()),
        body: "(defun $1 ())".into(),
    });
    s.type_text("el");
    s.keys("TAB");
    assert!(
        !s.editor.current_buffer().text().contains("defun"),
        "another mode's snippet was expanded: {:?}",
        s.editor.current_buffer().text()
    );
}

// ---- dired ---------------------------------------------------------------

fn dired_entry(name: &str, is_dir: bool, size: u64) -> maxgus_core::dired::Entry {
    maxgus_core::dired::Entry {
        name: name.into(),
        is_dir,
        link: None,
        size,
        permissions: if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" }.into(),
        modified: "Aug 29 15:03".into(),
    }
}

fn with_dired() -> Session {
    let mut s = tall_session("/project/main.rs", "fn main() {}\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::DiredListed {
            path: "/project/src".into(),
            entries: vec![
                dired_entry("nested", true, 0),
                dired_entry("alpha.rs", false, 100),
                dired_entry("beta.rs", false, 200),
            ],
        })
        .unwrap();
    s.editor.tasks.drain();
    s
}

#[test]
fn a_directory_opens_as_a_buffer_of_its_contents() {
    let mut s = with_dired();
    assert_eq!(s.editor.current_buffer().name(), "*dired*");
    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("/project/src"), "no title:\n{screen:#?}");
    assert!(has("nested/"), "no directory");
    assert!(has("alpha.rs"), "no file");
    assert!(has(".."), "no way up");
}

#[test]
fn marking_moves_on_so_a_run_of_files_is_m_m_m() {
    let mut s = with_dired();
    let line = |s: &Session| {
        s.editor
            .current_buffer()
            .line_of(s.editor.windows.current().point)
    };
    let start = line(&s);
    s.keys("m");
    assert!(line(&s) > start, "`m` did not move on");
    s.keys("m");
    let marked = s
        .editor
        .dired
        .as_ref()
        .expect("a listing")
        .with_mark(maxgus_core::dired::Mark::Marked);
    assert_eq!(marked.len(), 2, "two `m`s marked {} things", marked.len());
}

#[test]
fn an_operation_acts_on_the_marks_when_there_are_any() {
    let mut s = with_dired();
    s.keys("m"); // the first entry
    s.keys("m"); // and the second
    s.editor.tasks.drain();
    s.keys("D");
    s.type_text("yes");
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [
            Task::DiredAct {
                action: maxgus_core::task::FileAction::Delete(paths),
            },
        ] => assert_eq!(paths.len(), 2, "it acted on {} things", paths.len()),
        other => panic!("expected a delete, got {other:?}"),
    }
}

#[test]
fn deleting_asks_first_and_taking_it_back_stops_it() {
    let mut s = with_dired();
    s.editor.tasks.drain();
    s.keys("D");
    assert!(s.editor.minibuffer.is_active(), "it did not ask");
    s.type_text("no");
    s.keys("RET");
    assert!(s.editor.tasks.drain().is_empty(), "it deleted anyway");
    assert!(s.echo().contains("Nothing deleted"), "got `{}`", s.echo());
}

#[test]
fn flagging_and_executing_deletes_what_was_flagged() {
    let mut s = with_dired();
    s.keys("d"); // flag the first
    s.editor.tasks.drain();
    s.keys("x");
    match &s.editor.tasks.drain()[..] {
        [
            Task::DiredAct {
                action: maxgus_core::task::FileAction::Delete(paths),
            },
        ] => {
            assert_eq!(paths.len(), 1);
            assert!(paths[0].ends_with("nested"), "wrong target: {paths:?}");
        }
        other => panic!("expected a delete, got {other:?}"),
    }
}

#[test]
fn executing_with_nothing_flagged_says_so() {
    let mut s = with_dired();
    s.editor.tasks.drain();
    s.keys("x");
    assert!(
        s.echo().contains("Nothing is flagged"),
        "got `{}`",
        s.echo()
    );
    assert!(s.editor.tasks.drain().is_empty());
}

#[test]
fn return_on_a_directory_opens_it_and_on_a_file_reads_it() {
    // Point starts on the first entry, which is the directory.
    let mut s = with_dired();
    s.editor.tasks.drain();
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::Dired { path }] => assert!(path.ends_with("nested")),
        other => panic!("expected a listing, got {other:?}"),
    }

    let mut s = with_dired();
    s.keys("n"); // onto `alpha.rs`
    s.editor.tasks.drain();
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::ReadFile { path, .. }] => assert!(path.ends_with("alpha.rs")),
        other => panic!("expected a read, got {other:?}"),
    }
}

#[test]
fn a_shell_command_gets_the_marked_files_as_its_arguments() {
    let mut s = with_dired();
    s.keys("n"); // onto alpha.rs
    s.keys("m"); // mark it
    s.editor.tasks.drain();
    s.keys("!");
    s.type_text("wc -l");
    s.keys("RET");
    match &s.editor.tasks.drain()[..] {
        [Task::Shell { command, .. }] => {
            assert!(command.starts_with("wc -l "), "got `{command}`");
            assert!(
                command.contains("alpha.rs"),
                "the file is missing: `{command}`"
            );
            assert!(
                command.contains('\''),
                "the path is not quoted: `{command}`"
            );
        }
        other => panic!("expected a shell command, got {other:?}"),
    }
}

#[test]
fn a_refresh_keeps_point_on_the_file_it_was_on() {
    let mut s = with_dired();
    s.keys("n"); // alpha.rs
    let before = s
        .editor
        .dired
        .as_ref()
        .and_then(|v| {
            v.entry(
                s.editor
                    .current_buffer()
                    .line_of(s.editor.windows.current().point),
            )
        })
        .map(|e| e.name.clone());
    assert_eq!(before.as_deref(), Some("alpha.rs"));

    // Something else appears, which would move it if point followed lines.
    s.editor
        .apply_task_result(maxgus_core::TaskResult::DiredListed {
            path: "/project/src".into(),
            entries: vec![
                dired_entry("aaa", true, 0),
                dired_entry("nested", true, 0),
                dired_entry("alpha.rs", false, 100),
                dired_entry("beta.rs", false, 200),
            ],
        })
        .unwrap();
    let after = s.editor.dired.as_ref().and_then(|v| {
        v.entry(
            s.editor
                .current_buffer()
                .line_of(s.editor.windows.current().point),
        )
    });
    assert_eq!(
        after.map(|e| e.name.as_str()),
        Some("alpha.rs"),
        "point did not stay on the file"
    );
}

// ---- scripts -------------------------------------------------------------

#[cfg(feature = "full")]
fn with_script(source: &str) -> Session {
    let mut s = tall_session("/project/main.rs", "hello world\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::ScriptRead {
            source: source.into(),
            path: "/home/someone/.config/maxgus/init.rhai".into(),
        })
        .unwrap();
    s.editor.tasks.drain();
    s
}

#[cfg(feature = "full")]
#[test]
fn a_script_command_is_offered_by_m_x_and_runs() {
    let mut s = with_script(
        r#"
        fn shout(ctx) { insert("!"); }
        define("shout", "Add an exclamation mark.", shout);
        "#,
    );
    assert!(
        s.editor.command_names.iter().any(|n| n == "shout"),
        "`M-x` does not offer it"
    );
    s.dispatcher.execute(&mut s.editor, "shout", None);
    assert!(
        s.editor.current_buffer().text().starts_with('!'),
        "it did not run: {:?}",
        s.editor.current_buffer().text()
    );
}

#[cfg(feature = "full")]
#[test]
fn a_script_command_sees_where_it_was_called() {
    let mut s = with_script(
        r#"
        fn where_am_i(ctx) { message(`${ctx.buffer}:${ctx.line}:${ctx.column}`); }
        define("where-am-i", "…", where_am_i);
        "#,
    );
    s.editor.with_current_buffer(|b| b.set_point(6));
    s.editor.windows.current_mut().point = 6;
    s.dispatcher.execute(&mut s.editor, "where-am-i", None);
    assert_eq!(s.echo(), "main.rs:0:6", "got `{}`", s.echo());
}

#[cfg(feature = "full")]
#[test]
fn a_script_command_can_run_the_editors_own() {
    let mut s = with_script(
        r#"
        fn to_the_end(ctx) { run("end-of-buffer"); }
        define("to-the-end", "…", to_the_end);
        "#,
    );
    s.dispatcher.execute(&mut s.editor, "to-the-end", None);
    assert_eq!(
        s.editor.windows.current().point,
        s.editor.current_buffer().len_chars(),
        "the command it asked for did not run"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_script_that_fails_leaves_nothing_behind() {
    let mut s = with_script(
        r#"
        fn half(ctx) { insert("SHOULD NOT BE THERE"); fail("not today"); }
        define("half", "…", half);
        "#,
    );
    let before = s.editor.current_buffer().text();
    s.dispatcher.execute(&mut s.editor, "half", None);
    assert_eq!(
        s.editor.current_buffer().text(),
        before,
        "the edits before the failure were kept"
    );
    assert!(s.echo().contains("not today"), "got `{}`", s.echo());
}

#[cfg(feature = "full")]
#[test]
fn a_script_cannot_take_a_built_in_commands_name() {
    let mut s = with_script(
        r#"
        fn hijack(ctx) { insert("hijacked"); }
        define("save-buffer", "…", hijack);
        "#,
    );
    s.editor.tasks.drain();
    s.dispatcher.execute(&mut s.editor, "save-buffer", None);
    assert!(
        !s.editor.current_buffer().text().contains("hijacked"),
        "a script overrode a built-in command"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_script_that_will_not_load_is_reported_and_the_editor_carries_on() {
    let mut s = tall_session("/project/main.rs", "hello\n");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::ScriptRead {
            source: "fn broken( {".into(),
            path: "/home/someone/.config/maxgus/init.rhai".into(),
        })
        .unwrap();
    assert!(s.echo().contains("script:"), "got `{}`", s.echo());
    // And the editor still works.
    s.type_text("x");
    assert!(s.editor.current_buffer().text().starts_with('x'));
}

#[cfg(feature = "full")]
#[test]
fn reloading_replaces_what_the_last_script_offered() {
    let mut s = with_script(
        r#"
        fn first(ctx) { }
        define("first-command", "…", first);
        "#,
    );
    assert!(s.editor.command_names.iter().any(|n| n == "first-command"));

    s.editor
        .apply_task_result(maxgus_core::TaskResult::ScriptRead {
            source: r#"
            fn second(ctx) { }
            define("second-command", "…", second);
            "#
            .into(),
            path: "/home/someone/.config/maxgus/init.rhai".into(),
        })
        .unwrap();
    assert!(
        !s.editor.command_names.iter().any(|n| n == "first-command"),
        "the old command is still offered"
    );
    assert!(s.editor.command_names.iter().any(|n| n == "second-command"));
}

// ---- the bindings this editor is meant to share with Doom ----------------

/// The keys Doom's non-evil scheme puts things on, and the ones this
/// configuration adds on top of it.
///
/// A list rather than a description, because a binding that quietly moves is
/// a habit that quietly stops working. Each entry is the key and the command
/// it must reach.
#[test]
fn the_bindings_match_doom() {
    let s = Session::new(80, 24);
    let map = maxgus_core::global_keymap().expect("the global map");
    let expected: &[(&str, &str)] = &[
        // This configuration's own, from its `config.el`.
        ("C-<left>", "windmove-left"),
        ("C-<right>", "windmove-right"),
        ("C-<up>", "windmove-up"),
        ("C-<down>", "windmove-down"),
        ("C-S-<up>", "shrink-window"),
        ("C-S-<down>", "enlarge-window"),
        ("C-S-<left>", "shrink-window-horizontally"),
        ("C-S-<right>", "enlarge-window-horizontally"),
        ("C-d", "duplicate-line-or-region"),
        ("C-s-a", "treefile-toggle"),
        ("C-s-i", "panel-toggle-buffers-section"),
        ("C-s-p", "panel-toggle-tree-section"),
        // Doom's own globals.
        ("C-x b", "switch-to-buffer"),
        ("C-x C-b", "list-buffers"),
        ("C-x K", "kill-buffer-in-all-windows"),
        ("C-x 4 b", "switch-to-buffer-other-window"),
        ("<f9>", "treefile-toggle"),
        // Doom's leader maps.
        ("C-c f f", "find-file"),
        ("C-c f d", "dired"),
        ("C-c f D", "delete-this-file"),
        ("C-c f m", "move-this-file"),
        ("C-c f C", "copy-this-file"),
        ("C-c f y", "yank-buffer-path"),
        ("C-c f Y", "yank-buffer-path-relative-to-project"),
        ("C-c c w", "delete-trailing-whitespace"),
        ("C-c s b", "occur"),
        ("C-c o -", "dired"),
        ("C-c o p", "treefile-toggle"),
        ("C-c t l", "toggle-line-numbers"),
        ("C-c t r", "read-only-mode"),
        ("C-c t I", "toggle-indent-style"),
        ("C-c m n", "mark-next-like-this"),
        ("C-c m p", "mark-previous-like-this"),
        ("C-c m t", "mark-all-like-this"),
        ("C-c i s", "insert-snippet"),
        ("C-c q q", "save-buffers-kill-terminal"),
        ("C-c q s", "save-session"),
        ("C-c q l", "restore-session"),
    ];
    for (keys, command) in expected {
        let sequence = maxgus_keys::KeySequence::parse(keys)
            .unwrap_or_else(|_| panic!("`{keys}` does not parse"));
        assert_eq!(
            map.lookup(&sequence).command(),
            Some(*command),
            "Doom puts `{command}` on `{keys}`"
        );
    }
    // `C-=` needs the grammars behind it, so it is only there in a build
    // that has them.
    #[cfg(feature = "full")]
    {
        let sequence = maxgus_keys::KeySequence::parse("C-=").expect("it parses");
        assert_eq!(map.lookup(&sequence).command(), Some("expand-region"));
    }
    // And every one of them is a command that exists.
    for (_, command) in expected {
        assert!(
            s.dispatcher.registry.get(command).is_some(),
            "`{command}` is bound but not registered"
        );
    }
}

#[cfg(feature = "full")]
#[test]
fn the_language_server_lives_under_dooms_code_map() {
    // Doom keeps `C-c l` for the localleader and puts the language server
    // under `C-c c`. Taking `C-c l` would leave a mode's own bindings nowhere
    // to go.
    let map = maxgus_core::global_keymap().expect("the global map");
    let expected = [
        ("C-c c d", "lsp-find-definition"),
        ("C-c c D", "lsp-find-references"),
        ("C-c c f", "lsp-format-buffer"),
        ("C-c c r", "lsp-rename"),
        ("C-c c a", "lsp-code-action"),
        ("C-c c k", "lsp-describe-thing-at-point"),
        ("C-c c j", "lsp-workspace-symbol"),
        ("C-'", "lsp-document-symbols"),
    ];
    for (keys, command) in expected {
        let sequence = maxgus_keys::KeySequence::parse(keys).expect("it parses");
        assert_eq!(
            map.lookup(&sequence).command(),
            Some(command),
            "`{keys}` should reach `{command}`"
        );
    }
}

#[test]
fn the_classic_emacs_keys_are_still_there() {
    // Doom does not take these away, and neither does this. A leader map is
    // an addition, not a replacement.
    let map = maxgus_core::global_keymap().expect("the global map");
    for (keys, command) in [
        ("C-x C-f", "find-file"),
        ("C-x C-s", "save-buffer"),
        ("C-x C-c", "save-buffers-kill-terminal"),
        ("C-s", "isearch-forward"),
        ("C-w", "kill-region"),
        ("M-w", "kill-ring-save"),
        ("C-y", "yank"),
        ("C-/", "undo"),
        ("C-x o", "other-window"),
        ("M-x", "execute-extended-command"),
    ] {
        let sequence = maxgus_keys::KeySequence::parse(keys).expect("it parses");
        assert_eq!(
            map.lookup(&sequence).command(),
            Some(command),
            "`{keys}` is not `{command}` any more"
        );
    }
}

#[test]
fn the_shifted_arrows_resize_the_window() {
    // What this configuration binds them to. A split first, because a sole
    // window has no height of its own to change.
    let mut s = Session::new(80, 24);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("C-x 2");
    let height = |s: &Session| s.editor.windows.current().rect.height;
    let before = height(&s);

    s.keys("C-S-<down>");
    assert_eq!(
        height(&s),
        before + 1,
        "`C-S-<down>` did not make it taller"
    );
    s.keys("C-S-<up>");
    s.keys("C-S-<up>");
    assert_eq!(height(&s), before - 1, "`C-S-<up>` did not make it shorter");
}

#[test]
fn the_shifted_arrows_say_so_when_there_is_nothing_to_resize() {
    let mut s = Session::new(80, 24);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("C-S-<down>");
    assert!(
        s.echo().contains("layout decides"),
        "a sole window silently did nothing: `{}`",
        s.echo()
    );
}

#[test]
fn c_d_duplicates_the_line_and_then_the_region() {
    let mut s = tall_session("/project/main.rs", "alpha\nbeta\n");
    s.keys("C-d");
    assert_eq!(
        s.editor.current_buffer().text(),
        "alpha\nalpha\nbeta\n",
        "`C-d` did not duplicate the line"
    );

    // With a region, it is the region that is duplicated.
    let mut s = tall_session("/project/main.rs", "abcdef\n");
    s.editor.with_current_buffer(|b| {
        b.set_point(0);
        b.set_mark(0);
        b.set_point(3);
    });
    s.editor.windows.current_mut().point = 3;
    s.keys("C-d");
    assert_eq!(s.editor.current_buffer().text(), "abcabcdef\n");
}

#[test]
fn a_command_with_several_keys_is_shown_with_its_shortest() {
    // `dired` is on `C-x d`, `C-c f d` and `C-c o -`. The column is narrow
    // and the classic key is the one worth the space.
    let mut s = Session::new(100, 16);
    let id = s
        .editor
        .buffers
        .visit_file("/project/main.rs", "fn main() {}\n");
    s.editor.switch_to_buffer(id).unwrap();
    s.keys("M-x");
    s.type_text("dired");
    let row = s
        .screen()
        .into_iter()
        .skip(2)
        .find(|line| line.contains("dired") && !line.contains("M-x"))
        .expect("a candidate row");
    assert!(
        row.contains("C-x d"),
        "the shortest key is not the one shown: `{row}`"
    );
}

// ---- the light beside the cursor -----------------------------------------

/// A session with the beacon on, and a file long enough to jump about in.
fn with_beacon() -> Session {
    let text: String = (1..=200)
        .map(|n| format!("line {n} of the file\n"))
        .collect();
    let mut s = Session::new(80, 24);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    s.editor.settings.beacon = true;
    s.editor.settings.beacon_size = 12;
    s.editor.settings.beacon_blink_delay_ms = 300;
    s.editor.settings.beacon_blink_duration_ms = 300;
    s.editor.quench_beacon();
    s
}

#[test]
fn a_jump_lights_the_beacon() {
    let mut s = with_beacon();
    assert!(
        s.editor.beacon.is_none(),
        "it was lit before anything moved"
    );
    s.keys("M->"); // to the end of the buffer, which scrolls
    assert!(
        s.editor.beacon.is_some(),
        "a jump to the end of the file did not light it"
    );
}

#[test]
fn typing_does_not_light_it() {
    // The setting that would make it unbearable is off by default, as it is
    // in beacon: ordinary editing must leave it alone, and that includes
    // moving between lines, which is most of what editing is.
    let mut s = with_beacon();
    s.type_text("hello");
    s.keys("C-f");
    s.keys("C-b");
    s.keys("C-n");
    s.keys("C-n");
    s.keys("C-p");
    s.keys("RET");
    assert!(
        s.editor.beacon.is_none(),
        "ordinary editing lit it, which would make it unbearable"
    );
}

#[test]
fn a_prompt_being_open_keeps_it_dark() {
    // Beacon's own `window-minibuffer-p` guard. With a prompt open the cursor
    // is in the prompt, so a light in the buffer behind would be pointing at
    // a cursor that is not there.
    let mut s = with_beacon();
    let before = s.editor.beacon_watch();
    s.keys("M-x");
    assert!(s.editor.minibuffer.is_active(), "no prompt");

    // A move that would light it if the prompt were not there.
    s.editor.windows.current_mut().top_line = 40;
    s.editor.consider_beacon(&before);
    assert!(s.editor.beacon.is_none(), "it lit while a prompt was open");

    // And the same move without the prompt does light it.
    s.keys("C-g");
    assert!(!s.editor.minibuffer.is_active());
    s.editor.consider_beacon(&before);
    assert!(
        s.editor.beacon.is_some(),
        "it did not light without the prompt"
    );
}

#[test]
fn switching_buffer_lights_it() {
    let mut s = with_beacon();
    s.editor
        .buffers
        .visit_file("/project/other.rs", "elsewhere\n");
    let other = s
        .editor
        .buffers
        .find_by_name("other.rs")
        .expect("the buffer");
    s.dispatcher.execute(&mut s.editor, "next-buffer", None);
    let _ = other;
    assert!(s.editor.beacon.is_some(), "another buffer did not light it");
}

#[test]
fn the_beacon_is_drawn_beside_the_cursor_and_fades_along_its_length() {
    let mut s = with_beacon();
    s.keys("M->");
    let beacon = s.editor.beacon.expect("a beacon");
    let (line, column) = {
        let buffer = s.editor.current_buffer();
        let line = buffer.line_of(beacon.offset);
        (line, beacon.offset - buffer.line_start(line))
    };
    let row = (line - s.editor.windows.current().top_line) as u16;
    let gutter = 0;
    let head = s.face_at(gutter + column as u16, row).background;
    let along = s.face_at(gutter + column as u16 + 6, row).background;
    let past = s.face_at(gutter + column as u16 + 11 + 4, row).background;
    assert_ne!(head, past, "the beacon is not on screen at all");
    assert_ne!(head, along, "it does not fade along its length");
}

#[test]
fn the_beacon_shortens_as_it_fades_and_then_goes() {
    let mut s = with_beacon();
    s.keys("M->");
    let shape = s.editor.beacon_shape();

    // Nothing happens during the delay.
    assert!(
        s.editor
            .advance_beacon(std::time::Duration::from_millis(200))
    );
    assert_eq!(
        shape.consumed(s.editor.beacon.expect("a beacon").elapsed),
        0,
        "it started fading during the delay"
    );

    // Then it is eaten a cell at a time.
    assert!(
        s.editor
            .advance_beacon(std::time::Duration::from_millis(150))
    );
    assert!(
        shape.consumed(s.editor.beacon.expect("a beacon").elapsed) > 0,
        "it did not start fading"
    );

    // And it goes.
    assert!(!s.editor.advance_beacon(std::time::Duration::from_secs(1)));
    assert!(s.editor.beacon.is_none(), "it outlived its lifetime");
}

#[test]
fn a_beacon_that_is_off_never_lights() {
    let mut s = with_beacon();
    s.editor.settings.beacon = false;
    s.keys("M->");
    assert!(s.editor.beacon.is_none());
}

#[test]
fn a_prompt_is_not_somewhere_the_cursor_gets_lost() {
    let mut s = with_beacon();
    s.keys("M-x");
    s.type_text("save");
    assert!(s.editor.beacon.is_none(), "the minibuffer lit it");
}

#[test]
fn the_colour_is_read_the_way_beacon_reads_it() {
    let mut s = with_beacon();
    // A number is a grade against the background.
    s.editor.settings.beacon_color = "0.7".into();
    assert!(matches!(
        s.editor.beacon_light(),
        maxgus_core::beacon::Light::Grade(_)
    ));
    // A name or a hex string is that colour.
    s.editor.settings.beacon_color = "#ff0066".into();
    assert!(matches!(
        s.editor.beacon_light(),
        maxgus_core::beacon::Light::Colour(_)
    ));
    // And a spelling that is neither still shines.
    s.editor.settings.beacon_color = "not a colour".into();
    assert!(matches!(
        s.editor.beacon_light(),
        maxgus_core::beacon::Light::Grade(_)
    ));
}

#[cfg(feature = "full")]
#[test]
fn what_the_language_server_says_is_shown_beside_the_symbol() {
    // `lsp-ui-doc`. A reply used to open a help window, which pushed the
    // code aside to say one sentence about it.
    let text: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(100, 30);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    // What rust-analyzer really sends, once markdown is asked for.
    s.editor.doc = Some(maxgus_core::Doc {
        text: "### `add`\n\n---\n```rust\nfn add(a: i32, b: i32) -> i32\n```\n\n               Adds two numbers together."
            .into(),
        line: 3,
        window: s.editor.windows.current_id(),
    });
    let screen = s.screen();
    let shown = screen.join("\n");
    assert!(
        shown.contains("fn add(a: i32, b: i32) -> i32"),
        "the box says nothing:\n{shown}"
    );
    // The markdown that spelled it is not on the screen.
    assert!(
        !shown.contains("```") && !shown.contains("###"),
        "the punctuation was drawn:\n{shown}"
    );
    assert!(
        shown.contains("Adds two numbers together."),
        "the box lost half of it:\n{shown}"
    );

    // Beside the symbol, not over it: the line it is about is still there.
    assert!(
        screen.iter().any(|row| row.contains("line 4")),
        "the box covered the line it is about:\n{shown}"
    );
    // And it is a box.
    assert!(
        shown.contains('╭') && shown.contains('╯'),
        "no border:\n{shown}"
    );
}

#[cfg(feature = "full")]
#[test]
fn a_doc_near_the_bottom_of_the_window_goes_above_the_line() {
    // Below it would be off the screen, and a box that is off the screen is
    // a reply nobody reads.
    let text: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(100, 30);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    let last = s.editor.windows.current().top_line + 26;
    s.editor.doc = Some(maxgus_core::Doc {
        text: "one\ntwo\nthree".into(),
        line: last,
        window: s.editor.windows.current_id(),
    });
    let screen = s.screen();
    let top = screen
        .iter()
        .position(|row| row.contains('╭'))
        .expect("the box is drawn");
    let bottom = screen
        .iter()
        .rposition(|row| row.contains('╰'))
        .expect("the box is closed");
    assert!(
        bottom < 29,
        "the box ran into the echo area at row {bottom}"
    );
    assert!(top < bottom, "the box is upside down");
}

/// A suggestion list, driven the way someone drives one.
#[cfg(feature = "full")]
fn with_suggestions(s: &mut Session, labels: &[&str]) {
    let items: Vec<maxgus_core::autocomplete::Item> = labels
        .iter()
        .map(|l| maxgus_core::autocomplete::Item::new(*l))
        .collect();
    let point = s.editor.windows.current().point;
    let text = s.editor.current_buffer().text();
    let start = maxgus_core::autocomplete::word_start(&text, point);
    let prefix: String = text.chars().skip(start).take(point - start).collect();
    let buffer = s.editor.current_buffer_id();
    let list = maxgus_core::autocomplete::Autocomplete::new(buffer, start, &prefix, items);
    s.editor.open_autocomplete(list);
}

#[cfg(feature = "full")]
#[test]
fn the_suggestions_are_drawn_at_the_cursor_and_taken_with_return() {
    let mut s = Session::editing("/project/main.rs", "");
    s.type_text("let x = pu");
    with_suggestions(&mut s, &["push", "push_str", "pop"]);

    let screen = s.screen().join("\n");
    assert!(screen.contains("push_str"), "no list:\n{screen}");
    // Beside what is being typed, not in the echo area.
    let row = s
        .screen()
        .iter()
        .position(|l| l.contains("push_str"))
        .expect("a row");
    assert!(row <= 3, "the list is not near the cursor, at row {row}");

    // `C-n` moves, `RET` takes it, and the word being typed is replaced
    // rather than added to.
    s.keys("C-n");
    s.keys("RET");
    assert_eq!(s.text(), "let x = push_str", "got `{}`", s.text());
    assert!(
        s.editor.autocomplete.is_none(),
        "the list stayed up after being used"
    );
}

#[cfg(feature = "full")]
#[test]
fn typing_narrows_the_list_and_typing_past_it_puts_it_away() {
    let mut s = Session::editing("/project/main.rs", "");
    s.type_text("pu");
    with_suggestions(&mut s, &["push", "push_str", "pop"]);
    assert_eq!(s.editor.autocomplete.as_ref().map(|l| l.len()), Some(2));

    // A letter goes into the buffer *and* narrows the list: the map takes
    // only the keys a list needs.
    s.type_text("s");
    assert_eq!(s.text(), "pus");
    assert_eq!(
        s.editor.autocomplete.as_ref().map(|l| l.len()),
        Some(2),
        "`pus` still finds both by subsequence"
    );
    s.type_text("h_");
    assert_eq!(
        s.editor.autocomplete.as_ref().map(|l| l.len()),
        Some(1),
        "`push_` is only one of them"
    );

    // A space ends the word, and the list with it.
    s.type_text(" ");
    assert!(
        s.editor.autocomplete.is_none(),
        "the list outlived its word"
    );
}

#[cfg(feature = "full")]
#[test]
fn nothing_matching_puts_the_list_away_rather_than_leaving_it_empty() {
    let mut s = Session::editing("/project/main.rs", "");
    s.type_text("pu");
    with_suggestions(&mut s, &["push", "pop"]);
    s.type_text("zzz");
    assert!(s.editor.autocomplete.is_none());
    assert_eq!(s.text(), "puzzz", "the letters still went in");
}

#[cfg(feature = "full")]
#[test]
fn the_list_gives_its_keys_back_when_it_goes() {
    // `RET` means the list while it is up and means a newline the moment it
    // is not. A minor map that outlived the list would eat every `RET`.
    let mut s = Session::editing("/project/main.rs", "");
    s.type_text("pu");
    with_suggestions(&mut s, &["push"]);
    s.keys("C-g");
    assert!(s.editor.autocomplete.is_none());
    s.keys("RET");
    assert_eq!(s.text(), "pu\n", "`RET` did not insert a newline");
    s.keys("C-n");
    assert_eq!(s.text(), "pu\n", "`C-n` typed something instead of moving");
}

#[cfg(feature = "full")]
#[test]
fn a_long_list_shows_a_window_of_it_and_says_where_in_it_you_are() {
    let labels: Vec<String> = (0..30).map(|n| format!("item{n:02}")).collect();
    let borrowed: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut s = Session::editing("/project/main.rs", "");
    s.type_text("it");
    with_suggestions(&mut s, &borrowed);

    let screen = s.screen().join("\n");
    assert!(
        screen.contains("1/30"),
        "no position in the list:\n{screen}"
    );
    let shown = labels.iter().filter(|l| screen.contains(*l)).count();
    assert_eq!(shown, maxgus_core::autocomplete::ROWS, "got {shown} rows");
}

/// The table in the README says what is in each build. This is what keeps
/// it true.
///
/// By family rather than by count: a count needs editing every time a
/// command is added, which is how a number in a document becomes a number
/// nobody believes. What matters is that the whole of a feature is in or
/// out, and that is what is checked.
#[test]
fn each_build_has_the_families_the_table_promises_it() {
    let s = tall_session("/project/main.rs", "");
    let names = s.dispatcher.registry.interactive_names();
    let has = |prefix: &str| names.iter().any(|n| n.starts_with(prefix));

    // In every build, and the reason `minimal` is an editor rather than a
    // demonstration.
    for family in [
        "find-file",
        "switch-to-buffer",
        "treefile-",
        "dired",
        "undo-tree-",
        "mark-next-like-this",
        "kmacro-",
        "query-replace",
        "load-theme",
        "save-session",
        "snippet-",
        "describe-bindings",
    ] {
        assert!(has(family), "`{family}` should be in every build");
    }

    // In `full` and `gui` only. A `minimal` build that grew one of these
    // has grown the crate behind it, and the whole point of `minimal` is
    // that it did not.
    let extras = [
        "lsp-",
        "autocomplete-",
        "completion-at-point",
        "magit-",
        "terminal-",
        "grep-",
        "project-grep",
        "describe-grammars",
        "reload-scripts",
        "expand-region",
    ];
    for family in extras {
        assert_eq!(
            has(family),
            cfg!(feature = "full"),
            "`{family}` is in the wrong build"
        );
    }
}

/// A tree taller than the panel has to scroll with the cursor.
///
/// It did not: the tree drew from the window's `top_line` and nothing ever
/// moved it, so walking down a project with more files than the panel is
/// tall took the cursor off the bottom and left it there — invisible, and
/// with no way to see where it was.
#[cfg(feature = "full")]
#[test]
fn the_file_tree_follows_its_cursor_off_the_bottom() {
    use maxgus_tree::{NodeKind, VisibleNode};

    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("C-x t t");
    // Sixty files in a panel that shows a couple of dozen rows at most.
    let nodes: Vec<VisibleNode> = (0..60)
        .map(|n| VisibleNode {
            path: format!("/project/file{n:02}.rs").into(),
            name: format!("file{n:02}.rs"),
            depth: 1,
            kind: NodeKind::File,
            expanded: false,
            expandable: false,
            git: None,
            is_root: false,
        })
        .collect();
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TreeUpdated {
            nodes,
            select: None,
            show_hidden: false,
        })
        .unwrap();

    let tree_window = s.editor.tree_window.expect("the tree is open");
    let height = s
        .editor
        .windows
        .get(tree_window)
        .expect("the window")
        .text_height();
    assert!(height > 2 && height < 60, "the panel is {height} rows");

    // Down past the bottom of the panel.
    for _ in 0..height + 5 {
        s.editor
            .move_tree_cursor_to_line(s.editor.tree_cursor_line() + 1);
    }
    let cursor = s.editor.tree_cursor_line();
    let top = s
        .editor
        .windows
        .get(tree_window)
        .expect("the window")
        .top_line;
    assert!(
        cursor >= top && cursor < top + height,
        "the cursor is on line {cursor} and the panel shows {top}..{}",
        top + height
    );
    assert!(top > 0, "the panel never scrolled");

    // And back up to the top, which has to scroll the other way.
    for _ in 0..cursor {
        s.editor
            .move_tree_cursor_to_line(s.editor.tree_cursor_line().saturating_sub(1));
    }
    let top = s
        .editor
        .windows
        .get(tree_window)
        .expect("the window")
        .top_line;
    assert_eq!(top, 0, "it did not come back");

    // What is drawn agrees: the selected row is on the screen.
    s.editor.move_tree_cursor_to_line(50);
    let screen = s.screen().join("\n");
    assert!(
        screen.contains("file50.rs"),
        "line 50 is selected and not drawn:\n{screen}"
    );
}

// ---- the file tree's helpful panel --------------------------------------

/// A frame big enough for the whole tree keymap, which is what a window on
/// any ordinary display is.
const WIDE: u16 = 140;
const TALL: u16 = 44;

#[test]
fn the_tree_help_draws_the_keymap_in_named_columns() {
    // treemacs' helpful hydra, in the box `C-x` and `C-c` already draw into.
    let mut s = Session::new(WIDE, TALL);
    s.keys("C-x t t");
    s.editor
        .apply_task_result(maxgus_core::TaskResult::TreeUpdated {
            nodes: vec![node("/project", "project", true, 0, true)],
            select: None,
            show_hidden: false,
        })
        .unwrap();
    // `C-x t 1` is how the keys reach the tree; `C-x t t` only opens it.
    s.keys("C-x t 1");
    s.keys("?");

    let screen = s.screen();
    let has = |needle: &str| screen.iter().any(|line| line.contains(needle));
    assert!(has("File tree"), "no title in the border:\n{screen:#?}");
    assert!(has("Navigation"), "no headings:\n{screen:#?}");
    assert!(has("next line"), "no entries:\n{screen:#?}");
    assert!(has("create file"), "the later sections were dropped");
    // The same box which-key draws: bordered, along the bottom.
    assert!(has("╭"), "no border:\n{screen:#?}");
    // Everything, on a frame with room for it: the panel counts what it
    // drops, so a silent one here would mean a keymap quietly cut short.
    assert!(
        !has("more"),
        "the whole map did not fit a {WIDE}x{TALL} frame:\n{screen:#?}"
    );
    if std::env::var("SHOW").is_ok() {
        println!("{}", screen.join("\n"));
    }
}

#[test]
fn the_tree_help_goes_when_the_tree_stops_being_where_the_keys_go() {
    let mut s = with_tree();
    s.keys("C-x t 1");
    s.keys("?");
    assert!(s.editor.key_menu.is_some(), "the panel did not open");
    // `C-x o` out of the tree: the keys it describes are no longer live.
    s.keys("C-x o");
    maxgus_core::frontend::after_key(&mut s.editor, &mut s.dispatcher);
    assert!(
        s.editor.key_menu.is_none(),
        "a panel of tree keys is up over a window that is not the tree"
    );
}

// ---- the lines that fill a slide's gap ----------------------------------

#[test]
fn the_lines_arriving_are_the_ones_just_past_the_window() {
    // What smooth scrolling draws into the gap it opens. Getting the edge
    // wrong here shows as the window repeating a line it already had, or
    // skipping one, every time it scrolls.
    let text: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(40, 12);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    let window = s.editor.windows.current_id();
    let area = maxgus_core::text_area(&s.editor, window).expect("a text area");
    let mut scratch = Surface::new(Size::new(40, 12));

    // Downwards: the first line arriving is the one just below the window,
    // and they come in the order they will be drawn.
    let rows = maxgus_core::edge_rows(&mut s.editor, window, 1, 3, &mut scratch)
        .expect("three lines below the window");
    assert_eq!(rows.len(), 3);
    let said: Vec<String> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .into()
        })
        .collect();
    let first_below = area.height as usize + 1;
    assert_eq!(
        said,
        [
            format!("line {first_below}"),
            format!("line {}", first_below + 1),
            format!("line {}", first_below + 2),
        ],
        "the wrong lines are arriving"
    );

    // And one row is still the nearest one, so the wheel's own case is
    // unchanged by there being a way to ask for more.
    let one = maxgus_core::edge_row(&mut s.editor, window, 1, &mut scratch).expect("one line");
    let text: String = one.iter().map(|c| c.ch).collect();
    assert_eq!(text.trim_end(), format!("line {first_below}"));
}

#[test]
fn asking_for_lines_beyond_the_window_leaves_the_view_where_it_was() {
    // It moves `top_line` to draw them and has to put it back, or reading
    // the gap would scroll the window that asked.
    let text: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    let mut s = Session::new(40, 12);
    let id = s.editor.buffers.visit_file("/project/main.rs", &text);
    s.editor.switch_to_buffer(id).unwrap();
    let window = s.editor.windows.current_id();
    let was = s.editor.windows.get(window).unwrap().top_line;
    let mut scratch = Surface::new(Size::new(40, 12));
    maxgus_core::edge_rows(&mut s.editor, window, 1, 4, &mut scratch);
    assert_eq!(s.editor.windows.get(window).unwrap().top_line, was);
}

#[test]
fn nothing_arrives_past_the_end_of_the_buffer() {
    // The gap there really is empty, and drawing a line into it would be
    // drawing a line that does not exist.
    let mut s = Session::new(40, 12);
    let id = s
        .editor
        .buffers
        .visit_file("/project/short.rs", "one\ntwo\n");
    s.editor.switch_to_buffer(id).unwrap();
    let window = s.editor.windows.current_id();
    let mut scratch = Surface::new(Size::new(40, 12));
    assert!(maxgus_core::edge_rows(&mut s.editor, window, 1, 2, &mut scratch).is_none());
}

// ---- more than one directory in the tree ---------------------------------

#[test]
fn a_second_directory_can_be_asked_for_from_the_tree() {
    // A workspace is usually more than one directory, and closing the tree
    // to reopen it somewhere else is not having both.
    let mut s = with_tree();
    s.keys("C-x t 1");
    s.editor.tasks.drain();
    s.keys("r a");
    assert!(s.editor.minibuffer.is_active(), "it did not ask for one");
    assert!(
        s.editor.minibuffer.prompt().contains("Add directory"),
        "got `{}`",
        s.editor.minibuffer.prompt()
    );

    s.type_text("/other/project");
    s.keys("RET");
    let queued = s.editor.tasks.drain();
    let asked = queued.iter().any(|task| {
        matches!(
            task,
            maxgus_core::Task::Tree(maxgus_core::TreeAction::AddRoot(path))
                if path == std::path::Path::new("/other/project")
        )
    });
    assert!(asked, "nothing was queued to add it: {queued:?}");
}

#[test]
fn removing_a_directory_names_the_one_the_cursor_is_in() {
    // Rather than asking someone to put the cursor exactly on the heading
    // first, which is asking them to do the search themselves.
    let mut s = with_tree();
    s.keys("C-x t 1");
    // Down onto a file inside the root, not the heading itself.
    s.keys("n");
    assert!(
        !s.editor
            .tree_selection()
            .expect("something is selected")
            .is_root,
        "the cursor is still on the heading, so the test proves nothing"
    );
    s.editor.tasks.drain();

    s.keys("r k");
    let queued = s.editor.tasks.drain();
    let asked = queued.iter().any(|task| {
        matches!(
            task,
            maxgus_core::Task::Tree(maxgus_core::TreeAction::RemoveRoot(path))
                if path == std::path::Path::new("/project")
        )
    });
    assert!(asked, "it did not name the directory: {queued:?}");
}

#[test]
fn a_relative_directory_is_relative_to_where_the_tree_already_is() {
    // What someone typing `../lib` means by it.
    let mut s = with_tree();
    s.keys("C-x t 1");
    s.editor.tasks.drain();
    s.keys("r a");
    s.type_text("../lib");
    s.keys("RET");
    let queued = s.editor.tasks.drain();
    let named = queued.iter().find_map(|task| match task {
        maxgus_core::Task::Tree(maxgus_core::TreeAction::AddRoot(path)) => Some(path.clone()),
        _ => None,
    });
    assert_eq!(
        named,
        Some(s.editor.default_directory().join("../lib")),
        "got {queued:?}"
    );
}
