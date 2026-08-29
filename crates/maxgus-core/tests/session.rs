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
        let frame = Rect::new(0, 0, width, height);
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            frame,
        );
        let registry = maxgus_core::standard_registry();
        editor.command_names = registry.interactive_names();
        editor.command_docs =
            registry.iter().map(|c| (c.name.to_string(), c.doc.to_string())).collect();
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
            let key = if c == ' ' { "SPC".to_string() } else { c.to_string() };
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
        self.surface.to_lines().into_iter().map(|l| l.trim_end().to_string()).collect()
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
    assert!(s.echo().starts_with("failing I-search"), "got `{}`", s.echo());
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
    assert!(s.mode_line().contains(maxgus_core::icons::SAVED), "unmodified, got `{}`", s.mode_line());
    assert!(s.mode_line().contains("main.rs"));
    assert!(s.mode_line().contains("rust"));

    s.type_text("x");
    assert!(s.mode_line().contains(maxgus_core::icons::MODIFIED), "modified, got `{}`", s.mode_line());
}

#[test]
fn saving_queues_a_write_and_the_result_clears_the_flag() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.editor.with_current_buffer(|b| b.mark_saved());
    s.type_text("// note\n");
    assert!(s.editor.current_buffer().is_modified());

    s.keys("C-x C-s");
    let tasks = s.editor.tasks.drain();
    let Some(Task::WriteFile { path, contents, buffer, .. }) = tasks.into_iter().next() else {
        panic!("no write was queued");
    };
    assert_eq!(path, std::path::Path::new("/project/main.rs"));
    assert!(contents.starts_with("// note"));

    s.editor
        .apply_task_result(maxgus_core::TaskResult::FileWritten { path, buffer, bytes: 21, disk_time: None })
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
    assert!(s.echo().starts_with("Find file: /project/"), "got `{}`", s.echo());

    s.editor.tasks.drain();
    s.type_text("other.rs");
    s.keys("RET");
    let tasks = s.editor.tasks.drain();
    assert!(
        tasks.iter().any(
            |t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("other.rs"))
        ),
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
    assert_eq!(s.editor.windows.len(), 2);

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

    // The tree is drawn on the left, the buffer beside it.
    let screen = s.screen();
    // `v` is the arrow, then the directory glyph, then the name.
    assert!(screen[0].starts_with('v'), "the arrow, got `{}`", screen[0]);
    assert!(screen[0].contains("project"), "got `{}`", screen[0]);
    assert!(screen[0].contains("fn main()"), "got `{}`", screen[0]);

    // With the tree selected, `n` moves down it rather than inserting.
    s.editor.select_window(tree);
    s.keys("n");
    assert_eq!(s.editor.tree_selection().unwrap().name, "main.rs");
    // Depth one indents by two, and a file has no arrow, so four spaces.
    assert_eq!(s.editor.current_buffer().text(), "v project\n    main.rs\n");

    s.keys("q");
    assert!(s.editor.tree_window.is_none());
    assert_eq!(s.editor.windows.len(), 1);
}

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
    assert_eq!(s.surface.get(0, 0).unwrap().face, keyword, "`fn` is a keyword");
    assert_ne!(s.surface.get(3, 0).unwrap().face, keyword, "`main` is not");
}

