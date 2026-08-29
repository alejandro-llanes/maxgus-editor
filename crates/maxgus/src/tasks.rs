//! The task executor.
//!
//! Everything the editor cannot do synchronously ends up here: reading and
//! writing files, walking the project tree, running tree-sitter, talking to
//! language servers, running shell commands. The executor owns those resources
//! outright and runs in its own tokio task, taking [`Task`]s from a channel and
//! sending [`TaskResult`]s back. The editor never blocks on any of it.

use anyhow::Result;
use maxgus_config::{LspSpec, TreeConfig};
use maxgus_core::task::{LspQuery, Task, TaskResult, TreeAction};
use maxgus_lsp::{Client, ServerEvent};
use maxgus_syntax::Highlighter;
use maxgus_tree::FileTree;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A buffer's parser, and the text its tree describes.
struct BufferSyntax {
    language: String,
    highlighter: Highlighter,
    text: String,
}

/// Everything the executor owns.
pub struct Executor {
    root: PathBuf,
    tree: Option<FileTree>,
    tree_config: TreeConfig,
    /// One highlighter per *buffer*, with the text it last parsed.
    ///
    /// Per buffer rather than per language, because a highlighter's value is
    /// the syntax tree it is holding: sharing one between buffers would mean
    /// throwing that tree away on every switch, and a re-parse from nothing
    /// costs eighteen times what an incremental one does.
    highlighters: HashMap<maxgus_text::BufferId, BufferSyntax>,
    /// Running language servers, by language.
    servers: HashMap<String, Arc<Client>>,
    /// The text each open document was last sent as, so a change can be
    /// described as the region that differs rather than the whole file.
    documents: HashMap<String, String>,
    lsp_specs: Vec<LspSpec>,
    results: mpsc::UnboundedSender<TaskResult>,
}

impl Executor {
    pub fn new(
        root: PathBuf,
        tree_config: TreeConfig,
        lsp_specs: Vec<LspSpec>,
        results: mpsc::UnboundedSender<TaskResult>,
    ) -> Executor {
        Executor {
            root,
            tree: None,
            tree_config,
            highlighters: HashMap::new(),
            servers: HashMap::new(),
            documents: HashMap::new(),
            lsp_specs,
            results,
        }
    }

    /// Runs until the task channel closes.
    pub async fn run(mut self, mut tasks: mpsc::UnboundedReceiver<Task>) {
        while let Some(task) = tasks.recv().await {
            self.handle(task).await;
        }
        // Leaving without shutting servers down would orphan the processes.
        self.shutdown().await;
    }

    fn send(&self, result: TaskResult) {
        let _ = self.results.send(result);
    }

    /// Reports a failure to the editor rather than swallowing it.
    fn fail(&self, context: &str, error: impl std::fmt::Display) {
        self.send(TaskResult::Failed { context: context.to_string(), message: error.to_string() });
    }

