//! Asynchronous work.
//!
//! Commands are synchronous functions over editor state, which is what makes
//! them testable without a runtime. Anything that has to touch the filesystem,
//! a language server or a subprocess is expressed as a [`Task`]: the command
//! queues one, the event loop runs it on tokio, and the answer comes back as a
//! [`TaskResult`] that the editor applies. Nothing blocks redisplay.

use maxgus_lsp::{LspPosition, LspRange};
use maxgus_text::BufferId;
use std::path::PathBuf;

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
    /// Read a file, creating or reverting a buffer with its contents.
    ReadFile { path: PathBuf, reverting: Option<BufferId>, other_window: bool },
    /// Write `contents` to `path`, then mark `buffer` saved.
    WriteFile { path: PathBuf, contents: String, buffer: BufferId, backup: bool, guard: WriteGuard },
    /// List a directory, for `find-file` completion.
    ListDirectory { path: PathBuf },
    /// Act on the file tree. The tree itself lives in the event loop, where
    /// its directory reads can be awaited.
    Tree(TreeAction),
    /// Write `set theme="…"` into the configuration file, leaving the rest of
    /// it alone.
    PersistTheme { path: PathBuf, theme: String },
    /// Ask git which branch the project is on, for the mode line.
    GitBranch { root: PathBuf },
    /// Start a language server for `language`.
    StartLanguageServer { language: String },
    /// Stop the server for `language`.
    StopLanguageServer { language: String },
    /// Tell the server a document opened.
    LspDidOpen { language: String, uri: String, version: i64, text: String },
    /// Tell the server a document changed.
    LspDidChange { language: String, uri: String, version: i64, text: String },
    /// Tell the server a document was saved.
    LspDidSave { language: String, uri: String },
    /// Tell the server a document closed.
    LspDidClose { language: String, uri: String },
    /// Ask the server a question. The answer comes back as
    /// [`TaskResult::LspResponse`] tagged with the same [`LspQuery`].
    LspRequest { language: String, uri: String, query: LspQuery },
    /// The answer to a request the *server* made of us.
    ///
    /// The protocol requires every server request to be answered; a server
    /// that asked to apply an edit blocks until it hears back.
    LspRespond { language: String, id: maxgus_lsp::RequestId, applied: bool },
    /// Run a shell command and collect its output.
    Shell { command: String, directory: PathBuf, insert_at: Option<(BufferId, usize)> },
    /// Re-parse a buffer and highlight `range`.
    ///
    /// Only the region the user can see — plus a margin for scrolling — is
    /// queried. Highlighting a whole large file costs far more than parsing
    /// it, and almost all of the answer would never be drawn.
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
    /// Show or hide dotfiles.
    ToggleHidden,
    /// Show directories before files, or sort strictly by name.
    ToggleDirectoriesFirst,
    /// Show or hide the git status column.
    ToggleGitStatus,
    CreateFile { parent: PathBuf, name: String },
    CreateDirectory { parent: PathBuf, name: String },
    Delete(PathBuf),
    Rename { path: PathBuf, name: String },
    Move { path: PathBuf, destination: PathBuf },
}

/// A language-server question, kept as a small enum so the answer can be
/// routed back to whatever asked it.
#[derive(Debug, Clone, PartialEq)]
pub enum LspQuery {
    Definition(LspPosition),
    References(LspPosition),
    Hover(LspPosition),
    Completion(LspPosition),
    SignatureHelp(LspPosition),
    Rename { position: LspPosition, new_name: String },
    Format { tab_size: usize, insert_spaces: bool },
    /// The range, and the diagnostics inside it: a server offers a quick fix
    /// for a diagnostic it is shown, not for a bare position.
    CodeAction { range: LspRange, diagnostics: Vec<maxgus_lsp::Diagnostic> },
    DocumentSymbols,
    WorkspaceSymbols(String),
}

impl LspQuery {
    /// The name shown while the request is in flight.
    pub fn description(&self) -> &'static str {
        match self {
            LspQuery::Definition(_) => "finding definition",
            LspQuery::References(_) => "finding references",
            LspQuery::Hover(_) => "describing",
            LspQuery::Completion(_) => "completing",
            LspQuery::SignatureHelp(_) => "signature help",
            LspQuery::Rename { .. } => "renaming",
            LspQuery::Format { .. } => "formatting",
            LspQuery::CodeAction { .. } => "code actions",
            LspQuery::DocumentSymbols => "document symbols",
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
    },
    FileWritten {
        path: PathBuf,
        buffer: BufferId,
        bytes: usize,
        disk_time: Option<std::time::SystemTime>,
    },
    /// Nothing was written, because the file was not what the write insisted
    /// it would be. [`WriteGuard`] says which expectation failed.
    WriteRefused { path: PathBuf, buffer: BufferId, because: WriteGuard },
    DirectoryListed { path: PathBuf, entries: Vec<String> },
    /// The theme was written into the configuration file.
    ThemePersisted { path: PathBuf, theme: String },
    /// The branch the project is on, or none when it is not a repository.
    GitBranch { branch: Option<String> },
    /// A new snapshot of the file tree.
    TreeUpdated { nodes: Vec<maxgus_tree::VisibleNode>, select: Option<PathBuf>, show_hidden: bool },
    LanguageServerStarted { language: String },
    LanguageServerStopped { language: String },
    LspResponse { language: String, uri: String, query: LspQuery, result: serde_json::Value },
    Diagnostics { uri: String, diagnostics: Vec<maxgus_lsp::Diagnostic> },
    /// `workspace/applyEdit`: the server wants to change the text itself.
    ///
    /// Carries the request id, because the answer has to say whether the edit
    /// went in and the server is waiting for it.
    LspApplyEdit { language: String, id: maxgus_lsp::RequestId, edit: serde_json::Value },
    ShellOutput { command: String, output: String, status: i32, insert_at: Option<(BufferId, usize)> },
    Reparsed {
        buffer: BufferId,
        revision: u64,
        /// The byte range the highlights cover.
        range: std::ops::Range<usize>,
        highlights: Vec<maxgus_syntax::Highlight>,
    },
    /// Something went wrong. `context` names what was being attempted.
    Failed { context: String, message: String },
}