#[test]
fn a_diagnostic_is_underlined_and_counted_in_the_mode_line() {
    let mut s = Session::editing("/project/main.rs", "let unused = 1;\n");
    s.editor.apply_task_result(maxgus_core::TaskResult::Diagnostics {
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
    assert_eq!(s.surface.get(4, 0).unwrap().face.attributes.underline, Some(true));
    assert!(s.mode_line().contains(maxgus_core::icons::WARNING), "got `{}`", s.mode_line());

    // `M-g n` jumps to it and reports what it says.
    s.keys("M-g n");
    assert_eq!(s.point(), 4);
    assert!(s.echo().contains("unused variable"), "got `{}`", s.echo());
}

#[test]
fn editing_after_opening_a_file_is_reported_to_the_language_server() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    let id = s.editor.current_buffer_id();
    s.editor.request_language_server(id);
    s.editor.tasks.drain();

    s.type_text("x");
    assert!(s.editor.sync_language_server(id), "the change is worth reporting");
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
    let outcome = s.dispatcher.handle_keys(&mut s.editor, "<f9>");
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
        let Some(keys_cell) = row.split('|').nth(1) else { continue };
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
            let sequence = maxgus_keys::KeySequence::parse(description)
                .unwrap_or_else(|e| panic!("the README documents `{description}`, which is not a key: {e}"));
            let lookup = session.editor.keymaps.lookup(&sequence);
            assert!(
                !lookup.is_undefined(),
                "the README documents `{description}`, which is bound to nothing"
            );
            checked += 1;
        }
    }
    assert!(checked >= 15, "only {checked} keys were checked; is the table still there?");
}