    async fn handle(&mut self, task: Task) {
        match task {
            Task::ReadFile { path, reverting, other_window } => {
                self.read_file(path, reverting, other_window).await;
            }
            Task::WriteFile { path, contents, buffer, backup, guard } => {
                self.write_file(path, contents, buffer, backup, guard).await;
            }
            Task::ListDirectory { path } => self.list_directory(path).await,
            Task::Tree(action) => self.tree_action(action).await,
            Task::Reparse { buffer, language, text, revision, range } => {
                self.reparse(buffer, &language, text, revision, range).await;
            }
            Task::PersistTheme { path, theme } => {
                self.persist_theme(path, theme).await;
            }
            Task::GitBranch { root } => {
                let branch = maxgus_tree::git::branch(&root).await;
                self.send(TaskResult::GitBranch { branch });
            }
            Task::StartLanguageServer { language } => self.start_server(&language).await,
            Task::StopLanguageServer { language } => self.stop_server(&language).await,
            Task::LspDidOpen { language, uri, version, text } => {
                if let Some(client) = self.servers.get(&language) {
                    let file_language = language.clone();
                    let _ = client.did_open(&uri, &file_language, version, &text);
                    self.documents.insert(uri, text);
                }
            }
            Task::LspDidChange { language, uri, version, text } => {
                self.did_change(&language, uri, version, text).await;
            }
            Task::LspDidSave { language, uri } => {
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.did_save(&uri, None);
                }
            }
            Task::LspDidClose { language, uri } => {
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.did_close(&uri);
                }
                self.documents.remove(&uri);
            }
            Task::LspRequest { language, uri, query } => self.lsp_request(language, uri, query),
            Task::LspRespond { language, id, applied } => {
                // The editor has finished with the edit the server asked for;
                // tell the server whether it went in.
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.respond(id, serde_json::json!({ "applied": applied }));
                }
            }
            Task::Shell { command, directory, insert_at } => {
                self.shell(command, directory, insert_at).await;
            }
            Task::ForgetBuffer { buffer } => self.forget(buffer),
        }
    }

    // ---- files ---------------------------------------------------------

    async fn read_file(&self, path: PathBuf, reverting: Option<maxgus_text::BufferId>, other_window: bool) {
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                // Invalid UTF-8 is shown rather than refused, so a stray byte
                // in an otherwise readable file does not stop it being read.
                // What it cannot be is *saved*: the replacement characters
                // would go to disk over the bytes they stand in for, so the
                // buffer is opened read-only and says why.
                let lossy = std::str::from_utf8(&bytes).is_err();
                let contents = String::from_utf8_lossy(&bytes).into_owned();
                let metadata = tokio::fs::metadata(&path).await.ok();
                let read_only =
                    lossy || metadata.as_ref().is_some_and(|m| m.permissions().readonly());
                let disk_time = metadata.and_then(|m| m.modified().ok());
                self.send(TaskResult::FileRead {
                    path,
                    contents,
                    read_only,
                    lossy,
                    disk_time,
                    reverting,
                    other_window,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Visiting a file that does not exist yet creates an empty
                // buffer for it, as `find-file` does.
                self.send(TaskResult::FileRead {
                    path,
                    contents: String::new(),
                    read_only: false,
                    lossy: false,
                    disk_time: None,
                    reverting,
                    other_window,
                });
            }
            Err(error) => self.fail("find-file", error),
        }
    }

    /// Writes the chosen theme into the configuration file.
    async fn persist_theme(&self, path: PathBuf, theme: String) {
        // A file that cannot be read is not one to overwrite: the user may
        // simply not have one yet, and starting it is fine, but replacing
        // something unreadable is not.
        let source = match tokio::fs::read_to_string(&path).await {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                self.fail("visit-theme", error);
                return;
            }
        };
        let updated = with_theme(&source, &theme);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            self.fail("visit-theme", error);
            return;
        }
        match tokio::fs::write(&path, updated).await {
            Ok(()) => self.send(TaskResult::ThemePersisted { path, theme }),
            Err(error) => self.fail("visit-theme", error),
        }
    }

    async fn write_file(
        &self,
        path: PathBuf,
        contents: String,
        buffer: maxgus_text::BufferId,
        backup: bool,
        guard: maxgus_core::WriteGuard,
    ) {
        // Whatever the write insisted on is checked here, where a `stat` can
        // be awaited. Refusing beats overwriting: what was there would be gone
        // with no sign it had ever existed.
        let refuse = match guard {
            maxgus_core::WriteGuard::Regardless => false,
            maxgus_core::WriteGuard::Absent => {
                tokio::fs::try_exists(&path).await.unwrap_or(false)
            }
            maxgus_core::WriteGuard::Unchanged(expect) => match expect {
                Some(expect) => tokio::fs::metadata(&path)
                    .await
                    .is_ok_and(|m| m.modified().is_ok_and(|now| now != expect)),
                None => false,
            },
        };
        if refuse {
            self.send(TaskResult::WriteRefused { path, buffer, because: guard });
            return;
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            self.fail("save-buffer", error);
            return;
        }
        if backup && tokio::fs::try_exists(&path).await.unwrap_or(false) {
            let mut backup_path = path.clone().into_os_string();
            backup_path.push("~");
            // A failed backup is worth saying, but not worth refusing the save.
            if let Err(error) = tokio::fs::copy(&path, PathBuf::from(backup_path)).await {
                self.fail("backup", error);
            }
        }
        let bytes = contents.len();
        match tokio::fs::write(&path, contents).await {
            Ok(()) => {
                // Recorded from the file just written, so the next save
                // compares against what is actually there.
                let disk_time =
                    tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok());
                self.send(TaskResult::FileWritten { path, buffer, bytes, disk_time });
            }
            Err(error) => self.fail("save-buffer", error),
        }
    }

    async fn list_directory(&self, path: PathBuf) {
        let mut entries = Vec::new();
        match tokio::fs::read_dir(&path).await {
            Ok(mut reader) => {
                while let Ok(Some(entry)) = reader.next_entry().await {
                    let mut name = entry.path().to_string_lossy().into_owned();
                    // A trailing slash on directories makes completion
                    // continue into them rather than stopping at the name.
                    if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                        name.push('/');
                    }
                    entries.push(name);
                }
                entries.sort();
                self.send(TaskResult::DirectoryListed { path, entries });
            }
            Err(error) => self.fail("list-directory", error),
        }
    }

    // ---- the file tree -------------------------------------------------

    /// Opens the tree if it is not open yet.
    async fn ensure_tree(&mut self) -> Result<()> {
        if self.tree.is_none() {
            let tree = FileTree::open(self.root.clone(), self.tree_config.clone()).await?;
            self.tree = Some(tree);
        }
        Ok(())
    }

    async fn tree_action(&mut self, action: TreeAction) {
        if let Err(error) = self.ensure_tree().await {
            self.fail("treefile", error);
            return;
        }
        let Some(tree) = self.tree.as_mut() else { return };

        // Whatever the action, the cursor should end up somewhere sensible.
        let mut select = tree.selected_path().map(Path::to_path_buf);
        let outcome: Result<(), maxgus_tree::TreeError> = match action {
            TreeAction::Refresh => tree.refresh().await,
            TreeAction::Toggle(path) => tree.toggle(&path).await.map(|_| ()),
            TreeAction::Expand(path) => tree.expand(&path).await,
            TreeAction::Collapse(path) => {
                tree.collapse(&path);
                Ok(())
            }
            TreeAction::ExpandRecursively(path) => tree.expand_recursively(&path).await,
            TreeAction::Reveal(path) => {
                select = Some(path.clone());
                tree.reveal(&path).await.map(|_| ())
            }
            TreeAction::ToggleHidden => tree.toggle_show_hidden().await,
            TreeAction::ToggleDirectoriesFirst => {
                self.tree_config.directories_first = !self.tree_config.directories_first;
                let config = self.tree_config.clone();
                match FileTree::open(self.root.clone(), config).await {
                    Ok(fresh) => {
                        self.tree = Some(fresh);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            TreeAction::ToggleGitStatus => {
                self.tree_config.git_status = !self.tree_config.git_status;
                let config = self.tree_config.clone();
                match FileTree::open(self.root.clone(), config).await {
                    Ok(fresh) => {
                        self.tree = Some(fresh);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            TreeAction::CreateFile { parent, name } => match Self::at(tree, &parent) {
                Ok(()) => tree.create_file(&name).await.map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::CreateDirectory { parent, name } => match Self::at(tree, &parent) {
                Ok(()) => tree.create_directory(&name).await.map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::Delete(path) => match Self::at(tree, &path) {
                Ok(()) => tree.delete_selected().await.map(|_| select = None),
                Err(error) => Err(error),
            },
            TreeAction::Rename { path, name } => match Self::at(tree, &path) {
                Ok(()) => tree.rename_selected(&name).await.map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::Move { path, destination } => match Self::at(tree, &path) {
                Ok(()) => tree.move_selected(&destination).await.map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = outcome {
            self.fail("treefile", error);
        }
        let Some(tree) = self.tree.as_ref() else { return };
        self.send(TaskResult::TreeUpdated {
            nodes: tree.visible().to_vec(),
            select,
            show_hidden: tree.config().show_hidden,
        });
    }

    /// Puts the tree's own cursor on `path`, so the operations that act on the
    /// selection act on the node the editor meant.
    /// Puts the cursor on `path`, or says it could not.
    ///
    /// Every mutating action below works on the *selection*, so going ahead
    /// without this having succeeded renames or deletes whatever the cursor
    /// happened to be sitting on. It did: asking to delete a file inside a
    /// collapsed directory deleted the unrelated file the cursor was on.
    fn at(tree: &mut FileTree, path: &Path) -> maxgus_tree::Result<()> {
        match tree.goto_path(path) {
            true => Ok(()),
            false => Err(maxgus_tree::TreeError::NotInTree(path.to_path_buf())),
        }
    }

    // ---- syntax --------------------------------------------------------

    async fn reparse(
        &mut self,
        buffer: maxgus_text::BufferId,
        language: &str,
        text: String,
        revision: u64,
        range: std::ops::Range<usize>,
    ) {
        // A buffer whose language changed — after `write-file`, say — starts
        // over with the right grammar.
        if self.highlighters.get(&buffer).is_some_and(|s| s.language != language) {
            self.highlighters.remove(&buffer);
        }
        // Taken out of the map rather than borrowed, because the work below
        // leaves this thread and needs to own it.
        let mut syntax = match self.highlighters.remove(&buffer) {
            Some(syntax) => syntax,
            None => {
                // A language with no compiled-in grammar is not an error; it
                // simply goes unhighlighted.
                let Ok(highlighter) = Highlighter::new(language) else { return };
                BufferSyntax {
                    language: language.to_string(),
                    highlighter,
                    text: String::new(),
                }
            }
        };

        // Parsing a large file is a quarter of a second of solid CPU with
        // nothing in it to await. Run on a runtime thread it would stop tokio
        // polling anything else for that whole time — the language server's
        // transport and the terminal's input among them — so it goes to the
        // blocking pool and the workers stay free.
        let parsed = tokio::task::spawn_blocking(move || {
            // Telling the parser which region changed is what lets it keep
            // the rest of the tree.
            if syntax.highlighter.has_tree()
                && let Some(edit) = maxgus_syntax::InputEdit::between(&syntax.text, &text)
            {
                syntax.highlighter.edit(edit, &syntax.text, &text);
            }
            if syntax.highlighter.parse(&text).is_err() {
                return (syntax, None);
            }
            // Only the requested region is queried: running the highlight
            // query over a whole large file costs far more than parsing it,
            // and the answer beyond the window would never be drawn.
            let range = range.start..range.end.min(text.len());
            let highlights = syntax.highlighter.highlights_in(&text, range.clone());
            syntax.text = text;
            (syntax, Some((range, highlights)))
        })
        .await;

        // A parse that panicked must not take the buffer's grammar with it;
        // the next edit starts a fresh highlighter instead.
        let Ok((syntax, outcome)) = parsed else { return };
        self.highlighters.insert(buffer, syntax);
        if let Some((range, highlights)) = outcome {
            self.send(TaskResult::Reparsed { buffer, revision, range, highlights });
        }
    }

    /// Drops what was kept for a buffer that no longer exists.
    fn forget(&mut self, buffer: maxgus_text::BufferId) {
        self.highlighters.remove(&buffer);
    }

    // ---- language servers ----------------------------------------------

    fn spec_for(&self, language: &str) -> Option<&LspSpec> {
        self.lsp_specs.iter().find(|s| s.language == language)
    }

    async fn start_server(&mut self, language: &str) {
        if self.servers.contains_key(language) {
            return;
        }
        let Some(spec) = self.spec_for(language).cloned() else { return };
        // The project root is where the server is told to look. Walked in the
        // order the markers were configured, stopping at the first that hits.
        let mut root = None;
        for marker in &spec.root_markers {
            if let Some(found) = find_upwards(&self.root, marker).await {
                root = Some(found);
                break;
            }
        }
        let root = root.unwrap_or_else(|| self.root.clone());

        match Client::spawn(&spec.command, &spec.args, &root).await {
            Ok((client, events)) => {
                if let Err(error) = client.initialize(&root).await {
                    self.fail("language server", error);
                    return;
                }
                self.servers.insert(language.to_string(), Arc::clone(&client));
                // Diagnostics and messages arrive on their own schedule.
                tokio::spawn(forward_events(
                    events,
                    self.results.clone(),
                    language.to_string(),
                    Arc::clone(&client),
                ));
                self.send(TaskResult::LanguageServerStarted { language: language.to_string() });
            }
            Err(error) => self.fail(&format!("starting {language} server"), error),
        }
    }

    async fn stop_server(&mut self, language: &str) {
        let Some(client) = self.servers.remove(language) else { return };
        let _ = client.shutdown().await;
        self.send(TaskResult::LanguageServerStopped { language: language.to_string() });
    }

    /// Tells the server a document changed, in the form it asked for.
    ///
    /// A server that declared incremental sync is sent only the region that
    /// differs. Sending the whole file on every pause in typing makes the
    /// server re-parse it from nothing, which is exactly the cost incremental
    /// sync exists to avoid.
    async fn did_change(&mut self, language: &str, uri: String, version: i64, text: String) {
        let Some(client) = self.servers.get(language).cloned() else { return };
        let incremental = client.sync_kind().await == maxgus_lsp::client::SyncKind::Incremental;

        let sent = match (incremental, self.documents.get(&uri)) {
            (true, Some(previous)) => match changed_range(previous, &text, client.encoding().await) {
                // The texts are identical; there is nothing to report.
                None => return,
                Some((range, replacement)) => {
                    client.did_change_incremental(&uri, version, range, &replacement)
                }
            },
            _ => client.did_change_full(&uri, version, &text),
        };
        if sent.is_ok() {
            self.documents.insert(uri, text);
        }
    }

    /// Sends a request without waiting for it here, so a slow server cannot
    /// hold up the rest of the queue.
    fn lsp_request(&self, language: String, uri: String, query: LspQuery) {
        let Some(client) = self.servers.get(&language).cloned() else { return };
        let results = self.results.clone();
        tokio::spawn(async move {
            let outcome = match &query {
                LspQuery::Definition(p) => client.definition(&uri, *p).await,
                LspQuery::References(p) => client.references(&uri, *p).await,
                LspQuery::Hover(p) => client.hover(&uri, *p).await,
                LspQuery::Completion(p) => client.completion(&uri, *p).await,
                LspQuery::SignatureHelp(p) => client.signature_help(&uri, *p).await,
                LspQuery::Rename { position, new_name } => {
                    client.rename(&uri, *position, new_name).await
                }
                LspQuery::Format { tab_size, insert_spaces } => {
                    client.formatting(&uri, *tab_size, *insert_spaces).await
                }
                LspQuery::CodeAction { range, diagnostics } => {
                    client.code_action(&uri, *range, diagnostics).await
                }
                LspQuery::DocumentSymbols => client.document_symbols(&uri).await,
                LspQuery::WorkspaceSymbols(q) => client.workspace_symbols(q).await,
            };
            let result = match outcome {
                Ok(value) => TaskResult::LspResponse { language, uri, query, result: value },
                Err(error) => TaskResult::Failed {
                    context: "language server".into(),
                    message: error.to_string(),
                },
            };
            let _ = results.send(result);
        });
    }

    async fn shutdown(&mut self) {
        let languages: Vec<String> = self.servers.keys().cloned().collect();
        for language in languages {
            self.stop_server(&language).await;
        }
    }

    // ---- shell ---------------------------------------------------------

    async fn shell(
        &self,
        command: String,
        directory: PathBuf,
        insert_at: Option<(maxgus_text::BufferId, usize)>,
    ) {
        let mut process = tokio::process::Command::new("sh");
        process
            .arg("-c")
            .arg(&command)
            .current_dir(&directory)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        match process.output().await {
            Ok(output) => {
                // Both streams are shown: a command's error message is as
                // interesting as its output.
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                if !output.stderr.is_empty() {
                    text.push_str(&String::from_utf8_lossy(&output.stderr));
                }
                self.send(TaskResult::ShellOutput {
                    command,
                    output: text,
                    status: output.status.code().unwrap_or(-1),
                    insert_at,
                });
            }
            Err(error) => self.fail("shell-command", error),
        }
    }
}

/// The region in which `previous` and `current` differ, as the protocol wants
/// it: a range in the *old* document and the text now in its place.
fn changed_range(
    previous: &str,
    current: &str,
    encoding: maxgus_lsp::PositionEncoding,
) -> Option<(maxgus_lsp::LspRange, String)> {
    let edit = maxgus_syntax::InputEdit::between(previous, current)?;
    let range = maxgus_lsp::LspRange::new(
        maxgus_lsp::position::byte_to_position(previous, edit.start_byte, encoding),
        maxgus_lsp::position::byte_to_position(previous, edit.old_end_byte, encoding),
    );
    Some((range, current[edit.start_byte..edit.new_end_byte].to_string()))
}

/// Walks up from `start` looking for `marker`, returning the directory holding
/// it — how a project root is found.
pub async fn find_upwards(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut directory = Some(start);
    while let Some(current) = directory {
        // `tokio::fs`, not `Path::exists`: this runs while the editor is
        // already going, and a stat on a cold or networked filesystem is a
        // blocking call like any other.
        if tokio::fs::try_exists(current.join(marker)).await.unwrap_or(false) {
            return Some(current.to_path_buf());
        }
        directory = current.parent();
    }
    None
}

/// The JSON-RPC code for a method the receiver does not implement.
const METHOD_NOT_FOUND: i64 = -32601;

/// Rewrites `set theme="…"` in a configuration file, leaving all of it alone.
///
/// Text in, text out, and no filesystem: this is the user's own file, and the
/// one thing it must never do is lose the rest of it.
///
/// Only a `set` line is touched, and only its `theme=` property — a
/// `theme "name" { … }` block says `theme` too and means something else
/// entirely. With no `set theme=` anywhere, one is added at the end.
pub fn with_theme(source: &str, theme: &str) -> String {
    let replacement = format!("theme=\"{theme}\"");
    let mut out = String::with_capacity(source.len() + replacement.len());
    let mut done = false;

    for line in source.split_inclusive('\n') {
        if done || !line.trim_start().starts_with("set ") {
            out.push_str(line);
            continue;
        }
        match find_theme_property(line) {
            Some((start, end)) => {
                out.push_str(&line[..start]);
                out.push_str(&replacement);
                out.push_str(&line[end..]);
                done = true;
            }
            None => out.push_str(line),
        }
    }

    if !done {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&replacement.replace("theme=", "set theme="));
        out.push('\n');
    }
    out
}

/// The byte range of a `theme="…"` property within one line.
fn find_theme_property(line: &str) -> Option<(usize, usize)> {
    let start = line.find("theme=")?;
    // `set line-theme=` would also contain it; the property has to begin a
    // word.
    if start > 0 && !line.as_bytes()[start - 1].is_ascii_whitespace() {
        return None;
    }
    let rest = &line[start + "theme=".len()..];
    let open = rest.find('"')?;
    let close = rest[open + 1..].find('"')?;
    Some((start, start + "theme=".len() + open + 1 + close + 1))
}

/// Turns server-initiated messages into task results.
async fn forward_events(
    mut events: mpsc::UnboundedReceiver<ServerEvent>,
    results: mpsc::UnboundedSender<TaskResult>,
    language: String,
    client: Arc<Client>,
) {
    while let Some(event) = events.recv().await {
        let result = match event {
            ServerEvent::Diagnostics { uri, diagnostics } => {
                TaskResult::Diagnostics { uri, diagnostics }
            }
            ServerEvent::Message { severity, text } => {
                // Only what the user would want interrupting them.
                if severity > maxgus_lsp::Severity::Warning {
                    continue;
                }
                TaskResult::Failed { context: language.clone(), message: text }
            }
            ServerEvent::Exited => {
                TaskResult::LanguageServerStopped { language: language.clone() }
            }
            // Every server request must be answered — the protocol says so,
            // and a server that asked for something waits until it hears back.
            ServerEvent::Request(request) => match request.method.as_str() {
                "workspace/applyEdit" => {
                    // The edit has to go through the editor, so the answer
                    // cannot be given here; it comes back as `LspRespond`.
                    let edit = request.params.get("edit").cloned().unwrap_or_default();
                    TaskResult::LspApplyEdit {
                        language: language.clone(),
                        id: request.id,
                        edit,
                    }
                }
                _ => {
                    // Anything else is refused rather than ignored. Silence
                    // would leave the server waiting for ever.
                    let _ = client.respond_error(
                        request.id,
                        METHOD_NOT_FOUND,
                        &format!("{} is not supported", request.method),
                    );
                    continue;
                }
            },
            ServerEvent::Notification(_) => continue,
        };
        if results.send(result).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    // ---- writing a theme into the configuration -------------------------

    #[test]
    fn a_theme_replaces_the_one_already_set() {
        let before = "set tab-width=4\nset theme=\"maxgus-dark\"\nset line-numbers=#true\n";
        assert_eq!(
            with_theme(before, "nord"),
            "set tab-width=4\nset theme=\"nord\"\nset line-numbers=#true\n"
        );
    }

    #[test]
    fn a_theme_beside_other_properties_leaves_them_alone() {
        let before = "set tab-width=4 theme=\"maxgus-dark\" line-numbers=#true\n";
        assert_eq!(
            with_theme(before, "gruvbox"),
            "set tab-width=4 theme=\"gruvbox\" line-numbers=#true\n"
        );
    }

    #[test]
    fn a_file_that_never_set_a_theme_gets_one_at_the_end() {
        let before = "set tab-width=4\n";
        assert_eq!(with_theme(before, "nord"), "set tab-width=4\nset theme=\"nord\"\n");
    }

    #[test]
    fn an_empty_file_gets_just_the_one_line() {
        assert_eq!(with_theme("", "nord"), "set theme=\"nord\"\n");
    }

    #[test]
    fn a_file_with_no_final_newline_still_ends_up_well_formed() {
        assert_eq!(with_theme("set tab-width=4", "nord"), "set tab-width=4\nset theme=\"nord\"\n");
    }

    #[test]
    fn a_commented_out_setting_is_left_commented_out() {
        // Only the `set ` guard protects this one — the line does contain
        // `theme=`, so the property check alone would rewrite it and quietly
        // resurrect a setting the user had turned off.
        let before = "// set theme=\"old\"\nset tab-width=4\n";
        let after = with_theme(before, "nord");
        assert!(
            after.starts_with("// set theme=\"old\""),
            "the comment was rewritten:\n{after}"
        );
        assert!(after.ends_with("set theme=\"nord\"\n"), "the setting was not added:\n{after}");
    }

    #[test]
    fn a_theme_block_is_not_mistaken_for_the_setting() {
        // `theme "maxgus-dark" { … }` says `theme` and means something else
        // entirely; touching it would destroy the user's faces.
        let before = concat!(
            "theme \"maxgus-dark\" {\n",
            "    face \"region\" bg=\"#3a4048\"\n",
            "}\n",
        );
        let after = with_theme(before, "nord");
        assert!(after.starts_with(before), "the theme block was edited:\n{after}");
        assert!(after.ends_with("set theme=\"nord\"\n"), "the setting was not added:\n{after}");
    }

    #[test]
    fn everything_else_in_the_file_survives_untouched() {
        let before = concat!(
            "// a comment\n",
            "set tab-width=4\n",
            "\n",
            "keymap \"global\" {\n",
            "    bind \"C-c f\" \"lsp-format-buffer\"\n",
            "}\n",
            "\n",
            "set theme=\"maxgus-dark\"\n",
            "\n",
            "tree { width 32 }\n",
        );
        let after = with_theme(before, "dracula");
        assert_eq!(after, before.replace("maxgus-dark", "dracula"));
        assert_eq!(after.lines().count(), before.lines().count(), "no line was added or lost");
    }

    #[test]
    fn only_the_first_theme_setting_is_rewritten() {
        // A second one would be the one that wins on load, but rewriting both
        // would be changing more than was asked; the first is where the value
        // is read from anyway once the duplicate is resolved.
        let before = "set theme=\"a\"\nset theme=\"b\"\n";
        assert_eq!(with_theme(before, "nord"), "set theme=\"nord\"\nset theme=\"b\"\n");
    }

    #[test]
    fn a_property_merely_ending_in_theme_is_not_the_one() {
        let before = "set line-theme=\"x\"\n";
        let after = with_theme(before, "nord");
        assert!(after.starts_with("set line-theme=\"x\""), "got `{after}`");
        assert!(after.ends_with("set theme=\"nord\"\n"), "got `{after}`");
    }

    // ---- keeping the async claim honest ---------------------------------

    /// Calls that block the thread they run on. On a runtime thread each one
    /// stops tokio polling everything else for as long as it takes.
    const BLOCKING_CALLS: &[&str] = &[
        "std::fs::",
        "File::open",
        "File::create",
        "OpenOptions",
        ".exists()",
        "std::thread::sleep",
        "std::process::Command",
        "std::sync::Mutex",
    ];

    /// Files allowed to block, with the reason.
    ///
    /// `main.rs` reads the configuration and opens the log before the editor
    /// is doing anything: there is no one else to starve yet, and making that
    /// path async would buy nothing.
    const MAY_BLOCK: &[&str] = &["maxgus/src/main.rs"];

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }

    #[test]
    fn nothing_outside_the_startup_path_blocks_the_runtime() {
        // The README promises every file read, directory walk, parse and
        // subprocess runs on tokio. A grep cannot show that something works,
        // but it can show that something is absent, and absence is the whole
        // claim here.
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
        let mut files = Vec::new();
        // Only what ships. A `crates/*/tests` file is test code from its first
        // line and carries no `#[cfg(test)]` to mark where that starts.
        for entry in std::fs::read_dir(&crates).expect("the workspace").flatten() {
            rust_files(&entry.path().join("src"), &mut files);
        }
        assert!(files.len() > 30, "the walk found almost nothing: {}", files.len());

        let mut offences = Vec::new();
        for file in files {
            let shown = file.to_string_lossy().replace('\\', "/");
            if MAY_BLOCK.iter().any(|allowed| shown.ends_with(allowed)) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&file) else { continue };
            // Tests may block as much as they like; only what ships matters.
            let ships = source
                .lines()
                .take_while(|line| !line.starts_with("#[cfg(test)]"))
                .enumerate();
            for (n, line) in ships {
                let code = line.split("//").next().unwrap_or(line);
                for call in BLOCKING_CALLS {
                    if code.contains(call) {
                        offences.push(format!("{shown}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(offences.is_empty(), "blocking calls off the startup path:\n{}", offences.join("\n"));
    }

    // ---- answering the server -------------------------------------------

    /// A client wired to a pipe, with the far end for a test to play server on.
    async fn piped_client() -> (
        std::sync::Arc<maxgus_lsp::Client>,
        mpsc::UnboundedReceiver<ServerEvent>,
        tokio::io::DuplexStream,
        tokio::io::DuplexStream,
    ) {
        let (client_reader, server_writer) = tokio::io::duplex(64 * 1024);
        let (server_reader, client_writer) = tokio::io::duplex(64 * 1024);
        let (client, events) = maxgus_lsp::Client::connect(client_reader, client_writer);
        (client, events, server_reader, server_writer)
    }

    /// Reads one message off the server end of the pipe.
    ///
    /// Bounded, because the behaviour under test is *that an answer arrives*:
    /// without the bound a regression makes this wait for ever, and a hanging
    /// test says far less than a failing one.
    async fn next_message(reader: &mut tokio::io::DuplexStream) -> maxgus_lsp::Message {
        use tokio::io::AsyncReadExt;
        let read = async {
            let mut buffer = Vec::new();
            loop {
                if let Ok(maxgus_lsp::protocol::Decoded::Message(message, _)) =
                    maxgus_lsp::protocol::decode(&buffer)
                {
                    return *message;
                }
                let mut chunk = [0u8; 4096];
                let n = reader.read(&mut chunk).await.expect("the pipe is open");
                assert!(n > 0, "the pipe closed with nothing to read");
                buffer.extend_from_slice(&chunk[..n]);
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), read)
            .await
            .expect("the server end was never answered")
    }

    #[tokio::test]
    async fn a_server_request_we_cannot_serve_is_refused_rather_than_ignored() {
        // The protocol requires an answer to every request. Dropping one
        // leaves the server waiting for ever, which is what used to happen to
        // everything the client did not handle itself.
        let (client, events, mut server_reader, mut server_writer) = piped_client().await;
        let (results, _rx) = mpsc::unbounded_channel();
        tokio::spawn(forward_events(events, results, "rust".into(), client));

        use tokio::io::AsyncWriteExt;
        let request = maxgus_lsp::Message::Request(maxgus_lsp::Request {
            id: maxgus_lsp::RequestId::Number(11),
            method: "workspace/workspaceFolders".into(),
            params: serde_json::Value::Null,
        });
        server_writer.write_all(&request.encode()).await.unwrap();

        let maxgus_lsp::Message::Response(response) = next_message(&mut server_reader).await else {
            panic!("expected a response")
        };
        assert_eq!(response.id, Some(maxgus_lsp::RequestId::Number(11)));
        let error = response.error.expect("an error, not silence");
        assert_eq!(error.code, METHOD_NOT_FOUND);
        assert!(error.message.contains("workspace/workspaceFolders"), "got `{}`", error.message);
    }

    #[tokio::test]
    async fn an_apply_edit_request_is_carried_to_the_editor_with_its_id() {
        // This one cannot be answered here: the buffers live in the editor, so
        // the request travels as a result and the answer comes back as a task.
        let (client, events, _server_reader, mut server_writer) = piped_client().await;
        let (results, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(forward_events(events, results, "rust".into(), client));

        use tokio::io::AsyncWriteExt;
        let edit = serde_json::json!({ "changes": { "file:///a.rs": [] } });
        let request = maxgus_lsp::Message::Request(maxgus_lsp::Request {
            id: maxgus_lsp::RequestId::Number(7),
            method: "workspace/applyEdit".into(),
            params: serde_json::json!({ "edit": edit }),
        });
        server_writer.write_all(&request.encode()).await.unwrap();

        // Bounded for the same reason the pipe read is: if the edit stops
        // being carried, this must fail rather than wait for ever.
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the edit never reached the editor")
            .expect("a result");
        let TaskResult::LspApplyEdit { language, id, edit: carried } = received else {
            panic!("expected the edit to reach the editor, got {received:?}")
        };
        assert_eq!(language, "rust");
        assert_eq!(id, maxgus_lsp::RequestId::Number(7));
        assert_eq!(carried, edit, "the edit itself is carried, not just the fact of it");
    }

    use super::*;
    use maxgus_text::BufferId;

    /// A temporary directory, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        async fn new(tag: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!("maxgus-exec-{tag}"));
            tokio::fs::remove_dir_all(&dir).await.ok();
            tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
            tokio::fs::write(dir.join("Cargo.toml"), "[package]").await.unwrap();
            tokio::fs::write(dir.join("src/main.rs"), "fn main() {}\n").await.unwrap();
            Fixture(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// An executor over `root`, with the channel its results arrive on.
    fn executor(root: &Path) -> (Executor, mpsc::UnboundedReceiver<TaskResult>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let config = TreeConfig { git_status: false, ..Default::default() };
        (Executor::new(root.to_path_buf(), config, Vec::new(), tx), rx)
    }

    /// Runs one task and returns the first result it produced.
    async fn run_one(executor: &mut Executor, rx: &mut mpsc::UnboundedReceiver<TaskResult>, task: Task) -> TaskResult {
        executor.handle(task).await;
        rx.try_recv().expect("the task produced no result")
    }

    #[tokio::test]
    async fn reading_a_file_returns_its_contents() {
        let f = Fixture::new("read").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ReadFile {
                path: f.path().join("src/main.rs"),
                reverting: None,
                other_window: false,
            },
        )
        .await;
        let TaskResult::FileRead { contents, read_only, .. } = result else { panic!("{result:?}") };
        assert_eq!(contents, "fn main() {}\n");
        assert!(!read_only);
    }

    #[tokio::test]
    async fn reading_a_file_that_does_not_exist_yet_gives_an_empty_buffer() {
        let f = Fixture::new("readmissing").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ReadFile { path: f.path().join("new.rs"), reverting: None, other_window: false },
        )
        .await;
        let TaskResult::FileRead { contents, .. } = result else { panic!("{result:?}") };
        assert!(contents.is_empty(), "visiting a new file is not an error");
    }

    #[tokio::test]
    async fn reading_a_directory_is_reported_as_a_failure() {
        let f = Fixture::new("readdir").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ReadFile { path: f.path().join("src"), reverting: None, other_window: false },
        )
        .await;
        assert!(result.is_error(), "got {result:?}");
    }

    #[tokio::test]
    async fn writing_creates_the_file_and_reports_the_size() {
        let f = Fixture::new("write").await;
        let (mut e, mut rx) = executor(f.path());
        let path = f.path().join("out/deep.txt");
        let result = run_one(
            &mut e,
            &mut rx,
            Task::WriteFile {
                path: path.clone(),
                contents: "hello\n".into(),
                buffer: BufferId(1),
                backup: false,
                guard: maxgus_core::WriteGuard::Regardless,
            },
        )
        .await;
        let TaskResult::FileWritten { bytes, .. } = result else { panic!("{result:?}") };
        assert_eq!(bytes, 6);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "hello\n");
    }

    #[tokio::test]
    async fn a_backup_is_kept_when_the_setting_asks_for_one() {
        let f = Fixture::new("backup").await;
        let (mut e, mut rx) = executor(f.path());
        let path = f.path().join("src/main.rs");
        run_one(
            &mut e,
            &mut rx,
            Task::WriteFile {
                path: path.clone(),
                contents: "changed\n".into(),
                buffer: BufferId(1),
                backup: true,
                guard: maxgus_core::WriteGuard::Regardless,
            },
        )
        .await;
        let backup = tokio::fs::read_to_string(f.path().join("src/main.rs~")).await.unwrap();
        assert_eq!(backup, "fn main() {}\n", "the previous contents were kept");
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "changed\n");
    }

    #[tokio::test]
    async fn listing_a_directory_marks_the_directories() {
        let f = Fixture::new("list").await;
        let (mut e, mut rx) = executor(f.path());
        let result =
            run_one(&mut e, &mut rx, Task::ListDirectory { path: f.path().to_path_buf() }).await;
        let TaskResult::DirectoryListed { entries, .. } = result else { panic!("{result:?}") };
        assert!(entries.iter().any(|entry| entry.ends_with("/src/")), "got {entries:?}");
        assert!(entries.iter().any(|entry| entry.ends_with("Cargo.toml")), "got {entries:?}");
    }

    #[tokio::test]
    async fn listing_a_missing_directory_is_reported() {
        let (mut e, mut rx) = executor(Path::new("/nonexistent-maxgus-path"));
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ListDirectory { path: PathBuf::from("/nonexistent-maxgus-path") },
        )
        .await;
        assert!(result.is_error());
    }

    #[tokio::test]
    async fn a_tree_refresh_returns_a_snapshot() {
        let f = Fixture::new("tree").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let TaskResult::TreeUpdated { nodes, .. } = result else { panic!("{result:?}") };
        assert!(nodes.iter().any(|n| n.name == "src"), "got {:?}", nodes);
        assert!(nodes.iter().any(|n| n.name == "Cargo.toml"));
    }

    #[tokio::test]
    async fn expanding_a_directory_reveals_its_children() {
        let f = Fixture::new("treeexpand").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let result =
            run_one(&mut e, &mut rx, Task::Tree(TreeAction::Expand(f.path().join("src")))).await;
        let TaskResult::TreeUpdated { nodes, .. } = result else { panic!("{result:?}") };
        assert!(nodes.iter().any(|n| n.name == "main.rs"), "got {:?}", nodes);
    }

    #[tokio::test]
    async fn creating_through_the_tree_makes_the_file_and_selects_it() {
        let f = Fixture::new("treecreate").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::CreateFile {
                parent: f.path().to_path_buf(),
                name: "created.txt".into(),
            }),
        )
        .await;
        let TaskResult::TreeUpdated { select, nodes, .. } = result else { panic!("{result:?}") };
        assert_eq!(select, Some(f.path().join("created.txt")));
        assert!(nodes.iter().any(|n| n.name == "created.txt"));
        assert!(tokio::fs::try_exists(f.path().join("created.txt")).await.unwrap());
    }

    #[tokio::test]
    async fn renaming_through_the_tree_renames_the_node_it_was_given() {
        let f = Fixture::new("treerename").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Rename {
                path: f.path().join("Cargo.toml"),
                name: "Renamed.toml".into(),
            }),
        )
        .await;

        let TaskResult::TreeUpdated { select, nodes, .. } = result else { panic!("{result:?}") };
        assert_eq!(select, Some(f.path().join("Renamed.toml")));
        assert!(nodes.iter().any(|n| n.name == "Renamed.toml"));
        assert!(!tokio::fs::try_exists(f.path().join("Cargo.toml")).await.unwrap());
    }

    #[tokio::test]
    async fn deleting_through_the_tree_removes_the_node_it_was_given() {
        let f = Fixture::new("treedelete").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Delete(f.path().join("Cargo.toml"))),
        )
        .await;

        let TaskResult::TreeUpdated { select, nodes, .. } = result else { panic!("{result:?}") };
        assert_eq!(select, None, "nothing is selected after what was selected went");
        assert!(!nodes.iter().any(|n| n.name == "Cargo.toml"));
        assert!(!tokio::fs::try_exists(f.path().join("Cargo.toml")).await.unwrap());
        assert!(tokio::fs::try_exists(f.path().join("src")).await.unwrap(), "and nothing else");
    }

    #[tokio::test]
    async fn creating_a_directory_through_the_tree_makes_it() {
        let f = Fixture::new("treemkdir").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::CreateDirectory {
                parent: f.path().to_path_buf(),
                name: "made".into(),
            }),
        )
        .await;

        let TaskResult::TreeUpdated { select, nodes, .. } = result else { panic!("{result:?}") };
        assert_eq!(select, Some(f.path().join("made")));
        assert!(nodes.iter().any(|n| n.name == "made"));
        assert!(tokio::fs::metadata(f.path().join("made")).await.unwrap().is_dir());
    }

    #[tokio::test]
    async fn moving_through_the_tree_puts_it_in_the_destination() {
        let f = Fixture::new("treemove").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Move {
                path: f.path().join("Cargo.toml"),
                destination: f.path().join("src"),
            }),
        )
        .await;

        let TaskResult::TreeUpdated { select, .. } = result else { panic!("{result:?}") };
        assert_eq!(select, Some(f.path().join("src/Cargo.toml")));
        assert!(tokio::fs::try_exists(f.path().join("src/Cargo.toml")).await.unwrap());
        assert!(!tokio::fs::try_exists(f.path().join("Cargo.toml")).await.unwrap());
    }

    #[tokio::test]
    async fn stopping_a_server_that_was_never_started_is_quiet() {
        // The only exercise `stop_server` gets: nothing had been started, so
        // it must return without announcing a server that never existed.
        let f = Fixture::new("treestop").await;
        let (mut e, rx) = executor(f.path());
        e.handle(Task::StopLanguageServer { language: "rust".into() }).await;
        drop(e);
        let mut rx = rx;
        assert!(rx.try_recv().is_err(), "it reported stopping something that never ran");
    }

    #[tokio::test]
    async fn no_tree_action_acts_on_a_node_it_was_not_given() {
        // `at` positions the cursor, and every action below works on the
        // *selection*. Ignoring whether the positioning succeeded meant an
        // action naming a node the tree cannot see ran against whatever the
        // cursor happened to be on — asking to delete a file inside a
        // collapsed directory deleted the unrelated file the cursor was on.
        //
        // Every mutating action is checked, not just `Delete`: the guard was
        // added to all five, and covering one of them left the other four
        // free to lose it unnoticed.
        for action in [
            "delete",
            "rename",
            "move",
            "create-file",
            "create-directory",
        ] {
            let f = Fixture::new(&format!("treewrong-{action}")).await;
            let (mut e, mut rx) = executor(f.path());
            run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

            // The cursor goes on a real node first. Left on the root,
            // `delete_selected` would refuse outright and the guard being
            // absent would not show.
            run_one(
                &mut e,
                &mut rx,
                Task::Tree(TreeAction::Reveal(f.path().join("Cargo.toml"))),
            )
            .await;

            // `src` is collapsed, so what is inside it is not a visible node.
            let unseen = f.path().join("src/main.rs");
            let task = match action {
                "delete" => TreeAction::Delete(unseen.clone()),
                "rename" => TreeAction::Rename { path: unseen.clone(), name: "gone.txt".into() },
                // Into `src`, not the root: the cursor's node already lives
                // in the root, so moving it there would fail as "already
                // exists" and hide the guard being gone.
                "move" => {
                    TreeAction::Move { path: unseen.clone(), destination: f.path().join("src") }
                }
                "create-file" => {
                    TreeAction::CreateFile { parent: unseen.clone(), name: "made.txt".into() }
                }
                _ => TreeAction::CreateDirectory { parent: unseen.clone(), name: "made".into() },
            };
            run_one(&mut e, &mut rx, Task::Tree(task)).await;

            assert!(
                tokio::fs::try_exists(f.path().join("Cargo.toml")).await.unwrap(),
                "`{action}` touched the node the cursor was on"
            );
            assert!(
                !tokio::fs::try_exists(f.path().join("made.txt")).await.unwrap(),
                "`{action}` created something beside the cursor's node"
            );
            assert!(
                !tokio::fs::try_exists(f.path().join("made")).await.unwrap(),
                "`{action}` created something beside the cursor's node"
            );
        }
    }

    #[tokio::test]
    async fn a_failing_tree_action_still_returns_a_snapshot() {
        let f = Fixture::new("treefail").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        e.handle(Task::Tree(TreeAction::CreateFile {
            parent: f.path().to_path_buf(),
            name: "bad/name".into(),
        }))
        .await;
        let first = rx.try_recv().unwrap();
        assert!(first.is_error(), "the failure is reported");
        let second = rx.try_recv().unwrap();
        assert!(
            matches!(second, TaskResult::TreeUpdated { .. }),
            "and the tree is still redrawn"
        );
    }

    #[tokio::test]
    async fn toggling_hidden_files_changes_what_the_snapshot_holds() {
        let f = Fixture::new("treehidden").await;
        tokio::fs::write(f.path().join(".hidden"), "").await.unwrap();
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let TaskResult::TreeUpdated { nodes, show_hidden, .. } = result else { panic!() };
        assert!(!show_hidden);
        assert!(!nodes.iter().any(|n| n.name == ".hidden"));

        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::ToggleHidden)).await;
        let TaskResult::TreeUpdated { nodes, show_hidden, .. } = result else { panic!() };
        assert!(show_hidden);
        assert!(nodes.iter().any(|n| n.name == ".hidden"));
    }

    #[tokio::test]
    async fn reparsing_returns_highlight_spans() {
        let f = Fixture::new("parse").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: "fn main() { let x = 1; }".into(),
                revision: 7,
                range: 0..usize::MAX,
            },
        )
        .await;
        let TaskResult::Reparsed { revision, highlights, .. } = result else { panic!("{result:?}") };
        assert_eq!(revision, 7);
        assert!(!highlights.is_empty());
        assert!(highlights.iter().any(|h| h.face == "font-lock-keyword"));
    }

    #[tokio::test]
    async fn a_language_with_no_grammar_is_quietly_skipped() {
        let f = Fixture::new("parseunknown").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::Reparse {
            buffer: BufferId(1),
            language: "cobol".into(),
            text: "IDENTIFICATION DIVISION.".into(),
            revision: 1,
            range: 0..usize::MAX,
        })
        .await;
        assert!(rx.try_recv().is_err(), "no result, and no error either");
    }

    #[tokio::test]
    async fn each_buffer_keeps_its_own_parser() {
        let f = Fixture::new("parserperbuffer").await;
        let (mut e, mut rx) = executor(f.path());
        for (buffer, text) in [(1u64, "fn a() {}"), (2, "struct B;")] {
            let result = run_one(
                &mut e,
                &mut rx,
                Task::Reparse {
                    buffer: BufferId(buffer),
                    language: "rust".into(),
                    text: text.into(),
                    revision: 1,
                    range: 0..usize::MAX,
                },
            )
            .await;
            let TaskResult::Reparsed { highlights, .. } = result else { panic!() };
            assert!(!highlights.is_empty(), "`{text}` produced nothing");
        }
        // A parser's worth is the tree it holds, and a tree belongs to one
        // buffer; sharing would mean discarding it at every switch.
        assert_eq!(e.highlighters.len(), 2);
    }

    #[tokio::test]
    async fn a_reparse_keeps_the_text_its_tree_describes() {
        let f = Fixture::new("parsetext").await;
        let (mut e, mut rx) = executor(f.path());
        let reparse = |text: &str, revision: u64| Task::Reparse {
            buffer: BufferId(1),
            language: "rust".into(),
            text: text.into(),
            revision,
            range: 0..usize::MAX,
        };

        run_one(&mut e, &mut rx, reparse("fn a() {}", 1)).await;
        assert_eq!(e.highlighters[&BufferId(1)].text, "fn a() {}");

        // The second parse is handed the region that changed, and the stored
        // text moves on with it.
        run_one(&mut e, &mut rx, reparse("fn ab() {}", 2)).await;
        assert_eq!(e.highlighters[&BufferId(1)].text, "fn ab() {}");
    }

    #[tokio::test]
    async fn an_edit_between_reparses_still_highlights_correctly() {
        let f = Fixture::new("parseincremental").await;
        let (mut e, mut rx) = executor(f.path());
        let before = "fn main() { let x = 1; }";
        let after = "fn main() { let renamed = 1; }";

        run_one(
            &mut e,
            &mut rx,
            Task::Reparse { buffer: BufferId(1), language: "rust".into(), text: before.into(), revision: 1, range: 0..usize::MAX },
        )
        .await;
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse { buffer: BufferId(1), language: "rust".into(), text: after.into(), revision: 2, range: 0..usize::MAX },
        )
        .await;
        let TaskResult::Reparsed { highlights, .. } = result else { panic!("{result:?}") };

        // An incremental parse must produce the same answer a full one would.
        let mut fresh = Highlighter::new("rust").unwrap();
        fresh.parse(after).unwrap();
        assert_eq!(highlights, fresh.highlights(after));
    }

    #[tokio::test]
    async fn a_buffer_that_changes_language_starts_over() {
        let f = Fixture::new("parselanguage").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(
            &mut e,
            &mut rx,
            Task::Reparse { buffer: BufferId(1), language: "rust".into(), text: "fn a() {}".into(), revision: 1, range: 0..usize::MAX },
        )
        .await;
        // `write-file` under a new name can change a buffer's language.
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse { buffer: BufferId(1), language: "python".into(), text: "def a(): pass".into(), revision: 2, range: 0..usize::MAX },
        )
        .await;
        let TaskResult::Reparsed { highlights, .. } = result else { panic!() };
        assert!(!highlights.is_empty(), "the new grammar was used");
        assert_eq!(e.highlighters[&BufferId(1)].language, "python");
    }

    #[tokio::test]
    async fn forgetting_a_buffer_releases_its_parser_and_its_text() {
        let f = Fixture::new("parseforget").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(
            &mut e,
            &mut rx,
            Task::Reparse { buffer: BufferId(1), language: "rust".into(), text: "fn a() {}".into(), revision: 1, range: 0..usize::MAX },
        )
        .await;
        assert_eq!(e.highlighters.len(), 1);
        e.handle(Task::ForgetBuffer { buffer: BufferId(1) }).await;
        assert!(e.highlighters.is_empty(), "a killed buffer must not be held onto");
    }

    #[tokio::test]
    async fn a_shell_command_returns_its_output_and_status() {
        let f = Fixture::new("shell").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Shell {
                command: "echo hello".into(),
                directory: f.path().to_path_buf(),
                insert_at: None,
            },
        )
        .await;
        let TaskResult::ShellOutput { output, status, .. } = result else { panic!("{result:?}") };
        assert_eq!(output.trim(), "hello");
        assert_eq!(status, 0);
    }

    #[tokio::test]
    async fn a_failing_shell_command_reports_its_status_and_message() {
        let f = Fixture::new("shellfail").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Shell {
                command: "echo oops >&2; exit 3".into(),
                directory: f.path().to_path_buf(),
                insert_at: None,
            },
        )
        .await;
        let TaskResult::ShellOutput { output, status, .. } = result else { panic!("{result:?}") };
        assert_eq!(status, 3);
        assert!(output.contains("oops"), "stderr is shown too");
    }

    #[tokio::test]
    async fn a_shell_command_runs_in_the_directory_it_was_given() {
        let f = Fixture::new("shellcwd").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Shell { command: "pwd".into(), directory: f.path().join("src"), insert_at: None },
        )
        .await;
        let TaskResult::ShellOutput { output, .. } = result else { panic!() };
        assert!(output.trim().ends_with("/src"), "got `{output}`");
    }

    #[tokio::test]
    async fn a_request_to_a_server_that_is_not_running_is_a_no_op() {
        let f = Fixture::new("noserver").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::LspRequest {
            language: "rust".into(),
            uri: "file:///a.rs".into(),
            query: LspQuery::DocumentSymbols,
        })
        .await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn starting_a_server_with_no_configuration_does_nothing() {
        let f = Fixture::new("nospec").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::StartLanguageServer { language: "rust".into() }).await;
        assert!(rx.try_recv().is_err(), "an unconfigured language is not an error");
    }

    #[tokio::test]
    async fn starting_a_server_that_cannot_be_launched_is_reported() {
        let f = Fixture::new("badserver").await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let spec = LspSpec::new("rust", "maxgus-no-such-server");
        let mut e = Executor::new(
            f.path().to_path_buf(),
            TreeConfig::default(),
            vec![spec],
            tx,
        );
        e.handle(Task::StartLanguageServer { language: "rust".into() }).await;
        let result = rx.try_recv().expect("a failure was reported");
        assert!(result.is_error());
    }

    #[test]
    fn a_change_is_described_as_the_region_that_differs() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        let previous = "fn main() {\n    let x = 1;\n}\n";
        let current = "fn main() {\n    let renamed = 1;\n}\n";

        let (range, replacement) = changed_range(previous, current, encoding).unwrap();
        // Only the identifier changed, so only it is described.
        assert_eq!(range.start.line, 1);
        assert_eq!(range.end.line, 1);
        assert_eq!(replacement, "renamed");

        // Applying the change to the old text must reproduce the new one.
        let start = maxgus_lsp::position::position_to_offset(previous, range.start, encoding);
        let end = maxgus_lsp::position::position_to_offset(previous, range.end, encoding);
        let mut rebuilt: String = previous.chars().take(start).collect();
        rebuilt.push_str(&replacement);
        rebuilt.extend(previous.chars().skip(end));
        assert_eq!(rebuilt, current);
    }

    #[test]
    fn identical_texts_produce_no_change_to_report() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        assert!(changed_range("same", "same", encoding).is_none());
    }

    #[test]
    fn a_multiline_change_spans_the_lines_it_touches() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        let previous = "one\ntwo\nthree\nfour\n";
        let current = "one\nreplaced\nfour\n";
        let (range, replacement) = changed_range(previous, current, encoding).unwrap();

        let start = maxgus_lsp::position::position_to_offset(previous, range.start, encoding);
        let end = maxgus_lsp::position::position_to_offset(previous, range.end, encoding);
        let mut rebuilt: String = previous.chars().take(start).collect();
        rebuilt.push_str(&replacement);
        rebuilt.extend(previous.chars().skip(end));
        assert_eq!(rebuilt, current, "the described change does not reproduce the text");
    }

    #[test]
    fn a_change_in_multibyte_text_is_described_correctly() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        let previous = "let s = \"héllo wörld\";\n";
        let current = "let s = \"héllo 🎉 wörld\";\n";
        let (range, replacement) = changed_range(previous, current, encoding).unwrap();

        let start = maxgus_lsp::position::position_to_offset(previous, range.start, encoding);
        let end = maxgus_lsp::position::position_to_offset(previous, range.end, encoding);
        let mut rebuilt: String = previous.chars().take(start).collect();
        rebuilt.push_str(&replacement);
        rebuilt.extend(previous.chars().skip(end));
        assert_eq!(rebuilt, current);
    }

    #[test]
    fn a_changed_region_is_a_fraction_of_a_large_document() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        let previous: String = (0..5_000).map(|n| format!("line {n}\n")).collect();
        let at = previous.len() / 2;
        let at = (at..previous.len()).find(|i| previous.is_char_boundary(*i)).unwrap();
        let mut current = previous.clone();
        current.insert(at, 'x');

        let (_, replacement) = changed_range(&previous, &current, encoding).unwrap();
        // Sending the whole document is what incremental sync exists to avoid.
        assert!(
            replacement.len() < 32,
            "one typed character produced a {} byte change",
            replacement.len()
        );
    }

    #[tokio::test]
    async fn the_branch_of_a_real_repository_is_reported() {
        // The one test that runs git itself, since the branch reaching the
        // mode line depends on what git actually prints.
        let f = Fixture::new("gitbranch").await;
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(f.path())
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !run(&["init", "--initial-branch=trunk"]) {
            eprintln!("skipping: git is not available");
            return;
        }
        // A branch only exists once something is committed to it.
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "--allow-empty", "-m", "x"]);

        let (mut e, mut rx) = executor(f.path());
        let result =
            run_one(&mut e, &mut rx, Task::GitBranch { root: f.path().to_path_buf() }).await;
        let TaskResult::GitBranch { branch } = result else { panic!("{result:?}") };
        assert_eq!(branch.as_deref(), Some("trunk"));
    }

    #[tokio::test]
    async fn a_directory_outside_any_repository_reports_no_branch() {
        let f = Fixture::new("gitnone").await;
        let (mut e, mut rx) = executor(f.path());
        let result =
            run_one(&mut e, &mut rx, Task::GitBranch { root: f.path().to_path_buf() }).await;
        let TaskResult::GitBranch { branch } = result else { panic!("{result:?}") };
        assert_eq!(branch, None, "a plain directory has no branch");
    }

    #[tokio::test]
    async fn a_project_root_is_found_by_walking_upwards() {
        // The workspace this test is compiled in is itself a good fixture.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = find_upwards(here, "Cargo.toml").await.expect("this crate has one");
        assert!(found.join("Cargo.toml").exists());
        assert!(find_upwards(here, "no-such-marker-file").await.is_none());
    }
}

