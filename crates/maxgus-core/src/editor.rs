//! The editor state.
//!
//! One [`Editor`] holds everything: buffers, the window tree, the minibuffer,
//! the kill ring, registers, settings and the queue of asynchronous work.
//! Commands take `&mut Editor` and nothing else.
//!
//! Point lives in two places, as it does in Emacs: the buffer has one, and each
//! window showing that buffer has its own. The window's copy is authoritative
//! for the selected window and is written back to the buffer before any
//! operation that reads it, which is what lets the same file be open in two
//! windows at two positions.

use crate::{
    Result,
    buffers::BufferList,
    minibuffer::Minibuffer,
    prefix::Prefix,
    task::{Task, TaskQueue},
    window::{Direction, Window, WindowId, WindowTree},
};
use maxgus_config::{Settings, ThemeSpec};
use maxgus_faces::Theme;
use maxgus_keys::{KeySequence, KeymapSet};
use maxgus_text::{Buffer, BufferId, KillRing, Range, Registers};
use maxgus_tui::Rect;
use std::path::{Path, PathBuf};

/// Everything the editor knows.
/// How many screenfuls either side of the window are highlighted, so ordinary
/// scrolling does not outrun the last query.
pub const HIGHLIGHT_MARGIN_SCREENS: usize = 3;

#[derive(Debug)]
pub struct Editor {
    pub buffers: BufferList,
    pub windows: WindowTree,
    pub minibuffer: Minibuffer,
    pub kill_ring: KillRing,
    /// The system clipboard, when the front end has one to give. A kill
    /// goes there too, and a yank takes what another program left there.
    pub clipboard: Option<Box<dyn crate::clipboard::Clipboard>>,
    pub registers: Registers,
    pub settings: Settings,
    pub theme: Theme,
    /// The `theme "name" { … }` blocks from the configuration file, kept so a
    /// theme can be rebuilt later with the user's overrides still on it.
    pub theme_specs: Vec<ThemeSpec>,
    /// The buffer whose save was refused because its file had changed, which
    /// is the only one `save-buffer-anyway` will write.
    pub pending_overwrite: Option<BufferId>,
    /// The branch the project is on, for the mode line. `None` until the
    /// executor has been able to ask git, and when it is not a repository.
    #[cfg(feature = "full")]
    pub git_branch: Option<String>,
    /// The parser list as last fetched, so a line picked out of the menu
    /// finds the repository it came from.
    #[cfg(feature = "full")]
    pub grammar_catalog: Vec<maxgus_syntax::Parser>,
    /// The language an offer to install is about, held while the question is
    /// on screen. The current buffer can change under a prompt; the question
    /// asked cannot.
    #[cfg(feature = "full")]
    pub grammar_offer: Option<String>,
    /// The one parser a `yes` would install, held while that question is on
    /// screen. Set only just before the question is asked.
    #[cfg(feature = "full")]
    pub grammar_pending: Option<maxgus_syntax::Parser>,
    /// Languages the user has said no to. Asked once, not once a keystroke:
    /// the answer to "shall I install a grammar for this" does not change
    /// because the file was edited again.
    #[cfg(feature = "full")]
    pub grammars_declined: std::collections::HashSet<String>,
    /// The theme that was in use before `consult-theme` started previewing, so
    /// abandoning the prompt puts it back.
    pub theme_before_preview: Option<String>,
    /// Whether the visit now being made should be written down when it is
    /// accepted, which is what a prefix argument on `consult-theme` asks for.
    /// Remembered because the argument is given when the prompt opens and
    /// wanted when it closes.
    pub consult_theme_writes: bool,
    /// Where the configuration was read from, so a theme can be written back
    /// to it. `None` when the editor started without one.
    pub config_path: Option<PathBuf>,
    /// Where the editor keeps what is its own business rather than the
    /// user's — sessions, and whatever else comes to want a home.
    pub state_dir: Option<PathBuf>,
    /// The saved sets of directories, read at startup.
    pub workspaces: crate::workspace::Workspaces,
    /// Which of them is open, when one was opened by name. For the message
    /// that says what you are in, and so saving again offers the same name.
    pub workspace: Option<String>,
    /// The default the workspace prompt offered, kept for the moment
    /// between it being asked and being answered: an untouched prompt
    /// answers with nothing, and this is what nothing means.
    pub pending_workspace: Option<String>,
    /// The theme the configuration file names, as distinct from the one in
    /// use — which is how `consult-theme` knows whether there is anything worth
    /// writing down.
    pub config_says_theme: Option<String>,
    /// A file being read, and the line point should land on when it arrives.
    pub pending_line: Option<(PathBuf, usize)>,
    /// The light beside the cursor, while one is showing.
    pub beacon: Option<crate::beacon::Beacon>,
    /// The loaded script, and where it came from.
    #[cfg(feature = "full")]
    pub script: Option<maxgus_script::Script>,
    #[cfg(feature = "full")]
    pub script_path: Option<PathBuf>,
    /// The directory listing, when one is open.
    pub dired: Option<crate::dired::DiredView>,
    /// The file browser, when it is up: a box over the frame that narrows
    /// as you type. Nothing to do with `dired`, which is a buffer.
    pub browser: Option<crate::browser::Browser>,
    /// The snippets that have been loaded, from the configuration directory.
    pub snippets: Vec<crate::snippet::Snippet>,
    /// The fields of the snippet being filled in, as buffer offsets.
    pub snippet_fields: Vec<crate::snippet::Field>,
    /// Which of them point is on.
    pub snippet_field: usize,
    /// The buffer length the fields were last made right for, when a
    /// snippet command edited the buffer and moved them itself. The
    /// dispatcher moves them past whatever a command changed, and this
    /// tells it that part is done.
    pub snippet_fields_fit: Option<usize>,
    /// Where a restored session wants point in each of its files.
    pub session_points: std::collections::HashMap<PathBuf, (usize, usize)>,
    /// Whether the session being restored had the panel open.
    pub session_panel: bool,
    /// What each buffer's `.editorconfig` asked for.
    pub editor_configs: std::collections::HashMap<BufferId, crate::task::EditorConfig>,
    /// Cursors besides the window's own, which every editing command is run
    /// at as well.
    pub cursors: crate::multi::Cursors,
    /// The buffer whose history the visualiser is showing.
    pub undo_tree_subject: Option<BufferId>,
    /// The results of the last project search.
    #[cfg(feature = "full")]
    pub grep: Option<crate::grep::GrepView>,
    /// The search that produced them, so `g` can run it again.
    #[cfg(feature = "full")]
    pub grep_search: Option<maxgus_grep::Search>,
    /// What an empty answer at the search prompt means.
    #[cfg(feature = "full")]
    pub grep_default: Option<String>,
    /// How long the editor took to be ready, measured by whoever started it.
    pub startup_time: Option<std::time::Duration>,
    pub tasks: TaskQueue,
    /// The prefix argument for the command about to run.
    pub prefix: Prefix,
    /// The command that just ran, which `yank-pop` and friends consult.
    pub last_command: Option<String>,
    /// The command currently running.
    pub this_command: Option<String>,
    /// True once the editor should exit.
    pub quit: bool,
    /// True when the next kill should append to the previous one rather than
    /// pushing a new entry — Emacs' `last-command` check for consecutive kills.
    pub kill_appending: bool,
    /// Where the file tree window is, when it is open.
    pub tree_window: Option<WindowId>,
    /// A command waiting for one character, with the prefix argument it was
    /// invoked with. The next key goes to it instead of being dispatched.
    pub pending_char: Option<(String, Prefix)>,
    /// The keys `describe-key` has read so far, while the sequence is still
    /// a prefix of something.
    pub described_keys: KeySequence,
    /// The command an open prompt is collecting text for, with the prefix
    /// argument it was invoked with. Accepting the prompt re-enters it.
    pub pending_input: Option<(String, Prefix)>,
    /// Candidates the open prompt completes against.
    pub completion_candidates: Vec<String>,
    /// A command to run as soon as the current one finishes. Accepting a
    /// prompt uses this to re-enter the command that opened it.
    pub deferred: Option<(String, crate::command::Args)>,
    /// The active keymaps, in Emacs' precedence order. Minor maps are pushed
    /// and popped as the minibuffer, isearch and the tree take over input.
    pub keymaps: KeymapSet,
    /// Diagnostics from every language server, keyed by document URI.
    #[cfg(feature = "full")]
    pub diagnostics: maxgus_lsp::DiagnosticSet,
    /// The incremental search in progress, if any.
    pub isearch: Option<crate::commands::search::Isearch>,
    /// The `query-replace` in progress, if any.
    pub query_replace: Option<crate::commands::search::QueryReplace>,
    /// The most recent snapshot of the file tree. The tree itself lives in the
    /// event loop, where its directory reads can be awaited; this is what the
    /// editor draws and navigates.
    pub tree: Vec<maxgus_tree::VisibleNode>,
    /// The side panel's own state: which of its three sections are on, and
    /// the outline of whatever buffer was last asked about.
    pub panel: crate::panel::Panel,
    /// The panel's windows, top to bottom. The first is the file tree.
    pub panel_windows: Vec<WindowId>,
    /// Windows that were split off to show a listing or a help buffer, so
    /// `q` in one can take the window away again rather than leave a split
    /// nobody asked for.
    pub popped_windows: Vec<WindowId>,
    /// The listings on screen — `*Occur*`, `*xref*` — by buffer name: where
    /// each row points, and what to highlight in it.
    pub listings: std::collections::HashMap<String, crate::commands::listing::Listing>,
    /// How tall the outline and the buffer list are, in rows. The tree takes
    /// whatever they leave.
    pub symbols_height: u16,
    pub buffers_height: u16,
    /// The state of the repository, as the status view shows it.
    #[cfg(feature = "full")]
    pub git: crate::git::GitView,
    /// The repository the status view is about.
    #[cfg(feature = "full")]
    pub git_root: Option<PathBuf>,
    /// Branches, for the checkout and merge prompts.
    #[cfg(feature = "full")]
    pub git_branches: Vec<String>,
    /// Every reference, with what kind it is, for the references view.
    #[cfg(feature = "full")]
    pub git_references: Vec<maxgus_git::Reference>,
    /// The diff and revision buffers, by name.
    #[cfg(feature = "full")]
    pub git_diffs: std::collections::HashMap<String, crate::git::DiffView>,
    /// The log, references and process buffers, by name.
    #[cfg(feature = "full")]
    pub git_lists: std::collections::HashMap<String, crate::git::ListView>,
    /// What git has been asked to do this session, for the process buffer.
    #[cfg(feature = "full")]
    pub git_history: Vec<(String, String)>,
    /// Which buffer the answer being waited for belongs in.
    #[cfg(feature = "full")]
    pub git_pending_view: Option<&'static str>,
    /// True while the commit buffer is amending rather than committing.
    #[cfg(feature = "full")]
    pub committing_amend: bool,
    /// The switches the commit menu had on, kept until the message is done.
    #[cfg(feature = "full")]
    pub committing_arguments: Vec<String>,
    /// The menu showing, if one is.
    #[cfg(feature = "full")]
    pub transient: Option<crate::transient::Active>,
    /// Switches the menu was holding when it ran a command, for that command
    /// to read. Cleared when the next menu opens.
    #[cfg(feature = "full")]
    pub transient_arguments: Vec<String>,
    /// Terminal tabs, and which of them is showing.
    #[cfg(feature = "full")]
    pub terminals: crate::terminal::Terminals,
    /// The window the terminal panel is drawn in, when it is open.
    #[cfg(feature = "full")]
    pub terminal_window: Option<WindowId>,
    /// How tall the terminal panel is, in rows.
    #[cfg(feature = "full")]
    pub terminal_height: u16,
    /// Whether the tree is showing dotfiles, for the header line.
    pub tree_shows_hidden: bool,
    /// The directory the tree is rooted at.
    pub tree_root: Option<PathBuf>,
    /// Where it was rooted when it opened, which is what
    /// `treefile-root-reset` comes back to after walking into a
    /// subdirectory. Kept separately so root-down cannot lose it.
    pub tree_home: Option<PathBuf>,
    /// The tree window's width in columns.
    pub tree_width: u16,
    /// True while the width is pinned against the resize commands.
    pub tree_width_locked: bool,
    /// True while the tree follows the selected buffer.
    pub tree_follow: bool,
    /// The keyboard macro being recorded, if any.
    pub recording_macro: Option<Vec<maxgus_keys::Key>>,
    /// The last macro recorded, which `C-x e` replays.
    pub last_macro: Vec<maxgus_keys::Key>,
    /// True while a macro is replaying, so recording does not capture itself.
    pub replaying_macro: bool,
    /// How many times the loop should replay the last macro. Cleared once it
    /// has done so.
    pub macro_repeats: usize,
    /// True once the editor should suspend itself.
    pub suspend: bool,
    /// Command names `M-x` completes against, filled in at startup.
    pub command_names: Vec<String>,
    /// Command documentation, for `describe-function`.
    pub command_docs: Vec<(String, String)>,
    /// The position encoding each running language server negotiated.
    #[cfg(feature = "full")]
    pub lsp_encodings: Vec<(String, maxgus_lsp::PositionEncoding)>,
    /// The buffer revision each language server was last told about, so a
    /// change notification is sent exactly when the document has moved on.
    #[cfg(feature = "full")]
    pub lsp_versions: std::collections::HashMap<BufferId, u64>,
    /// A jump waiting on a file to be read, applied once it arrives.
    #[cfg(feature = "full")]
    pub pending_jump: Option<(PathBuf, maxgus_lsp::LspPosition)>,
    /// Keymaps defined for a major mode by the configuration, activated when
    /// a buffer of that mode is selected.
    pub mode_keymaps: Vec<maxgus_keys::Keymap>,
    /// The whole terminal, including the echo area's row. The window tree only
    /// covers everything above it.
    pub frame: Rect,
    /// The half-typed key sequence, echoed so the user can see where they are.
    pub pending_keys: Option<String>,
    /// The suggestions on screen, if any.
    #[cfg(feature = "full")]
    pub autocomplete: Option<crate::autocomplete::Autocomplete>,
    /// Where point was when the suggestions now showing were asked for, so
    /// an idle pause in the same place does not ask again.
    #[cfg(feature = "full")]
    pub completions_asked_at: Option<(maxgus_text::BufferId, usize)>,
    /// What the language server said about the symbol under point, and
    /// which line it was about, so the box can sit beside it.
    ///
    /// `lsp-ui-doc` for Emacs. A reply used to open a help window, which
    /// pushed the code aside to say one sentence about it.
    #[cfg(feature = "full")]
    pub doc: Option<crate::Doc>,
    /// Where point was when the doc now showing was asked for, so an idle
    /// pause in the same place does not ask again.
    #[cfg(feature = "full")]
    pub doc_asked_at: Option<(maxgus_text::BufferId, usize)>,
    /// The half-typed sequence whose continuations are on show.
    ///
    /// Set by the front end once someone has paused long enough to want
    /// help, and cleared the moment the sequence finishes or is abandoned.
    /// Kept apart from `pending_keys` because the two have their own delays:
    /// the echo says where you are, this says where you can go.
    pub which_key: Option<String>,
    /// A whole keymap on screen, asked for rather than waited for.
    ///
    /// Separate from `which_key` because the two are dismissed by different
    /// things: a half-typed sequence is over the moment a command runs, and
    /// this stays up across the commands it is describing — which is what
    /// makes it possible to read it and walk the tree at the same time.
    pub key_menu: Option<crate::which_key::Menu>,
    /// The current buffer's text, kept between operations that need it whole.
    /// An incremental search does one per keystroke; rendering the rope each
    /// time would cost the size of the buffer for every character typed.
    text_cache: Option<(BufferId, u64, String)>,
    /// Syntax highlighting per buffer: the revision it was computed for, the
    /// byte range it covers, and the spans themselves.
    ///
    /// A stale entry is still drawn — colours a keystroke behind are better
    /// than none — and replaced when the re-parse comes back.
    #[cfg(feature = "full")]
    pub highlights: std::collections::HashMap<
        BufferId,
        (u64, std::ops::Range<usize>, Vec<maxgus_syntax::Highlight>),
    >,
}

impl Editor {
    /// A fresh editor showing `*scratch*` in one window.
    pub fn new(settings: Settings, theme: Theme, frame: Rect) -> Editor {
        let panel = crate::panel::Panel::from_settings(&settings);
        let (symbols_height, buffers_height) = (
            settings.panel_symbols_height as u16,
            settings.panel_buffers_height as u16,
        );
        let buffers = BufferList::new();
        let first = *buffers
            .ids()
            .first()
            .expect("the list starts with *scratch*");
        let kill_ring = KillRing::new(settings.kill_ring_max);
        // Windows occupy everything but the echo area's row.
        let (body, _) = frame.split_bottom(1);
        let mut editor = Editor {
            windows: WindowTree::new(first, body),
            buffers,
            minibuffer: Minibuffer::new(),
            kill_ring,
            clipboard: None,
            registers: Registers::new(),
            settings,
            theme,
            theme_specs: Vec::new(),
            pending_overwrite: None,
            #[cfg(feature = "full")]
            git_branch: None,
            #[cfg(feature = "full")]
            grammar_catalog: Vec::new(),
            #[cfg(feature = "full")]
            grammar_offer: None,
            #[cfg(feature = "full")]
            grammar_pending: None,
            #[cfg(feature = "full")]
            grammars_declined: std::collections::HashSet::new(),
            theme_before_preview: None,
            consult_theme_writes: false,
            config_path: None,
            state_dir: None,
            workspaces: crate::workspace::Workspaces::default(),
            workspace: None,
            pending_workspace: None,
            config_says_theme: None,
            pending_line: None,
            beacon: None,
            #[cfg(feature = "full")]
            script: None,
            #[cfg(feature = "full")]
            script_path: None,
            dired: None,
            browser: None,
            snippets: Vec::new(),
            snippet_fields: Vec::new(),
            snippet_field: 0,
            snippet_fields_fit: None,
            session_points: std::collections::HashMap::new(),
            session_panel: false,
            editor_configs: std::collections::HashMap::new(),
            cursors: crate::multi::Cursors::new(),
            undo_tree_subject: None,
            #[cfg(feature = "full")]
            grep: None,
            #[cfg(feature = "full")]
            grep_search: None,
            #[cfg(feature = "full")]
            grep_default: None,
            startup_time: None,
            tasks: TaskQueue::new(),
            prefix: Prefix::None,
            last_command: None,
            this_command: None,
            quit: false,
            kill_appending: false,
            tree_window: None,
            pending_char: None,
            described_keys: KeySequence::empty(),
            pending_input: None,
            completion_candidates: Vec::new(),
            panel,
            panel_windows: Vec::new(),
            popped_windows: Vec::new(),
            listings: std::collections::HashMap::new(),
            symbols_height,
            buffers_height,
            #[cfg(feature = "full")]
            git: crate::git::GitView::new(),
            #[cfg(feature = "full")]
            git_root: None,
            #[cfg(feature = "full")]
            git_branches: Vec::new(),
            #[cfg(feature = "full")]
            git_references: Vec::new(),
            #[cfg(feature = "full")]
            git_diffs: std::collections::HashMap::new(),
            #[cfg(feature = "full")]
            git_lists: std::collections::HashMap::new(),
            #[cfg(feature = "full")]
            git_history: Vec::new(),
            #[cfg(feature = "full")]
            git_pending_view: None,
            #[cfg(feature = "full")]
            committing_amend: false,
            #[cfg(feature = "full")]
            committing_arguments: Vec::new(),
            #[cfg(feature = "full")]
            transient: None,
            #[cfg(feature = "full")]
            transient_arguments: Vec::new(),
            #[cfg(feature = "full")]
            terminals: crate::terminal::Terminals::new(),
            #[cfg(feature = "full")]
            terminal_window: None,
            #[cfg(feature = "full")]
            terminal_height: 14,
            deferred: None,
            keymaps: KeymapSet::new(
                crate::keymap::global_keymap().expect("the built-in global map is well formed"),
            ),
            #[cfg(feature = "full")]
            diagnostics: maxgus_lsp::DiagnosticSet::new(),
            isearch: None,
            query_replace: None,
            tree: Vec::new(),
            tree_shows_hidden: false,
            tree_root: None,
            tree_home: None,
            tree_width: 32,
            tree_width_locked: false,
            tree_follow: true,
            recording_macro: None,
            last_macro: Vec::new(),
            replaying_macro: false,
            macro_repeats: 0,
            suspend: false,
            command_names: Vec::new(),
            command_docs: Vec::new(),
            #[cfg(feature = "full")]
            lsp_encodings: Vec::new(),
            #[cfg(feature = "full")]
            lsp_versions: std::collections::HashMap::new(),
            #[cfg(feature = "full")]
            pending_jump: None,
            mode_keymaps: Vec::new(),
            frame,
            pending_keys: None,
            which_key: None,
            key_menu: None,
            #[cfg(feature = "full")]
            autocomplete: None,
            #[cfg(feature = "full")]
            completions_asked_at: None,
            #[cfg(feature = "full")]
            doc: None,
            #[cfg(feature = "full")]
            doc_asked_at: None,
            text_cache: None,
            #[cfg(feature = "full")]
            highlights: std::collections::HashMap::new(),
        };
        editor.sync_from_buffer();
        editor.apply_settings_everywhere();
        editor
    }

