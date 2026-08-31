//! Asynchronous work.
//!
//! Commands are synchronous functions over editor state, which is what makes
//! them testable without a runtime. Anything that has to touch the filesystem,
//! a language server or a subprocess is expressed as a [`Task`]: the command
//! queues one, the event loop runs it on tokio, and the answer comes back as a
//! [`TaskResult`] that the editor applies. Nothing blocks redisplay.

#[cfg(feature = "full")]
use maxgus_lsp::{LspPosition, LspRange};
use maxgus_text::BufferId;
use std::path::PathBuf;

#[cfg(feature = "full")]
/// What to ask git to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitAction {
    /// Read everything the status view shows.
    Refresh,
    Stage(Vec<PathBuf>),
    Unstage(Vec<PathBuf>),
    StageAll,
    UnstageAll,
    /// Throws the working-tree change away. The one irreversible action here.
    Discard(Vec<PathBuf>),
    /// Deletes an untracked file, which `git checkout` will not do.
    DeleteUntracked(Vec<PathBuf>),
    /// Feeds a patch to `git apply`, which is how one hunk is staged,
    /// unstaged or discarded.
    ApplyPatch {
        patch: String,
        arguments: Vec<String>,
        describe: String,
    },
    /// The switches come from the menu, so `--signoff` and the rest need no
    /// variant of their own here.
    Commit {
        message: String,
        amend: bool,
        arguments: Vec<String>,
    },
    Push {
        arguments: Vec<String>,
    },
    Pull {
        arguments: Vec<String>,
    },
    Fetch {
        arguments: Vec<String>,
    },
    Checkout(String),
    CreateBranch(String),
    Merge(String),
    Stash {
        message: Option<String>,
        arguments: Vec<String>,
    },
    StashPop(String),
    StashApply(String),
    StashDrop(String),
    /// Runs any git command, for the keys that are a thin wrapper over one.
    Run {
        arguments: Vec<String>,
        describe: String,
    },
    /// Reads a log into its own buffer.
    Log {
        arguments: Vec<String>,
        title: String,
    },
    /// Reads a diff into its own buffer.
    Diff {
        arguments: Vec<String>,
        title: String,
    },
    /// Reads one commit: its header, its message and its diff.
    Show {
        revision: String,
    },
}

#[cfg(feature = "full")]
/// Everything the status view needs, read in one pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitSnapshot {
    /// The top of the working tree, as git resolves it. Asked for rather than
    /// walked up to by hand: git knows about worktrees, submodules and a
    /// `.git` that is a file, and a walk looking for a directory does not.
    pub root: PathBuf,
    pub status: maxgus_git::Status,
    pub unstaged: Vec<maxgus_git::FileDiff>,
    pub staged: Vec<maxgus_git::FileDiff>,
    pub stashes: Vec<maxgus_git::Stash>,
    pub unpushed: Vec<maxgus_git::Commit>,
    pub unpulled: Vec<maxgus_git::Commit>,
    pub recent: Vec<maxgus_git::Commit>,
    pub head_subject: String,
    /// Short names for the checkout and merge prompts.
    pub branches: Vec<String>,
    /// Every reference, with what kind it is, for the references view.
    pub references: Vec<maxgus_git::Reference>,
}

#[cfg(feature = "full")]
/// Which terminal a task or an answer is about.
///
/// Its own type rather than an index: tabs are opened and closed in any order,
/// and an answer arriving for a terminal that has since been closed must be
/// recognisable as such rather than landing on whoever now holds that
/// position in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerminalId(pub u64);

#[cfg(feature = "full")]
impl std::fmt::Display for TerminalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Something to do to files, as dired asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    Delete(Vec<PathBuf>),
    Copy {
        from: Vec<PathBuf>,
        to: PathBuf,
    },
    Rename {
        from: Vec<PathBuf>,
        to: PathBuf,
    },
    CreateDirectory(PathBuf),
    /// The directory to list again once it is done.
    Chmod {
        paths: Vec<PathBuf>,
        mode: u32,
    },
}

impl FileAction {
    /// What to say about it afterwards.
    pub fn describe(&self) -> String {
        match self {
            FileAction::Delete(paths) => format!("Deleted {} item(s)", paths.len()),
            FileAction::Copy { from, .. } => format!("Copied {} item(s)", from.len()),
            FileAction::Rename { from, .. } => format!("Renamed {} item(s)", from.len()),
            FileAction::CreateDirectory(path) => format!("Created {}", path.display()),
            FileAction::Chmod { paths, .. } => format!("Changed {} item(s)", paths.len()),
        }
    }
}