#[cfg(test)]
mod scale {
    use super::*;
    use maxgus_text::BufferId;
    use std::time::Instant;

    /// A file large enough that a full parse is visible to a person.
    fn source(lines: usize) -> String {
        (0..lines)
            .map(|n| format!("fn function_{n}(argument: &str) -> usize {{ argument.len() + {n} }}\n"))
            .collect()
    }

    /// Typing a character into the middle of `text`.
    fn typed_into(text: &str) -> String {
        let at = text.len() / 2;
        let at = (at..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
        let mut edited = text.to_string();
        edited.insert(at, 'x');
        edited
    }

    #[tokio::test]
    async fn parsing_does_not_stop_the_runtime_polling_anything_else() {
        // `#[tokio::test]` gives a single-threaded runtime, which is what
        // makes this decisive: run on that thread, a parse with no await in
        // it starves every other task until it finishes. Off on the blocking
        // pool, the await yields and they carry on.
        //
        // A quarter of a second of that would stall the language server's
        // transport and the terminal's input, which is the whole reason the
        // editor is on tokio at all.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut executor = Executor::new(
            PathBuf::from("/tmp"),
            TreeConfig { git_status: false, ..Default::default() },
            Vec::new(),
            tx,
        );

        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        tokio::spawn(async move { flag.store(true, Ordering::SeqCst) });

        let text = source(if cfg!(debug_assertions) { 3_000 } else { 20_000 });
        executor
            .handle(Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: text.clone(),
                revision: 1,
                range: 0..text.len().min(80 * 160),
            })
            .await;