    /// Replaces a buffer's contents, keeping every window showing it in range.
    ///
    /// A window carries its own point and scroll position. Replacing the text
    /// underneath one — as `*Help*`, `*Occur*` and the tree all do — leaves
    /// that point past the end of what is now there, and everything drawn
    /// afterwards is measured against a buffer that no longer has those
    /// characters.
    pub fn replace_buffer_contents(&mut self, buffer: BufferId, text: &str) -> Result<()> {
        self.buffers.revert(buffer, text)?;
        let Some(target) = self.buffers.get(buffer) else {
            return Ok(());
        };
        let (length, lines) = (target.len_chars(), target.len_lines());
        for id in self.windows.showing(buffer) {
            if let Some(window) = self.windows.get_mut(id) {
                window.point = window.point.min(length);
                window.top_line = window.top_line.min(lines.saturating_sub(1));
                window.goal_column = None;
            }
        }
        Ok(())
    }

    /// Gives `buffer` the editor's settings.
    ///
    /// A buffer carries its own tab width and indentation style, so that a
    /// mode could override them; they start from the configuration. Without
    /// this a configured `tab-width` would reach the indent commands but not
    /// redisplay, and a tab would be inserted as one width and drawn as
    /// another.
    pub fn apply_settings(&mut self, buffer: BufferId) {
        let (mut width, mut tabs) = (self.settings.tab_width, self.settings.indent_with_tabs);
        // What the file's own `.editorconfig` said wins: it is the project
        // speaking about this file, and the configuration is the user
        // speaking about files in general.
        if let Some(asked) = self.editor_configs.get(&buffer) {
            width = asked.tab_width.unwrap_or(width);
            tabs = asked.indent_with_tabs.unwrap_or(tabs);
        }
        if let Some(buffer) = self.buffers.get_mut(buffer) {
            buffer.set_tab_width(width);
            buffer.set_indent_with_tabs(tabs);
        }
    }

    /// Records what a file's `.editorconfig` asked for, and applies it.
    pub fn set_editor_config(&mut self, buffer: BufferId, asked: crate::task::EditorConfig) {
        if asked.is_empty() {
            self.editor_configs.remove(&buffer);
            return;
        }
        if let Some(crlf) = asked.crlf
            && let Some(target) = self.buffers.get_mut(buffer)
        {
            target.set_line_ending(match crlf {
                true => maxgus_text::LineEnding::Crlf,
                false => maxgus_text::LineEnding::Lf,
            });
        }
        self.editor_configs.insert(buffer, asked);
        self.apply_settings(buffer);
    }

    /// What a buffer's `.editorconfig` asked for, for the settings that are
    /// read at the moment they are used rather than applied to the buffer.
    pub fn editor_config(&self, buffer: BufferId) -> Option<&crate::task::EditorConfig> {
        self.editor_configs.get(&buffer)
    }

    /// Whether trailing whitespace should be trimmed from `buffer` on save.
    pub fn trims_trailing_whitespace(&self, buffer: BufferId) -> bool {
        self.editor_config(buffer)
            .and_then(|asked| asked.trim_trailing_whitespace)
            .unwrap_or(self.settings.delete_trailing_whitespace)
    }

    /// Whether `buffer` should end with a newline when it is written.
    pub fn requires_final_newline(&self, buffer: BufferId) -> bool {
        self.editor_config(buffer)
            .and_then(|asked| asked.final_newline)
            .unwrap_or(self.settings.require_final_newline)
    }

    /// The column the fill commands and the indicator use for `buffer`.
    pub fn fill_column_for(&self, buffer: BufferId) -> usize {
        self.editor_config(buffer)
            .and_then(|asked| asked.fill_column)
            .unwrap_or(self.settings.fill_column)
    }

    /// Applies the settings to every buffer, for startup and after the
    /// configuration changes.
    pub fn apply_settings_everywhere(&mut self) {
        for id in self.buffers.ids().to_vec() {
            self.apply_settings(id);
        }
    }

    // ---- current buffer and window -------------------------------------

    /// The buffer the selected window shows.
    pub fn current_buffer_id(&self) -> BufferId {
        self.windows.current().buffer
    }

    /// The selected window's buffer, with point already synchronised.
    pub fn current_buffer(&self) -> &Buffer {
        self.buffers
            .get(self.current_buffer_id())
            .expect("every window shows a live buffer")
    }