/// What a file's `.editorconfig` asks for.
///
/// Only what the editor can honour: a property it has no setting for is left
/// out rather than carried around unused. Each is optional because
/// `.editorconfig` files say only what they mean to change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorConfig {
    pub tab_width: Option<usize>,
    pub indent_with_tabs: Option<bool>,
    pub crlf: Option<bool>,
    pub trim_trailing_whitespace: Option<bool>,
    pub final_newline: Option<bool>,
    pub fill_column: Option<usize>,
}

impl EditorConfig {
    /// True when it asks for nothing, which is the usual case.
    pub fn is_empty(&self) -> bool {
        *self == EditorConfig::default()
    }
}

/// What a write insists is true of the file before it goes ahead.
///
/// Checking any of it means a `stat`, which is the executor's work rather than
/// a command's — so the intention travels with the task and the answer comes
/// back as [`TaskResult::WriteRefused`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGuard {
    /// The file is still as this buffer last read or wrote it. `save-buffer`,
    /// so that somebody else's change is not written over.
    Unchanged(Option<std::time::SystemTime>),
    /// There is no file there yet. `write-file`, so that a file the user never
    /// opened is not destroyed by being named.
    Absent,
    /// Write whatever is there. What `save-buffer-anyway` asks for.
    Regardless,
}