impl TaskResult {
    /// True when the result reports a failure.
    pub fn is_error(&self) -> bool {
        matches!(self, TaskResult::Failed { .. })
    }

    /// The message to show in the echo area, if any.
    pub fn message(&self) -> Option<String> {
        match self {
            TaskResult::FileRead { path, .. } => Some(format!("Read {}", path.display())),
            TaskResult::FileWritten { path, bytes, .. } => {
                Some(format!("Wrote {} ({bytes} bytes)", path.display()))
            }
            TaskResult::LanguageServerStarted { language } => {
                Some(format!("Language server for {language} started"))
            }
            TaskResult::LanguageServerStopped { language } => {
                Some(format!("Language server for {language} stopped"))
            }
            TaskResult::ShellOutput { status, command, .. } if *status != 0 => {
                Some(format!("`{command}` exited with status {status}"))
            }
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
        let ok = TaskResult::TreeUpdated { nodes: Vec::new(), select: None, show_hidden: false };
        assert!(!ok.is_error());
        let bad = TaskResult::Failed { context: "find-file".into(), message: "no such file".into() };
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
        };
        assert_eq!(read.message().unwrap(), "Read /tmp/a.rs");

        let written =
            TaskResult::FileWritten { path: path(), buffer: BufferId(1), bytes: 120, disk_time: None };
        assert_eq!(written.message().unwrap(), "Wrote /tmp/a.rs (120 bytes)");
    }

    #[test]
    fn quiet_results_say_nothing() {
        assert_eq!(
            TaskResult::TreeUpdated { nodes: Vec::new(), select: None, show_hidden: false }.message(),
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
            query: LspQuery::DocumentSymbols,
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

    #[test]
    fn language_server_lifecycle_results_are_announced() {
        assert_eq!(
            TaskResult::LanguageServerStarted { language: "rust".into() }.message().unwrap(),
            "Language server for rust started"
        );
        assert_eq!(
            TaskResult::LanguageServerStopped { language: "rust".into() }.message().unwrap(),
            "Language server for rust stopped"
        );
    }

    #[test]
    fn every_query_kind_describes_itself() {
        let queries = [
            LspQuery::Definition(LspPosition::ZERO),
            LspQuery::References(LspPosition::ZERO),
            LspQuery::Hover(LspPosition::ZERO),
            LspQuery::Completion(LspPosition::ZERO),
            LspQuery::SignatureHelp(LspPosition::ZERO),
            LspQuery::Rename { position: LspPosition::ZERO, new_name: "x".into() },
            LspQuery::Format { tab_size: 4, insert_spaces: true },
            LspQuery::CodeAction { range: LspRange::empty(LspPosition::ZERO), diagnostics: Vec::new() },
            LspQuery::DocumentSymbols,
            LspQuery::WorkspaceSymbols("q".into()),
        ];
        let mut descriptions: Vec<&str> = queries.iter().map(LspQuery::description).collect();
        let before = descriptions.len();
        descriptions.sort_unstable();
        descriptions.dedup();
        assert_eq!(descriptions.len(), before, "descriptions must be distinguishable");
    }

    #[test]
    fn only_the_navigating_queries_push_the_mark() {
        assert!(LspQuery::Definition(LspPosition::ZERO).jumps());
        assert!(LspQuery::References(LspPosition::ZERO).jumps());
        assert!(!LspQuery::Hover(LspPosition::ZERO).jumps());
        assert!(!LspQuery::Format { tab_size: 4, insert_spaces: true }.jumps());
    }

    #[test]
    fn a_response_carries_enough_context_to_be_routed_back() {
        let result = TaskResult::LspResponse {
            language: "rust".into(),
            uri: "file:///a.rs".into(),
            query: LspQuery::Hover(LspPosition::new(3, 4)),
            result: json!({"contents": "docs"}),
        };
        let TaskResult::LspResponse { query, uri, .. } = result else { panic!() };
        assert_eq!(uri, "file:///a.rs");
        assert_eq!(query, LspQuery::Hover(LspPosition::new(3, 4)));
    }
}