    /// The selected window's buffer for mutation. Point is written from the
    /// window first, so the command sees the cursor where the user sees it.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        self.sync_to_buffer();
        let id = self.current_buffer_id();
        self.buffers
            .get_mut(id)
            .expect("every window shows a live buffer")
    }

    /// Copies the selected window's point and goal column into its buffer.
    pub fn sync_to_buffer(&mut self) {
        let window = self.windows.current();
        let (point, goal, id) = (window.point, window.goal_column, window.buffer);
        if let Some(buffer) = self.buffers.get_mut(id) {
            buffer.set_point_keeping_goal(point);
            buffer.set_goal_column(goal);
        }
    }

    /// Copies point back out of the buffer into the selected window, after a
    /// command has moved it.
    pub fn sync_from_buffer(&mut self) {
        let id = self.current_buffer_id();
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let (point, goal) = (buffer.point(), buffer.goal_column());
        let window = self.windows.current_mut();
        window.point = point;
        window.goal_column = goal;
    }

    /// Runs `f` on the current buffer with point synchronised in both
    /// directions, which is what every editing command needs.
    pub fn with_current_buffer<T>(&mut self, f: impl FnOnce(&mut Buffer) -> T) -> T {
        self.sync_to_buffer();
        let id = self.current_buffer_id();
        let (out, adjustments) = {
            let buffer = self
                .buffers
                .get_mut(id)
                .expect("every window shows a live buffer");
            let out = f(buffer);
            (out, buffer.take_adjustments())
        };
        self.sync_from_buffer();
        self.follow_edits(id, &adjustments);
        out
    }

    /// Moves the other windows showing `buffer` across the edits just made.
    ///
    /// Point and the mark move with an edit inside the buffer, but a window
    /// keeps its own point, and the same buffer can be shown in several. Emacs
    /// does this with markers; the effect is what matters — a second window
    /// stays on the text it was on rather than on the offset it happened to
    /// have.
    fn follow_edits(&mut self, buffer: BufferId, adjustments: &[(usize, usize, usize)]) {
        if adjustments.is_empty() {
            return;
        }
        let selected = self.windows.current_id();
        let length = self.buffers.get(buffer).map_or(0, |b| b.len_chars());
        let lines = self.buffers.get(buffer).map_or(1, |b| b.len_lines());
        for id in self.windows.showing(buffer) {
            if id == selected {
                continue;
            }
            let Some(window) = self.windows.get_mut(id) else {
                continue;
            };
            for (at, removed, inserted) in adjustments {
                window.point = Buffer::adjust_position(window.point, *at, *removed, *inserted);
            }
            window.point = window.point.min(length);
            window.top_line = window.top_line.min(lines.saturating_sub(1));
        }
    }

    /// Shows `buffer` in the selected window, remembering it as most recently
    /// used.
    pub fn switch_to_buffer(&mut self, buffer: BufferId) -> Result<()> {
        if self.buffers.get(buffer).is_none() {
            return Err(crate::CoreError::NoSuchBuffer);
        }
        self.sync_to_buffer();
        let point = self.buffers.get(buffer).expect("checked above").point();
        let window = self.windows.current_mut();
        window.buffer = buffer;
        window.point = point;
        window.top_line = 0;
        window.goal_column = None;
        self.buffers.touch(buffer);
        self.apply_settings(buffer);
        self.activate_mode_keymap();
        self.follow_point();
        self.follow_panel_to_buffer();
        Ok(())
    }

    /// The window the terminal panel is in, if this build has one at all.
    ///
    /// A build without the terminal has no such window, and the windows that
    /// ask — which one is the file being edited in — need the same answer
    /// either way rather than a `cfg` at each of them.
    pub fn terminal_pane(&self) -> Option<WindowId> {
        #[cfg(feature = "full")]
        {
            self.terminal_window
        }
        #[cfg(not(feature = "full"))]
        {
            None
        }
    }

    /// Puts point at the start of `line`, clamped into the buffer.
    ///
    /// A line rather than a protocol position: what a hunk header, a compiler
    /// message and `M-g g` all mean by "go there".
    pub fn go_to_line(&mut self, line: usize) {
        let offset = {
            let buffer = self.current_buffer();
            buffer.line_start(line.min(buffer.len_lines().saturating_sub(1)))
        };
        self.windows.current_mut().point = offset;
        self.with_current_buffer(move |b| b.set_point(offset));
        self.follow_point();
    }

    // ---- what a pointer means ------------------------------------------

    /// The buffer position under the cell at `column`, `row` of the frame.
    ///
    /// `None` when the cell is not in a window at all — the echo area, or a
    /// mode line — or when it is past the end of the text, which is a click
    /// on empty space rather than on a character.
    pub fn position_at_cell(&self, column: u16, row: u16) -> Option<(WindowId, usize)> {
        let id = self.windows.window_at(column, row)?;
        let window = self.windows.get(id)?;
        // The last row of a window is its mode line, which is not text.
        let (text, _) = window.rect.split_bottom(1);
        if !text.contains(column, row) {
            return None;
        }
        let buffer = self.buffers.get(window.buffer)?;
        let line = window.top_line + (row - text.y) as usize;
        if line >= buffer.len_lines() {
            // Past the last line: the end of the buffer, as clicking below
            // the text does everywhere.
            return Some((id, buffer.len_chars()));
        }
        // The gutter is drawn in the window but is not part of the line.
        let gutter = crate::render::line_number_width(self, buffer);
        let start = buffer.line_start(line);
        let end = maxgus_text::Motion::line_end(buffer.rope(), start);
        let into = (column.saturating_sub(text.x + gutter)) as usize + window.left_column;
        Some((id, (start + into).min(end)))
    }

    /// Selects the window under the pointer and puts point where it is.
    pub fn point_at_cell(&mut self, column: u16, row: u16) -> bool {
        let Some((window, offset)) = self.position_at_cell(column, row) else {
            return false;
        };
        self.select_window(window);
        self.windows.current_mut().point = offset;
        self.with_current_buffer(move |b| b.set_point(offset));
        self.follow_point();
        true
    }

    /// Puts the mark where point is, so a drag has something to extend from.
    pub fn set_mark_here(&mut self) {
        let point = self.windows.current().point;
        self.with_current_buffer(move |b| b.set_mark(point));
    }

    /// A double click: the word under point becomes the region, mark at
    /// its start and point at its end. Nothing happens off a word.
    pub fn select_word_at_point(&mut self) -> bool {
        let point = self.windows.current().point;
        let Some((start, end)) =
            self.with_current_buffer(|b| maxgus_text::Motion::word_bounds(b.rope(), point))
        else {
            return false;
        };
        self.select(start, end);
        true
    }

    /// A triple click: the line under point becomes the region, newline
    /// and all, so a yank of it is a line and not a fragment.
    pub fn select_line_at_point(&mut self) {
        let point = self.windows.current().point;
        let (start, end) = self.with_current_buffer(|b| {
            let line = b.line_of(point);
            (b.line_start(line), b.line_start(line + 1))
        });
        self.select(start, end);
    }

    /// Makes `start..end` the region, with point at the end of it.
    fn select(&mut self, start: usize, end: usize) {
        self.with_current_buffer(move |b| {
            b.set_mark(start);
            b.set_point(end);
        });
        self.windows.current_mut().point = end;
        self.follow_point();
    }

    /// The right button: the region grows to take in the cell under the
    /// pointer, from wherever the mark is — or from point, when there is
    /// no region yet, so a click and a right click select what is between.
    pub fn extend_region_to_cell(&mut self, column: u16, row: u16) -> bool {
        let Some((window, _)) = self.position_at_cell(column, row) else {
            return false;
        };
        if window != self.windows.current_id() {
            return false;
        }
        if self.current_buffer().region().is_none() {
            self.set_mark_here();
        }
        self.extend_to_cell(column, row)
    }

    /// Drags the selection to the cell under the pointer.
    ///
    /// Only within the window the drag started in: a drag that wanders into
    /// another window should not move point into it.
    pub fn extend_to_cell(&mut self, column: u16, row: u16) -> bool {
        let Some((window, offset)) = self.position_at_cell(column, row) else {
            return false;
        };
        if window != self.windows.current_id() {
            return false;
        }
        self.windows.current_mut().point = offset;
        self.with_current_buffer(move |b| b.set_point(offset));
        self.follow_point();
        true
    }

    /// The text between mark and point, if there is a region.
    pub fn region_text(&self) -> Option<String> {
        let buffer = self.current_buffer();
        let region = buffer.region()?;
        Some(buffer.slice(region).chars().collect())
    }

    /// Scrolls the selected window by whole lines, as the wheel asks.
    ///
    /// Point follows the view rather than the other way round, which is what
    /// a wheel means: the text moves and the cursor stays where it is on the
    /// screen until it would leave it.
    pub fn scroll_lines(&mut self, lines: isize) {
        self.scroll_window_lines(self.windows.current_id(), lines);
    }

    /// Scrolls a window that is not necessarily the selected one.
    ///
    /// A wheel turned over a window scrolls that window, whether or not it
    /// is the one being typed into — which is what every other program does,
    /// and what stops a turn of the wheel over the file tree from moving the
    /// code beside it.
    pub fn scroll_window_lines(&mut self, id: crate::window::WindowId, lines: isize) {
        let Some(buffer_id) = self.windows.get(id).map(|window| window.buffer) else {
            return;
        };
        let Some(total) = self.buffers.get(buffer_id).map(|b| b.len_lines()) else {
            return;
        };
        let Some(window) = self.windows.get_mut(id) else {
            return;
        };
        let top = window.top_line as isize + lines;
        window.top_line = top.clamp(0, total.saturating_sub(1) as isize) as usize;
        let top_line = window.top_line;
        let height = window.rect.height.saturating_sub(1) as usize;
        let point = window.point;
        // Point stays on screen: a wheel that has scrolled past it drags it
        // along rather than leaving it somewhere nobody can see.
        let Some(buffer) = self.buffers.get(buffer_id) else {
            return;
        };
        let point_line = buffer.line_of(point);
        let wanted = point_line.clamp(top_line, top_line + height.saturating_sub(1));
        if wanted == point_line {
            return;
        }
        let offset = buffer.line_start(wanted);
        if let Some(window) = self.windows.get_mut(id) {
            window.point = offset;
        }
        // The buffer's own point belongs to whichever window is selected, so
        // scrolling somebody else's window leaves it alone.
        if id == self.windows.current_id() {
            self.with_current_buffer(move |b| b.set_point(offset));
        }
    }

    /// Puts a list of suggestions on screen and gives it the keys it needs.
    ///
    /// A minor keymap rather than a mode: `C-n` and `RET` mean the list
    /// while it is up and mean what they always mean the moment it is not,
    /// which is the whole behaviour someone expects of a popup.
    #[cfg(feature = "full")]
    pub fn open_autocomplete(&mut self, list: crate::autocomplete::Autocomplete) {
        if self.autocomplete.is_none() {
            self.push_minor_map(
                crate::keymap::autocomplete_keymap()
                    .expect("the built-in autocomplete map is well formed"),
            );
        }
        self.autocomplete = Some(list);
    }

    /// Takes it away, and the keys with it.
    #[cfg(feature = "full")]
    pub fn close_autocomplete(&mut self) {
        if self.autocomplete.take().is_some() {
            self.remove_minor_map("autocomplete-mode");
        }
    }

    #[cfg(not(feature = "full"))]
    pub fn close_autocomplete(&mut self) {}

    /// Moves point in the selected window and its buffer together.
    pub fn move_point_to(&mut self, offset: usize) {
        let offset = offset.min(self.current_buffer().len_chars());
        self.windows.current_mut().point = offset;
        self.with_current_buffer(move |b| b.set_point(offset));
        self.follow_point();
    }

    /// The name of the current buffer's major mode, for the things that are
    /// arranged by mode — snippets, and the keymaps.
    pub fn current_mode_name(&self) -> Option<String> {
        self.mode_keymap_name(self.current_buffer_id())
    }

    /// Moves the fields of the snippet being filled in past an edit.
    ///
    /// The same problem the extra cursors have: a field is an offset into
    /// text that is being changed under it, and one that is not moved points
    /// at the wrong characters after the first keystroke.
    pub fn shift_snippet_fields(&mut self, at: usize, delta: isize) {
        for field in &mut self.snippet_fields {
            field.start = crate::multi::shift_by_delta(field.start, at, delta);
            field.end = crate::multi::shift_by_delta(field.end, at, delta);
        }
    }

    /// Takes out the field's default before it is typed over.
    ///
    /// Yasnippet's behaviour, and the reason for showing a default at all:
    /// it is a suggestion to accept by tabbing past or replace by typing.
    /// Vanilla Emacs has no `delete-selection-mode`, so this is scoped to a
    /// snippet rather than turned on everywhere.
    pub fn take_snippet_field(&mut self) -> Result<()> {
        if !self.in_snippet() {
            return Ok(());
        }
        let Some(region) = self.current_buffer().region() else {
            return Ok(());
        };
        if region.is_empty() {
            return Ok(());
        }
        self.with_current_buffer(move |b| -> maxgus_text::Result<()> {
            b.delete(region)?;
            b.set_point(region.start);
            b.deactivate_mark();
            Ok(())
        })?;
        self.windows.current_mut().point = region.start;
        Ok(())
    }

    /// Stops filling in a snippet.
    pub fn end_snippet(&mut self) {
        self.snippet_fields.clear();
        self.snippet_field = 0;
        self.with_current_buffer(|b| b.deactivate_mark());
    }

    /// True while a snippet is being filled in, which is what makes `TAB`
    /// move to the next field rather than indent.
    pub fn in_snippet(&self) -> bool {
        !self.snippet_fields.is_empty()
    }

    /// True when a script defined a command by this name.
    #[cfg(feature = "full")]
    pub fn has_script_command(&self, name: &str) -> bool {
        self.script
            .as_ref()
            .is_some_and(|script| script.commands().iter().any(|c| c.name == name))
    }

    /// Takes a freshly loaded script, and offers its commands to `M-x`.
    #[cfg(feature = "full")]
    pub fn set_script(&mut self, script: maxgus_script::Script) {
        // Whatever the last script offered is no longer on offer.
        if let Some(previous) = self.script.take() {
            let gone: Vec<&str> = previous
                .commands()
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            self.command_names
                .retain(|name| !gone.contains(&name.as_str()));
            self.command_docs
                .retain(|(name, _)| !gone.contains(&name.as_str()));
        }
        let count = script.commands().len();
        for command in script.commands() {
            if !self.command_names.contains(&command.name) {
                self.command_names.push(command.name.clone());
            }
            self.command_docs
                .push((command.name.clone(), command.doc.clone()));
        }
        self.command_names.sort();
        self.script = Some(script);
        self.message(format!(
            "{} from the script",
            crate::count(count, "command")
        ));
    }

    // ---- the light beside the cursor -----------------------------------

    /// What the beacon watches: which buffer, which window, and where in it.
    pub fn beacon_watch(&self) -> crate::beacon::Watch {
        let window = self.windows.current();
        let line = self
            .buffers
            .get(window.buffer)
            .map(|buffer| buffer.line_of(window.point.min(buffer.len_chars())))
            .unwrap_or(0);
        crate::beacon::Watch {
            buffer: window.buffer,
            window: window.id,
            top_line: window.top_line,
            line,
        }
    }

    /// How the beacon is drawn and how long it lasts, from the settings.
    pub fn beacon_shape(&self) -> crate::beacon::Shape {
        crate::beacon::Shape {
            size: self.settings.beacon_size.max(1),
            delay: std::time::Duration::from_millis(self.settings.beacon_blink_delay_ms as u64),
            duration: std::time::Duration::from_millis(
                self.settings.beacon_blink_duration_ms.max(1) as u64,
            ),
        }
    }

    /// The colour the light is, as `beacon-color` gives it.
    ///
    /// A number is a grade against the background; anything else is read as a
    /// colour, and a spelling that is not one falls back to the middle grade
    /// rather than refusing to shine.
    pub fn beacon_light(&self) -> crate::beacon::Light {
        let written = self.settings.beacon_color.trim();
        if let Ok(grade) = written.parse::<f32>() {
            return crate::beacon::Light::Grade(grade);
        }
        match maxgus_faces::Color::parse(written) {
            Ok(colour) => crate::beacon::Light::Colour(colour),
            Err(_) => crate::beacon::Light::Grade(0.5),
        }
    }

    /// Which movements light it, from the settings.
    pub fn beacon_triggers(&self) -> crate::beacon::Triggers {
        crate::beacon::Triggers {
            on: self.settings.beacon,
            buffer_changes: self.settings.beacon_blink_when_buffer_changes,
            window_scrolls: self.settings.beacon_blink_when_window_scrolls,
            window_changes: self.settings.beacon_blink_when_window_changes,
            point_moves_vertically: self.settings.beacon_blink_when_point_moves_vertically,
        }
    }

    /// Lights the beacon if the move from `before` deserves one.
    ///
    /// Called after every command, which is where `beacon` hangs its own
    /// hook. The minibuffer is left alone, as it is there: a prompt is not
    /// somewhere the cursor gets lost.
    pub fn consider_beacon(&mut self, before: &crate::beacon::Watch) {
        if self.minibuffer.is_active() {
            return;
        }
        let after = self.beacon_watch();
        if !crate::beacon::should_blink(&self.beacon_triggers(), before, &after) {
            return;
        }
        self.beacon = Some(crate::beacon::Beacon::new(
            after.window,
            self.windows.current().point,
        ));
    }

    /// Moves the beacon on by `elapsed`, putting it out when it is spent.
    ///
    /// Returns whether there is still one to draw, which is what tells the
    /// event loop whether another frame is owed.
    pub fn advance_beacon(&mut self, elapsed: std::time::Duration) -> bool {
        let shape = self.beacon_shape();
        let Some(beacon) = self.beacon.as_mut() else {
            return false;
        };
        beacon.elapsed += elapsed;
        if shape.is_over(beacon.elapsed) {
            self.beacon = None;
            return false;
        }
        true
    }

    /// Puts the beacon out now.
    pub fn quench_beacon(&mut self) {
        self.beacon = None;
    }

    /// Shows `buffer` in the window a file is edited in, never in the panel.
    ///
    /// Everything the editor pops up on its own — a listing, a help buffer, a
    /// magit view — goes through here. Put in one of the panel's windows it
    /// would replace what that window is for, and survive into the next
    /// rebuild of the column as a window that does not belong to it.
    pub fn show_in_editing_window(&mut self, buffer: BufferId) -> Result<()> {
        if self.panel_windows.contains(&self.windows.current_id())
            && let Some(target) = self.editing_window()
        {
            self.select_window(target);
        }
        self.switch_to_buffer(buffer)
    }

    /// Shows `buffer` in a window other than the one being edited in, and
    /// selects it — `pop-to-buffer`.
    ///
    /// A listing or a help buffer is looked at alongside the text it is
    /// about, not instead of it: `*Occur*` replacing the buffer it searched
    /// left nowhere to go when a line in it was chosen. The buffer goes to
    /// the window already showing it, else to another editing window, else
    /// to a window split off below — which is remembered, so quitting the
    /// buffer can close it again.
    pub fn pop_to_buffer(&mut self, buffer: BufferId) -> Result<()> {
        let editing: Vec<WindowId> = self
            .windows
            .ids()
            .into_iter()
            .filter(|id| !self.panel_windows.contains(id) && Some(*id) != self.terminal_pane())
            .collect();
        if let Some(window) = editing
            .iter()
            .find(|id| self.windows.get(**id).is_some_and(|w| w.buffer == buffer))
        {
            self.select_window(*window);
            return Ok(());
        }
        if !editing.contains(&self.windows.current_id())
            && let Some(target) = self.editing_window()
        {
            self.select_window(target);
        }
        let current = self.windows.current_id();
        match editing.iter().find(|id| **id != current) {
            Some(other) => {
                self.select_window(*other);
            }
            None => {
                let window = self.split_window(Direction::Vertical)?;
                self.select_window(window);
                self.popped_windows.push(window);
            }
        }
        self.switch_to_buffer(buffer)
    }

    /// `quit-window`: puts a popped-up buffer away.
    ///
    /// The window is deleted when it was split off for the buffer and there
    /// is another to go to; otherwise the buffer is buried, so the window
    /// shows whatever it showed before. The buffer is killed either way
    /// when `kill` is set, which is what a listing that is finished with
    /// wants.
    pub fn quit_window(&mut self, kill: bool) {
        let window = self.windows.current_id();
        let buffer = self.current_buffer_id();
        let popped = self.popped_windows.contains(&window);
        self.popped_windows.retain(|id| *id != window);
        if popped && self.windows.len() > 1 {
            self.sync_to_buffer();
            self.windows.delete(window).ok();
            self.activate_mode_keymap();
        } else {
            self.bury_buffer();
        }
        if kill {
            // Only when nothing else shows it: a listing open in two windows
            // is still wanted in the other.
            if self.windows.showing(buffer).is_empty() {
                self.kill_buffer(buffer).ok();
            }
        }
        self.follow_point();
    }

    /// Selects `window`, saving the outgoing window's point first.
    pub fn select_window(&mut self, window: WindowId) -> bool {
        self.sync_to_buffer();
        if !self.windows.select(window) {
            return false;
        }
        let buffer = self.windows.current().buffer;
        self.buffers.touch(buffer);
        // A different window may be showing a different language.
        self.activate_mode_keymap();
        true
    }

    /// `other-window`.
    pub fn other_window(&mut self, n: i32) {
        self.sync_to_buffer();
        let id = self.windows.other_window(n);
        self.select_window(id);
    }

    /// Splits the selected window, leaving it selected.
    pub fn split_window(&mut self, direction: Direction) -> Result<WindowId> {
        self.sync_to_buffer();
        self.windows.split(direction)
    }

    /// Kills `buffer`, pointing any window showing it at the replacement.
    pub fn kill_buffer(&mut self, buffer: BufferId) -> Result<BufferId> {
        // The server and the highlighter are told before the buffer goes, so
        // they can still be asked what it was.
        self.notify_closed(buffer);
        let replacement = self.buffers.kill(buffer)?;
        self.windows.replace_buffer(buffer, replacement);
        self.forget_highlights(buffer);
        // The executor is holding a parser and a copy of the text for it.
        self.spawn(Task::ForgetBuffer { buffer });
        // The windows are showing a different buffer now, which is as much a
        // change of buffer as switching to it: without this the settings and
        // the keymap are still the dead buffer's, and `q` in the magit buffer
        // underneath types a `q`.
        let current = self.current_buffer_id();
        self.apply_settings(current);
        self.activate_mode_keymap();
        self.follow_point();
        Ok(replacement)
    }

    // ---- scrolling -----------------------------------------------------

    /// Scrolls the selected window so point is visible, honouring
    /// `scroll-margin` and truncation.
    pub fn follow_point(&mut self) {
        let id = self.current_buffer_id();
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let point = self.windows.current().point.min(buffer.len_chars());
        let line = buffer.line_of(point);
        let column = buffer.display_column(point);
        let total = buffer.len_lines();
        let margin = self.settings.scroll_margin;
        let width = crate::render::wrap_width(self, self.windows.current(), buffer);
        // The columns the text itself has: the window's, less the gutter.
        let text_columns = crate::render::text_columns(self, self.windows.current(), buffer);

        let window = self.windows.current_mut();
        let Some(width) = width else {
            window.top_row = 0;
            window.scroll_to_show(line, total, margin);
            window.scroll_to_column(column, text_columns);
            return;
        };
        // Wrapping. There is no horizontal scroll — there is nothing off to
        // the side to scroll to — and the sums are in screen rows, because
        // that is what the window is full of.
        window.left_column = 0;
        let height = window.text_height();
        if height == 0 || total == 0 {
            return;
        }
        let margin = margin.min(height.saturating_sub(1) / 2);
        let at = crate::wrap::Place::new(line, crate::wrap::row_at(buffer, point, width).0);

        // Where the window starts, made safe first: the buffer can shrink
        // under a window, and a line can lose rows to an edit, either of
        // which leaves the top pointing at a row that is not there.
        let top_line = window.top_line.min(total - 1);
        let rows = crate::wrap::row_count(buffer, top_line, width);
        let mut top = crate::wrap::Place::new(top_line, window.top_row.min(rows - 1));

        let wanted_above = crate::wrap::backward(buffer, at, margin, width);
        if wanted_above < top {
            top = wanted_above;
        } else {
            let last = crate::wrap::forward(buffer, top, height - 1, width);
            let wanted_below = crate::wrap::forward(buffer, at, margin, width);
            if wanted_below > last {
                top = crate::wrap::backward(buffer, wanted_below, height - 1, width);
            }
        }
        window.top_line = top.line;
        window.top_row = top.row;
    }

    /// The place the selected window starts at, made safe against a buffer
    /// that has changed under it.
    fn top_place(&self, buffer: &Buffer, width: usize) -> crate::wrap::Place {
        let window = self.windows.current();
        let line = window.top_line.min(buffer.len_lines().saturating_sub(1));
        let rows = crate::wrap::row_count(buffer, line, width);
        crate::wrap::Place::new(line, window.top_row.min(rows.saturating_sub(1)))
    }

    /// Scrolls the selected window `delta` screen rows.
    ///
    /// Rows, not lines. With wrapping the two differ, and a page that moved
    /// by lines would move by however many screenfuls those lines happened
    /// to take.
    pub fn scroll_rows(&mut self, delta: isize) {
        let id = self.current_buffer_id();
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let total = buffer.len_lines();
        if total == 0 {
            return;
        }
        let width = crate::render::wrap_width(self, self.windows.current(), buffer);
        let top = width.map(|width| self.top_place(buffer, width));
        let window = self.windows.current_mut();
        let (Some(width), Some(top)) = (width, top) else {
            window.top_row = 0;
            window.top_line = window.top_line.saturating_add_signed(delta).min(total - 1);
            return;
        };
        let moved = match delta >= 0 {
            true => crate::wrap::forward(buffer, top, delta as usize, width),
            false => crate::wrap::backward(buffer, top, delta.unsigned_abs(), width),
        };
        window.top_line = moved.line;
        window.top_row = moved.row;
    }

    /// Puts the selected window's point `above` screen rows below its top.
    ///
    /// What `recenter` is underneath: the middle of the window is `above`
    /// half a screenful, the top is nought, the bottom is a screenful less
    /// one.
    pub fn scroll_point_to_row(&mut self, above: usize) {
        let id = self.current_buffer_id();
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let point = self.windows.current().point.min(buffer.len_chars());
        let line = buffer.line_of(point);
        let width = crate::render::wrap_width(self, self.windows.current(), buffer);
        let window = self.windows.current_mut();
        let Some(width) = width else {
            window.top_row = 0;
            window.top_line = line.saturating_sub(above);
            return;
        };
        let at = crate::wrap::Place::new(line, crate::wrap::row_at(buffer, point, width).0);
        let top = crate::wrap::backward(buffer, at, above, width);
        window.top_line = top.line;
        window.top_row = top.row;
    }

    /// How many rows below the selected window's top point sits, when it is
    /// on screen at all.
    pub fn point_row(&self) -> Option<usize> {
        let buffer = self.buffers.get(self.current_buffer_id())?;
        let window = self.windows.current();
        let point = window.point.min(buffer.len_chars());
        let line = buffer.line_of(point);
        let Some(width) = crate::render::wrap_width(self, window, buffer) else {
            return line.checked_sub(window.top_line);
        };
        let at = crate::wrap::Place::new(line, crate::wrap::row_at(buffer, point, width).0);
        let top = self.top_place(buffer, width);
        crate::wrap::rows_between(buffer, top, at, width, window.text_height().max(1))
    }

    /// The last buffer line the selected window shows.
    pub fn bottom_visible_line(&self) -> usize {
        let Some(buffer) = self.buffers.get(self.current_buffer_id()) else {
            return self.windows.current().bottom_line();
        };
        let window = self.windows.current();
        let Some(width) = crate::render::wrap_width(self, window, buffer) else {
            return window.bottom_line();
        };
        let top = self.top_place(buffer, width);
        crate::wrap::forward(buffer, top, window.text_height().saturating_sub(1), width).line
    }

    /// Where the hardware cursor belongs on screen: the selected window's
    /// point, or the minibuffer when it is prompting.
    pub fn cursor_position(&self) -> (u16, u16) {
        if self.minibuffer.is_active() {
            let frame = self.frame;
            // A completing prompt is drawn on the popup's first line, so the
            // cursor goes there rather than to the echo area it left behind.
            if let Some(popup) = crate::render::completion_popup(self, frame) {
                let inner = popup.inset(1);
                let lead = crate::render::completion_count(self).chars().count();
                let x = inner.x + (lead + self.minibuffer.cursor_column()) as u16;
                return (x.min(inner.right().saturating_sub(1)), inner.y);
            }
            let y = frame.y + frame.height.saturating_sub(1);
            let x = (self.minibuffer.cursor_column() as u16).min(frame.width.saturating_sub(1));
            return (x, y);
        }
        let window = self.windows.current();
        // A terminal draws its own cursor position, which is the program's
        // rather than a buffer's — and in reading mode it is the reader's.
        #[cfg(feature = "full")]
        if Some(window.id) == self.terminal_window
            && let Some(terminal) = self.terminals.current()
        {
            // One row for the tab bar.
            let origin = (window.rect.x, window.rect.y + 1);
            if let Some(cursor) = terminal.copy_cursor {
                let row = cursor.line.saturating_sub(terminal.top_line()) as u16;
                return (
                    (origin.0 + cursor.column as u16).min(window.rect.right().saturating_sub(1)),
                    (origin.1 + row).min(window.rect.bottom().saturating_sub(2)),
                );
            }
            if let Some((row, column)) = terminal.cursor() {
                return (
                    (origin.0 + column as u16).min(window.rect.right().saturating_sub(1)),
                    (origin.1 + row as u16).min(window.rect.bottom().saturating_sub(2)),
                );
            }
            return origin;
        }
        let Some(buffer) = self.buffers.get(window.buffer) else {
            return (window.rect.x, window.rect.y);
        };
        let point = window.point.min(buffer.len_chars());
        // Asked of the rows the window draws rather than worked out from the
        // line number: with wrapping a line is not a row, and the cursor has
        // to land on the cell the character was actually drawn in.
        let (row, column) = crate::render::point_cell(self, window, buffer, point);
        // The line-number column shifts the text right, and the cursor with it.
        let gutter = crate::render::line_number_width(self, buffer);
        let x = window.rect.x + gutter + column;
        let y = window.rect.y + row;
        (
            x.min(window.rect.right().saturating_sub(1)),
            y.min(window.rect.bottom().saturating_sub(1)),
        )
    }

    // ---- keymaps -------------------------------------------------------

    /// Activates a minor-mode map at the highest precedence.
    pub fn push_minor_map(&mut self, map: maxgus_keys::Keymap) {
        // Re-activating an already-active map would shadow itself.
        self.keymaps.remove_minor(map.name());
        self.keymaps.push_minor(map);
    }

    /// Deactivates a minor-mode map by name.
    pub fn remove_minor_map(&mut self, name: &str) -> bool {
        self.keymaps.remove_minor(name)
    }

    /// The name of the keymap a buffer's language uses, if it has one.
    ///
    /// A `rust` buffer looks for a map called `rust-mode`, which is what the
    /// configuration names them.
    pub fn mode_keymap_name(&self, buffer: BufferId) -> Option<String> {
        let buffer = self.buffers.get(buffer)?;
        // The panel's three windows each have a mode of their own.
        if buffer.name() == crate::commands::tree::TREE_BUFFER_NAME {
            return Some(crate::commands::tree::TREE_MODE.to_string());
        }
        if buffer.name() == crate::commands::tree::SYMBOLS_BUFFER_NAME {
            return Some(crate::commands::tree::SYMBOLS_MODE.to_string());
        }
        if buffer.name() == crate::commands::tree::BUFFERS_BUFFER_NAME {
            return Some(crate::commands::tree::BUFFERS_MODE.to_string());
        }
        // Every magit buffer shares one keymap, as magit's do: `q`, `g`, `n`
        // and the menus mean the same thing in all of them, and the commands
        // that differ ask which buffer they are in.
        #[cfg(feature = "full")]
        {
            if crate::commands::git::MAGIT_BUFFERS.contains(&buffer.name()) {
                return Some(crate::commands::git::GIT_MODE.to_string());
            }
            if buffer.name() == crate::commands::git::COMMIT_BUFFER_NAME {
                return Some(crate::commands::git::COMMIT_MODE.to_string());
            }
        }
        if buffer.name() == crate::commands::dired::DIRED_BUFFER_NAME {
            return Some(crate::commands::dired::DIRED_MODE.to_string());
        }
        if buffer.name() == crate::commands::search::OCCUR_NAME {
            return Some(crate::commands::listing::OCCUR_MODE.to_string());
        }
        #[cfg(feature = "full")]
        if buffer.name() == crate::commands::lsp::XREF_NAME {
            return Some(crate::commands::listing::XREF_MODE.to_string());
        }
        if buffer.name() == crate::commands::help::HELP_BUFFER_NAME {
            return Some(crate::commands::listing::HELP_MODE.to_string());
        }
        if buffer.name() == crate::commands::undo_tree::VISUALIZER_BUFFER_NAME {
            return Some(crate::commands::undo_tree::VISUALIZER_MODE.to_string());
        }
        #[cfg(feature = "full")]
        if buffer.name() == crate::commands::grep::GREP_BUFFER_NAME {
            let writing = self.grep.as_ref().is_some_and(|view| view.editable);
            return Some(
                match writing {
                    true => crate::commands::grep::GREP_EDIT_MODE,
                    false => crate::commands::grep::GREP_MODE,
                }
                .to_string(),
            );
        }
        // A terminal has two: one where the keys go to the shell, and one
        // where they move a cursor over what the shell has already written.
        #[cfg(feature = "full")]
        if buffer.name() == crate::commands::terminal::TERMINAL_BUFFER_NAME {
            let reading = self.terminals.current().is_some_and(|t| t.in_copy_mode());
            return Some(
                if reading {
                    crate::commands::terminal::TERMINAL_COPY_MODE
                } else {
                    crate::commands::terminal::TERMINAL_MODE
                }
                .to_string(),
            );
        }
        let language = buffer.language()?;
        Some(format!("{language}-mode"))
    }

    /// What the mode line calls the buffer's major mode.
    ///
    /// A file buffer is named for its language. A special buffer — dired, a
    /// magit view, the terminal, `*Help*` — has no language, and used to
    /// show as `Fundamental` for it, which is what Emacs calls a buffer with
    /// no mode at all; these have one, and it is the keymap they use. Only a
    /// buffer with neither is Fundamental.
    pub fn mode_name(&self, buffer: BufferId) -> String {
        if let Some(language) = self.buffers.get(buffer).and_then(|b| b.language()) {
            return language.to_string();
        }
        match self.mode_keymap_name(buffer) {
            Some(keymap) => mode_display_name(&keymap),
            None => "Fundamental".to_string(),
        }
    }

    /// Installs the major-mode keymap for the selected buffer.
    ///
    /// Called whenever the selected buffer changes: a binding defined for
    /// `rust-mode` should be in effect in a Rust buffer and nowhere else. The
    /// tree's map is one of these rather than a minor map — it binds the arrow
    /// keys, and a minor map applies in *every* buffer, so having the tree
    /// open took the arrows away from the file being edited.
    ///
    /// Minor maps — the minibuffer, isearch — still take precedence, as they
    /// do in Emacs.
    pub fn activate_mode_keymap(&mut self) {
        let wanted = self.mode_keymap_name(self.current_buffer_id());
        // Nothing to do when the right map is already in place.
        if self.keymaps.major.as_ref().map(|m| m.name().to_string()) == wanted {
            return;
        }
        self.keymaps
            .set_major(wanted.and_then(|name| self.mode_keymap(&name)));
    }

    /// The keymap for a named mode: the built-in one where there is one, with
    /// whatever the configuration defined for that mode laid over it.
    fn mode_keymap(&self, name: &str) -> Option<maxgus_keys::Keymap> {
        let configured = self.mode_keymaps.iter().find(|m| m.name() == name);
        // The built-in maps, which configuration adds to rather than replaces
        // — rebinding one key should not cost every other binding in the mode.
        let built_in = match name {
            #[cfg(feature = "full")]
            crate::commands::git::GIT_MODE => crate::keymap::magit_keymap().ok(),
            #[cfg(feature = "full")]
            crate::commands::git::COMMIT_MODE => crate::keymap::commit_keymap().ok(),
            #[cfg(feature = "full")]
            crate::commands::grep::GREP_MODE => crate::keymap::grep_keymap().ok(),
            #[cfg(feature = "full")]
            crate::commands::grep::GREP_EDIT_MODE => crate::keymap::grep_edit_keymap().ok(),
            crate::commands::dired::DIRED_MODE => crate::keymap::dired_keymap().ok(),
            crate::commands::listing::OCCUR_MODE | crate::commands::listing::XREF_MODE => {
                crate::keymap::listing_keymap(name).ok()
            }
            crate::commands::listing::HELP_MODE => crate::keymap::help_keymap().ok(),
            crate::commands::undo_tree::VISUALIZER_MODE => crate::keymap::undo_tree_keymap().ok(),
            crate::commands::tree::SYMBOLS_MODE => crate::keymap::symbols_keymap().ok(),
            crate::commands::tree::BUFFERS_MODE => crate::keymap::buffers_keymap().ok(),
            #[cfg(feature = "full")]
            crate::commands::terminal::TERMINAL_MODE => crate::keymap::terminal_keymap().ok(),
            #[cfg(feature = "full")]
            crate::commands::terminal::TERMINAL_COPY_MODE => {
                crate::keymap::terminal_copy_keymap().ok()
            }
            _ => None,
        };
        if let Some(mut map) = built_in {
            if let Some(configured) = configured {
                map.merge(configured);
            }
            return Some(map);
        }
        if name != crate::commands::tree::TREE_MODE {
            return configured.cloned();
        }
        // The tree's own map is built in; a `keymap "treefile-mode"` block in
        // the configuration adds to it rather than replacing it, or the user
        // would lose every treemacs binding by rebinding one key.
        let mut map = maxgus_tree::treemacs_keymap().ok()?;
        if let Some(configured) = configured {
            map.merge(configured);
        }
        Some(map)
    }

    /// Opens a minibuffer prompt, giving the minibuffer keymap priority.
    pub fn prompt(&mut self, kind: crate::MinibufferKind, prompt: impl Into<String>) {
        self.prompt_with(kind, prompt, "");
    }

    /// Opens a prompt whose answer re-enters `command`.
    ///
    /// This is how an interactive command is written: it prompts on its first
    /// invocation and does the work on the second, when `Args::input` holds
    /// what the user typed.
    pub fn prompt_for(
        &mut self,
        command: &str,
        kind: crate::MinibufferKind,
        prompt: impl Into<String>,
        initial: &str,
        candidates: Vec<String>,
    ) {
        self.pending_input = Some((command.to_string(), self.prefix));
        self.completion_candidates = candidates;
        self.prompt_with(kind, prompt, initial);
        // Shown straight away rather than waiting for TAB: `M-x` and `C-x b`
        // are far more useful when they say what is on offer.
        self.refresh_completions();
    }

    /// Opens the file browser to answer `command` with a directory.
    ///
    /// The counterpart of [`Editor::prompt_for`] for the one question a
    /// path prompt is worst at. Typing a directory in full is the slowest
    /// way to name one you could point at, and completion only helps once
    /// you know how it is spelt; the box is walked with the arrows and
    /// answers with `RET`. The command is re-entered exactly as a prompt
    /// re-enters it, with the chosen path in `Args::input`, so a command
    /// written for the prompt needs nothing new to be asked this way.
    pub fn browse_for(
        &mut self,
        command: &str,
        prompt: impl Into<String>,
        start: impl Into<std::path::PathBuf>,
    ) {
        let start = start.into();
        self.pending_input = Some((command.to_string(), self.prefix));
        self.browser = Some(crate::browser::Browser::choosing(&start, prompt));
        self.push_minor_map(
            crate::keymap::browse_keymap().expect("the built-in browse map is well formed"),
        );
        self.spawn(crate::task::Task::Browse { path: start });
    }

    /// How many candidate rows the popup has room for.
    ///
    /// Shared with the drawing code so a page moves exactly one screenful:
    /// the two disagreeing would make `PgDn` skip or repeat rows.
    pub fn completion_rows(&self) -> usize {
        // Fifteen rows, or half the frame on one tall enough for that to be
        // more: a list that scrolls in a box a third of a big window high
        // is a list being read through a letterbox.
        let most = ((self.frame.height / 2) as usize).max(15);
        // Leave the frame room for the popup's own two border rows, its
        // prompt line, and something of the buffer behind it.
        let room = (self.frame.height as usize).saturating_sub(6);
        self.minibuffer
            .completion()
            .len()
            .min(most)
            .min(room.max(1))
    }

    /// Moves the highlight `delta` rows and brings it into view.
    ///
    /// The move wraps; the scrolling is what makes the wrap visible, since a
    /// highlight that has gone round to the far end of a list is off the box
    /// until the list under it moves too.
    pub fn move_completion_selection(&mut self, delta: isize) -> bool {
        if !self.minibuffer.move_selection(delta) {
            return false;
        }
        self.follow_completion_selection();
        true
    }

    /// Scrolls the candidate list so the highlight is one of the rows drawn.
    pub fn follow_completion_selection(&mut self) {
        let rows = self.completion_rows();
        self.minibuffer.scroll_completion(rows);
    }

    /// Re-filters the candidate list against what is now in the minibuffer.
    ///
    /// Called after anything that changes the input, so the list on screen is
    /// always the list the input actually matches.
    pub fn refresh_completions(&mut self) {
        let candidates = std::mem::take(&mut self.completion_candidates);
        self.minibuffer.filter_completions(&candidates);
        self.completion_candidates = candidates;
        self.follow_completion_selection();
        self.preview_theme();
    }

    /// While `consult-theme` is prompting, shows whatever the input now names.
    ///
    /// Driven from the same refresh every prompt edit already goes through, so
    /// typing and cycling both preview without either needing to know about
    /// the other.
    fn preview_theme(&mut self) {
        if self.theme_before_preview.is_none() {
            return;
        }
        // The candidate under the cursor if one is selected, else what has
        // been typed — so TAB-cycling previews as readily as typing does.
        let wanted = self
            .minibuffer
            .completion()
            .current()
            .map(str::to_string)
            .unwrap_or_else(|| self.minibuffer.input().to_string());
        if wanted.is_empty() || self.settings.theme == wanted {
            return;
        }
        // A half-typed name is not an error here; it simply is not a theme yet.
        let _ = self.set_theme(&wanted);
    }

    /// Puts back the theme that was in use before previewing began.
    pub fn end_theme_preview(&mut self, restore: bool) {
        let Some(before) = self.theme_before_preview.take() else {
            return;
        };
        if restore && self.settings.theme != before {
            let _ = self.set_theme(&before);
        }
    }

    /// Opens a prompt pre-filled with `initial`.
    pub fn prompt_with(
        &mut self,
        kind: crate::MinibufferKind,
        prompt: impl Into<String>,
        initial: &str,
    ) {
        self.minibuffer.activate_with(kind, prompt, initial);
        self.push_minor_map(
            crate::keymap::minibuffer_keymap().expect("the built-in minibuffer map is well formed"),
        );
    }

    /// Closes the prompt and returns what was typed.
    pub fn accept_prompt(&mut self) -> Option<String> {
        let out = self.minibuffer.accept();
        self.remove_minor_map("minibuffer-mode");
        self.completion_candidates.clear();
        out
    }

    /// Abandons the prompt and whatever command was waiting on it.
    pub fn abort_prompt(&mut self) {
        // Abandoning `consult-theme` means abandoning what it was showing.
        self.end_theme_preview(true);
        self.minibuffer.abort();
        self.remove_minor_map("minibuffer-mode");
        self.pending_input = None;
        self.completion_candidates.clear();
    }

    // ---- messages ------------------------------------------------------

    /// Records how long the editor took to start.
    ///
    /// Kept rather than shown: the files named on the command line are still
    /// arriving and will have their own say, so the announcement waits for
    /// them. `M-x startup-time` asks for it again afterwards, which is what
    /// `emacs-init-time` is for.
    pub fn set_startup_time(&mut self, elapsed: std::time::Duration) {
        self.startup_time = Some(elapsed);
    }

    /// What the echo area says about the startup, once startup is over.
    pub fn startup_message(&self) -> Option<String> {
        self.startup_time
            .map(|elapsed| format!("maxgus started in {}", crate::human_duration(elapsed)))
    }

    pub fn message(&mut self, text: impl Into<String>) {
        self.minibuffer.show_message(text);
    }

    /// A message that gives way to an error already on show.
    ///
    /// Startup reports configuration problems into the echo area, and the
    /// files named on the command line finish loading a moment later; without
    /// this the routine "(N lines)" notice talks over the complaint and the
    /// user never learns their config file has a mistake in it.
    pub fn message_unless_error(&mut self, text: impl Into<String>) {
        if self.minibuffer.message_is_error() {
            return;
        }
        self.message(text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.minibuffer.show_error(text);
    }

    /// Arranges for the next key to be delivered to `command`, showing
    /// `prompt` while it waits.
    pub fn read_char(&mut self, command: &str, prompt: impl Into<String>) {
        self.pending_char = Some((command.to_string(), self.prefix));
        self.message(prompt);
    }

    /// Queues asynchronous work.
    pub fn spawn(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Re-lays out the windows for a new frame size, keeping the echo area's
    /// row out of their reach.
    pub fn set_frame(&mut self, frame: Rect) {
        self.frame = frame;
        let (body, _) = frame.split_bottom(1);
        self.windows.layout(body);
        self.follow_point();
        // A program in a terminal has to be told, or it goes on drawing to
        // the shape the window used to be.
        #[cfg(feature = "full")]
        self.resize_terminals();
    }

    /// The directory file prompts start in: the current buffer's, or the
    /// working directory when it has no file.
    pub fn default_directory(&self) -> PathBuf {
        self.current_buffer()
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// The project's root: what git calls the top of the tree, else the
    /// directory the file tree is rooted at, else where the editor started.
    pub fn project_root(&self) -> PathBuf {
        #[cfg(feature = "full")]
        if let Some(root) = self.git_root.clone() {
            return root;
        }
        self.tree_root
            .clone()
            .unwrap_or_else(|| self.default_directory())
    }

    /// The word point is in or beside, which is what a search prompt offers.
    pub fn word_at_point(&self) -> Option<String> {
        let buffer = self.current_buffer();
        let point = self.windows.current().point.min(buffer.len_chars());
        let text: Vec<char> = buffer.text().chars().collect();
        let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
        // Point sits between characters: the word is the one it is inside, or
        // the one it is at the end of, which is where a cursor usually is.
        let mut start = point;
        while start > 0 && text.get(start - 1).is_some_and(is_word) {
            start -= 1;
        }
        let mut end = point;
        while text.get(end).is_some_and(is_word) {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some(text[start..end].iter().collect())
    }

    /// What is open now, as a session.
    ///
    /// Buffers with no file are left out: a `*scratch*` restored from a
    /// previous run would be a surprise, and the ones the editor makes for
    /// itself are made again when they are needed.
    pub fn session(&self) -> crate::session::Session {
        let current = self
            .current_buffer()
            .path()
            .map(std::path::Path::to_path_buf);
        let mut files = Vec::new();
        for buffer in self.buffers.iter() {
            let Some(path) = buffer.path() else { continue };
            // Where the window showing it was, when one is.
            let window = self.windows.iter().find(|w| w.buffer == buffer.id);
            files.push(crate::session::OpenFile {
                path: path.to_path_buf(),
                point: window.map(|w| w.point).unwrap_or_else(|| buffer.point()),
                top_line: window.map(|w| w.top_line).unwrap_or(0),
            });
        }
        crate::session::Session {
            root: self.tree_root.clone(),
            files,
            current,
            panel_open: !self.panel_windows.is_empty(),
        }
    }

    /// Opens everything a session remembers.
    ///
    /// The files are read asynchronously like any others; where point should
    /// land in each is recorded and applied as they arrive.
    pub fn restore_session(&mut self, session: crate::session::Session) {
        if session.files.is_empty() {
            return;
        }
        let count = session.files.len();
        for file in &session.files {
            self.session_points
                .insert(file.path.clone(), (file.point, file.top_line));
            self.spawn(crate::task::Task::ReadFile {
                path: file.path.clone(),
                reverting: None,
                other_window: false,
            });
        }
        // Read last so it is the buffer left showing: files arrive in the
        // order they were asked for.
        if let Some(current) = session.current {
            self.spawn(crate::task::Task::ReadFile {
                path: current,
                reverting: None,
                other_window: false,
            });
        }
        self.session_panel = session.panel_open;
        self.message(format!("Restoring {}", crate::count(count, "file")));
    }

    /// Applies the outcome of a [`Task`].
    ///
    /// This is the other half of the asynchronous story: a command queues work,
    /// the event loop runs it on tokio, and the answer comes back here to be
    /// folded into editor state.
    pub fn apply_task_result(&mut self, result: crate::task::TaskResult) -> Result<()> {
        use crate::task::TaskResult;
        match result {
            TaskResult::FileRead {
                path,
                contents,
                read_only,
                lossy,
                disk_time,
                reverting,
                other_window,
                editor_config,
            } => {
                let id = match reverting {
                    Some(id) => {
                        self.replace_buffer_contents(id, &contents)?;
                        id
                    }
                    None => self.buffers.visit_file(path.clone(), &contents),
                };
                if let Some(buffer) = self.buffers.get_mut(id) {
                    buffer.set_read_only(read_only);
                    buffer.set_disk_time(disk_time);
                }
                if lossy {
                    // Said as an error, so the "(N lines)" notice that follows
                    // gives way to it rather than talking over it. Saving this
                    // buffer would put replacement characters on disk where
                    // the original bytes are, so it is read-only and the
                    // reason has to reach the user.
                    self.error(format!(
                        "{} holds bytes that are not text; opened read-only to avoid \
                         writing them back changed",
                        path.display()
                    ));
                }
                if other_window && self.windows.len() < 2 {
                    self.split_window(Direction::Vertical)?;
                    self.other_window(1);
                } else if other_window {
                    self.other_window(1);
                }
                self.switch_to_buffer(id)?;
                // A jump that was waiting on this file can finish now.
                if let Some((waiting, line)) = self.pending_line.take()
                    && waiting == path
                {
                    self.go_to_line(line);
                }
                #[cfg(feature = "full")]
                if let Some((waiting, position)) = self.pending_jump.take() {
                    if waiting == path {
                        let offset = crate::position::offset_of_position(
                            self.current_buffer(),
                            position,
                            maxgus_lsp::PositionEncoding::Utf16,
                        );
                        self.with_current_buffer(|b| b.set_point(offset));
                    } else {
                        self.pending_jump = Some((waiting, position));
                    }
                }
                let lines = self.current_buffer().len_lines();
                let noun = if lines == 1 { "line" } else { "lines" };
                self.message_unless_error(format!("{} ({lines} {noun})", path.display()));
                // Where a restored session left the reader in this file.
                if let Some((point, top_line)) = self.session_points.remove(&path) {
                    let point = point.min(self.buffers.get(id).map_or(0, |b| b.len_chars()));
                    for window in self.windows.showing(id) {
                        if let Some(window) = self.windows.get_mut(window) {
                            window.point = point;
                            window.top_line = top_line;
                        }
                    }
                    if let Some(buffer) = self.buffers.get_mut(id) {
                        buffer.set_point(point);
                    }
                    self.follow_point();
                }
                // Before the highlighting and the server, so a buffer whose
                // project says four-space indent is four-space indent from
                // the first frame it is drawn in.
                self.set_editor_config(id, editor_config);
                self.request_highlighting(id);
                #[cfg(feature = "full")]
                self.request_language_server(id);
                Ok(())
            }
            TaskResult::FileWritten {
                path,
                buffer,
                bytes,
                disk_time,
            } => {
                // A save is as good a moment as any to notice the branch has
                // moved — a checkout between edits is the usual way it does.
                #[cfg(feature = "full")]
                if let Some(root) = self.tree_root.clone() {
                    self.spawn(crate::task::Task::GitBranch { root });
                }
                if let Some(target) = self.buffers.get_mut(buffer) {
                    target.mark_saved();
                    // What is on disk is now what this buffer holds, so the
                    // next save compares against the file just written.
                    target.set_disk_time(disk_time);
                }
                self.notify_saved(buffer);
                let noun = if bytes == 1 { "byte" } else { "bytes" };
                self.message(format!("Wrote {} ({bytes} {noun})", path.display()));
                Ok(())
            }
            TaskResult::WriteRefused {
                path,
                buffer,
                because,
            } => {
                // Nothing was written. Whichever expectation failed will fail
                // the same way next time, so the only way on is the command
                // that says to write regardless.
                self.pending_overwrite = Some(buffer);
                self.error(match because {
                    crate::task::WriteGuard::Absent => format!(
                        "{} already exists; M-x save-buffer-anyway to overwrite it",
                        path.display()
                    ),
                    _ => format!(
                        "{} has changed on disk since it was read; \
                         C-x x g to re-read it, or M-x save-buffer-anyway to overwrite",
                        path.display()
                    ),
                });
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::Grammars { report } => {
                crate::commands::help::show_help(self, &report)?;
                self.message(String::new());
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GrammarCatalog {
                language,
                parsers,
                error,
            } => crate::commands::grammar::show_catalog(self, language, parsers, error),
            #[cfg(feature = "full")]
            TaskResult::GrammarMissing {
                language,
                candidates,
            } => crate::commands::grammar::offer(self, &language, candidates),
            #[cfg(feature = "full")]
            TaskResult::GrammarInstalled {
                language,
                summary,
                log,
                failed,
            } => crate::commands::grammar::installed(self, &language, &summary, &log, failed),
            TaskResult::TreeUpdated {
                nodes,
                select,
                show_hidden,
            } => {
                // The node the cursor was on, before the tree it indexes into
                // is replaced. Expanding a directory adds lines below it and
                // collapsing one takes lines away, so the line number the
                // cursor is on means something different afterwards; the
                // node it was on does not.
                let was = self.tree_selection().map(|node| node.path.clone());
                self.tree = nodes;
                self.tree_shows_hidden = show_hidden;
                self.render_panel_buffer();
                // A node that is no longer there — one just deleted, or one
                // inside a directory that was collapsed — leaves the cursor
                // on the line it was on, which is the nearest thing to where
                // it was.
                if let Some(path) = select.or(was) {
                    self.select_tree_path(&path);
                }
                Ok(())
            }
            TaskResult::DirectoryListed { entries, .. } => {
                // Fills in completion for an open file prompt.
                if self.minibuffer.is_active() {
                    self.completion_candidates = entries;
                }
                Ok(())
            }
            TaskResult::ThemePersisted { path, theme } => {
                self.config_says_theme = Some(theme.clone());
                self.message(format!("Theme {theme}, written to {}", path.display()));
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GrepFinished { pattern, found } => {
                if let Err(error) = crate::commands::grep::show(self, &pattern, found) {
                    self.error(error.to_string());
                }
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GrepApplied { applied, paths } => {
                // The buffers for the files that were written are stale now,
                // and a buffer showing an old copy of a file that has just
                // been rewritten is how work gets lost.
                for path in paths {
                    if let Some(id) = self.buffers.find_by_path(&path) {
                        self.spawn(crate::task::Task::ReadFile {
                            path,
                            reverting: Some(id),
                            other_window: false,
                        });
                    }
                }
                self.message(format!(
                    "Wrote {} in {}",
                    crate::count(applied.lines, "line"),
                    crate::count(applied.files, "file")
                ));
                Ok(())
            }
            TaskResult::Said(_) => Ok(()),
            TaskResult::WorkspacesRead { workspaces } => {
                self.workspaces = workspaces;
                Ok(())
            }
            TaskResult::Browsed { path, entries } => {
                // Only while it is still open: a listing arriving after the
                // box has been closed is an answer to a question nobody is
                // asking any more.
                if let Some(browser) = self.browser.as_mut() {
                    browser.listed(path, entries);
                }
                Ok(())
            }
            TaskResult::DirectoriesFound {
                root,
                paths,
                capped,
            } => {
                // Only into a box that is still waiting for one. A walk takes
                // long enough that it can outlive the question.
                if let Some(browser) = self.browser.as_mut().filter(|b| b.searched) {
                    browser.found(root, paths, capped);
                }
                Ok(())
            }
            TaskResult::DiredListed { path, entries } => {
                if let Err(error) = crate::commands::dired::show(self, path, entries) {
                    self.error(error.to_string());
                }
                Ok(())
            }
            TaskResult::DiredDone { said, relist } => {
                self.message(said);
                self.spawn(crate::task::Task::Dired { path: relist });
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::ScriptRead { source, path } => {
                self.script_path = Some(path);
                match maxgus_script::Script::load(&source) {
                    Ok(script) => self.set_script(script),
                    // A script that will not load is reported and the editor
                    // carries on: it is an extension, not a prerequisite.
                    Err(error) => self.error(format!("script: {error}")),
                }
                Ok(())
            }
            TaskResult::SessionRead { session } => {
                self.restore_session(session);
                Ok(())
            }
            TaskResult::SessionSaved { path } => {
                self.message(format!("Session saved to {}", path.display()));
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GitBranch { branch } => {
                self.git_branch = branch;
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::Reparsed {
                buffer,
                revision,
                range,
                highlights,
            } => {
                // A buffer killed while its parse was still running has
                // already been cleaned up by `kill_buffer`; filing the answer
                // now would put back an entry nothing ever removes. Buffer ids
                // are never reused, so it would sit there for the session and
                // grow with every file opened and closed.
                if self.buffers.get(buffer).is_some() {
                    self.highlights
                        .insert(buffer, (revision, range, highlights));
                }
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::LanguageServerStarted { language, encoding } => {
                // Which servers are up is not idle bookkeeping: the panel's
                // symbol section is hidden entirely when there is nothing to
                // ask, and `lsp-*` commands address positions in whatever
                // encoding the server settled on.
                self.lsp_encodings.retain(|(name, _)| *name != language);
                self.lsp_encodings.push((language, encoding));
                self.request_document_symbols();
                // The outline window appears now, whether or not the panel
                // was open before the server was.
                self.sync_panel_sections();
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::LanguageServerStopped { language } => {
                self.lsp_encodings.retain(|(name, _)| *name != language);
                if !self.symbols_available() {
                    self.panel.forget_symbols();
                    self.sync_panel_sections();
                    self.render_panel_buffer();
                }
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GitRefreshed(snapshot) => {
                let snapshot = *snapshot;
                self.git_root = Some(snapshot.root);
                self.git.status = snapshot.status;
                self.git.unstaged = snapshot.unstaged;
                self.git.staged = snapshot.staged;
                self.git.stashes = snapshot.stashes;
                self.git.unpushed = snapshot.unpushed;
                self.git.unpulled = snapshot.unpulled;
                self.git.recent = snapshot.recent;
                self.git.head_subject = snapshot.head_subject;
                self.git.loaded = true;
                self.git_branches = snapshot.branches;
                self.git_references = snapshot.references;
                // The branch on the mode line comes from the same reading, so
                // it can never disagree with the status view.
                self.git_branch = self.git.status.branch.clone();
                self.render_git_buffer();
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::GitLog { title, commits } => {
                let view = crate::git::ListView::from_log(title, &commits);
                let name = self
                    .git_pending_view
                    .take()
                    .unwrap_or(crate::commands::git::LOG_BUFFER_NAME);
                self.open_git_list(name, view)
            }
            #[cfg(feature = "full")]
            TaskResult::GitDiff {
                title,
                preamble,
                files,
            } => {
                let view = crate::git::DiffView::new(
                    title,
                    preamble.into_iter().map(|line| (line, "shadow")).collect(),
                    files,
                );
                let name = self
                    .git_pending_view
                    .take()
                    .unwrap_or(crate::commands::git::DIFF_BUFFER_NAME);
                self.open_git_diff(name, view)
            }
            #[cfg(feature = "full")]
            TaskResult::GitDone {
                action,
                command,
                output,
            } => {
                // Kept whatever the outcome, so the process buffer can show
                // what was run even when nothing went wrong.
                self.git_history.push((command, output.clone()));
                self.message(match output.trim() {
                    "" => format!("{action} done"),
                    said => format!("{action}: {}", said.lines().next().unwrap_or(said)),
                });
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::TerminalOutput { terminal, bytes } => {
                if let Some(tab) = self.terminals.get_mut(terminal) {
                    tab.receive(&bytes);
                    // Some sequences are questions. A shell that asks where
                    // the cursor is and is never told will sit there waiting,
                    // so the answer goes back the way it came.
                    let replies = tab.emulator.take_replies();
                    if !replies.is_empty() {
                        self.spawn(crate::task::Task::TerminalInput {
                            terminal,
                            bytes: replies,
                        });
                    }
                }
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::TerminalExited { terminal, status } => {
                if let Some(tab) = self.terminals.get_mut(terminal) {
                    tab.exited = Some(status);
                    // Said on the screen rather than only in the echo area,
                    // which the next message would wipe.
                    let note = format!("\r\n[exited with status {status}]\r\n");
                    tab.receive(note.as_bytes());
                }
                Ok(())
            }
            #[cfg(feature = "full")]
            TaskResult::Diagnostics { uri, diagnostics } => {
                self.diagnostics.replace(uri, diagnostics);
                Ok(())
            }
            TaskResult::Failed { context, message } => {
                self.error(format!("{context}: {message}"));
                Ok(())
            }
            other => {
                if let Some(text) = other.message() {
                    self.message(text);
                }
                Ok(())
            }
        }
    }

    #[cfg(feature = "full")]
    /// Folds a language server's answer into editor state.
    pub fn apply_lsp_response(&mut self, result: crate::task::TaskResult) {
        match result {
            crate::task::TaskResult::LspResponse {
                query, result, uri, ..
            } => {
                crate::commands::lsp::apply_response(self, &uri, &query, &result);
            }
            // `workspace/applyEdit`: the server asked to change the text and
            // is waiting to hear whether it worked. The edit has to be applied
            // here, where the buffers are, and the answer queued back.
            crate::task::TaskResult::LspApplyEdit { language, id, edit } => {
                let applied = crate::commands::lsp::apply_workspace_edit(self, &edit) > 0;
                self.spawn(crate::task::Task::LspRespond {
                    language,
                    id,
                    applied,
                });
            }
            _ => {}
        }
    }

    /// Renders `buffer` into the cache if what is there is stale.
    ///
    /// Kept separate from reading it so a caller can refresh the cache and
    /// then borrow the editor immutably alongside it; taking `&mut self` to
    /// read would force a copy at every call and defeat the point.
    pub fn refresh_text_cache(&mut self, buffer: BufferId) {
        let revision = self.buffers.get(buffer).map(|b| b.revision()).unwrap_or(0);
        let fresh = matches!(&self.text_cache, Some((id, known, _))
            if *id == buffer && *known == revision);
        if !fresh {
            let text = self
                .buffers
                .get(buffer)
                .map(|b| b.text())
                .unwrap_or_default();
            self.text_cache = Some((buffer, revision, text));
        }
    }

    /// The text most recently put in the cache.
    pub fn cached_text(&self) -> &str {
        self.text_cache
            .as_ref()
            .map(|(_, _, text)| text.as_str())
            .unwrap_or_default()
    }

    #[cfg(feature = "full")]
    /// The highlight spans for `buffer`, empty when none have been computed.
    pub fn highlights_for(&self, buffer: BufferId) -> &[maxgus_syntax::Highlight] {
        self.highlights
            .get(&buffer)
            .map_or(&[], |(_, _, spans)| spans.as_slice())
    }

    #[cfg(feature = "full")]
    /// True when the highlighting for `buffer` is behind its contents, or does
    /// not reach as far as the window now shows.
    pub fn highlights_are_stale(&self, buffer: BufferId) -> bool {
        let Some(current) = self.buffers.get(buffer).map(|b| b.revision()) else {
            return false;
        };
        // Staleness is judged against what is actually on screen, not against
        // the wider region that would be fetched. Comparing with the margin
        // included would make every scroll forward look uncovered, and the
        // margin would buy nothing.
        let visible = self.visible_byte_range(buffer);
        self.highlights
            .get(&buffer)
            .is_none_or(|(revision, covered, _)| {
                *revision != current || covered.start > visible.start || covered.end < visible.end
            })
    }

    /// The byte range a window showing `buffer` actually displays.
    pub fn visible_byte_range(&self, buffer: BufferId) -> std::ops::Range<usize> {
        self.byte_range_for(buffer, 0)
    }

    /// The byte range worth asking for: what is on screen plus a few screens
    /// either side, so ordinary scrolling does not have to wait for a query.
    pub fn highlight_request_range(&self, buffer: BufferId) -> std::ops::Range<usize> {
        self.byte_range_for(buffer, HIGHLIGHT_MARGIN_SCREENS)
    }

    /// The bytes of the lines a window shows, widened by `margin` screenfuls.
    fn byte_range_for(&self, buffer: BufferId, margin: usize) -> std::ops::Range<usize> {
        let Some(text) = self.buffers.get(buffer) else {
            return 0..0;
        };
        let showing: Vec<&Window> = self.windows.iter().filter(|w| w.buffer == buffer).collect();
        let height = showing
            .iter()
            .map(|w| w.text_height())
            .max()
            .unwrap_or(40)
            .max(1);
        let top = showing.iter().map(|w| w.top_line).min().unwrap_or(0);
        let widen = height * margin;
        let first = top.saturating_sub(widen);
        let last = (top + height + widen).min(text.len_lines());
        let rope = text.rope();
        rope.char_to_byte(text.line_start(first))..rope.char_to_byte(text.line_start(last))
    }

    /// Forgets a buffer's highlighting, as killing it should.
    pub fn forget_highlights(&mut self, buffer: BufferId) {
        let _ = buffer;
        #[cfg(feature = "full")]
        {
            self.highlights.remove(&buffer);
        }
    }

    /// Redraws the tree buffer from the current snapshot, keeping the cursor
    /// on whichever line it was on.
    /// Rewrites every panel window's buffer.
    pub fn render_panel_buffer(&mut self) {
        self.render_tree_buffer();
        self.render_symbols_buffer();
        self.render_buffers_buffer();
    }

    /// One line per node, which is what makes the tree's own commands index
    /// straight into the snapshot.
    pub fn render_tree_buffer(&mut self) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::tree::TREE_BUFFER_NAME)
        else {
            return;
        };
        let text: String = self
            .tree
            .iter()
            .map(|n| format!("{}\n", n.render()))
            .collect();
        let line = self.line_in(id);
        self.replace_buffer_contents(id, &text).ok();
        self.move_point_in(id, line.min(self.tree.len().saturating_sub(1)));
    }

    /// One line per symbol that is on show.
    pub fn render_symbols_buffer(&mut self) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::tree::SYMBOLS_BUFFER_NAME)
        else {
            return;
        };
        let visible = self.panel.visible_symbols();
        let text: String = visible
            .iter()
            .filter_map(|index| self.panel.symbols.get(*index))
            .map(|symbol| {
                format!(
                    "{}{}{}\n",
                    "  ".repeat(symbol.depth),
                    symbol.arrow(),
                    symbol.name
                )
            })
            .collect();
        let line = self.line_in(id);
        self.replace_buffer_contents(id, &text).ok();
        self.move_point_in(id, line.min(visible.len().saturating_sub(1)));
    }

    /// One line per open buffer.
    pub fn render_buffers_buffer(&mut self) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::tree::BUFFERS_BUFFER_NAME)
        else {
            return;
        };
        let listed = self.panel_buffers();
        let text: String = listed
            .iter()
            .map(|(_, name)| format!("  {name}\n"))
            .collect();
        let line = self.line_in(id);
        self.replace_buffer_contents(id, &text).ok();
        self.move_point_in(id, line.min(listed.len().saturating_sub(1)));
    }

    /// The line point is on in a buffer, wherever that buffer is shown.
    pub fn line_in(&self, id: BufferId) -> usize {
        let Some(buffer) = self.buffers.get(id) else {
            return 0;
        };
        let point = self
            .windows
            .showing(id)
            .first()
            .and_then(|w| self.windows.get(*w))
            .map_or(buffer.point(), |w| w.point);
        buffer.line_of(point)
    }

    /// Moves point to a line of a buffer, in the buffer and in every window
    /// showing it.
    pub fn move_point_in(&mut self, id: BufferId, line: usize) {
        let Some(offset) = self.buffers.get(id).map(|b| b.line_start(line)) else {
            return;
        };
        // The window has to follow. Setting the point alone let the cursor
        // walk off the bottom of a panel and stay there: the tree drew from
        // `top_line` and nothing had moved it, so a project with more files
        // than the panel is tall could be scrolled into and never out of.
        //
        // Every panel goes through here — the file tree, the symbol
        // outline, the buffer list, dired and the undo tree — so all of
        // them followed their cursor the moment one of them did.
        let total = self.buffers.get(id).map_or(0, |b| b.len_lines());
        let margin = self.settings.scroll_margin;
        for window in self.windows.showing(id) {
            if let Some(window) = self.windows.get_mut(window) {
                window.point = offset;
                window.scroll_to_show(line, total, margin);
            }
        }
        if let Some(buffer) = self.buffers.get_mut(id) {
            buffer.set_point(offset);
        }
    }

    /// The buffers the panel lists, most recently used first and without the
    /// panel's own, which nobody wants to switch to from inside it.
    pub fn panel_buffers(&self) -> Vec<(BufferId, String)> {
        self.buffers
            .iter()
            .filter(|buffer| !crate::commands::tree::PANEL_BUFFERS.contains(&buffer.name()))
            .filter(|buffer| !buffer.name().starts_with(' '))
            .map(|buffer| (buffer.id, buffer.name().to_string()))
            .collect()
    }

    /// True when a language server is running for the buffer being edited,
    /// which is what decides whether the outline window exists at all.
    ///
    /// Always false in a build with no language server in it, so the panel
    /// leaves the outline window out without having to know why.
    pub fn symbols_available(&self) -> bool {
        #[cfg(feature = "full")]
        {
            let Some(buffer) = self.editing_buffer() else {
                return false;
            };
            let Some(buffer) = self.buffers.get(buffer) else {
                return false;
            };
            buffer
                .language()
                .is_some_and(|language| self.lsp_encodings.iter().any(|(name, _)| name == language))
        }
        #[cfg(not(feature = "full"))]
        false
    }

    /// The window a file belongs in: the first that is neither part of the
    /// side panel nor the terminal.
    ///
    /// "Not the tree" is not enough any more — the panel is a column of
    /// windows, and the one after the tree is the symbol outline.
    pub fn editing_window(&self) -> Option<WindowId> {
        self.windows
            .ids()
            .into_iter()
            .find(|id| !self.panel_windows.contains(id) && Some(*id) != self.terminal_pane())
    }

    /// The buffer being edited: the one in the first window that is not part
    /// of the panel or the terminal.
    pub fn editing_buffer(&self) -> Option<BufferId> {
        self.windows
            .iter()
            .find(|window| {
                !self.panel_windows.contains(&window.id) && Some(window.id) != self.terminal_pane()
            })
            .map(|window| window.buffer)
    }

    /// Asks the server for the outline of the buffer the panel is following.
    pub fn request_document_symbols(&mut self) {
        #[cfg(feature = "full")]
        {
            if !self.panel.is_enabled(crate::panel::PanelSection::Symbols)
                || self.panel_windows.is_empty()
            {
                return;
            }
            let Some(buffer) = self.editing_buffer() else {
                return;
            };
            let (language, path) = {
                let Some(text) = self.buffers.get(buffer) else {
                    return;
                };
                let Some(language) = text.language().map(str::to_string) else {
                    return;
                };
                let Some(path) = text.path().map(Path::to_path_buf) else {
                    return;
                };
                (language, path)
            };
            if !self.lsp_encodings.iter().any(|(name, _)| *name == language) {
                return;
            }
            // The outline belongs to one buffer; recording which before the
            // answer arrives is what lets a late reply for the previous file be
            // thrown away instead of shown against this one.
            self.panel.symbols_pending = true;
            self.panel.symbols_buffer = Some(buffer);
            self.spawn(Task::LspRequest {
                language,
                uri: maxgus_lsp::client::path_to_uri(&path),
                query: crate::task::LspQuery::DocumentSymbols { for_panel: true },
                announced: false,
            });
        }
    }

    /// Adds or removes the outline window as a server comes and goes.
    ///
    /// Which windows the column has is decided when it is built, so a server
    /// starting after the panel was opened would otherwise never get one —
    /// and a buffer with no server would keep an outline window over nothing.
    pub fn sync_panel_sections(&mut self) {
        if self.panel_windows.is_empty()
            || !self.panel.is_enabled(crate::panel::PanelSection::Symbols)
        {
            return;
        }
        let showing = self
            .buffers
            .find_by_name(crate::commands::tree::SYMBOLS_BUFFER_NAME)
            .is_some_and(|id| !self.windows.showing(id).is_empty());
        // Only a change is worth a rebuild. In a build with no language
        // server `symbols_available` is always false, so a column that has
        // never shown an outline is already right and this returns every
        // time — which is what keeps a plain buffer switch from rebuilding
        // the panel.
        if showing == self.symbols_available() {
            return;
        }
        crate::commands::tree::rebuild(self).ok();
    }

    /// Points the outline at whatever buffer is now being edited.
    pub fn follow_panel_to_buffer(&mut self) {
        if self.panel_windows.is_empty()
            || !self.panel.is_enabled(crate::panel::PanelSection::Symbols)
        {
            return;
        }
        let showing = self.editing_buffer();
        if showing == self.panel.symbols_buffer && !self.panel.symbols.is_empty() {
            return;
        }
        self.panel.forget_symbols();
        #[cfg(feature = "full")]
        self.request_document_symbols();
        self.sync_panel_sections();
        self.render_symbols_buffer();
    }

    // ---- the git views --------------------------------------------------

    #[cfg(feature = "full")]
    /// The tags there are, for the tag prompts.
    pub fn git_tags(&self) -> Vec<String> {
        self.git_references
            .iter()
            .filter(|reference| reference.kind == maxgus_git::RefKind::Tag)
            .map(|reference| reference.name.clone())
            .collect()
    }

    #[cfg(feature = "full")]
    /// What git has been asked to do, as a buffer to read.
    pub fn git_process_view(&self) -> crate::git::ListView {
        let mut lines = Vec::new();
        for (command, output) in &self.git_history {
            lines.push(crate::git::ListLine {
                spans: vec![
                    ("$ ".into(), "shadow"),
                    (command.clone(), "magit-section-heading"),
                ],
                target: None,
            });
            for line in output.lines() {
                lines.push(crate::git::ListLine::plain(format!("  {line}"), "default"));
            }
        }
        if lines.is_empty() {
            lines.push(crate::git::ListLine::plain(
                "Nothing has been run yet",
                "shadow",
            ));
        }
        crate::git::ListView {
            title: "Git output".into(),
            lines,
        }
    }

    #[cfg(feature = "full")]
    /// Shows a list view in its own buffer.
    pub fn open_git_list(&mut self, name: &'static str, view: crate::git::ListView) -> Result<()> {
        let id = self.read_only_buffer(name);
        let text: String = view
            .lines
            .iter()
            .map(|line| format!("{}\n", line.text()))
            .collect();
        self.git_lists.insert(name.to_string(), view);
        self.replace_buffer_contents(id, &text).ok();
        self.show_in_editing_window(id)?;
        self.move_point_in(id, 0);
        Ok(())
    }

    #[cfg(feature = "full")]
    /// Shows a diff view in its own buffer, point at the top.
    pub fn open_git_diff(&mut self, name: &'static str, view: crate::git::DiffView) -> Result<()> {
        self.open_git_diff_showing(name, view, None)
    }

    #[cfg(feature = "full")]
    /// Shows a diff view, putting point on `keep` if that row is still there.
    ///
    /// Folding re-lays-out the whole buffer, so the line a row was on means
    /// nothing afterwards — the row does. This is what `render_git_buffer`
    /// does for the status buffer, and what folding a file in a commit needs
    /// so a reader is not sent back to the first line each time.
    pub fn open_git_diff_showing(
        &mut self,
        name: &'static str,
        mut view: crate::git::DiffView,
        keep: Option<crate::git::DiffRow>,
    ) -> Result<()> {
        let id = self.read_only_buffer(name);
        view.lay_out();
        let text: String = view
            .rows()
            .iter()
            .map(|row| format!("{}\n", crate::render::git_diff_row_text(&view, row)))
            .collect();
        let line = keep.and_then(|row| view.line_of(&row)).unwrap_or(0);
        self.git_diffs.insert(name.to_string(), view);
        self.replace_buffer_contents(id, &text).ok();
        self.show_in_editing_window(id)?;
        self.move_point_in(id, line);
        Ok(())
    }

    /// A buffer the editor writes and the user only reads.
    #[cfg(feature = "full")]
    fn read_only_buffer(&mut self, name: &str) -> BufferId {
        match self.buffers.find_by_name(name) {
            Some(id) => id,
            None => {
                let id = self.buffers.create_with_text(name, "");
                self.buffers
                    .get_mut(id)
                    .expect("just created")
                    .set_read_only(true);
                id
            }
        }
    }

    #[cfg(feature = "full")]
    /// The list view the current buffer is showing, if it is one.
    pub fn git_list(&self) -> Option<&crate::git::ListView> {
        self.git_lists.get(self.current_buffer().name())
    }

    #[cfg(feature = "full")]
    /// The diff view the current buffer is showing, if it is one.
    pub fn git_diff_view(&self) -> Option<&crate::git::DiffView> {
        self.git_diffs.get(self.current_buffer().name())
    }

    /// What the line point is on in a list view refers to.
    #[cfg(feature = "full")]
    pub fn git_list_target(&self) -> Option<String> {
        let view = self.git_list()?;
        let line = self.current_buffer().line_of(self.windows.current().point);
        view.lines.get(line)?.target.clone()
    }

    #[cfg(feature = "full")]
    /// Rewrites the status buffer from the view.
    ///
    /// Point comes back to the *row* it was on rather than the line: folding
    /// a section moves everything below it, and every command there acts on
    /// the row under point, which must still be under it afterwards.
    pub fn render_git_buffer(&mut self) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::git::STATUS_BUFFER_NAME)
        else {
            return;
        };
        let keep = self.git_row_at_cursor().cloned();
        let was_at = self.git_cursor_line();
        let rows = self.git.lay_out();
        let text: String = rows
            .iter()
            .map(|row| format!("{}\n", self.git_row_text(row)))
            .collect();
        self.replace_buffer_contents(id, &text).ok();
        // The row point was on, or the nearest still shown; a whole section
        // gone leaves the line number, which is at least where the eye is.
        let line = keep
            .and_then(|row| self.git.line_near(&row))
            .unwrap_or(was_at);
        self.move_git_cursor_to_line(line);
    }

    #[cfg(feature = "full")]
    /// The text of one status row, which is what point moves through.
    ///
    /// Deliberately plain: the faces and the alignment are the drawing's
    /// business, and point has to be able to sit anywhere on the line.
    fn git_row_text(&self, row: &crate::git::Row) -> String {
        use crate::git::Row;
        match row {
            Row::Header(head) => format!("{:<9}{} {}", head.label, head.reference, head.subject),
            Row::Blank => String::new(),
            Row::Section(section) => format!("{} ({})", section.title(), self.git.count(*section)),
            Row::Empty(_) => "Nothing to commit, the working tree is clean".to_string(),
            Row::File { section, file } => {
                let path = self
                    .git
                    .paths(*section)
                    .get(*file)
                    .cloned()
                    .unwrap_or_default();
                match self.git.files(*section).get(*file) {
                    Some(diff) => {
                        let (added, removed) = diff.counts();
                        format!("{path}  +{added} -{removed}")
                    }
                    None => path,
                }
            }
            Row::Hunk {
                section,
                file,
                hunk,
            } => self
                .git
                .files(*section)
                .get(*file)
                .and_then(|diff| diff.hunks.get(*hunk))
                .map(|hunk| hunk.header.clone())
                .unwrap_or_default(),
            Row::Line {
                section,
                file,
                hunk,
                line,
            } => self
                .git
                .files(*section)
                .get(*file)
                .and_then(|diff| diff.hunks.get(*hunk))
                .and_then(|hunk| hunk.lines.get(*line))
                .map(|line| line.to_patch_line())
                .unwrap_or_default(),
            Row::Stash(index) => match self.git.stashes.get(*index) {
                Some(stash) => format!("{} {}", stash.name, stash.subject),
                None => String::new(),
            },
            Row::Commit { section, commit } => match self.git.commits(*section).get(*commit) {
                Some(commit) => format!("{} {}", commit.short, commit.subject),
                None => String::new(),
            },
        }
    }

    #[cfg(feature = "full")]
    pub fn git_row_at_cursor(&self) -> Option<&crate::git::Row> {
        self.git.row(self.git_cursor_line())
    }

    #[cfg(feature = "full")]
    pub fn git_cursor_line(&self) -> usize {
        match self
            .buffers
            .find_by_name(crate::commands::git::STATUS_BUFFER_NAME)
        {
            Some(id) => self.line_in(id),
            None => 0,
        }
    }

    #[cfg(feature = "full")]
    pub fn move_git_cursor_to_line(&mut self, line: usize) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::git::STATUS_BUFFER_NAME)
        else {
            return;
        };
        self.move_point_in(id, line.min(self.git.rows().len().saturating_sub(1)));
    }

    /// Shows whatever was here before this buffer, without killing it.
    pub fn bury_buffer(&mut self) {
        let current = self.windows.current().buffer;
        let next = self
            .buffers
            .ids()
            .iter()
            .copied()
            .find(|id| *id != current)
            .unwrap_or(current);
        if next != current {
            self.switch_to_buffer(next).ok();
        }
    }

    #[cfg(feature = "full")]
    /// The size the terminal panel gives a program, in rows and columns.
    ///
    /// One row of the panel is the tab bar and one is the mode line, so what
    /// the program gets is smaller than the window. Telling it the window's
    /// size would put its last line under the mode line.
    pub fn terminal_size(&self) -> (usize, usize) {
        let Some(window) = self.terminal_window.and_then(|id| self.windows.get(id)) else {
            return (24, 80);
        };
        let rows = window.rect.height.saturating_sub(2).max(1) as usize;
        let columns = window.rect.width.max(1) as usize;
        (rows, columns)
    }

    #[cfg(feature = "full")]
    /// Tells every terminal, and every program in one, the new size.
    pub fn resize_terminals(&mut self) {
        if self.terminal_window.is_none() {
            return;
        }
        let (rows, columns) = self.terminal_size();
        let already = self
            .terminals
            .current()
            .map(|terminal| {
                let grid = terminal.emulator.grid();
                (grid.rows(), grid.columns())
            })
            .unwrap_or_default();
        if already == (rows, columns) {
            return;
        }
        self.terminals.resize(rows, columns);
        let ids: Vec<_> = self.terminals.iter().map(|terminal| terminal.id).collect();
        for terminal in ids {
            self.spawn(crate::task::Task::TerminalResize {
                terminal,
                rows: rows as u16,
                columns: columns as u16,
            });
        }
    }

    /// Puts the tree cursor on `line`, clamped to the snapshot.
    pub fn move_tree_cursor_to_line(&mut self, line: usize) {
        let Some(id) = self
            .buffers
            .find_by_name(crate::commands::tree::TREE_BUFFER_NAME)
        else {
            return;
        };
        self.move_point_in(id, line.min(self.tree.len().saturating_sub(1)));
    }

    /// Every theme that can be loaded: the ones built in, and any the
    /// configuration names.
    pub fn theme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = maxgus_faces::defaults::BUILTIN_THEMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        for spec in &self.theme_specs {
            if !names.contains(&spec.name) {
                names.push(spec.name.clone());
            }
        }
        names
    }

    /// Switches to a named theme.
    ///
    /// Unknown names are refused rather than quietly falling back, because
    /// here the name came from the user this moment; a name in the
    /// configuration file is a different matter and [`build_theme`] tolerates
    /// it so a typo cannot stop the editor from starting.
    pub fn set_theme(&mut self, name: &str) -> Result<()> {
        if !self.theme_names().iter().any(|known| known == name) {
            return Err(crate::CoreError::Message(format!(
                "No theme named `{name}`"
            )));
        }
        self.theme = build_theme(&self.theme_specs, name);
        // `describe-settings` reads this back, so it has to keep up.
        self.settings.theme = name.to_string();
        Ok(())
    }

    /// Puts the tree cursor on the line showing `path`, if it is visible.
    pub fn select_tree_path(&mut self, path: &Path) -> bool {
        match self.tree.iter().position(|n| n.path == path) {
            Some(line) => {
                self.move_tree_cursor_to_line(line);
                true
            }
            None => false,
        }
    }

    /// The tree line the cursor is on.
    pub fn tree_cursor_line(&self) -> usize {
        match self
            .buffers
            .find_by_name(crate::commands::tree::TREE_BUFFER_NAME)
        {
            Some(id) => self.line_in(id),
            None => 0,
        }
    }

    /// The tree node under the cursor.
    pub fn tree_selection(&self) -> Option<&maxgus_tree::VisibleNode> {
        self.tree.get(self.tree_cursor_line())
    }

    /// The symbol point is on in the outline window.
    pub fn symbol_at_cursor(&self) -> Option<usize> {
        let id = self
            .buffers
            .find_by_name(crate::commands::tree::SYMBOLS_BUFFER_NAME)?;
        self.panel.symbol_on_line(self.line_in(id))
    }

    /// The buffer the buffer-list window's point is on.
    pub fn listed_buffer_at_cursor(&self) -> Option<BufferId> {
        let id = self
            .buffers
            .find_by_name(crate::commands::tree::BUFFERS_BUFFER_NAME)?;
        self.panel_buffers()
            .get(self.line_in(id))
            .map(|(id, _)| *id)
    }

    /// Queues a re-parse for syntax highlighting, when the buffer has a
    /// language and highlighting is switched on.
    pub fn request_highlighting(&mut self, id: BufferId) {
        let _ = id;
        #[cfg(feature = "full")]
        {
            if !self.settings.syntax_highlighting {
                return;
            }
            let Some(buffer) = self.buffers.get(id) else {
                return;
            };
            let Some(language) = buffer.language().map(str::to_string) else {
                return;
            };
            let (text, revision) = (buffer.text(), buffer.revision());
            let range = self.highlight_request_range(id);
            self.spawn(Task::Reparse {
                buffer: id,
                language,
                text,
                revision,
                range,
            });
        }
    }

    #[cfg(feature = "full")]
    /// Opens the buffer to its language server, starting one if needed.
    pub fn request_language_server(&mut self, id: BufferId) {
        if !self.settings.lsp_enabled {
            return;
        }
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let Some(language) = buffer.language().map(str::to_string) else {
            return;
        };
        let Some(path) = buffer.path().map(Path::to_path_buf) else {
            return;
        };
        let (text, version) = (buffer.text(), buffer.revision() as i64);
        let uri = maxgus_lsp::client::path_to_uri(&path);
        self.spawn(Task::StartLanguageServer {
            language: language.clone(),
        });
        self.spawn(Task::LspDidOpen {
            language,
            uri,
            version,
            text,
        });
        self.lsp_versions.insert(id, version as u64);
    }

    #[cfg(feature = "full")]
    /// The language and document URI of `buffer`, when it has a server to talk
    /// to at all.
    fn lsp_document(&self, buffer: BufferId) -> Option<(String, String)> {
        if !self.settings.lsp_enabled {
            return None;
        }
        let buffer = self.buffers.get(buffer)?;
        let language = buffer.language()?.to_string();
        let uri = maxgus_lsp::client::path_to_uri(buffer.path()?);
        Some((language, uri))
    }

    /// Tells the language server about edits to `buffer`, if there are any it
    /// has not been told about.
    ///
    /// Without this the server's copy of the document goes stale the moment
    /// the user types, and everything it says afterwards is about the wrong
    /// text. Returns whether a notification was queued.
    pub fn sync_language_server(&mut self, buffer: BufferId) -> bool {
        #[cfg(not(feature = "full"))]
        {
            let _ = buffer;
            false
        }
        #[cfg(feature = "full")]
        {
            let Some((language, uri)) = self.lsp_document(buffer) else {
                return false;
            };
            let Some(revision) = self.buffers.get(buffer).map(|b| b.revision()) else {
                return false;
            };
            // Nothing to say if the server already has this version, and nothing
            // to say about a document it was never told about.
            match self.lsp_versions.get(&buffer) {
                Some(known) if *known == revision => return false,
                None => return false,
                Some(_) => {}
            }
            let text = self.buffers.get(buffer).expect("checked above").text();
            self.lsp_versions.insert(buffer, revision);
            self.spawn(Task::LspDidChange {
                language,
                uri,
                version: revision as i64,
                text,
            });
            true
        }
    }

    /// Tells the language server a buffer was saved.
    pub fn notify_saved(&mut self, buffer: BufferId) {
        let _ = buffer;
        #[cfg(feature = "full")]
        {
            // A save is only meaningful for a document the server knows about.
            if !self.lsp_versions.contains_key(&buffer) {
                return;
            }
            if let Some((language, uri)) = self.lsp_document(buffer) {
                self.spawn(Task::LspDidSave { language, uri });
            }
        }
    }

    /// Tells the language server a buffer is gone, and forgets its version.
    pub fn notify_closed(&mut self, buffer: BufferId) {
        let _ = buffer;
        #[cfg(feature = "full")]
        {
            if self.lsp_versions.remove(&buffer).is_none() {
                return;
            }
            if let Some((language, uri)) = self.lsp_document(buffer) {
                self.spawn(Task::LspDidClose { language, uri });
            }
        }
    }

    // ---- the region ----------------------------------------------------

    /// The active region in the current buffer, or an error naming what is
    /// missing — the message `C-w` shows when there is no mark.
    pub fn region(&mut self) -> Result<Range> {
        self.sync_to_buffer();
        let buffer = self.current_buffer();
        match buffer.region() {
            Some(range) => Ok(range),
            None if buffer.mark().is_none() => Err(crate::CoreError::Message(
                "The mark is not set now, so there is no region".into(),
            )),
            None => Err(crate::CoreError::Message(
                "The mark is not active now".into(),
            )),
        }
    }

    /// Adds `text` to the kill ring, appending when the previous command was
    /// also a kill, and puts the result on the system clipboard.
    pub fn kill(&mut self, text: &str, before: bool) {
        if self.kill_appending {
            self.kill_ring.kill_append(text, before);
        } else {
            self.kill_ring.kill_new(text);
        }
        // The whole entry, not the piece: `C-k C-k` copies both lines.
        if let Some(clipboard) = self.clipboard.as_mut()
            && let Some(newest) = self.kill_ring.newest()
        {
            clipboard.write(newest);
        }
    }

    /// Brings what another program put on the clipboard into the kill ring,
    /// so a yank takes it first — Emacs' `interprogram-paste-function`.
    ///
    /// Only when it is new: the last kill went to the clipboard too, and
    /// finding it there is not another program's doing. Nothing here
    /// happens for `M-y`, which is walking the ring, not asking what is
    /// newest.
    pub fn interprogram_paste(&mut self) {
        let Some(text) = self
            .clipboard
            .as_mut()
            .and_then(|clipboard| clipboard.read())
        else {
            return;
        };
        if text.is_empty() || self.kill_ring.newest() == Some(text.as_str()) {
            return;
        }
        self.kill_ring.kill_new(text);
    }

    // ---- the mode line -------------------------------------------------

    /// The mode line for `window`, in Emacs' layout: modification flags, the
    /// buffer name, the position, and the major mode.
    pub fn mode_line(&self, window: WindowId) -> String {
        self.mode_line_segments(window)
            .into_iter()
            .map(|segment| segment.text)
            .collect::<Vec<_>>()
            .join("")
    }

    /// The mode line broken into the pieces it is painted from.
    ///
    /// Each piece carries its own face, so state, position, language and
    /// diagnostics can be told apart by colour rather than by counting
    /// punctuation — the thing a doom-style mode line gets right. The plain
    /// text is still available through [`Editor::mode_line`].
    pub fn mode_line_segments(&self, window: WindowId) -> Vec<ModeLineSegment> {
        let Some(window) = self.windows.get(window) else {
            return Vec::new();
        };
        let Some(buffer) = self.buffers.get(window.buffer) else {
            return Vec::new();
        };
        let icons = self.settings.nerd_font_icons;
        let mut out = Vec::new();

        // What state the buffer is in, said once and in colour.
        let (state, state_face) = match (buffer.is_read_only(), buffer.is_modified()) {
            (true, _) => (
                if icons {
                    crate::icons::READ_ONLY.to_string()
                } else {
                    "%%".into()
                },
                "warning",
            ),
            (false, true) => (
                if icons {
                    crate::icons::MODIFIED.to_string()
                } else {
                    "**".into()
                },
                "error",
            ),
            (false, false) => (
                if icons {
                    crate::icons::SAVED.to_string()
                } else {
                    "--".into()
                },
                "success",
            ),
        };
        out.push(ModeLineSegment::new(format!(" {state} "), state_face));

        // How big it is, as a person would say it.
        out.push(ModeLineSegment::new(
            format!("{} ", human_size(buffer.len_chars())),
            "shadow",
        ));

        // The buffer, behind the glyph for whatever kind of file it is.
        let mut name = String::new();
        if icons {
            let glyph = match buffer.path() {
                Some(path) => crate::icons::for_file(path),
                None => crate::icons::for_language(buffer.language().unwrap_or_default()),
            };
            name.push(glyph);
            name.push(' ');
        }
        // The path within the project rather than the bare file name: three
        // buffers called `mod.rs` are told apart by where they are, and that
        // is exactly what the bare name leaves out.
        name.push_str(&self.project_path(buffer));
        let mut name = ModeLineSegment::new(name, "mode-line-buffer-id");
        name.shortens = true;
        out.push(name);

        // The coding system, which is how `set-buffer-file-coding-system`
        // reports itself. Only shown when it is not the ordinary one, so the
        // bar stays quiet in the common case.
        if buffer.line_ending() != maxgus_text::LineEnding::Lf {
            out.push(ModeLineSegment::new(
                format!("  {}", buffer.line_ending().mode_line_mnemonic()),
                "warning",
            ));
        }

        let position = buffer.position_of(window.point);
        let percent = self.scroll_indicator(window.top_line, buffer);
        let marker = if icons {
            format!("{} ", crate::icons::POSITION)
        } else {
            String::new()
        };
        out.push(ModeLineSegment::new(
            format!(
                "  {marker}{}:{}  {percent}",
                position.line + 1,
                position.column
            ),
            "shadow",
        ));

        if buffer.is_narrowed() {
            out.push(ModeLineSegment::new("  Narrow".to_string(), "warning"));
        }

        // From here to the right edge.
        #[cfg(feature = "full")]
        for mut segment in self.diagnostic_segments(buffer) {
            segment.right = true;
            out.push(segment);
        }

        #[cfg(feature = "full")]
        if let Some(branch) = &self.git_branch {
            let glyph = if icons {
                format!("{} ", crate::icons::BRANCH)
            } else {
                String::new()
            };
            out.push(ModeLineSegment::right(
                format!("  {glyph}{branch}"),
                "success",
            ));
        }

        let mode = self.mode_name(window.buffer);
        let glyph = match icons && buffer.language().is_some() {
            true => format!("{} ", crate::icons::for_language(&mode)),
            false => String::new(),
        };
        out.push(ModeLineSegment::right(
            format!("  {glyph}{mode}  "),
            "mode-line",
        ));
        out
    }

    /// Where the buffer's file is, relative to the project it is in.
    ///
    /// Falls back to the bare name for a buffer with no file, and for a file
    /// outside the project — an absolute path in the mode line is usually
    /// longer than the bar and tells the reader nothing they wanted.
    fn project_path(&self, buffer: &maxgus_text::Buffer) -> String {
        let Some(path) = buffer.path() else {
            return buffer.name().to_string();
        };
        #[cfg(feature = "full")]
        let root = self.git_root.as_ref().or(self.tree_root.as_ref());
        #[cfg(not(feature = "full"))]
        let root = self.tree_root.as_ref();
        let Some(root) = root else {
            return buffer.name().to_string();
        };
        match path.strip_prefix(root) {
            // The project's own name in front, as `doom` shows it: it is what
            // tells two checkouts of the same repository apart.
            Ok(inside) => match root.file_name() {
                Some(project) => format!("{}/{}", project.to_string_lossy(), inside.display()),
                None => inside.display().to_string(),
            },
            Err(_) => buffer.name().to_string(),
        }
    }

    #[cfg(feature = "full")]
    /// The error and warning counts, each in its own colour.
    fn diagnostic_segments(&self, buffer: &Buffer) -> Vec<ModeLineSegment> {
        let Some(path) = buffer.path() else {
            return Vec::new();
        };
        let counts = self
            .diagnostics
            .counts(&maxgus_lsp::client::path_to_uri(path));
        let icons = self.settings.nerd_font_icons;
        [
            (counts[0], crate::icons::ERROR, 'E', "diagnostic-error"),
            (counts[1], crate::icons::WARNING, 'W', "diagnostic-warning"),
        ]
        .into_iter()
        .filter(|(n, _, _, _)| *n > 0)
        .map(|(n, glyph, letter, face)| {
            let mark = if icons {
                glyph.to_string()
            } else {
                format!("{letter}:")
            };
            ModeLineSegment::new(format!("  {mark} {n}"), face)
        })
        .collect()
    }

    /// `Top`, `Bot`, `All`, or a percentage — the indicator Emacs shows.
    fn scroll_indicator(&self, top_line: usize, buffer: &Buffer) -> String {
        let window = self.windows.current();
        let height = window.text_height();
        let total = buffer.len_lines();
        if height == 0 || total == 0 {
            return "All".to_string();
        }
        let at_top = top_line == 0;
        let at_bottom = top_line + height >= total;
        match (at_top, at_bottom) {
            (true, true) => "All".to_string(),
            (true, false) => "Top".to_string(),
            (false, true) => "Bot".to_string(),
            (false, false) => {
                let above = top_line;
                let below = total.saturating_sub(top_line + height);
                let percent = above * 100 / (above + below).max(1);
                format!("{percent}%")
            }
        }
    }

    /// The frame title: the buffer name and its directory, as Emacs sets it.
    pub fn frame_title(&self) -> String {
        let buffer = self.current_buffer();
        match buffer.path().and_then(|p| p.parent()) {
            Some(directory) => format!("{} — {}", buffer.name(), directory.display()),
            None => buffer.name().to_string(),
        }
    }
}

/// One piece of the mode line, with the face it is painted in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeLineSegment {
    pub text: String,
    /// The face name, resolved against the theme at drawing time.
    pub face: &'static str,
    /// True for the segments that sit against the right edge.
    ///
    /// What is being edited belongs on the left, where the eye starts; what
    /// the editor knows *about* it — the language, the branch, how many
    /// problems there are — belongs on the right, where it can be glanced at
    /// without reading past it.
    pub right: bool,
    /// True for a segment that may lose its front to fit a narrow bar,
    /// rather than being left off it. The buffer's name is the one.
    pub shortens: bool,
}

impl ModeLineSegment {
    fn new(text: impl Into<String>, face: &'static str) -> ModeLineSegment {
        ModeLineSegment {
            text: text.into(),
            face,
            right: false,
            shortens: false,
        }
    }

    fn right(text: impl Into<String>, face: &'static str) -> ModeLineSegment {
        ModeLineSegment {
            text: text.into(),
            face,
            right: true,
            shortens: false,
        }
    }
}

/// The theme `name` describes: the built-in theme of that name, with the
/// configuration's `theme "name" { … }` block laid over it.
///
/// Tolerant of an unknown name, which falls back to the default theme — a
/// mistyped `theme` setting must not stop the editor from starting. The one
/// construction both startup and `load-theme` go through, so the two cannot
/// drift apart.
pub fn build_theme(specs: &[ThemeSpec], name: &str) -> Theme {
    let spec = specs.iter().find(|spec| spec.name == name);
    // A theme file that names a `base` starts from that built-in; otherwise
    // from the built-in of the same name, and failing that from the default.
    // Without this a light theme would inherit dark values for every face it
    // did not happen to set.
    let base = spec.and_then(|s| s.base.as_deref()).unwrap_or(name);
    let mut theme = maxgus_faces::defaults::builtin_or_fallback(base);
    if let Some(spec) = spec {
        // The theme keeps the name it was asked for, not the base's.
        theme.set_name(name);
        // A bad colour in the user's block leaves the built-in one intact.
        theme.apply_spec(spec).ok();
    }
    theme
}

/// A size as a person would say it: `812`, `6.6k`, `1.2M`.
///
/// Counted in characters rather than bytes, which is what the buffer holds
/// and what a reader of a text file means by how big it is.
fn human_size(characters: usize) -> String {
    const K: usize = 1024;
    match characters {
        _ if characters < K => format!("{characters}"),
        _ if characters < K * K => format!("{:.1}k", characters as f64 / K as f64),
        _ => format!("{:.1}M", characters as f64 / (K * K) as f64),
    }
}

/// `dired-mode` as the mode line writes it: `Dired`. Emacs shows a mode
/// by its name with `-mode` taken off and the words capitalised.
pub fn mode_display_name(keymap: &str) -> String {
    keymap
        .strip_suffix("-mode")
        .unwrap_or(keymap)
        .split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_faces::defaults;

    fn editor() -> Editor {
        Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        )
    }

    /// An editor showing a buffer with `text`, point at the start.
    fn editor_with(text: &str) -> Editor {
        let mut e = editor();
        let id = e.buffers.create_with_text("test", text);
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.set_point(0));
        e
    }

    #[test]
    fn a_new_editor_shows_scratch_in_one_window() {
        let e = editor();
        assert_eq!(e.windows.len(), 1);
        assert_eq!(e.current_buffer().name(), crate::buffers::SCRATCH_NAME);
        assert!(!e.quit);
        assert!(e.tasks.is_empty());
    }

    #[test]
    fn point_moves_between_the_window_and_the_buffer() {
        let mut e = editor_with("hello world");
        e.windows.current_mut().point = 6;
        e.sync_to_buffer();
        assert_eq!(e.current_buffer().point(), 6);

        e.with_current_buffer(|b| b.set_point(2));
        assert_eq!(
            e.windows.current().point,
            2,
            "the window followed the buffer"
        );
    }

    #[test]
    fn two_windows_on_one_buffer_keep_separate_points() {
        let mut e = editor_with("0123456789");
        let second = e.split_window(Direction::Vertical).unwrap();
        e.windows.current_mut().point = 2;
        e.windows.get_mut(second).unwrap().point = 8;

        e.select_window(second);
        e.sync_to_buffer();
        assert_eq!(e.current_buffer().point(), 8);

        e.other_window(1);
        e.sync_to_buffer();
        assert_eq!(
            e.current_buffer().point(),
            2,
            "the first window's point survived"
        );
    }

    #[test]
    fn editing_through_the_helper_updates_both_copies_of_point() {
        let mut e = editor_with("");
        e.with_current_buffer(|b| b.insert_at_point("typed").unwrap());
        assert_eq!(e.current_buffer().text(), "typed");
        assert_eq!(e.windows.current().point, 5);
    }

    #[test]
    fn switching_buffers_remembers_where_point_was() {
        let mut e = editor();
        let a = e.buffers.create_with_text("a", "aaaa");
        let b = e.buffers.create_with_text("b", "bbbb");

        e.switch_to_buffer(a).unwrap();
        e.with_current_buffer(|buffer| buffer.set_point(3));
        e.switch_to_buffer(b).unwrap();
        assert_eq!(e.current_buffer().name(), "b");

        e.switch_to_buffer(a).unwrap();
        assert_eq!(e.windows.current().point, 3, "point came back");
    }

    #[test]
    fn switching_makes_the_buffer_most_recently_used() {
        let mut e = editor();
        let a = e.buffers.create("a");
        e.buffers.create("b");
        e.switch_to_buffer(a).unwrap();
        assert_eq!(e.buffers.ids().first(), Some(&a));
    }

    #[test]
    fn switching_to_an_unknown_buffer_is_an_error() {
        let mut e = editor();
        assert!(matches!(
            e.switch_to_buffer(BufferId(999)),
            Err(crate::CoreError::NoSuchBuffer)
        ));
    }

    #[test]
    fn killing_a_displayed_buffer_repoints_its_windows() {
        let mut e = editor();
        let a = e.buffers.create_with_text("a", "text");
        e.switch_to_buffer(a).unwrap();
        e.split_window(Direction::Vertical).unwrap();

        let replacement = e.kill_buffer(a).unwrap();
        assert!(e.buffers.get(a).is_none());
        assert!(
            e.windows.iter().all(|w| w.buffer == replacement),
            "no window still shows the dead buffer"
        );
    }

    #[test]
    fn the_view_follows_point_when_it_leaves_the_screen() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let mut e = editor_with(&text);
        assert_eq!(e.windows.current().top_line, 0);

        e.with_current_buffer(|b| {
            let end = b.len_chars();
            b.set_point(end);
        });
        e.follow_point();
        let window = e.windows.current();
        assert!(window.top_line > 0, "the window scrolled down");
        assert!(window.shows_line(e.current_buffer().line_of(window.point)));
    }

    #[test]
    fn the_scroll_margin_is_honoured_when_following_point() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let mut e = editor_with(&text);
        e.settings.scroll_margin = 5;
        e.with_current_buffer(|b| b.set_point(b.line_start(40)));
        e.follow_point();
        let window = e.windows.current();
        assert!(window.top_line <= 35, "five lines of context above point");
    }

    #[test]
    fn horizontal_scrolling_only_happens_when_lines_are_truncated() {
        let long = "x".repeat(500);
        let mut e = editor_with(&long);
        e.settings.truncate_lines = true;
        e.with_current_buffer(|b| b.set_point(400));
        e.follow_point();
        assert!(e.windows.current().left_column > 0);

        e.settings.truncate_lines = false;
        e.follow_point();
        assert_eq!(
            e.windows.current().left_column,
            0,
            "wrapped lines never scroll sideways"
        );
    }

    #[test]
    fn the_cursor_sits_where_point_is_on_screen() {
        let mut e = editor_with("line one\nline two\nline three");
        e.with_current_buffer(|b| b.set_point(b.line_start(1) + 4));
        e.follow_point();
        assert_eq!(e.cursor_position(), (4, 1));
    }

    #[test]
    fn the_cursor_moves_to_the_minibuffer_while_it_prompts() {
        let mut e = editor_with("text");
        e.minibuffer
            .activate(crate::MinibufferKind::Command, "M-x ");
        e.minibuffer.insert("save");
        let (x, y) = e.cursor_position();
        assert_eq!(y, 23, "the last row of the frame");
        assert_eq!(x, 8, "after `M-x save`");
    }

    #[test]
    fn the_cursor_accounts_for_scrolling() {
        let text: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let mut e = editor_with(&text);
        e.with_current_buffer(|b| b.set_point(b.line_start(50)));
        e.follow_point();
        let (_, y) = e.cursor_position();
        let window = e.windows.current();
        assert!(
            y < window.rect.bottom(),
            "the cursor stays inside the window"
        );
        assert_eq!(y as usize, 50 - window.top_line);
    }

    #[test]
    fn the_cursor_has_no_gutter_in_a_view_drawn_without_line_numbers() {
        let mut e = editor_with("line one\nline two");
        e.settings.line_numbers = true;
        e.follow_point();
        assert_eq!(e.cursor_position().0, 2, "a digit and a space");

        let id = e
            .buffers
            .create_with_text(crate::commands::tree::TREE_BUFFER_NAME, "root\n  file");
        e.switch_to_buffer(id).unwrap();
        e.follow_point();
        assert_eq!(e.cursor_position().0, 0, "the tree draws no line numbers");
    }

    #[test]
    fn horizontal_scrolling_keeps_point_inside_the_text_columns() {
        let long = "x".repeat(500);
        let mut e = editor_with(&long);
        e.settings.truncate_lines = true;
        e.settings.line_numbers = true;
        e.with_current_buffer(|b| b.set_point(400));
        e.follow_point();
        let window = e.windows.current();
        let gutter = 2; // one digit and a space
        let edge = 1; // the column kept for the `$`
        assert_eq!(
            window.left_column,
            400 + 1 - (window.rect.width as usize - gutter - edge),
            "point is the rightmost text column, after the gutter"
        );
    }

    #[test]
    fn the_cursor_accounts_for_tab_expansion() {
        let mut e = editor_with("\t\tx");
        e.with_current_buffer(|b| {
            b.set_tab_width(4);
            b.set_point(2);
        });
        e.follow_point();
        assert_eq!(e.cursor_position().0, 8, "two tabs at width four");
    }

    #[test]
    fn the_region_reports_why_it_is_unavailable() {
        let mut e = editor_with("hello world");
        let err = e.region().unwrap_err().to_string();
        assert!(err.contains("mark is not set"), "got `{err}`");

        e.with_current_buffer(|b| {
            b.set_mark(0);
            b.set_point(5);
        });
        assert_eq!(e.region().unwrap(), Range::new(0, 5));

        e.with_current_buffer(|b| b.deactivate_mark());
        let err = e.region().unwrap_err().to_string();
        assert!(err.contains("not active"), "got `{err}`");
    }

    #[test]
    fn consecutive_kills_append_to_one_entry() {
        let mut e = editor();
        e.kill("first", false);
        assert_eq!(e.kill_ring.front(), Some("first"));
        assert_eq!(e.kill_ring.len(), 1);

        e.kill_appending = true;
        e.kill(" second", false);
        assert_eq!(e.kill_ring.front(), Some("first second"));
        assert_eq!(e.kill_ring.len(), 1, "still one entry");

        e.kill_appending = false;
        e.kill("separate", false);
        assert_eq!(e.kill_ring.len(), 2);
    }

    #[test]
    fn a_backward_kill_prepends_when_appending() {
        let mut e = editor();
        e.kill("word", false);
        e.kill_appending = true;
        e.kill("before ", true);
        assert_eq!(e.kill_ring.front(), Some("before word"));
    }

    #[test]
    fn the_mode_line_shows_the_buffer_state() {
        let mut e = editor();
        let id = e
            .buffers
            .visit_file("/project/src/main.rs", "fn main() {}\n");
        e.switch_to_buffer(id).unwrap();

        let line = e.mode_line(e.windows.current_id());
        assert!(line.contains("main.rs"), "got `{line}`");
        assert!(line.contains("rust"), "the major mode, got `{line}`");
        assert!(line.contains("1:0"), "the position, got `{line}`");
        assert!(
            line.contains(crate::icons::SAVED),
            "unmodified and writable, got `{line}`"
        );
    }

    #[test]
    fn the_mode_line_flags_modification_and_read_only_state() {
        let mut e = editor_with("text");
        let state = |e: &Editor| e.mode_line(e.windows.current_id());

        e.with_current_buffer(|b| b.insert_at_point("more").unwrap());
        assert!(
            state(&e).contains(crate::icons::MODIFIED),
            "got `{}`",
            state(&e)
        );

        // Read-only is reported over modified: it is the more useful of the
        // two to know, because it is the one that stops a save.
        e.with_current_buffer(|b| b.set_read_only(true));
        assert!(
            state(&e).contains(crate::icons::READ_ONLY),
            "got `{}`",
            state(&e)
        );

        e.with_current_buffer(|b| {
            b.set_read_only(false);
            b.mark_saved();
        });
        assert!(
            state(&e).contains(crate::icons::SAVED),
            "got `{}`",
            state(&e)
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn killing_a_buffer_forgets_the_highlights_it_had() {
        // The ordinary case, and the one the late-result guard does *not*
        // cover: a buffer whose highlights were already filed. Without this,
        // every file opened and closed leaves its spans behind.
        let mut editor = editor();
        let id = editor.buffers.create_with_text("doomed", "fn main() {}\n");
        editor.switch_to_buffer(id).unwrap();
        editor
            .apply_task_result(crate::TaskResult::Reparsed {
                buffer: id,
                revision: 1,
                range: 0..12,
                highlights: vec![maxgus_syntax::Highlight {
                    start: 0,
                    end: 2,
                    face: "font-lock-keyword",
                }],
            })
            .unwrap();
        assert!(
            editor.highlights.contains_key(&id),
            "filed while it was alive"
        );

        editor.kill_buffer(id).unwrap();
        assert!(
            !editor.highlights.contains_key(&id),
            "the killed buffer's highlights were left behind"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn highlights_for_a_buffer_that_is_still_alive_are_filed_normally() {
        // The other half: a guard that drops everything protects nothing.
        let mut editor = editor();
        let id = editor.buffers.create_with_text("live", "fn main() {}\n");
        editor.switch_to_buffer(id).unwrap();

        editor
            .apply_task_result(crate::TaskResult::Reparsed {
                buffer: id,
                revision: 1,
                range: 0..12,
                highlights: vec![maxgus_syntax::Highlight {
                    start: 0,
                    end: 2,
                    face: "font-lock-keyword",
                }],
            })
            .unwrap();

        assert_eq!(
            editor.highlights_for(id).len(),
            1,
            "the answer was thrown away"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn highlights_for_a_buffer_killed_mid_parse_are_not_filed_against_it() {
        // The parse is already running when the buffer is killed, so its
        // answer arrives after `kill_buffer` has cleaned up. Filing it anyway
        // leaves an entry nothing will ever remove — buffer ids are never
        // reused — and a session that opens and closes files grows a map of
        // highlight spans for buffers that no longer exist.
        let mut editor = editor();
        let id = editor.buffers.create_with_text("doomed", "fn main() {}\n");
        editor.switch_to_buffer(id).unwrap();
        editor.kill_buffer(id).unwrap();
        assert!(editor.buffers.get(id).is_none(), "the buffer is gone");

        editor
            .apply_task_result(crate::TaskResult::Reparsed {
                buffer: id,
                revision: 1,
                range: 0..12,
                highlights: vec![maxgus_syntax::Highlight {
                    start: 0,
                    end: 2,
                    face: "font-lock-keyword",
                }],
            })
            .unwrap();

        assert!(
            !editor.highlights.contains_key(&id),
            "a dead buffer kept an entry in the highlight map"
        );
    }

    #[test]
    fn a_mistyped_theme_in_the_configuration_still_starts_the_editor() {
        // Startup is deliberately forgiving where `load-theme` is not: a typo
        // in the config file must not leave the user with no editor.
        let theme = crate::build_theme(&[], "solarized");
        assert_eq!(theme.name(), maxgus_faces::defaults::FALLBACK_THEME);
    }

    #[test]
    fn a_theme_starts_from_the_base_it_names() {
        // A light theme that started from the dark built-in would come out
        // dark everywhere it did not happen to set a face — which is most of
        // them, since the point of `base` is not having to set them all.
        let mut spec = maxgus_config::ThemeSpec::new("daylight");
        spec.base = Some("maxgus-light".into());
        spec.faces.push(maxgus_config::FaceSpec {
            name: "region".into(),
            background: Some("#cceeff".into()),
            ..Default::default()
        });

        let theme = crate::build_theme(std::slice::from_ref(&spec), "daylight");
        assert_eq!(
            theme.name(),
            "daylight",
            "it keeps its own name, not the base's"
        );
        assert_eq!(
            theme.resolve("default"),
            maxgus_faces::defaults::builtin("maxgus-light")
                .unwrap()
                .resolve("default"),
            "a face it did not set came from the wrong built-in"
        );
        assert_eq!(
            theme.resolve("region").background,
            Some(maxgus_faces::Color::Rgb(204, 238, 255)),
            "the face it did set was lost"
        );
    }

    #[test]
    fn a_theme_with_no_base_starts_from_the_built_in_of_its_own_name() {
        // Which is what a block adjusting a built-in relies on.
        let mut spec = maxgus_config::ThemeSpec::new("maxgus-light");
        spec.faces.push(maxgus_config::FaceSpec {
            name: "region".into(),
            background: Some("#cceeff".into()),
            ..Default::default()
        });
        let theme = crate::build_theme(std::slice::from_ref(&spec), "maxgus-light");
        assert_eq!(
            theme.resolve("default"),
            maxgus_faces::defaults::builtin("maxgus-light")
                .unwrap()
                .resolve("default")
        );
    }

    #[test]
    fn build_theme_lays_the_configuration_over_the_built_in_theme() {
        let mut spec = maxgus_config::ThemeSpec::new("maxgus-dark");
        spec.faces.push(maxgus_config::FaceSpec {
            name: "region".into(),
            foreground: Some("#00ff00".into()),
            ..Default::default()
        });
        let theme = crate::build_theme(std::slice::from_ref(&spec), "maxgus-dark");
        assert_eq!(theme.name(), "maxgus-dark");
        assert_eq!(
            theme.resolve("region").foreground,
            Some(maxgus_faces::Color::Rgb(0, 255, 0))
        );
    }

    #[test]
    fn setting_an_unknown_theme_leaves_the_one_in_use_alone() {
        let mut editor = editor();
        let before = editor.theme.name().to_string();
        assert!(editor.set_theme("solarized").is_err());
        assert_eq!(editor.theme.name(), before);
        assert_eq!(editor.settings.theme, "maxgus-dark");
    }

    #[test]
    fn the_mode_line_shows_the_line_ending() {
        let mut e = editor();
        let id = e.buffers.create_with_text("dos", "a\r\nb\r\n");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.set_line_ending(maxgus_text::LineEnding::Crlf));
        let line = e.mode_line(e.windows.current_id());
        assert!(
            line.contains(maxgus_text::LineEnding::Crlf.mode_line_mnemonic()),
            "backslash means CRLF, got `{line}`"
        );

        // And it stays quiet for the ordinary one, which is most files.
        e.with_current_buffer(|b| b.set_line_ending(maxgus_text::LineEnding::Lf));
        let line = e.mode_line(e.windows.current_id());
        assert!(
            !line.contains(maxgus_text::LineEnding::Crlf.mode_line_mnemonic()),
            "LF should say nothing, got `{line}`"
        );
    }

    #[test]
    fn the_mode_line_reports_the_scroll_position() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let mut e = editor_with(&text);
        assert!(e.mode_line(e.windows.current_id()).contains("Top"));

        e.with_current_buffer(|b| {
            let end = b.len_chars();
            b.set_point(end);
        });
        e.follow_point();
        assert!(e.mode_line(e.windows.current_id()).contains("Bot"));

        e.with_current_buffer(|b| b.set_point(b.line_start(100)));
        e.follow_point();
        let line = e.mode_line(e.windows.current_id());
        assert!(
            line.contains('%'),
            "a percentage in the middle, got `{line}`"
        );
    }

    #[test]
    fn a_buffer_shorter_than_the_window_reports_all() {
        let e = editor_with("one line");
        assert!(e.mode_line(e.windows.current_id()).contains("All"));
    }

    #[test]
    fn the_mode_line_flags_a_narrowed_buffer() {
        let mut e = editor_with("0123456789");
        e.with_current_buffer(|b| b.narrow(Range::new(2, 6)));
        assert!(e.mode_line(e.windows.current_id()).contains("Narrow"));
    }

    #[test]
    fn the_mode_line_of_an_unknown_window_is_empty() {
        let e = editor();
        assert_eq!(e.mode_line(WindowId(999)), "");
    }

    #[test]
    fn the_frame_title_names_the_buffer_and_its_directory() {
        let mut e = editor();
        assert_eq!(
            e.frame_title(),
            crate::buffers::SCRATCH_NAME,
            "no file, no directory"
        );
        let id = e.buffers.visit_file("/project/src/main.rs", "");
        e.switch_to_buffer(id).unwrap();
        assert_eq!(e.frame_title(), "main.rs — /project/src");
    }

    /// An editor visiting a real file, with the opening tasks drained.
    fn editor_visiting(path: &str, text: &str) -> (Editor, BufferId) {
        let mut e = editor();
        let id = e.buffers.visit_file(path, text);
        e.switch_to_buffer(id).unwrap();
        #[cfg(feature = "full")]
        e.request_language_server(id);
        e.tasks.drain();
        (e, id)
    }

    #[cfg(feature = "full")]
    #[test]
    fn opening_a_file_records_the_version_the_server_was_told() {
        let (e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        assert!(
            e.lsp_versions.contains_key(&id),
            "the document is now known to the server"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn an_edit_is_reported_to_the_language_server() {
        let (mut e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        assert!(!e.sync_language_server(id), "nothing has changed yet");

        e.with_current_buffer(|b| b.insert_at_point("// edit\n").unwrap());
        assert!(e.sync_language_server(id), "the change is worth reporting");

        let tasks = e.tasks.drain();
        let Some(Task::LspDidChange { text, version, .. }) = tasks.into_iter().next() else {
            panic!("no change notification was queued");
        };
        assert!(
            text.starts_with("// edit"),
            "the server is sent the new text"
        );
        assert_eq!(version as u64, e.current_buffer().revision());
    }

    #[cfg(feature = "full")]
    #[test]
    fn the_same_edit_is_not_reported_twice() {
        let (mut e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        assert!(e.sync_language_server(id));
        e.tasks.drain();
        assert!(
            !e.sync_language_server(id),
            "the server is already up to date"
        );
        assert!(e.tasks.is_empty());
    }

    #[test]
    fn a_buffer_the_server_never_saw_is_not_reported() {
        let mut e = editor();
        let id = e.buffers.create_with_text("notes", "text");
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.insert_at_point("more").unwrap());
        assert!(
            !e.sync_language_server(id),
            "there is no open document to change"
        );
        assert!(e.tasks.is_empty());
    }

    #[test]
    fn nothing_is_reported_when_language_server_support_is_off() {
        let (mut e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        e.settings.lsp_enabled = false;
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        assert!(!e.sync_language_server(id));
        assert!(e.tasks.is_empty());
    }

    #[cfg(feature = "full")]
    #[test]
    fn saving_tells_the_server_the_file_is_on_disk() {
        let (mut e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        e.apply_task_result(crate::TaskResult::FileWritten {
            path: std::path::PathBuf::from("/project/main.rs"),
            buffer: id,
            bytes: 13,
            disk_time: None,
        })
        .unwrap();
        let tasks = e.tasks.drain();
        assert!(
            tasks.iter().any(|t| matches!(t, Task::LspDidSave { .. })),
            "no save notification, got {tasks:?}"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn killing_a_buffer_closes_its_document_and_drops_its_highlighting() {
        let (mut e, id) = editor_visiting("/project/main.rs", "fn main() {}\n");
        e.highlights.insert(id, (0, 0..usize::MAX, Vec::new()));
        e.buffers.create("somewhere-else");

        e.kill_buffer(id).unwrap();
        let tasks = e.tasks.drain();
        assert!(
            tasks.iter().any(|t| matches!(t, Task::LspDidClose { .. })),
            "no close notification, got {tasks:?}"
        );
        assert!(
            !e.lsp_versions.contains_key(&id),
            "the version was forgotten"
        );
        assert!(
            e.highlights_for(id).is_empty(),
            "the highlighting went with it"
        );
    }

    #[test]
    fn a_document_is_only_closed_once() {
        let (mut e, id) = editor_visiting("/project/main.rs", "text");
        e.notify_closed(id);
        e.tasks.drain();
        e.notify_closed(id);
        assert!(e.tasks.is_empty(), "the second close has nothing to say");
    }

    /// An editor showing a file long enough that the window covers only part
    /// of it, with the opening tasks drained.
    #[cfg(feature = "full")]
    fn editor_with_long_file() -> (Editor, BufferId) {
        let text: String = (0..2_000).map(|n| format!("line {n}\n")).collect();
        let mut e = editor();
        let id = e.buffers.visit_file("/project/long.rs", &text);
        e.switch_to_buffer(id).unwrap();
        e.tasks.drain();
        (e, id)
    }

    /// A keymap named `<language>-mode` binding `C-t` to `command`.
    fn mode_map(language: &str, command: &str) -> maxgus_keys::Keymap {
        let mut map = maxgus_keys::Keymap::new(format!("{language}-mode"));
        map.define_str("C-t", command).unwrap();
        map
    }

    #[test]
    fn an_edit_in_one_window_moves_point_in_the_others() {
        // The same buffer shown twice: typing in one window must not leave the
        // other pointing at an offset that no longer means what it did.
        let mut e = editor_with("alpha beta gamma");
        let other = e.split_window(Direction::Vertical).unwrap();
        e.windows.get_mut(other).unwrap().point = 11; // on `gamma`

        // Insert three characters at the start, in the selected window.
        e.with_current_buffer(|b| b.set_point(0));
        e.with_current_buffer(|b| b.insert_at_point(">> ").unwrap());

        assert_eq!(
            e.windows.get(other).unwrap().point,
            14,
            "the other window should still be on `gamma`"
        );
        let buffer = e.current_buffer();
        assert_eq!(buffer.slice(Range::new(14, 19)), "gamma");
    }

    #[test]
    fn a_deletion_never_leaves_another_window_past_the_end() {
        // This is what a random walk over the keymap found: four windows on
        // one buffer, an edit in one, and the rest left past the end.
        let mut e = editor_with("0123456789");
        let other = e.split_window(Direction::Vertical).unwrap();
        e.windows.get_mut(other).unwrap().point = 10;

        e.with_current_buffer(|b| {
            b.set_point(0);
            b.delete(Range::new(0, 8)).unwrap();
        });
        let window = e.windows.get(other).unwrap();
        let length = e.current_buffer().len_chars();
        assert!(
            window.point <= length,
            "point {} is past {length}",
            window.point
        );
        assert_eq!(window.point, 2, "it followed the text that is left");
    }

    #[test]
    fn an_edit_inside_what_another_window_was_on_collapses_to_its_start() {
        let mut e = editor_with("alpha beta gamma");
        let other = e.split_window(Direction::Vertical).unwrap();
        e.windows.get_mut(other).unwrap().point = 8; // inside `beta`

        e.with_current_buffer(|b| {
            b.set_point(0);
            b.delete(Range::new(6, 10)).unwrap();
        });
        assert_eq!(
            e.windows.get(other).unwrap().point,
            6,
            "collapsed to where it began"
        );
    }

    #[test]
    fn replacing_a_buffer_pulls_its_windows_back_into_range() {
        // A window keeps its own point. Replacing the text underneath one with
        // something shorter used to leave that point past the end, and every
        // measurement made against it afterwards was wrong.
        let mut e = editor();
        let id = e.buffers.create_with_text("*Help*", &"x".repeat(200));
        e.switch_to_buffer(id).unwrap();
        e.with_current_buffer(|b| b.set_point(150));
        assert_eq!(e.windows.current().point, 150);

        e.replace_buffer_contents(id, "much shorter").unwrap();
        assert_eq!(
            e.windows.current().point,
            12,
            "the window should have been pulled back to the end of the new text"
        );
        assert!(e.windows.current().point <= e.current_buffer().len_chars());
    }

    #[test]
    fn replacing_a_buffer_pulls_back_every_window_showing_it() {
        let mut e = editor();
        let id = e.buffers.create_with_text("*Occur*", &"line\n".repeat(100));
        e.switch_to_buffer(id).unwrap();
        let other = e.split_window(Direction::Vertical).unwrap();
        for window in [e.windows.current_id(), other] {
            if let Some(window) = e.windows.get_mut(window) {
                window.point = 400;
                window.top_line = 80;
            }
        }

        e.replace_buffer_contents(id, "one line\n").unwrap();
        for window in e.windows.iter() {
            assert!(
                window.point <= 9,
                "a window kept a point of {}",
                window.point
            );
            assert!(
                window.top_line < 2,
                "a window stayed scrolled to line {}",
                window.top_line
            );
        }
    }

    #[test]
    fn replacing_a_buffer_no_window_shows_is_harmless() {
        let mut e = editor();
        let id = e.buffers.create_with_text("hidden", "original");
        e.replace_buffer_contents(id, "replaced").unwrap();
        assert_eq!(e.buffers.get(id).unwrap().text(), "replaced");
    }

    #[test]
    fn a_mode_keymap_is_in_effect_only_in_a_buffer_of_that_mode() {
        let mut e = editor();
        e.mode_keymaps.push(mode_map("rust", "rust-only-command"));

        let rust = e.buffers.visit_file("/project/main.rs", "fn main() {}");
        let text = e.buffers.visit_file("/project/notes.txt", "plain");
        let sequence = maxgus_keys::KeySequence::parse("C-t").unwrap();

        e.switch_to_buffer(rust).unwrap();
        assert_eq!(
            e.keymaps.lookup(&sequence).command(),
            Some("rust-only-command"),
            "a rust-mode binding should apply in a Rust buffer"
        );

        e.switch_to_buffer(text).unwrap();
        assert_eq!(
            e.keymaps.lookup(&sequence).command(),
            Some("transpose-chars"),
            "and give way to the global binding elsewhere"
        );
    }

    #[test]
    fn a_language_with_no_configured_map_uses_the_global_one() {
        let mut e = editor();
        e.mode_keymaps
            .push(mode_map("python", "python-only-command"));
        let rust = e.buffers.visit_file("/project/main.rs", "fn main() {}");
        e.switch_to_buffer(rust).unwrap();
        assert!(
            e.keymaps.major.is_none(),
            "no map should have been installed"
        );
    }

    #[test]
    fn the_minibuffer_still_wins_over_a_mode_map() {
        let mut e = editor();
        e.mode_keymaps.push(mode_map("rust", "rust-only-command"));
        let rust = e.buffers.visit_file("/project/main.rs", "fn main() {}");
        e.switch_to_buffer(rust).unwrap();

        e.prompt(crate::MinibufferKind::Command, "M-x ");
        let sequence = maxgus_keys::KeySequence::parse("C-a").unwrap();
        assert_eq!(
            e.keymaps.lookup(&sequence).command(),
            Some("minibuffer-beginning-of-line"),
            "a prompt takes the keyboard from the mode map too"
        );
    }

    #[test]
    fn selecting_a_window_showing_another_language_swaps_the_map() {
        let mut e = editor();
        e.mode_keymaps.push(mode_map("rust", "rust-only-command"));
        let rust = e.buffers.visit_file("/project/main.rs", "fn main() {}");
        let text = e.buffers.visit_file("/project/notes.txt", "plain");

        e.switch_to_buffer(rust).unwrap();
        let other = e.split_window(Direction::Vertical).unwrap();
        e.select_window(other);
        e.switch_to_buffer(text).unwrap();

        let sequence = maxgus_keys::KeySequence::parse("C-t").unwrap();
        assert_eq!(
            e.keymaps.lookup(&sequence).command(),
            Some("transpose-chars")
        );

        e.other_window(1);
        assert_eq!(
            e.keymaps.lookup(&sequence).command(),
            Some("rust-only-command"),
            "going back to the Rust window restores its map"
        );
    }

    #[test]
    fn a_buffer_takes_the_configured_tab_width() {
        let settings = Settings {
            tab_width: 8,
            indent_with_tabs: true,
            ..Default::default()
        };
        let mut e = Editor::new(
            settings,
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        // The buffer that already existed at startup.
        assert_eq!(e.current_buffer().tab_width(), 8);
        assert!(e.current_buffer().indent_with_tabs());

        // And one opened afterwards.
        let id = e.buffers.visit_file("/project/main.rs", "\tindented\n");
        e.switch_to_buffer(id).unwrap();
        assert_eq!(e.current_buffer().tab_width(), 8);
    }

    #[test]
    fn a_tab_is_drawn_at_the_width_the_indent_commands_use() {
        // These disagreeing is what makes indentation look wrong: a tab
        // inserted as one width but displayed as another.
        let settings = Settings {
            tab_width: 8,
            ..Default::default()
        };
        let mut e = Editor::new(
            settings.clone(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = e.buffers.create_with_text("test", "\tx");
        e.switch_to_buffer(id).unwrap();
        assert_eq!(
            e.current_buffer().display_column(1),
            settings.tab_width,
            "the drawn width and the configured width must agree"
        );
    }

    #[test]
    fn changing_the_settings_reaches_every_buffer() {
        let mut e = editor();
        e.buffers.create("one");
        e.buffers.create("two");
        e.settings.tab_width = 2;
        e.apply_settings_everywhere();
        assert!(
            e.buffers.iter().all(|b| b.tab_width() == 2),
            "a buffer kept the old width"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn highlighting_is_asked_for_only_around_what_the_window_shows() {
        let (mut e, id) = editor_with_long_file();
        e.request_highlighting(id);
        let tasks = e.tasks.drain();
        let Some(Task::Reparse { range, text, .. }) = tasks
            .into_iter()
            .find(|t| matches!(t, Task::Reparse { .. }))
        else {
            panic!("no reparse was queued");
        };
        assert_eq!(range.start, 0, "the window is at the top of the file");
        assert!(
            range.end < text.len(),
            "the whole file was asked for: {range:?}"
        );
        assert!(range.end > 0);
    }

    #[cfg(feature = "full")]
    #[test]
    fn scrolling_past_the_highlighted_region_asks_for_more() {
        let (mut e, id) = editor_with_long_file();
        let revision = e.current_buffer().revision();
        let covered = e.highlight_request_range(id);
        e.highlights
            .insert(id, (revision, covered.clone(), Vec::new()));
        assert!(!e.highlights_are_stale(id), "what is on screen is covered");

        // Scroll far enough that the window itself leaves the region.
        e.windows.current_mut().top_line = 1_500;
        assert!(
            e.highlights_are_stale(id),
            "the window moved past {covered:?} but no new highlighting was asked for"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn scrolling_within_the_margin_does_not_ask_again() {
        let (mut e, id) = editor_with_long_file();
        let revision = e.current_buffer().revision();
        e.highlights
            .insert(id, (revision, e.highlight_request_range(id), Vec::new()));

        // A page down stays inside the margin already highlighted.
        let height = e.windows.current().text_height();
        e.windows.current_mut().top_line = height;
        assert!(
            !e.highlights_are_stale(id),
            "an ordinary scroll should not re-query"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn an_edit_makes_the_highlighting_stale_wherever_the_window_is() {
        let (mut e, id) = editor_with_long_file();
        e.highlights.insert(
            id,
            (
                e.current_buffer().revision(),
                e.highlight_request_range(id),
                Vec::new(),
            ),
        );
        assert!(!e.highlights_are_stale(id));
        e.with_current_buffer(|b| b.insert_at_point("x").unwrap());
        assert!(e.highlights_are_stale(id), "the buffer moved on");
    }

    #[test]
    fn a_short_file_is_highlighted_whole() {
        let mut e = editor();
        let id = e.buffers.visit_file("/project/short.rs", "fn main() {}\n");
        e.switch_to_buffer(id).unwrap();
        let range = e.highlight_request_range(id);
        assert_eq!(range.start, 0);
        assert_eq!(
            range.end,
            e.current_buffer().text().len(),
            "nothing is left out"
        );
    }

    #[test]
    fn queued_work_accumulates_until_it_is_drained() {
        let mut e = editor();
        e.spawn(Task::Tree(crate::TreeAction::Refresh));
        assert_eq!(e.tasks.len(), 1);
        assert_eq!(e.tasks.drain().len(), 1);
        assert!(e.tasks.is_empty());
    }

    #[test]
    fn messages_and_errors_reach_the_echo_area() {
        let mut e = editor();
        e.message("Wrote file");
        assert_eq!(e.minibuffer.display(), "Wrote file");
        assert!(!e.minibuffer.message_is_error());
        e.error("No such file");
        assert!(e.minibuffer.message_is_error());
    }

    #[test]
    fn the_kill_ring_honours_the_configured_maximum() {
        let settings = Settings {
            kill_ring_max: 2,
            ..Default::default()
        };
        let mut e = Editor::new(
            settings,
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        for text in ["a", "b", "c"] {
            e.kill(text, false);
        }
        assert_eq!(e.kill_ring.len(), 2);
    }
}