/// A unit of asynchronous work.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    /// Ask which grammars this editor can reach, and why any it was told
    /// about would not load. Only the executor knows: it is what holds them.
    #[cfg(feature = "full")]
    DescribeGrammars,
    /// Read a file, creating or reverting a buffer with its contents.
    ReadFile {
        path: PathBuf,
        reverting: Option<BufferId>,
        other_window: bool,
    },
    /// Write `contents` to `path`, then mark `buffer` saved.
    WriteFile {
        path: PathBuf,
        contents: String,
        buffer: BufferId,
        backup: bool,
        guard: WriteGuard,
    },
    /// List a directory, for `find-file` completion.
    ListDirectory { path: PathBuf },
    /// Act on the file tree. The tree itself lives in the event loop, where
    /// its directory reads can be awaited.
    Tree(TreeAction),
    /// Write `set theme="…"` into the configuration file, leaving the rest of
    /// it alone.
    PersistTheme { path: PathBuf, theme: String },
    /// List a directory with the detail dired shows.
    Dired { path: PathBuf },
    /// List a directory for the file browser. Its own task rather than
    /// dired's, so the two cannot answer each other's questions.
    Browse { path: PathBuf },
    /// Act on files, from dired.
    DiredAct { action: FileAction },
    /// Read the script file.
    #[cfg(feature = "full")]
    ReadScript { path: PathBuf },
    /// Write the session for a project.
    SaveSession { path: PathBuf, contents: String },
    /// Write the saved workspaces out.
    SaveWorkspaces { path: PathBuf, contents: String },
    /// Read them back at startup.
    ReadWorkspaces { path: PathBuf },
    /// Read one back.
    ReadSession { path: PathBuf },
    /// Ask git which branch the project is on, for the mode line.
    #[cfg(feature = "full")]
    GitBranch { root: PathBuf },
    /// Start a language server for `language`.
    #[cfg(feature = "full")]
    StartLanguageServer { language: String },
    /// Stop the server for `language`.
    #[cfg(feature = "full")]
    StopLanguageServer { language: String },
    /// Tell the server a document opened.
    #[cfg(feature = "full")]
    LspDidOpen {
        language: String,
        uri: String,
        version: i64,
        text: String,
    },
    /// Tell the server a document changed.
    #[cfg(feature = "full")]
    LspDidChange {
        language: String,
        uri: String,
        version: i64,
        text: String,
    },
    /// Tell the server a document was saved.
    #[cfg(feature = "full")]
    LspDidSave { language: String, uri: String },
    /// Tell the server a document closed.
    #[cfg(feature = "full")]
    LspDidClose { language: String, uri: String },
    /// Ask the server a question. The answer comes back as
    /// [`TaskResult::LspResponse`] tagged with the same [`LspQuery`].
    #[cfg(feature = "full")]
    LspRequest {
        language: String,
        uri: String,
        query: LspQuery,
        /// True when a command said "Language server: ..." in the echo area
        /// before queuing this.
        ///
        /// It decides whether a request nobody can answer is reported. One
        /// that was announced has to be: the message it put on screen would
        /// otherwise stay there for ever. One that was not — the symbols
        /// panel filling itself, the doc box after a pause — must not be,
        /// because those are issued while the server is still starting and
        /// would complain about a race that resolves itself a moment later.
        announced: bool,
    },
    /// The answer to a request the *server* made of us.
    ///
    /// The protocol requires every server request to be answered; a server
    /// that asked to apply an edit blocks until it hears back.
    #[cfg(feature = "full")]
    LspRespond {
        language: String,
        id: maxgus_lsp::RequestId,
        applied: bool,
    },
    /// Run a shell command and collect its output.
    Shell {
        command: String,
        directory: PathBuf,
        insert_at: Option<(BufferId, usize)>,
    },
    /// Starts a shell on a pseudo-terminal of the given size.
    #[cfg(feature = "full")]
    TerminalOpen {
        terminal: TerminalId,
        shell: Option<String>,
        directory: PathBuf,
        rows: u16,
        columns: u16,
    },
    /// Sends keystrokes, a paste, or a reply the program asked for.
    #[cfg(feature = "full")]
    TerminalInput {
        terminal: TerminalId,
        bytes: Vec<u8>,
    },
    /// Tells the program the window changed size. Without this, `vim` and
    /// friends go on drawing to the shape they started with.
    #[cfg(feature = "full")]
    TerminalResize {
        terminal: TerminalId,
        rows: u16,
        columns: u16,
    },
    /// Ends the shell and closes the pseudo-terminal.
    #[cfg(feature = "full")]
    TerminalClose { terminal: TerminalId },
    /// Something to do with git, in the repository the editor is in.
    #[cfg(feature = "full")]
    Git { root: PathBuf, action: GitAction },
    /// Search the project for a pattern.
    #[cfg(feature = "full")]
    Grep {
        root: PathBuf,
        search: maxgus_grep::Search,
    },
    /// Write edited result lines back to the files they came from.
    #[cfg(feature = "full")]
    ApplyGrep {
        replacements: Vec<maxgus_grep::Replacement>,
    },
    /// Re-parse a buffer and highlight `range`.
    ///
    /// Only the region the user can see — plus a margin for scrolling — is
    /// queried. Highlighting a whole large file costs far more than parsing
    /// it, and almost all of the answer would never be drawn.
    #[cfg(feature = "full")]
    Reparse {
        buffer: BufferId,
        language: String,
        text: String,
        revision: u64,
        range: std::ops::Range<usize>,
    },
    /// Release what is being kept for a buffer that has been killed.
    ForgetBuffer { buffer: BufferId },
}

/// Something to do to the file tree.
///
/// Navigation is not here: moving the cursor is pure and happens immediately,
/// so the tree stays responsive while a directory read is in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    /// Read the tree from scratch, preserving what is expanded.
    Refresh,
    /// Expand a collapsed directory, or collapse an expanded one.
    Toggle(PathBuf),
    Expand(PathBuf),
    Collapse(PathBuf),
    /// Expand everything beneath a directory.
    ExpandRecursively(PathBuf),
    /// Expand whatever it takes to show `path`, and select it.
    Reveal(PathBuf),
    /// Draw the tree from `path` down, instead of from wherever it was.
    ///
    /// The tree's root only. The project root — what the language server is
    /// told about, what a project search walks — does not move, because
    /// looking at a subdirectory is not the same as working in a different
    /// project.
    SetRoot {
        from: PathBuf,
        to: PathBuf,
    },
    /// Show another directory in the tree, below the ones already there.
    ///
    /// treemacs' `treemacs-add-project-to-workspace`. The project root does
    /// not move: the first directory stays the one a language server is
    /// told about, because looking at a second one is not changing project.
    AddRoot(PathBuf),
    /// Stop showing one of them. The last one stays.
    RemoveRoot(PathBuf),
    /// Show exactly these and no others, which is what opening a workspace
    /// does. Directories that cannot be read are dropped and reported.
    SetRoots(Vec<PathBuf>),
    /// Show or hide dotfiles.
    ToggleHidden,
    /// Show directories before files, or sort strictly by name.
    ToggleDirectoriesFirst,
    /// Show or hide the git status column.
    ToggleGitStatus,
    CreateFile {
        parent: PathBuf,
        name: String,
    },
    CreateDirectory {
        parent: PathBuf,
        name: String,
    },
    Delete(PathBuf),
    Rename {
        path: PathBuf,
        name: String,
    },
    Move {
        path: PathBuf,
        destination: PathBuf,
    },
}