/// The commands the README names in its feature list must exist.
#[test]
fn every_command_the_readme_names_is_registered() {
    let readme = include_str!("../../../README.md");
    let registry = maxgus_core::standard_registry();
    for line in readme.lines() {
        for word in line.split('`') {
            // Command names in the configuration examples.
            if word.starts_with("lsp-") || word.starts_with("treefile-") {
                assert!(
                    registry.contains(word),
                    "the README names `{word}`, which is not a command"
                );
            }
        }
    }
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
        let Ok(entries) = std::fs::read_dir(&directory) else { continue };
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
    assert!(!editor.buffers.is_empty(), "after `{after}` there are no buffers left");
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
    for (buffer, (_, _, spans)) in &editor.highlights {
        let Some(length) = editor.buffers.get(*buffer).map(|b| b.text().len()) else { continue };
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
        let Some(kind) = session.editor.minibuffer.kind() else { return };
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
    let mut session = Session::editing("/project/main.rs", "fn main() {\n    let x = 1;\n    let y = 2;\n}\n");
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
    let text: String = (0..500).map(|n| format!("line {n} of a long file\n")).collect();
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
    assert_eq!(lines.len(), size.height as usize, "after `{after}` the frame changed height");

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
    "a", "b", "SPC", "RET", "x", "1", "y", "n", "q", "DEL", "TAB",
    "C-f", "C-b", "C-n", "C-p", "C-a", "C-e", "C-g", "M-f", "M-b",
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
        let mut session = if seed % 3 == 0 { furnished_long() } else { furnished() };
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
    "a", "b", "SPC", "RET", "x", "1", "TAB",
    "C-o", "C-d", "DEL", "C-k", "C-w", "M-w", "C-y", "M-y", "C-j",
    "C-t", "M-t", "M-u", "M-l", "M-c", "M-d", "M-DEL", "C-M-k",
    "M-\\", "M-SPC", "M-^", "C-x C-o", "M-;", "M-q", "C-S-DEL",
    "C-x C-u", "C-x C-l", "C-x C-t",
];

/// Keys that move around without changing anything.
const MOVES: &[&str] = &[
    "C-f", "C-b", "C-n", "C-p", "C-a", "C-e", "M-f", "M-b",
    "M-<", "M->", "C-SPC", "C-x C-x", "M-m", "C-M-f", "C-M-b",
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
    let second = session.editor.buffers.visit_file("/project/second.rs", "two\n");
    session.editor.switch_to_buffer(second).unwrap();
    session.editor.tasks.drain();
    assert!(session.mode_line().contains("second.rs"));

    session.keys("C-x <right>");
    let after_next = session.mode_line();
    assert!(!after_next.contains("second.rs"), "the buffer did not change: `{after_next}`");

    session.keys("C-x <left>");
    assert!(
        session.mode_line().contains("second.rs"),
        "going back did not return: `{}`",
        session.mode_line()
    );

    // Emacs binds the control-modified arrows to the same pair.
    session.keys("C-x C-<right>");
    assert_eq!(session.mode_line(), after_next, "`C-x C-<right>` is the same command");
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
    assert!(narrowed.len() < everything, "typing did not narrow anything");
    assert!(narrowed.iter().all(|c| c.starts_with("save-")), "got {narrowed:?}");
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
    assert!(shown.candidates.iter().any(|c| c == "main.rs"), "got {:?}", shown.candidates);
    assert!(shown.candidates.iter().any(|c| c == "other.rs"), "got {:?}", shown.candidates);
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
    let edits = ["a", "DEL", "C-k", "C-a", "C-e", "M-DEL", "C-d", "M-p", "M-n"];
    for keys in edits {
        let mut session = Session::editing("/project/main.rs", "fn main() {}\n");
        session.keys("M-x");
        session.type_text("save");
        session.keys(keys);

        let input = session.editor.minibuffer.input().to_string();
        let shown = session.editor.minibuffer.completion().candidates.clone();
        let expected =
            maxgus_core::fuzzy::matches(&input, session.editor.command_names.iter());
        assert_eq!(shown, expected, "after `{keys}` the list does not match `{input}`");
    }
}

fn node(path: &str, name: &str, dir: bool, depth: usize, root: bool) -> maxgus_tree::VisibleNode {
    maxgus_tree::VisibleNode {
        path: path.into(),
        name: name.into(),
        kind: if dir { maxgus_tree::NodeKind::Directory } else { maxgus_tree::NodeKind::File },
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
    let Some(path) = s.editor.current_buffer().path().map(std::path::Path::to_path_buf) else {
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
    assert_eq!(s.editor.windows.current_id(), tree, "C-<left> did not reach the tree");

    s.keys("C-<right>");
    assert_eq!(s.editor.windows.current_id(), code, "C-<right> did not come back");

    // And repeating it does not wander off somewhere else.
    s.keys("C-<right>");
    assert_eq!(s.editor.windows.current_id(), code, "there is nothing to the right");
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
    assert_eq!(s.editor.windows.current_id(), top, "C-<up> did not come back");
}

#[test]
fn moving_where_there_is_no_window_says_so_and_stays_put() {
    let mut s = Session::editing("/project/main.rs", "one\n");
    let only = s.editor.windows.current_id();
    for keys in ["C-<left>", "C-<right>", "C-<up>", "C-<down>"] {
        s.keys(keys);
        assert_eq!(s.editor.windows.current_id(), only, "`{keys}` moved somewhere");
        assert!(s.editor.minibuffer.message_is_error(), "`{keys}` said nothing");
    }
}

#[test]
fn moving_between_windows_keeps_each_one_where_it_was() {
    // The point of a window is that it remembers its own place.
    let mut s = with_tree();
    s.keys("C-n C-n");
    let code_line = s.editor.current_buffer().line_of(s.editor.windows.current().point);

    s.keys("C-<left>");
    assert_eq!(s.editor.current_buffer().name(), "*treefile*");
    s.keys("C-<right>");
    assert_eq!(
        s.editor.current_buffer().line_of(s.editor.windows.current().point),
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
    assert_eq!(s.editor.current_buffer().name(), "main.rs", "focus left the file");

    // And the tree did follow: its cursor sits on the file being edited.
    let tree = s.editor.tree_window.expect("the tree");
    let line = s.editor.buffers.get(s.editor.windows.get(tree).unwrap().buffer)
        .map(|b| b.line_of(s.editor.windows.get(tree).unwrap().point))
        .expect("a tree buffer");
    assert_eq!(
        s.editor.tree.get(line).map(|n| n.name.as_str()),
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
    assert_eq!(s.editor.current_buffer().name(), "main.rs", "focus is in the file");

    let start = s.editor.windows.current().point;
    s.keys("<down>");
    let down = s.editor.windows.current().point;
    assert!(down > start, "<down> did not move point in the file");

    s.keys("<right>");
    assert!(s.editor.windows.current().point > down, "<right> did not move point");

    s.keys("<up>");
    assert!(s.editor.windows.current().point < down + 1, "<up> did not move point back");
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

// ---- visiting themes ----------------------------------------------------

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
fn visiting_themes_shows_each_one_as_it_comes_under_the_cursor() {
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();

    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    assert!(s.editor.minibuffer.is_active(), "the prompt did not open");
    assert!(s.editor.minibuffer.completion().visible, "the themes are not listed");

    // Typing a name shows it without anything being accepted yet.
    s.type_text("daylight");
    assert_eq!(s.editor.theme.name(), "daylight", "the theme was not previewed");
    assert!(s.editor.minibuffer.is_active(), "the prompt closed early");
    assert_ne!(started_as, "daylight", "the fixture proves nothing otherwise");
}

#[test]
fn abandoning_the_visit_puts_the_old_theme_back() {
    let mut s = with_two_themes();
    let started_as = s.editor.theme.name().to_string();

    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    s.type_text("daylight");
    assert_eq!(s.editor.theme.name(), "daylight");

    s.keys("C-g");
    assert_eq!(s.editor.theme.name(), started_as, "C-g did not put it back");
    assert!(!s.editor.minibuffer.is_active());
}

#[test]
fn keeping_a_visited_theme_asks_whether_to_write_it_down() {
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");

    assert_eq!(s.editor.theme.name(), "daylight", "the choice did not stick");
    assert!(s.editor.minibuffer.is_active(), "it did not ask about the config file");
    assert!(
        s.editor.minibuffer.prompt().contains("config file"),
        "got `{}`",
        s.editor.minibuffer.prompt()
    );
}

#[test]
fn answering_no_keeps_the_theme_for_the_session_only() {
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");
    s.editor.tasks.drain();

    s.type_text("no");
    s.keys("RET");

    assert_eq!(s.editor.theme.name(), "daylight", "the theme did not stay");
    assert!(
        s.editor.tasks.drain().is_empty(),
        "answering no still wrote to the configuration file"
    );
    assert!(s.editor.minibuffer.display().contains("session"), "got `{}`", s.editor.minibuffer.display());
}

#[test]
fn answering_yes_queues_the_write() {
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    s.type_text("daylight");
    s.keys("RET");
    s.editor.tasks.drain();

    s.type_text("yes");
    s.keys("RET");

    let queued = s.editor.tasks.drain();
    let wrote = queued.iter().any(|task| {
        matches!(task, maxgus_core::Task::PersistTheme { theme, .. } if theme == "daylight")
    });
    assert!(wrote, "nothing was queued to write the theme: {queued:?}");
}

#[test]
fn visiting_the_theme_already_in_the_config_asks_nothing() {
    // There would be nothing to write, so the question would be noise.
    let mut s = with_two_themes();
    s.keys("M-x");
    s.type_text("visit-theme");
    s.keys("RET");
    s.type_text("maxgus-dark");
    s.keys("RET");

    assert!(!s.editor.minibuffer.is_active(), "it asked anyway");
    assert_eq!(s.editor.theme.name(), "maxgus-dark");
}

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
    assert!(line.contains(maxgus_core::icons::BRANCH), "without its glyph: `{line}`");
}

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

    assert!(screen[0].starts_with('\u{256d}'), "no popup border: `{}`", screen[0]);
    // Where the highlight is, out of how many match: the first of all of them.
    assert!(
        screen[1].contains(&format!("1/{total} M-x")),
        "the prompt line does not count the list: `{}`",
        screen[1]
    );
    assert!(screen[2].starts_with('\u{2502}'), "no candidates under it: `{}`", screen[2]);
    // The prompt went up with the list instead of being drawn in both places.
    assert!(screen[9].is_empty(), "the echo area still prompts: `{}`", screen[9]);

    // Narrowing changes the right half of the count and walking the list
    // changes the left. Telling them apart needs a case where the two differ
    // from each other and from the size of the whole set.
    s.type_text("buffer");
    let matched = s.editor.minibuffer.completion().len();
    assert!(matched < total, "`buffer` matched the whole command set");
    let row = s.screen()[1].clone();
    assert!(row.contains(&format!("1/{matched} M-x buffer")), "count line is `{row}`");

    s.keys("<down>");
    let row = s.screen()[1].clone();
    assert!(row.contains(&format!("2/{matched} M-x buffer")), "count line is `{row}`");
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
    let row = s.screen().into_iter().skip(2).find(|l| l.contains("save-buffer")).unwrap();
    assert!(row.contains("C-x C-s"), "no key binding beside the name: `{row}`");
    // Clipped at the right edge of a sixty-column frame, which is what the
    // column is for: the name and the key stay put and the prose gives way.
    assert!(row.contains("Save this buffer"), "no summary: `{row}`");
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
    assert!(s.screen().iter().any(|l| l.contains("save-buffer")), "it is not on screen");
}

#[test]
fn the_arrow_keys_walk_the_candidate_list() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("buffer");
    let first = s.editor.minibuffer.completion().current().unwrap().to_string();

    s.keys("<down>");
    let second = s.editor.minibuffer.completion().current().unwrap().to_string();
    assert_ne!(first, second, "`<down>` did not move the highlight");
    // The highlight is on the row it moved to, not the one it left.
    let chosen = s.editor.theme.resolve("completion-selected").background;
    assert_eq!(s.face_at(1, 3).background, chosen, "the second row is not marked");
    assert_ne!(s.face_at(1, 2).background, chosen, "the first row is still marked");

    s.keys("<up>");
    assert_eq!(s.editor.minibuffer.completion().current(), Some(first.as_str()));
}

#[test]
fn the_page_keys_move_a_screenful_of_candidates() {
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    let page = s.editor.completion_rows();
    assert!(page > 1, "a page of {page} rows cannot tell paging from stepping");

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
    assert_eq!(s.editor.region().unwrap(), whole, "`mark-whole-buffer` did not run");
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
    assert!(before.ends_with("M-x save"), "the cursor is not after what was typed: `{before}`");
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
        s.editor.minibuffer.completion().candidates.iter().any(|c| c.ends_with("2024.md")),
        "the older file should still be offered"
    );
    s.keys("RET");

    let tasks = s.editor.tasks.drain();
    assert!(
        tasks.iter().any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("notes"))),
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
        tasks.iter().any(|t| matches!(t, Task::ReadFile { path, .. } if path.ends_with("notes-2024.md"))),
        "the highlighted file was not the one visited: {tasks:?}"
    );
}