        assert!(
            ran.load(Ordering::SeqCst),
            "nothing else was polled while the file was parsed"
        );
        rx.try_recv().expect("the parse still produced highlights");
    }

    #[tokio::test]
    async fn a_reparse_after_typing_costs_far_less_than_the_first_one() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut executor = Executor::new(
            PathBuf::from("/tmp"),
            TreeConfig { git_status: false, ..Default::default() },
            Vec::new(),
            tx,
        );

        let text = source(if cfg!(debug_assertions) { 3_000 } else { 20_000 });
        // A window's worth plus the scroll margin, which is what the editor
        // asks for; highlighting the whole file would cost more than parsing
        // it and almost none of it would be drawn.
        let window = 0..text.len().min(80 * 160);
        let reparse = |text: &str, revision: u64| Task::Reparse {
            buffer: BufferId(1),
            language: "rust".into(),
            text: text.into(),
            revision,
            range: window.clone(),
        };

        let start = Instant::now();
        executor.handle(reparse(&text, 1)).await;
        let first = start.elapsed();
        rx.try_recv().expect("the first parse produced highlights");

        let edited = typed_into(&text);
        let start = Instant::now();
        executor.handle(reparse(&edited, 2)).await;
        let second = start.elapsed();
        rx.try_recv().expect("the second parse produced highlights");

        println!("first parse:  {first:>8.2?}");
        println!("after typing: {second:>8.2?}");
        let ratio = first.as_secs_f64() / second.as_secs_f64().max(1e-9);
        println!("cheaper by:   {ratio:>8.1}x");

        // The executor used to discard the tree before every parse, so every
        // pause in typing cost a full parse — a visible freeze on a large
        // file. If that comes back, this ratio collapses to one.
        assert!(
            ratio > 3.0,
            "a reparse after typing was only {ratio:.1}x cheaper than the first; \
             the syntax tree is being thrown away between parses"
        );
    }
}