#[cfg(feature = "full")]
/// A language-server question, kept as a small enum so the answer can be
/// routed back to whatever asked it.
#[derive(Debug, Clone, PartialEq)]
pub enum LspQuery {
    Definition(LspPosition),
    References(LspPosition),
    Hover(LspPosition),
    Completion {
        position: LspPosition,
        /// True when a key asked for it rather than a pause in typing.
        ///
        /// A pause must never insert anything on its own — text nobody
        /// typed is the worst thing an editor can do — so the automatic
        /// path always offers a list, while `C-M-i` on a single candidate
        /// still completes it outright, the way Emacs does.
        manual: bool,
    },
    SignatureHelp(LspPosition),
    Rename {
        position: LspPosition,
        new_name: String,
    },
    Format {
        tab_size: usize,
        insert_spaces: bool,
    },
    /// The range, and the diagnostics inside it: a server offers a quick fix
    /// for a diagnostic it is shown, not for a bare position.
    CodeAction {
        range: LspRange,
        diagnostics: Vec<maxgus_lsp::Diagnostic>,
    },
    /// `for_panel` is who asked. Two requests can be in flight at once —
    /// the panel refreshing itself and a person running the command — and
    /// the answers are told apart by what was asked, not by editor state a
    /// first answer would already have cleared.
    DocumentSymbols {
        for_panel: bool,
    },
    WorkspaceSymbols(String),
}

#[cfg(feature = "full")]
impl LspQuery {
    /// The name shown while the request is in flight.
    pub fn description(&self) -> &'static str {
        match self {
            LspQuery::Definition(_) => "finding definition",
            LspQuery::References(_) => "finding references",
            LspQuery::Hover(_) => "describing",
            LspQuery::Completion { .. } => "completing",
            LspQuery::SignatureHelp(_) => "signature help",
            LspQuery::Rename { .. } => "renaming",
            LspQuery::Format { .. } => "formatting",
            LspQuery::CodeAction { .. } => "code actions",
            LspQuery::DocumentSymbols { .. } => "document symbols",
            LspQuery::WorkspaceSymbols(_) => "workspace symbols",
        }
    }

    /// True when the result should move point, so the editor knows to push the
    /// mark before applying it.
    pub fn jumps(&self) -> bool {
        matches!(self, LspQuery::Definition(_) | LspQuery::References(_))
    }
}