#[test]
fn return_on_an_untouched_command_prompt_runs_the_highlighted_one() {
    // `M-x` is the one completing prompt with no default to fall back on: an
    // empty command name answers nothing, so what is highlighted is the only
    // useful reading of `RET`. The prompts that do name a default keep it.
    let mut s = Session::editing("/project/main.rs", "    indented\n");
    s.keys("M-x");
    assert_eq!(
        s.editor.minibuffer.completion().current(),
        Some("back-to-indentation"),
        "this test runs whatever `M-x` highlights first; teach it the new one"
    );

    s.keys("RET");
    assert_eq!(s.point(), 4, "the highlighted command did not run");
}

#[test]
fn a_query_that_matches_nothing_keeps_the_popup_where_it_is() {
    // Dropping the box when the last match goes would throw the prompt to the
    // bottom of the screen on the keystroke that stops matching and back up
    // on the one that deletes it. It stays, and says nothing matched.
    let mut s = Session::editing("/project/main.rs", "fn main() {}\n");
    s.keys("M-x");
    s.type_text("zzzq");
    assert!(s.editor.minibuffer.completion().is_empty(), "`zzzq` matched something");

    let screen = s.screen();
    assert!(screen[0].starts_with('\u{256d}'), "the popup left: {screen:#?}");
    assert!(screen[1].contains("0/0 M-x zzzq"), "prompt line is `{}`", screen[1]);
    assert!(screen[9].is_empty(), "the prompt fell back to the echo area: `{}`", screen[9]);
}