/// The outcome of a [`Task`].
#[derive(Debug, Clone, PartialEq)]
pub enum TaskResult {
    /// The answer to [`Task::DescribeGrammars`], ready to be shown.
    #[cfg(feature = "full")]
    Grammars {
        report: String,
    },
    FileRead {
        path: PathBuf,
        contents: String,
        read_only: bool,
        /// True when the file held bytes that are not valid UTF-8 and had to
        /// be replaced to be shown. What is in the buffer no longer matches
        /// what is on disk, so saving it would write the replacements over
        /// the original bytes.
        lossy: bool,
        /// The file's modification time as read, to be compared against the
        /// file before this buffer is written back over it.
        disk_time: Option<std::time::SystemTime>,
        reverting: Option<BufferId>,
        other_window: bool,
        /// What the file's `.editorconfig` asks for, if anything does.
        editor_config: EditorConfig,
    },
    FileWritten {
        path: PathBuf,
        buffer: BufferId,
        bytes: usize,
        disk_time: Option<std::time::SystemTime>,
    },
    /// Nothing was written, because the file was not what the write insisted
    /// it would be. [`WriteGuard`] says which expectation failed.
    WriteRefused {
        path: PathBuf,
        buffer: BufferId,
        because: WriteGuard,
    },
    DirectoryListed {
        path: PathBuf,
        entries: Vec<String>,
    },
    /// The theme was written into the configuration file.
    ThemePersisted {
        path: PathBuf,
        theme: String,
    },
    /// The branch the project is on, or none when it is not a repository.
    #[cfg(feature = "full")]
    GitBranch {
        branch: Option<String>,
    },
    /// A new snapshot of the file tree.
    TreeUpdated {
        nodes: Vec<maxgus_tree::VisibleNode>,
        select: Option<PathBuf>,
        show_hidden: bool,
    },
    /// The encoding is the one the server negotiated at `initialize`, and it
    /// is carried here because nothing else ever learns it: addressing a
    /// UTF-8 server in UTF-16 puts every position in a line with non-ASCII
    /// text at the wrong column.
    #[cfg(feature = "full")]
    LanguageServerStarted {
        language: String,
        encoding: maxgus_lsp::PositionEncoding,
    },
    #[cfg(feature = "full")]
    LanguageServerStopped {
        language: String,
    },
    #[cfg(feature = "full")]
    LspResponse {
        language: String,
        uri: String,
        query: LspQuery,
        result: serde_json::Value,
    },
    #[cfg(feature = "full")]
    Diagnostics {
        uri: String,
        diagnostics: Vec<maxgus_lsp::Diagnostic>,
    },
    /// `workspace/applyEdit`: the server wants to change the text itself.
    ///
    /// Carries the request id, because the answer has to say whether the edit
    /// went in and the server is waiting for it.
    #[cfg(feature = "full")]
    LspApplyEdit {
        language: String,
        id: maxgus_lsp::RequestId,
        edit: serde_json::Value,
    },
    ShellOutput {
        command: String,
        output: String,
        status: i32,
        insert_at: Option<(BufferId, usize)>,
    },
    /// Everything the status view shows, read in one go.
    ///
    /// One result rather than eight, because a status assembled from answers
    /// arriving separately shows a diff that disagrees with the status it is
    /// listed under — which is exactly the moment a user stages the wrong
    /// thing.
    #[cfg(feature = "full")]
    GitRefreshed(Box<GitSnapshot>),
    /// A git command finished. The output is shown, since git says useful
    /// things on the way past, and the whole command line is kept so the
    /// process buffer can show what was actually run.
    #[cfg(feature = "full")]
    GitDone {
        action: String,
        command: String,
        output: String,
    },
    /// A log, for its own buffer.
    #[cfg(feature = "full")]
    GitLog {
        title: String,
        commits: Vec<maxgus_git::Commit>,
    },
    /// A diff or one commit, for its own buffer.
    #[cfg(feature = "full")]
    GitDiff {
        title: String,
        /// Lines above the diff: a commit's author, date and message.
        preamble: Vec<String>,
        files: Vec<maxgus_git::FileDiff>,
    },
    /// Bytes a terminal's program wrote.
    #[cfg(feature = "full")]
    TerminalOutput {
        terminal: TerminalId,
        bytes: Vec<u8>,
    },
    /// The program ended. The tab says so rather than vanishing, so that a
    /// command which failed on the way out can still be read.
    #[cfg(feature = "full")]
    TerminalExited {
        terminal: TerminalId,
        status: i32,
    },
    #[cfg(feature = "full")]
    Reparsed {
        buffer: BufferId,
        revision: u64,
        /// The byte range the highlights cover.
        range: std::ops::Range<usize>,
        highlights: Vec<maxgus_syntax::Highlight>,
    },
    /// Something went wrong. `context` names what was being attempted.
    Failed {
        context: String,
        message: String,
    },
    /// What a search found.
    #[cfg(feature = "full")]
    GrepFinished {
        pattern: String,
        found: maxgus_grep::Found,
    },
    /// What writing the edited results did.
    #[cfg(feature = "full")]
    GrepApplied {
        applied: maxgus_grep::Applied,
        /// The files that were written, so their buffers can be re-read.
        paths: Vec<PathBuf>,
    },
    /// A session, as it was read. Absent when there was none.
    SessionRead {
        session: crate::session::Session,
    },
    /// A session was written.
    SessionSaved {
        path: PathBuf,
    },
    /// Something worth saying and nothing else: a note for the echo area
    /// from work that otherwise has no result to report.
    Said(String),
    /// The saved workspaces, as they were on disk.
    WorkspacesRead {
        workspaces: crate::workspace::Workspaces,
    },
    /// A directory, listed for the file browser.
    Browsed {
        path: PathBuf,
        entries: Vec<crate::dired::Entry>,
    },
    /// A directory, listed with the detail dired shows.
    DiredListed {
        path: PathBuf,
        entries: Vec<crate::dired::Entry>,
    },
    /// Something dired asked for is done, and the directory should be listed
    /// again to show it.
    DiredDone {
        said: String,
        relist: PathBuf,
    },
    /// The script file, as it was read.
    #[cfg(feature = "full")]
    ScriptRead {
        source: String,
        path: PathBuf,
    },
}

impl TaskResult {
    /// True when the result reports a failure.
    pub fn is_error(&self) -> bool {
        matches!(self, TaskResult::Failed { .. })
    }

    /// The message to show in the echo area, if any.
    pub fn message(&self) -> Option<String> {
        match self {
            TaskResult::Said(said) => Some(said.clone()),
            TaskResult::FileRead { path, .. } => Some(format!("Read {}", path.display())),
            TaskResult::FileWritten { path, bytes, .. } => {
                Some(format!("Wrote {} ({bytes} bytes)", path.display()))
            }
            #[cfg(feature = "full")]
            TaskResult::LanguageServerStarted { language, .. } => {
                Some(format!("Language server for {language} started"))
            }
            #[cfg(feature = "full")]
            TaskResult::LanguageServerStopped { language } => {
                Some(format!("Language server for {language} stopped"))
            }
            #[cfg(feature = "full")]
            TaskResult::GitDone { action, output, .. } => Some(match output.trim() {
                "" => format!("{action} done"),
                said => format!("{action}: {}", said.lines().next().unwrap_or(said)),
            }),
            #[cfg(feature = "full")]
            TaskResult::TerminalExited { status, .. } if *status != 0 => {
                Some(format!("Terminal exited with status {status}"))
            }
            TaskResult::ShellOutput {
                status, command, ..
            } if *status != 0 => Some(format!("`{command}` exited with status {status}")),
            TaskResult::Failed { context, message } => Some(format!("{context}: {message}")),
            _ => None,
        }
    }
}

/// Tasks a command has queued, drained by the event loop each cycle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskQueue {
    tasks: Vec<Task>,
}

impl TaskQueue {
    pub fn new() -> TaskQueue {
        TaskQueue::default()
    }

    pub fn push(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Takes everything queued, leaving the queue empty.
    pub fn drain(&mut self) -> Vec<Task> {
        std::mem::take(&mut self.tasks)
    }

    /// Discards everything queued, as `keyboard-quit` does.
    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    /// Queued tasks, for inspection.
    pub fn peek(&self) -> &[Task] {
        &self.tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "full")]
    use serde_json::json;

    fn path() -> PathBuf {
        PathBuf::from("/tmp/a.rs")
    }

    #[test]
    fn a_queue_starts_empty_and_drains_in_order() {
        let mut q = TaskQueue::new();
        assert!(q.is_empty());
        q.push(Task::Tree(TreeAction::Refresh));
        q.push(Task::ListDirectory { path: path() });
        assert_eq!(q.len(), 2);

        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], Task::Tree(TreeAction::Refresh));
        assert!(q.is_empty(), "draining empties the queue");
    }

    #[test]
    fn clearing_discards_queued_work() {
        let mut q = TaskQueue::new();
        q.push(Task::Tree(TreeAction::Refresh));
        q.clear();
        assert!(q.is_empty());
        assert!(q.peek().is_empty());
    }

    #[test]
    fn queued_tasks_can_be_inspected_without_draining() {
        let mut q = TaskQueue::new();
        q.push(Task::Tree(TreeAction::Refresh));
        assert_eq!(q.peek(), [Task::Tree(TreeAction::Refresh)]);
        assert_eq!(q.len(), 1, "peeking does not consume");
    }

    #[test]
    fn results_report_whether_they_failed() {
        let ok = TaskResult::TreeUpdated {
            nodes: Vec::new(),
            select: None,
            show_hidden: false,
        };
        assert!(!ok.is_error());
        let bad = TaskResult::Failed {
            context: "find-file".into(),
            message: "no such file".into(),
        };
        assert!(bad.is_error());
        assert_eq!(bad.message().unwrap(), "find-file: no such file");
    }

    #[test]
    fn successful_results_describe_themselves_for_the_echo_area() {
        let read = TaskResult::FileRead {
            path: path(),
            contents: String::new(),
            read_only: false,
            lossy: false,
            disk_time: None,
            reverting: None,
            other_window: false,
            editor_config: Default::default(),
        };
        assert_eq!(read.message().unwrap(), "Read /tmp/a.rs");

        let written = TaskResult::FileWritten {
            path: path(),
            buffer: BufferId(1),
            bytes: 120,
            disk_time: None,
        };
        assert_eq!(written.message().unwrap(), "Wrote /tmp/a.rs (120 bytes)");
    }

    #[cfg(feature = "full")]
    #[test]
    fn quiet_results_say_nothing() {
        assert_eq!(
            TaskResult::TreeUpdated {
                nodes: Vec::new(),
                select: None,
                show_hidden: false
            }
            .message(),
            None
        );
        assert_eq!(
            TaskResult::Reparsed {
                buffer: BufferId(1),
                revision: 3,
                range: 0..0,
                highlights: Vec::new(),
            }
            .message(),
            None
        );
        let response = TaskResult::LspResponse {
            language: "rust".into(),
            uri: "file:///a.rs".into(),
            query: LspQuery::DocumentSymbols { for_panel: false },
            result: json!([]),
        };
        assert_eq!(response.message(), None);
    }

    #[test]
    fn a_failed_shell_command_reports_its_status() {
        let out = TaskResult::ShellOutput {
            command: "false".into(),
            output: String::new(),
            status: 1,
            insert_at: None,
        };
        assert_eq!(out.message().unwrap(), "`false` exited with status 1");

        let ok = TaskResult::ShellOutput {
            command: "true".into(),
            output: String::new(),
            status: 0,
            insert_at: None,
        };
        assert_eq!(ok.message(), None, "a successful command says nothing");
    }

    #[cfg(feature = "full")]
    #[test]
    fn language_server_lifecycle_results_are_announced() {
        assert_eq!(
            TaskResult::LanguageServerStarted {
                language: "rust".into(),
                encoding: maxgus_lsp::PositionEncoding::Utf16,
            }
            .message()
            .unwrap(),
            "Language server for rust started"
        );
        assert_eq!(
            TaskResult::LanguageServerStopped {
                language: "rust".into()
            }
            .message()
            .unwrap(),
            "Language server for rust stopped"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn every_query_kind_describes_itself() {
        let queries = [
            LspQuery::Definition(LspPosition::ZERO),
            LspQuery::References(LspPosition::ZERO),
            LspQuery::Hover(LspPosition::ZERO),
            LspQuery::Completion {
                position: LspPosition::ZERO,
                manual: true,
            },
            LspQuery::SignatureHelp(LspPosition::ZERO),
            LspQuery::Rename {
                position: LspPosition::ZERO,
                new_name: "x".into(),
            },
            LspQuery::Format {
                tab_size: 4,
                insert_spaces: true,
            },
            LspQuery::CodeAction {
                range: LspRange::empty(LspPosition::ZERO),
                diagnostics: Vec::new(),
            },
            LspQuery::DocumentSymbols { for_panel: false },
            LspQuery::WorkspaceSymbols("q".into()),
        ];
        let mut descriptions: Vec<&str> = queries.iter().map(LspQuery::description).collect();
        let before = descriptions.len();
        descriptions.sort_unstable();
        descriptions.dedup();
        assert_eq!(
            descriptions.len(),
            before,
            "descriptions must be distinguishable"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn only_the_navigating_queries_push_the_mark() {
        assert!(LspQuery::Definition(LspPosition::ZERO).jumps());
        assert!(LspQuery::References(LspPosition::ZERO).jumps());
        assert!(!LspQuery::Hover(LspPosition::ZERO).jumps());
        assert!(
            !LspQuery::Format {
                tab_size: 4,
                insert_spaces: true
            }
            .jumps()
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn a_response_carries_enough_context_to_be_routed_back() {
        let result = TaskResult::LspResponse {
            language: "rust".into(),
            uri: "file:///a.rs".into(),
            query: LspQuery::Hover(LspPosition::new(3, 4)),
            result: json!({"contents": "docs"}),
        };
        let TaskResult::LspResponse { query, uri, .. } = result else {
            panic!()
        };
        assert_eq!(uri, "file:///a.rs");
        assert_eq!(query, LspQuery::Hover(LspPosition::new(3, 4)));
    }
}
