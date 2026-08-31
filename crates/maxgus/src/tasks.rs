//! The task executor.
//!
//! Everything the editor cannot do synchronously ends up here: reading and
//! writing files, walking the project tree, running tree-sitter, talking to
//! language servers, running shell commands. The executor owns those resources
//! outright and runs in its own tokio task, taking [`Task`]s from a channel and
//! sending [`TaskResult`]s back. The editor never blocks on any of it.

use anyhow::Result;
use maxgus_config::{LspSpec, TreeConfig};
#[cfg(feature = "full")]
use maxgus_core::task::LspQuery;
#[cfg(feature = "full")]
use maxgus_core::task::TerminalId;
use maxgus_core::task::{EditorConfig, Task, TaskResult, TreeAction};
#[cfg(feature = "full")]
use maxgus_core::task::{GitAction, GitSnapshot};
#[cfg(feature = "full")]
use maxgus_lsp::{Client, ServerEvent};
#[cfg(feature = "full")]
use maxgus_syntax::Highlighter;
use maxgus_tree::FileTree;
#[cfg(feature = "full")]
use std::collections::HashMap;
#[cfg(feature = "full")]
use std::io::Write as _;
use std::path::{Path, PathBuf};
#[cfg(feature = "full")]
use std::sync::Arc;
use tokio::sync::mpsc;

/// A buffer's parser, and the text its tree describes.
#[cfg(feature = "full")]
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
    #[cfg(feature = "full")]
    highlighters: HashMap<maxgus_text::BufferId, BufferSyntax>,
    /// The grammars this editor can reach: compiled in, plus whatever the
    /// configuration pointed it at.
    #[cfg(feature = "full")]
    grammars: maxgus_syntax::Grammars,
    /// Running language servers, by language.
    #[cfg(feature = "full")]
    servers: HashMap<String, Arc<Client>>,
    /// The text each open document was last sent as, so a change can be
    /// described as the region that differs rather than the whole file.
    #[cfg(feature = "full")]
    documents: HashMap<String, String>,
    #[cfg(feature = "full")]
    lsp_specs: Vec<LspSpec>,
    /// Shells running on pseudo-terminals, by tab.
    #[cfg(feature = "full")]
    terminals: HashMap<TerminalId, Terminal>,
    results: mpsc::UnboundedSender<TaskResult>,
}

#[cfg(feature = "full")]
/// One running shell: what to write to it, and how to change its size.
///
/// The reading half is not here. A pty read blocks until the program writes
/// something, which may be never, so it lives on its own blocking thread that
/// pushes straight down the results channel.
struct Terminal {
    commands: std::sync::mpsc::Sender<PtyCommand>,
}

#[cfg(feature = "full")]
/// What the thread minding a pty can be asked to do.
///
/// The pty handles never leave that thread. Writing to a pty can block when
/// the program is not reading, and resizing and killing both talk to the same
/// handles, so all three go down one channel and are done in order by the
/// thread that owns them — rather than behind a lock the runtime could end up
/// waiting on.
enum PtyCommand {
    Write(Vec<u8>),
    Resize(u16, u16),
    Close,
}

impl Executor {
    /// An executor that looks for no grammars beyond the compiled-in ones,
    /// which is every caller that is not reading a configuration file.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(
        root: PathBuf,
        tree_config: TreeConfig,
        #[cfg_attr(not(feature = "full"), allow(unused_variables))] lsp_specs: Vec<LspSpec>,
        results: mpsc::UnboundedSender<TaskResult>,
    ) -> Executor {
        Executor::with_grammars(root, tree_config, lsp_specs, Default::default(), results)
    }

    /// The same, told where to look for grammars the editor was not built
    /// with. Nothing is looked for unless this says where.
    pub fn with_grammars(
        root: PathBuf,
        tree_config: TreeConfig,
        #[cfg_attr(not(feature = "full"), allow(unused_variables))] lsp_specs: Vec<LspSpec>,
        #[cfg_attr(not(feature = "full"), allow(unused_variables))]
        grammars: maxgus_config::GrammarConfig,
        results: mpsc::UnboundedSender<TaskResult>,
    ) -> Executor {
        Executor {
            root,
            tree: None,
            tree_config,
            #[cfg(feature = "full")]
            highlighters: HashMap::new(),
            #[cfg(feature = "full")]
            grammars: maxgus_syntax::Grammars::new(maxgus_syntax::Search {
                libraries: grammars.search,
                queries: grammars.queries,
                named: grammars
                    .named
                    .into_iter()
                    .map(|g| maxgus_syntax::Named {
                        language: g.language,
                        library: g.library,
                        queries: g.queries,
                    })
                    .collect(),
            }),
            #[cfg(feature = "full")]
            servers: HashMap::new(),
            #[cfg(feature = "full")]
            terminals: HashMap::new(),
            #[cfg(feature = "full")]
            documents: HashMap::new(),
            #[cfg(feature = "full")]
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
        self.send(TaskResult::Failed {
            context: context.to_string(),
            message: error.to_string(),
        });
    }

    async fn handle(&mut self, task: Task) {
        match task {
            Task::ReadFile {
                path,
                reverting,
                other_window,
            } => {
                self.read_file(path, reverting, other_window).await;
            }
            Task::WriteFile {
                path,
                contents,
                buffer,
                backup,
                guard,
            } => {
                self.write_file(path, contents, buffer, backup, guard).await;
            }
            Task::ListDirectory { path } => self.list_directory(path).await,
            Task::Tree(action) => self.tree_action(action).await,
            #[cfg(feature = "full")]
            Task::Reparse {
                buffer,
                language,
                text,
                revision,
                range,
            } => {
                self.reparse(buffer, &language, text, revision, range).await;
            }
            #[cfg(feature = "full")]
            Task::DescribeGrammars => {
                let report = self.grammar_report();
                self.send(TaskResult::Grammars { report });
            }
            Task::Dired { path } => self.dired(path).await,
            Task::Browse { path } => self.browse(path).await,
            Task::FindDirectories { root } => self.find_directories(root).await,
            Task::DiredAct { action } => self.dired_act(action).await,
            #[cfg(feature = "full")]
            Task::ReadScript { path } => self.read_script(path).await,
            Task::SaveSession { path, contents } => self.save_session(path, contents).await,
            Task::ReadSession { path } => self.read_session(path).await,
            Task::SaveWorkspaces { path, contents } => self.save_workspaces(path, contents).await,
            Task::ReadWorkspaces { path } => self.read_workspaces(path).await,
            Task::PersistTheme { path, theme } => {
                self.persist_theme(path, theme).await;
            }
            #[cfg(feature = "full")]
            Task::GitBranch { root } => {
                let branch = maxgus_tree::git::branch(&root).await;
                self.send(TaskResult::GitBranch { branch });
            }
            #[cfg(feature = "full")]
            Task::StartLanguageServer { language } => self.start_server(&language).await,
            #[cfg(feature = "full")]
            Task::StopLanguageServer { language } => self.stop_server(&language).await,
            #[cfg(feature = "full")]
            Task::LspDidOpen {
                language,
                uri,
                version,
                text,
            } => {
                if let Some(client) = self.servers.get(&language) {
                    let file_language = language.clone();
                    let _ = client.did_open(&uri, &file_language, version, &text);
                    self.documents.insert(uri, text);
                }
            }
            #[cfg(feature = "full")]
            Task::LspDidChange {
                language,
                uri,
                version,
                text,
            } => {
                self.did_change(&language, uri, version, text).await;
            }
            #[cfg(feature = "full")]
            Task::LspDidSave { language, uri } => {
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.did_save(&uri, None);
                }
            }
            #[cfg(feature = "full")]
            Task::LspDidClose { language, uri } => {
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.did_close(&uri);
                }
                self.documents.remove(&uri);
            }
            #[cfg(feature = "full")]
            Task::LspRequest {
                language,
                uri,
                query,
                announced,
            } => self.lsp_request(language, uri, query, announced),
            #[cfg(feature = "full")]
            Task::LspRespond {
                language,
                id,
                applied,
            } => {
                // The editor has finished with the edit the server asked for;
                // tell the server whether it went in.
                if let Some(client) = self.servers.get(&language) {
                    let _ = client.respond(id, serde_json::json!({ "applied": applied }));
                }
            }
            Task::Shell {
                command,
                directory,
                insert_at,
            } => {
                self.shell(command, directory, insert_at).await;
            }
            #[cfg(feature = "full")]
            Task::TerminalOpen {
                terminal,
                shell,
                directory,
                rows,
                columns,
            } => {
                self.open_terminal(terminal, shell, directory, rows, columns);
            }
            #[cfg(feature = "full")]
            Task::TerminalInput { terminal, bytes } => self.terminal_input(terminal, bytes),
            #[cfg(feature = "full")]
            Task::TerminalResize {
                terminal,
                rows,
                columns,
            } => {
                self.resize_terminal(terminal, rows, columns);
            }
            #[cfg(feature = "full")]
            Task::TerminalClose { terminal } => self.close_terminal(terminal),
            #[cfg(feature = "full")]
            Task::Git { root, action } => self.git(root, action).await,
            #[cfg(feature = "full")]
            Task::Grep { root, search } => self.grep(root, search).await,
            #[cfg(feature = "full")]
            Task::ApplyGrep { replacements } => self.apply_grep(replacements).await,
            Task::ForgetBuffer { buffer } => self.forget(buffer),
        }
    }

    // ---- files ---------------------------------------------------------

    async fn read_file(
        &self,
        path: PathBuf,
        reverting: Option<maxgus_text::BufferId>,
        other_window: bool,
    ) {
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
                let read_only = lossy
                    || metadata
                        .as_ref()
                        .is_some_and(|m| m.permissions().readonly());
                let disk_time = metadata.and_then(|m| m.modified().ok());
                // Reading `.editorconfig` walks up the tree looking at files,
                // which is blocking work and belongs off the runtime.
                let asked = {
                    let path = path.clone();
                    tokio::task::spawn_blocking(move || Executor::editor_config(&path))
                        .await
                        .unwrap_or_default()
                };
                self.send(TaskResult::FileRead {
                    path,
                    contents,
                    read_only,
                    lossy,
                    disk_time,
                    reverting,
                    other_window,
                    editor_config: asked,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Visiting a file that does not exist yet creates an empty
                // buffer for it, as `find-file` does.
                // A file that does not exist yet still belongs to a project
                // that has an opinion about how it should be written.
                let asked = {
                    let path = path.clone();
                    tokio::task::spawn_blocking(move || Executor::editor_config(&path))
                        .await
                        .unwrap_or_default()
                };
                self.send(TaskResult::FileRead {
                    path,
                    contents: String::new(),
                    read_only: false,
                    lossy: false,
                    disk_time: None,
                    reverting,
                    other_window,
                    editor_config: asked,
                });
            }
            Err(error) => self.fail("find-file", error),
        }
    }

    // ---- directories ---------------------------------------------------

    /// Lists a directory with the detail dired shows.
    /// Lists a directory for the file browser.
    ///
    /// The same reading dired does, sent to a different place. A directory
    /// that will not open is reported and nothing is sent, so the browser
    /// stays where it was rather than emptying itself over a typo.
    async fn browse(&self, path: PathBuf) {
        match Self::listing(&path).await {
            Ok(entries) => self.send(TaskResult::Browsed { path, entries }),
            Err(error) => self.fail(&format!("browse {}", path.display()), error),
        }
    }

    /// Every directory under `root`, for the browser to narrow by typing.
    ///
    /// Breadth first, so what turns up first is what is nearest the top —
    /// the thing being looked for is far more often two directories down
    /// than ten, and a walk that has to be capped should be capped at the
    /// far end rather than the near one.
    async fn find_directories(&self, root: PathBuf) {
        /// Deep enough to reach a project inside a couple of levels of
        /// grouping, shallow enough not to wander into a source tree.
        const DEPTH: usize = 6;
        /// Enough to hold anyone's projects, and a bound on the memory and
        /// the time either way.
        const MOST: usize = 20_000;

        let mut paths: Vec<String> = Vec::new();
        let mut queue = std::collections::VecDeque::from([(root.clone(), 0usize)]);
        let mut capped = false;
        while let Some((directory, depth)) = queue.pop_front() {
            if paths.len() >= MOST {
                capped = true;
                break;
            }
            let Ok(mut reader) = tokio::fs::read_dir(&directory).await else {
                // Unreadable is not a failure here: somewhere under a home
                // directory there is always something the owner cannot open,
                // and one of them should not end the search.
                continue;
            };
            while let Ok(Some(entry)) = reader.next_entry().await {
                // `file_type` rather than `metadata`, so a symlink reads as a
                // symlink instead of as whatever it points at. Following them
                // is how a walk finds the same tree twice, or itself.
                if !entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if skip(&name) {
                    continue;
                }
                let path = directory.join(&name);
                if let Ok(relative) = path.strip_prefix(&root) {
                    paths.push(relative.to_string_lossy().into_owned());
                }
                if depth + 1 < DEPTH {
                    queue.push_back((path, depth + 1));
                }
            }
        }
        capped |= paths.len() >= MOST;
        paths.truncate(MOST);
        paths.sort();
        self.send(TaskResult::DirectoriesFound {
            root,
            paths,
            capped,
        });
    }

    async fn dired(&self, path: PathBuf) {
        match Self::listing(&path).await {
            Ok(entries) => self.send(TaskResult::DiredListed { path, entries }),
            Err(error) => self.fail(&format!("dired {}", path.display()), error),
        }
    }

    /// What is in a directory, with the detail dired shows.
    async fn listing(path: &Path) -> std::io::Result<Vec<maxgus_core::dired::Entry>> {
        let mut entries = Vec::new();
        let mut reader = tokio::fs::read_dir(path).await?;
        while let Ok(Some(entry)) = reader.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            // `metadata` follows links, so what it says about a link is what
            // it says about the target. Where it points is read separately.
            let link = tokio::fs::read_link(entry.path())
                .await
                .ok()
                .map(|target| target.to_string_lossy().into_owned());
            entries.push(maxgus_core::dired::Entry {
                name,
                is_dir: metadata.is_dir(),
                link,
                size: metadata.len(),
                permissions: permissions_of(&metadata),
                modified: modified_of(&metadata),
            });
        }
        Ok(entries)
    }

    /// Does what dired asked, and says which directory to list again.
    async fn dired_act(&self, action: maxgus_core::task::FileAction) {
        use maxgus_core::task::FileAction;
        let said = action.describe();
        let relist = match &action {
            FileAction::Delete(paths) | FileAction::Chmod { paths, .. } => paths
                .first()
                .and_then(|p| p.parent())
                .map(std::path::Path::to_path_buf),
            FileAction::Copy { from, .. } | FileAction::Rename { from, .. } => from
                .first()
                .and_then(|p| p.parent())
                .map(std::path::Path::to_path_buf),
            FileAction::CreateDirectory(path) => path.parent().map(std::path::Path::to_path_buf),
        };
        let outcome = match action {
            FileAction::Delete(paths) => delete_all(&paths).await,
            FileAction::Copy { from, to } => copy_all(&from, &to).await,
            FileAction::Rename { from, to } => rename_all(&from, &to).await,
            FileAction::CreateDirectory(path) => tokio::fs::create_dir_all(&path).await,
            FileAction::Chmod { .. } => Ok(()),
        };
        match (outcome, relist) {
            (Ok(()), Some(relist)) => self.send(TaskResult::DiredDone { said, relist }),
            (Ok(()), None) => self.send(TaskResult::Failed {
                context: "dired".into(),
                message: "nowhere to list again".into(),
            }),
            (Err(error), _) => self.fail("dired", error),
        }
    }

    /// Reads the script file. A project with none is the usual case and not
    /// a failure.
    #[cfg(feature = "full")]
    async fn read_script(&self, path: PathBuf) {
        match tokio::fs::read_to_string(&path).await {
            Ok(source) => self.send(TaskResult::ScriptRead { source, path }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.fail("reading the script", error),
        }
    }

    // ---- sessions ------------------------------------------------------

    async fn save_session(&self, path: PathBuf, contents: String) {
        if let Some(parent) = path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            self.fail("saving the session", error);
            return;
        }
        match tokio::fs::write(&path, contents).await {
            Ok(()) => self.send(TaskResult::SessionSaved { path }),
            Err(error) => self.fail("saving the session", error),
        }
    }

    /// Reads a session back. A project that has never been opened has none,
    /// which is not a failure and is reported as an empty session.
    async fn read_session(&self, path: PathBuf) {
        let session = match tokio::fs::read_to_string(&path).await {
            Ok(source) => maxgus_core::session::Session::from_kdl(&source),
            Err(_) => maxgus_core::session::Session::default(),
        };
        self.send(TaskResult::SessionRead { session });
    }

    async fn save_workspaces(&self, path: PathBuf, contents: String) {
        if let Some(parent) = path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            self.fail("saving the workspaces", error);
            return;
        }
        if let Err(error) = tokio::fs::write(&path, contents).await {
            self.fail("saving the workspaces", error);
        }
    }

    /// Reads them back. Nobody having saved one is not a failure and is
    /// reported as none, the way a project with no session is.
    async fn read_workspaces(&self, path: PathBuf) {
        let workspaces = match tokio::fs::read_to_string(&path).await {
            Ok(source) => maxgus_core::workspace::Workspaces::from_kdl(&source),
            Err(_) => maxgus_core::workspace::Workspaces::default(),
        };
        self.send(TaskResult::WorkspacesRead { workspaces });
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
                self.fail("save-theme", error);
                return;
            }
        };
        let updated = with_theme(&source, &theme);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            self.fail("save-theme", error);
            return;
        }
        match tokio::fs::write(&path, updated).await {
            Ok(()) => self.send(TaskResult::ThemePersisted { path, theme }),
            Err(error) => self.fail("save-theme", error),
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
            maxgus_core::WriteGuard::Absent => tokio::fs::try_exists(&path).await.unwrap_or(false),
            maxgus_core::WriteGuard::Unchanged(expect) => match expect {
                Some(expect) => tokio::fs::metadata(&path)
                    .await
                    .is_ok_and(|m| m.modified().is_ok_and(|now| now != expect)),
                None => false,
            },
        };
        if refuse {
            self.send(TaskResult::WriteRefused {
                path,
                buffer,
                because: guard,
            });
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
                let disk_time = tokio::fs::metadata(&path)
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok());
                self.send(TaskResult::FileWritten {
                    path,
                    buffer,
                    bytes,
                    disk_time,
                });
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
        let Some(tree) = self.tree.as_mut() else {
            return;
        };

        // Where the cursor should end up, when the action is one that moves
        // it. `None` leaves it where the user put it — which is nearly
        // always, and which used to be the executor's own idea of what was
        // selected: stale, usually the root, and the reason expanding a
        // directory sent the cursor back to the top of the tree.
        let mut select: Option<PathBuf> = None;
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
            TreeAction::SetRoot { from, to } => {
                // The one that moved, not all of them: the others are
                // separate directories somebody asked to see, and a command
                // that says it moves *the* root should not take them away.
                //
                // The new root is what the cursor should be on — it is the
                // thing that just moved, and the top of a tree nobody has
                // looked at yet is where anyone would look first anyway.
                select = Some(to.clone());
                tree.replace_root(&from, to).await
            }
            TreeAction::AddRoot(path) => {
                select = Some(path.clone());
                tree.add_root(path).await
            }
            TreeAction::RemoveRoot(path) => {
                select = None;
                tree.remove_root(&path)
            }
            TreeAction::SetRoots(directories) => {
                select = directories.first().cloned();
                match tree.set_roots(directories).await {
                    // Directories that have moved or gone are dropped
                    // rather than refused, and said out loud: a workspace
                    // outlives the disk it was saved on, and silently
                    // showing three of four is how someone comes to think
                    // they deleted something.
                    Ok(dropped) if !dropped.is_empty() => {
                        let names: Vec<String> = dropped
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect();
                        self.send(TaskResult::Said(format!(
                            "Not readable, left out: {}",
                            names.join(", ")
                        )));
                        Ok(())
                    }
                    Ok(_) => Ok(()),
                    Err(error) => Err(error),
                }
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
                Ok(()) => tree
                    .create_file(&name)
                    .await
                    .map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::CreateDirectory { parent, name } => match Self::at(tree, &parent) {
                Ok(()) => tree
                    .create_directory(&name)
                    .await
                    .map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::Delete(path) => match Self::at(tree, &path) {
                Ok(()) => tree.delete_selected().await.map(|_| select = None),
                Err(error) => Err(error),
            },
            TreeAction::Rename { path, name } => match Self::at(tree, &path) {
                Ok(()) => tree
                    .rename_selected(&name)
                    .await
                    .map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
            TreeAction::Move { path, destination } => match Self::at(tree, &path) {
                Ok(()) => tree
                    .move_selected(&destination)
                    .await
                    .map(|path| select = Some(path)),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = outcome {
            self.fail("treefile", error);
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
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

    #[cfg(feature = "full")]
    // ---- syntax --------------------------------------------------------
    #[cfg(feature = "full")]
    /// The grammar for `language`, loading it from disk the first time if
    /// the configuration said where to look.
    ///
    /// Opening a shared library reads from the disk and runs the library's
    /// own initialisers, so it goes to a blocking thread. Doing it on the
    /// runtime would stall every other task in the editor for as long as it
    /// took — which is the whole reason `maxgus-syntax/src/dynamic.rs` is on
    /// the list of files allowed to block, and why this is the only way in.
    #[cfg(feature = "full")]
    async fn grammar_for(&mut self, language: &str) -> Option<maxgus_syntax::SyntaxLanguage> {
        let search = match self.grammars.ready(language) {
            maxgus_syntax::Ready::Have(grammar) => return Some(grammar),
            maxgus_syntax::Ready::Absent => return None,
            maxgus_syntax::Ready::MustLoad(search) => search,
        };
        let name = language.to_string();
        let outcome =
            tokio::task::spawn_blocking(move || maxgus_syntax::dynamic::load(&name, &search))
                .await
                .ok()?;
        let grammar = match &outcome {
            Ok(grammar) => Some(grammar.clone()),
            Err(_) => None,
        };
        self.grammars.remember(language, outcome);
        grammar
    }

    /// What `describe-grammars` shows: what is built in, what was loaded,
    /// and what would not load and why — which is the only way to find out
    /// that a path in the configuration has a typo in it.
    #[cfg(feature = "full")]
    fn grammar_report(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("Tree-sitter grammars\n\n");
        out.push_str("Compiled in\n");
        for name in maxgus_syntax::supported_languages() {
            let _ = writeln!(out, "  {name}");
        }

        let search = self.grammars.search();
        out.push_str("\nLoaded from disk\n");
        match search.is_empty() {
            true => out.push_str(
                "  none — no `grammars` block in the configuration, so none\n                   are looked for. See docs/grammars.md.\n",
            ),
            false => {
                let loaded = self.grammars.loaded();
                match loaded.is_empty() {
                    true => out.push_str("  none yet\n"),
                    false => {
                        for name in loaded {
                            let _ = writeln!(out, "  {name}");
                        }
                    }
                }
                let failures = self.grammars.failures();
                if !failures.is_empty() {
                    out.push_str("\nWould not load\n");
                    for (name, why) in failures {
                        let _ = writeln!(out, "  {name}: {why}");
                    }
                }
                out.push_str("\nLooked for in\n");
                for path in &search.libraries {
                    let _ = writeln!(out, "  {}", path.display());
                }
                if !search.named.is_empty() {
                    out.push_str("\nNamed outright\n");
                    for named in &search.named {
                        let _ = writeln!(out, "  {}: {}", named.language, named.library.display());
                    }
                }
                out.push_str("\nQueries looked for in\n");
                match search.queries.is_empty() {
                    true => out.push_str("  nowhere — a grammar with no query cannot colour\n"),
                    false => {
                        for path in &search.queries {
                            let _ = writeln!(out, "  {}/<language>/highlights.scm", path.display());
                        }
                    }
                }
            }
        }
        let _ = write!(
            out,
            "\ntree-sitter ABI {}..={} is what this build reads.\n",
            maxgus_syntax::MIN_ABI,
            maxgus_syntax::MAX_ABI
        );
        out
    }

    #[cfg(feature = "full")]
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
        if self
            .highlighters
            .get(&buffer)
            .is_some_and(|s| s.language != language)
        {
            self.highlighters.remove(&buffer);
        }
        // Taken out of the map rather than borrowed, because the work below
        // leaves this thread and needs to own it.
        let mut syntax = match self.highlighters.remove(&buffer) {
            Some(syntax) => syntax,
            None => {
                // Compiled in, or loaded from where the configuration said.
                // A language with neither is not an error; it simply goes
                // unhighlighted, as it did before there was a grammar for
                // anything.
                let Some(grammar) = self.grammar_for(language).await else {
                    return;
                };
                let Ok(highlighter) = Highlighter::with_grammar(language, grammar) else {
                    return;
                };
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
        let Ok((syntax, outcome)) = parsed else {
            return;
        };
        self.highlighters.insert(buffer, syntax);
        if let Some((range, highlights)) = outcome {
            self.send(TaskResult::Reparsed {
                buffer,
                revision,
                range,
                highlights,
            });
        }
    }

    /// Drops what was kept for a buffer that no longer exists.
    fn forget(&mut self, buffer: maxgus_text::BufferId) {
        let _ = buffer;
        #[cfg(feature = "full")]
        self.highlighters.remove(&buffer);
    }

    // ---- git -------------------------------------------------------------

    #[cfg(feature = "full")]
    /// Runs one git command, or reads the whole status.
    async fn git(&self, root: PathBuf, action: GitAction) {
        match action {
            GitAction::Refresh => self.git_refresh(root).await,
            // These three answer with a buffer rather than with a line of
            // output, so they never reach `git_do`.
            GitAction::Log { arguments, title } => self.git_log(root, arguments, title).await,
            GitAction::Diff { arguments, title } => self.git_diff(root, arguments, title).await,
            GitAction::Show { revision } => self.git_show(root, revision).await,
            other => self.git_do(root, other).await,
        }
    }

    #[cfg(feature = "full")]
    /// Reads everything the status view shows, in one pass.
    ///
    /// One answer rather than eight: a view assembled from results arriving
    /// separately shows a diff that disagrees with the status it is listed
    /// under, and that is exactly the moment somebody stages the wrong thing.
    async fn git_refresh(&self, from: PathBuf) {
        // Where the repository actually is. `git rev-parse` is the only
        // answer that is right for a worktree, a submodule, or a `.git` that
        // is a file rather than a directory.
        let top = git_output(&from, &["rev-parse", "--show-toplevel"]).await;
        let Some(root) = top
            .lines()
            .next()
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
        else {
            return self.fail("git", "not inside a repository");
        };
        let run = |args: Vec<&'static str>| {
            let root = root.clone();
            async move { git_output(&root, &args).await }
        };
        // The prefixes are forced rather than left to configuration: modern
        // git writes `i/` and `w/` for a worktree diff when `diff.mnemonicPrefix`
        // is on, and the patches this produces have to be predictable.
        let unstaged_args = DIFF_ARGS.to_vec();
        let mut staged_args = DIFF_ARGS.to_vec();
        staged_args.push("--cached");

        let status_bytes = git_raw(&root, &["status", "--porcelain=v2", "-z", "--branch"]).await;
        let snapshot = GitSnapshot {
            root: root.clone(),
            status: maxgus_git::status::parse(&status_bytes),
            unstaged: maxgus_git::diff::parse(&git_output(&root, &unstaged_args).await),
            staged: maxgus_git::diff::parse(&git_output(&root, &staged_args).await),
            stashes: maxgus_git::log::parse_stashes(
                &run(vec!["stash", "list", "--format=%gd%x1f%s%x1e"]).await,
            ),
            unpushed: maxgus_git::log::parse_log(
                &run(vec!["log", LOG_FORMAT_ARG, "@{upstream}..HEAD"]).await,
            ),
            unpulled: maxgus_git::log::parse_log(
                &run(vec!["log", LOG_FORMAT_ARG, "HEAD..@{upstream}"]).await,
            ),
            recent: maxgus_git::log::parse_log(&run(vec!["log", "-n", "10", LOG_FORMAT_ARG]).await),
            head_subject: run(vec!["log", "-1", "--format=%s"])
                .await
                .trim()
                .to_string(),
            branches: Vec::new(),
            references: maxgus_git::log::parse_refs(
                &run(vec!["for-each-ref", "--format=%(refname)"]).await,
            ),
        };
        let mut snapshot = snapshot;
        // The prompts want the names a person types; the references view
        // wants to know what each one is. Both come from the one reading.
        snapshot.branches = snapshot
            .references
            .iter()
            .filter(|reference| reference.kind != maxgus_git::RefKind::Tag)
            .map(|reference| reference.name.clone())
            .collect();
        self.send(TaskResult::GitRefreshed(Box::new(snapshot)));
    }

    #[cfg(feature = "full")]
    /// Reads a log into its own buffer.
    async fn git_log(&self, root: PathBuf, arguments: Vec<String>, title: String) {
        let mut args: Vec<String> = vec!["log".into(), LOG_FORMAT_ARG.into()];
        args.extend(arguments);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = git_output(&root, &borrowed).await;
        self.send(TaskResult::GitLog {
            title,
            commits: maxgus_git::log::parse_log(&output),
        });
    }

    #[cfg(feature = "full")]
    /// Reads a diff into its own buffer.
    async fn git_diff(&self, root: PathBuf, arguments: Vec<String>, title: String) {
        let mut args: Vec<String> = DIFF_ARGS.iter().map(|a| a.to_string()).collect();
        args.extend(arguments);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = git_output(&root, &borrowed).await;
        self.send(TaskResult::GitDiff {
            title,
            preamble: Vec::new(),
            files: maxgus_git::diff::parse(&output),
        });
    }

    #[cfg(feature = "full")]
    /// Reads one commit: who made it, what they said, and what it changed.
    ///
    /// Two commands rather than one `git show`: the header is asked for in a
    /// format this can read field by field, and the diff is asked for with
    /// the same arguments every other diff uses, so the patches agree.
    async fn git_show(&self, root: PathBuf, revision: String) {
        let header = git_output(
            &root,
            &[
                "show",
                "--no-patch",
                "--format=%H%n%an <%ae>%n%ad%n%cn <%ce>%n%cd%n%B",
                "--date=format:%Y-%m-%d %H:%M",
                &revision,
            ],
        )
        .await;
        let mut lines = header.lines();
        let hash = lines.next().unwrap_or_default().to_string();
        let author = lines.next().unwrap_or_default().to_string();
        let author_date = lines.next().unwrap_or_default().to_string();
        let committer = lines.next().unwrap_or_default().to_string();
        let commit_date = lines.next().unwrap_or_default().to_string();
        let mut preamble = vec![
            format!("Author:     {author}"),
            format!("AuthorDate: {author_date}"),
        ];
        // Only when it differs: on most commits the two are the same person
        // at the same moment, and saying so twice is noise.
        if committer != author || commit_date != author_date {
            preamble.push(format!("Commit:     {committer}"));
            preamble.push(format!("CommitDate: {commit_date}"));
        }
        preamble.push(String::new());
        preamble.extend(lines.map(|line| format!("    {line}")));

        let mut args: Vec<String> = DIFF_ARGS.iter().map(|a| a.to_string()).collect();
        args[0] = "show".into();
        args.push("--format=".into());
        args.push(revision.clone());
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = git_output(&root, &borrowed).await;
        self.send(TaskResult::GitDiff {
            title: format!("commit {hash}"),
            preamble,
            files: maxgus_git::diff::parse(&output),
        });
    }

    #[cfg(feature = "full")]
    /// Runs one git command and reports what it said, then refreshes.
    async fn git_do(&self, root: PathBuf, action: GitAction) {
        let Some((arguments, describe, stdin)) = git_command(action) else {
            return self.fail(
                "git",
                "that action answers with a buffer and should not have come here",
            );
        };
        let mut process = tokio::process::Command::new("git");
        process
            .args(&arguments)
            .current_dir(&root)
            .stdin(match stdin {
                Some(_) => std::process::Stdio::piped(),
                None => std::process::Stdio::null(),
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let output = match (process.spawn(), stdin) {
            (Ok(mut child), Some(text)) => {
                if let Some(mut pipe) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt as _;
                    let _ = pipe.write_all(text.as_bytes()).await;
                    let _ = pipe.shutdown().await;
                }
                child.wait_with_output().await
            }
            (Ok(child), None) => child.wait_with_output().await,
            (Err(error), _) => return self.fail(&describe, error),
        };
        match output {
            Ok(output) => {
                let said = if output.stderr.is_empty() {
                    &output.stdout
                } else {
                    &output.stderr
                };
                let text = String::from_utf8_lossy(said).into_owned();
                let line = format!("git {}", arguments.join(" "));
                if output.status.success() {
                    self.send(TaskResult::GitDone {
                        action: describe,
                        command: line,
                        output: text,
                    });
                } else {
                    self.fail(&format!("{describe} ({line})"), text.trim());
                }
                // Whatever happened, the view is now out of date.
                self.git_refresh(root).await;
            }
            Err(error) => self.fail(&describe, error),
        }
    }

    // ---- terminals -------------------------------------------------------

    #[cfg(feature = "full")]
    /// Starts a shell on a pseudo-terminal and reads from it forever.
    fn open_terminal(
        &mut self,
        terminal: TerminalId,
        shell: Option<String>,
        directory: PathBuf,
        rows: u16,
        columns: u16,
    ) {
        let size = portable_pty::PtySize {
            rows,
            cols: columns,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = match portable_pty::native_pty_system().openpty(size) {
            Ok(pair) => pair,
            Err(error) => return self.fail("opening a terminal", error),
        };

        let program = shell.unwrap_or_else(default_shell);
        let mut command = portable_pty::CommandBuilder::new(&program);
        command.cwd(&directory);
        // `TERM` decides what the program believes it may send. Claiming more
        // than is implemented would invite sequences that are then dropped,
        // and a wrong screen is worse than a plain one.
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");

        let child = match pair.slave.spawn_command(command) {
            Ok(child) => child,
            Err(error) => return self.fail(&format!("starting {program}"), error),
        };
        // The slave is dropped on purpose: while this process holds it open,
        // reading the master never reaches end-of-file, and closing the tab
        // would leave the reader thread alive for the rest of the session.
        drop(pair.slave);

        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => return self.fail("reading from the terminal", error),
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => return self.fail("writing to the terminal", error),
        };

        // The reading half. A pty read blocks until the program writes, which
        // may be never, so it gets a thread rather than a slice of the runtime.
        let results = self.results.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut chunk = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let output = TaskResult::TerminalOutput {
                            terminal,
                            bytes: chunk[..read].to_vec(),
                        };
                        if results.send(output).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // The controlling half, which owns everything that can block.
        let (commands, orders) = std::sync::mpsc::channel();
        let results = self.results.clone();
        std::thread::spawn(move || {
            let (mut writer, master, mut child) = (writer, pair.master, child);
            while let Ok(order) = orders.recv() {
                match order {
                    PtyCommand::Write(bytes) => {
                        if writer
                            .write_all(&bytes)
                            .and_then(|()| writer.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                    PtyCommand::Resize(rows, cols) => {
                        // A program learns its window changed from a signal
                        // the pty sends. Without this, `vim` goes on drawing
                        // to the shape it started with.
                        let size = portable_pty::PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        };
                        let _ = master.resize(size);
                    }
                    PtyCommand::Close => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                }
            }
            // The shell went on its own. Say so once, so the tab can report it.
            let status = child.wait().ok().map(|s| s.exit_code() as i32).unwrap_or(0);
            let _ = results.send(TaskResult::TerminalExited { terminal, status });
        });

        self.terminals.insert(terminal, Terminal { commands });
    }

    #[cfg(feature = "full")]
    fn terminal_input(&mut self, terminal: TerminalId, bytes: Vec<u8>) {
        self.order(terminal, PtyCommand::Write(bytes));
    }

    #[cfg(feature = "full")]
    fn resize_terminal(&mut self, terminal: TerminalId, rows: u16, columns: u16) {
        self.order(terminal, PtyCommand::Resize(rows, columns));
    }

    #[cfg(feature = "full")]
    fn close_terminal(&mut self, terminal: TerminalId) {
        self.order(terminal, PtyCommand::Close);
        self.terminals.remove(&terminal);
    }

    #[cfg(feature = "full")]
    /// Sends one order to a terminal's thread, forgetting the terminal if the
    /// thread has already gone.
    fn order(&mut self, terminal: TerminalId, order: PtyCommand) {
        let gone = match self.terminals.get(&terminal) {
            Some(running) => running.commands.send(order).is_err(),
            None => return,
        };
        if gone {
            self.terminals.remove(&terminal);
        }
    }

    // ---- language servers ----------------------------------------------

    #[cfg(feature = "full")]
    fn spec_for(&self, language: &str) -> Option<&LspSpec> {
        self.lsp_specs.iter().find(|s| s.language == language)
    }

    #[cfg(feature = "full")]
    #[cfg(feature = "full")]
    async fn start_server(&mut self, language: &str) {
        if self.servers.contains_key(language) {
            return;
        }
        let Some(spec) = self.spec_for(language).cloned() else {
            // Nothing configured. Quiet on purpose: a server is started
            // whenever a file is opened, so complaining here would put a
            // message on the screen for every buffer in a language nobody
            // has configured one for. A request that needed a server says
            // so instead — see `lsp_request`.
            return;
        };
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
                self.servers
                    .insert(language.to_string(), Arc::clone(&client));
                // Diagnostics and messages arrive on their own schedule.
                tokio::spawn(forward_events(
                    events,
                    self.results.clone(),
                    language.to_string(),
                    Arc::clone(&client),
                ));
                self.send(TaskResult::LanguageServerStarted {
                    language: language.to_string(),
                    encoding: client.encoding().await,
                });
            }
            Err(error) => self.fail(&format!("starting {language} server"), error),
        }
    }

    #[cfg(feature = "full")]
    async fn stop_server(&mut self, language: &str) {
        let Some(client) = self.servers.remove(language) else {
            return;
        };
        let _ = client.shutdown().await;
        self.send(TaskResult::LanguageServerStopped {
            language: language.to_string(),
        });
    }

    #[cfg(feature = "full")]
    /// Tells the server a document changed, in the form it asked for.
    ///
    /// A server that declared incremental sync is sent only the region that
    /// differs. Sending the whole file on every pause in typing makes the
    /// server re-parse it from nothing, which is exactly the cost incremental
    /// sync exists to avoid.
    async fn did_change(&mut self, language: &str, uri: String, version: i64, text: String) {
        let Some(client) = self.servers.get(language).cloned() else {
            return;
        };
        // Incremental sync needs a diff between the old text and the new, and
        // the differ is tree-sitter's. A build without the grammars sends the
        // whole document instead — correct, just larger on the wire.
        let incremental = cfg!(feature = "full")
            && client.sync_kind().await == maxgus_lsp::client::SyncKind::Incremental;

        let sent = match (incremental, self.documents.get(&uri)) {
            #[cfg(feature = "full")]
            (true, Some(previous)) => match changed_range(previous, &text, client.encoding().await)
            {
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

    #[cfg(feature = "full")]
    /// Sends a request without waiting for it here, so a slow server cannot
    /// hold up the rest of the queue.
    fn lsp_request(&self, language: String, uri: String, query: LspQuery, announced: bool) {
        let Some(client) = self.servers.get(&language).cloned() else {
            // A request nobody can answer. Only worth saying when a command
            // announced it: that message is on screen now and would stay
            // there for ever. The symbols panel and the doc box ask without
            // announcing, while a server may still be starting, and a
            // complaint about that race would be wrong a moment later.
            if announced {
                self.fail(
                    &format!("language server: {}", query.description()),
                    match self.spec_for(&language).is_some() {
                        true => format!("the server for `{language}` is not running yet"),
                        false => format!("none is configured for `{language}`"),
                    },
                );
            }
            return;
        };
        let results = self.results.clone();
        tokio::spawn(async move {
            let outcome = match &query {
                LspQuery::Definition(p) => client.definition(&uri, *p).await,
                LspQuery::References(p) => client.references(&uri, *p).await,
                LspQuery::Hover(p) => client.hover(&uri, *p).await,
                LspQuery::Completion { position, .. } => client.completion(&uri, *position).await,
                LspQuery::SignatureHelp(p) => client.signature_help(&uri, *p).await,
                LspQuery::Rename { position, new_name } => {
                    client.rename(&uri, *position, new_name).await
                }
                LspQuery::Format {
                    tab_size,
                    insert_spaces,
                } => client.formatting(&uri, *tab_size, *insert_spaces).await,
                LspQuery::CodeAction { range, diagnostics } => {
                    client.code_action(&uri, *range, diagnostics).await
                }
                LspQuery::DocumentSymbols { .. } => client.document_symbols(&uri).await,
                LspQuery::WorkspaceSymbols(q) => client.workspace_symbols(q).await,
            };
            let result = match outcome {
                Ok(value) => TaskResult::LspResponse {
                    language,
                    uri,
                    query,
                    result: value,
                },
                Err(error) => TaskResult::Failed {
                    context: "language server".into(),
                    message: error.to_string(),
                },
            };
            let _ = results.send(result);
        });
    }

    async fn shutdown(&mut self) {
        #[cfg(feature = "full")]
        {
            let languages: Vec<String> = self.servers.keys().cloned().collect();
            for language in languages {
                self.stop_server(&language).await;
            }
        }
    }

    // ---- what a project asks of a file ---------------------------------

    /// Reads the `.editorconfig` rules that apply to `path`.
    ///
    /// Only what the editor can honour: a property with no setting behind it
    /// is left out rather than carried around. A file with no `.editorconfig`
    /// above it — the usual case — produces nothing and costs one failed
    /// lookup.
    fn editor_config(path: &Path) -> EditorConfig {
        let Ok(properties) = ec4rs::properties_of(path) else {
            return EditorConfig::default();
        };
        use ec4rs::property::*;
        let mut asked = EditorConfig::default();
        if let Ok(style) = properties.get::<IndentStyle>() {
            asked.indent_with_tabs = Some(matches!(style, IndentStyle::Tabs));
        }
        // `indent_size` is what a level of indentation costs; `tab_width` is
        // what a tab character is drawn as. The editor has one number, and
        // the indent size is the one a person means.
        if let Ok(IndentSize::Value(size)) = properties.get::<IndentSize>() {
            asked.tab_width = Some(size);
        } else if let Ok(TabWidth::Value(width)) = properties.get::<TabWidth>() {
            asked.tab_width = Some(width);
        }
        if let Ok(ending) = properties.get::<EndOfLine>() {
            asked.crlf = match ending {
                EndOfLine::CrLf => Some(true),
                EndOfLine::Lf => Some(false),
                // `cr` alone is not something the editor can hold, so it is
                // left to whatever the file itself turns out to use.
                _ => None,
            };
        }
        if let Ok(trim) = properties.get::<TrimTrailingWs>() {
            asked.trim_trailing_whitespace = Some(matches!(trim, TrimTrailingWs::Value(true)));
        }
        if let Ok(final_newline) = properties.get::<FinalNewline>() {
            asked.final_newline = Some(matches!(final_newline, FinalNewline::Value(true)));
        }
        if let Ok(MaxLineLen::Value(length)) = properties.get::<MaxLineLen>() {
            asked.fill_column = Some(length);
        }
        asked
    }

    // ---- searching the project -----------------------------------------

    /// Searches the project on a blocking thread.
    ///
    /// Walking a tree and reading every file in it is exactly the work tokio
    /// asks not to be done on its own threads, and a large project would stop
    /// every other task while it ran.
    #[cfg(feature = "full")]
    async fn grep(&self, root: PathBuf, search: maxgus_grep::Search) {
        let pattern = search.pattern.clone();
        let outcome =
            tokio::task::spawn_blocking(move || maxgus_grep::search(&root, &search)).await;
        match outcome {
            Ok(Ok(found)) => self.send(TaskResult::GrepFinished { pattern, found }),
            Ok(Err(error)) => self.fail("search", error),
            Err(error) => self.fail("search", error),
        }
    }

    /// Writes edited result lines back to their files.
    #[cfg(feature = "full")]
    async fn apply_grep(&self, replacements: Vec<maxgus_grep::Replacement>) {
        let mut paths: Vec<PathBuf> = replacements.iter().map(|r| r.path.clone()).collect();
        paths.sort();
        paths.dedup();
        let outcome = tokio::task::spawn_blocking(move || maxgus_grep::apply(&replacements)).await;
        match outcome {
            Ok(Ok(applied)) => self.send(TaskResult::GrepApplied { applied, paths }),
            Ok(Err(error)) => self.fail("writing the results", error),
            Err(error) => self.fail("writing the results", error),
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

#[cfg(feature = "full")]
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
    Some((
        range,
        current[edit.start_byte..edit.new_end_byte].to_string(),
    ))
}

#[cfg(feature = "full")]
/// Walks up from `start` looking for `marker`, returning the directory holding
/// it — how a project root is found.
pub async fn find_upwards(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut directory = Some(start);
    while let Some(current) = directory {
        // `tokio::fs`, not `Path::exists`: this runs while the editor is
        // already going, and a stat on a cold or networked filesystem is a
        // blocking call like any other.
        if tokio::fs::try_exists(current.join(marker))
            .await
            .unwrap_or(false)
        {
            return Some(current.to_path_buf());
        }
        directory = current.parent();
    }
    None
}

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
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
                TaskResult::Failed {
                    context: language.clone(),
                    message: text,
                }
            }
            ServerEvent::Exited => TaskResult::LanguageServerStopped {
                language: language.clone(),
            },
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
        assert_eq!(
            with_theme(before, "nord"),
            "set tab-width=4\nset theme=\"nord\"\n"
        );
    }

    #[test]
    fn an_empty_file_gets_just_the_one_line() {
        assert_eq!(with_theme("", "nord"), "set theme=\"nord\"\n");
    }

    #[test]
    fn a_file_with_no_final_newline_still_ends_up_well_formed() {
        assert_eq!(
            with_theme("set tab-width=4", "nord"),
            "set tab-width=4\nset theme=\"nord\"\n"
        );
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
        assert!(
            after.ends_with("set theme=\"nord\"\n"),
            "the setting was not added:\n{after}"
        );
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
        assert!(
            after.starts_with(before),
            "the theme block was edited:\n{after}"
        );
        assert!(
            after.ends_with("set theme=\"nord\"\n"),
            "the setting was not added:\n{after}"
        );
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
        assert_eq!(
            after.lines().count(),
            before.lines().count(),
            "no line was added or lost"
        );
    }

    #[test]
    fn only_the_first_theme_setting_is_rewritten() {
        // A second one would be the one that wins on load, but rewriting both
        // would be changing more than was asked; the first is where the value
        // is read from anyway once the duplicate is resolved.
        let before = "set theme=\"a\"\nset theme=\"b\"\n";
        assert_eq!(
            with_theme(before, "nord"),
            "set theme=\"nord\"\nset theme=\"b\"\n"
        );
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
    ///
    /// `maxgus-grep` blocks on purpose: walking a project and reading every
    /// file in it is precisely the work `spawn_blocking` exists for, and it
    /// is only ever reached that way. A second test below checks that.
    const MAY_BLOCK: &[&str] = &[
        "maxgus/src/main.rs",
        "maxgus-grep/src/lib.rs",
        // Opening a shared library is a blocking operation with no async
        // form — `dlopen` reads the file and runs its initialisers. It is
        // reached only through `spawn_blocking`, which the test below
        // holds to.
        "maxgus-syntax/src/dynamic.rs",
    ];

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }

    /// The grammar exception above is only safe while it holds.
    #[cfg(feature = "full")]
    #[test]
    fn a_grammar_is_only_ever_loaded_off_a_blocking_thread() {
        let source = include_str!("tasks.rs");
        let ships = source
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"))
            .collect::<Vec<_>>();
        let calls: Vec<(usize, &str)> = ships
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("dynamic::load"))
            .map(|(n, line)| (n + 1, *line))
            .collect();
        assert!(
            !calls.is_empty(),
            "nothing loads a grammar any more; this test has nothing to hold"
        );
        for (n, line) in calls {
            assert!(
                line.contains("spawn_blocking"),
                "line {n}: `{}` loads a grammar on the runtime",
                line.trim()
            );
        }
    }

    /// The exception above is only safe while it holds.
    #[cfg(feature = "full")]
    #[test]
    fn the_search_is_only_ever_reached_through_spawn_blocking() {
        let source = include_str!("tasks.rs");
        let ships = source
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"));
        for (n, line) in ships.enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            if !code.contains("maxgus_grep::search") && !code.contains("maxgus_grep::apply") {
                continue;
            }
            assert!(
                code.contains("spawn_blocking"),
                "line {}: `{}` calls into the search off a blocking thread",
                n + 1,
                line.trim()
            );
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
        assert!(
            files.len() > 30,
            "the walk found almost nothing: {}",
            files.len()
        );

        let mut offences = Vec::new();
        for file in files {
            let shown = file.to_string_lossy().replace('\\', "/");
            if MAY_BLOCK.iter().any(|allowed| shown.ends_with(allowed)) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
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
        assert!(
            offences.is_empty(),
            "blocking calls off the startup path:\n{}",
            offences.join("\n")
        );
    }

    // ---- answering the server -------------------------------------------

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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
        assert!(
            error.message.contains("workspace/workspaceFolders"),
            "got `{}`",
            error.message
        );
    }

    #[cfg(feature = "full")]
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
        let TaskResult::LspApplyEdit {
            language,
            id,
            edit: carried,
        } = received
        else {
            panic!("expected the edit to reach the editor, got {received:?}")
        };
        assert_eq!(language, "rust");
        assert_eq!(id, maxgus_lsp::RequestId::Number(7));
        assert_eq!(
            carried, edit,
            "the edit itself is carried, not just the fact of it"
        );
    }

    use super::*;
    use maxgus_text::BufferId;

    /// A temporary directory, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        /// A directory of its own, named by `tag`.
        ///
        /// The tag has to be unique across the whole module. Two tests
        /// sharing one share the directory, and `Drop` removes it — so the
        /// first to finish deletes the ground out from under the second,
        /// which then fails on an unwrap somewhere unrelated. Two did share
        /// one, and it was an intermittent failure in a file it never
        /// mentioned.
        async fn new(tag: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!("maxgus-exec-{tag}"));
            tokio::fs::remove_dir_all(&dir).await.ok();
            tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
            tokio::fs::write(dir.join("Cargo.toml"), "[package]")
                .await
                .unwrap();
            tokio::fs::write(dir.join("src/main.rs"), "fn main() {}\n")
                .await
                .unwrap();
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
        let config = TreeConfig {
            git_status: false,
            ..Default::default()
        };
        (
            Executor::new(root.to_path_buf(), config, Vec::new(), tx),
            rx,
        )
    }

    /// Runs one task and returns the first result it produced.
    async fn run_one(
        executor: &mut Executor,
        rx: &mut mpsc::UnboundedReceiver<TaskResult>,
        task: Task,
    ) -> TaskResult {
        executor.handle(task).await;
        rx.try_recv().expect("the task produced no result")
    }

    #[tokio::test]
    async fn workspaces_are_written_and_read_back() {
        // The whole cycle on disk, which is the part the command tests
        // cannot reach: they prove the right contents are queued, and this
        // proves the queue puts them somewhere they come back from.
        let f = Fixture::new("workspaces").await;
        let (mut e, mut rx) = executor(f.path());
        let path = maxgus_core::workspace::path_for(&f.path().join("state"));

        let mut workspaces = maxgus_core::workspace::Workspaces::default();
        workspaces.save("editor", vec![f.path().join("src"), f.path().to_path_buf()]);
        e.handle(Task::SaveWorkspaces {
            path: path.clone(),
            contents: workspaces.to_kdl(),
        })
        .await;
        assert!(
            tokio::fs::try_exists(&path).await.unwrap(),
            "nothing was written to {}",
            path.display()
        );

        let result = run_one(&mut e, &mut rx, Task::ReadWorkspaces { path }).await;
        let TaskResult::WorkspacesRead { workspaces: read } = result else {
            panic!("{result:?}")
        };
        assert_eq!(read, workspaces);
    }

    #[tokio::test]
    async fn a_missing_workspace_file_is_no_workspaces_rather_than_a_failure() {
        // Nobody having saved one yet is the normal state of a new install.
        let f = Fixture::new("workspaces-none").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ReadWorkspaces {
                path: f.path().join("nothing-here.kdl"),
            },
        )
        .await;
        let TaskResult::WorkspacesRead { workspaces } = result else {
            panic!("{result:?}")
        };
        assert!(workspaces.is_empty());
    }

    #[tokio::test]
    async fn opening_a_workspace_shows_exactly_its_directories() {
        let f = Fixture::new("workspaces-open").await;
        tokio::fs::create_dir_all(f.path().join("docs"))
            .await
            .unwrap();
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::SetRoots(vec![
                f.path().join("src"),
                f.path().join("docs"),
            ])),
        )
        .await;
        let TaskResult::TreeUpdated { nodes, .. } = result else {
            panic!("{result:?}")
        };
        let roots: Vec<&std::path::Path> = nodes
            .iter()
            .filter(|node| node.is_root)
            .map(|node| node.path.as_path())
            .collect();
        assert_eq!(roots, [f.path().join("src"), f.path().join("docs")]);
    }

    #[tokio::test]
    async fn a_workspace_whose_directories_have_moved_opens_what_is_left() {
        // A saved workspace outlives the disk it was saved on. Losing one
        // directory of two is not a reason to open neither, and the one
        // that went is said out loud rather than quietly left out.
        let f = Fixture::new("workspaces-moved").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        e.handle(Task::Tree(TreeAction::SetRoots(vec![
            f.path().join("src"),
            f.path().join("gone-away"),
        ])))
        .await;
        let mut said = Vec::new();
        let mut roots = Vec::new();
        while let Ok(result) = rx.try_recv() {
            match result {
                TaskResult::Said(note) => said.push(note),
                TaskResult::TreeUpdated { nodes, .. } => {
                    roots = nodes
                        .iter()
                        .filter(|node| node.is_root)
                        .map(|node| node.path.clone())
                        .collect();
                }
                _ => {}
            }
        }
        assert_eq!(
            roots,
            [f.path().join("src")],
            "it did not open what was left"
        );
        assert!(
            said.iter().any(|note| note.contains("gone-away")),
            "it did not say what it left out: {said:?}"
        );
    }

    #[tokio::test]
    async fn a_workspace_with_nothing_readable_in_it_is_refused() {
        // Rather than emptying the tree, which has nothing to draw and no
        // way to ask for a directory back.
        let f = Fixture::new("workspaces-all-gone").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;

        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::SetRoots(vec![f.path().join("nowhere")])),
        )
        .await;
        assert!(result.is_error(), "{result:?}");
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
        let TaskResult::FileRead {
            contents,
            read_only,
            ..
        } = result
        else {
            panic!("{result:?}")
        };
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
            Task::ReadFile {
                path: f.path().join("new.rs"),
                reverting: None,
                other_window: false,
            },
        )
        .await;
        let TaskResult::FileRead { contents, .. } = result else {
            panic!("{result:?}")
        };
        assert!(contents.is_empty(), "visiting a new file is not an error");
    }

    #[tokio::test]
    async fn reading_a_directory_is_reported_as_a_failure() {
        let f = Fixture::new("readdir").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ReadFile {
                path: f.path().join("src"),
                reverting: None,
                other_window: false,
            },
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
        let TaskResult::FileWritten { bytes, .. } = result else {
            panic!("{result:?}")
        };
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
        let backup = tokio::fs::read_to_string(f.path().join("src/main.rs~"))
            .await
            .unwrap();
        assert_eq!(backup, "fn main() {}\n", "the previous contents were kept");
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "changed\n");
    }

    #[tokio::test]
    async fn listing_a_directory_marks_the_directories() {
        let f = Fixture::new("list").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ListDirectory {
                path: f.path().to_path_buf(),
            },
        )
        .await;
        let TaskResult::DirectoryListed { entries, .. } = result else {
            panic!("{result:?}")
        };
        assert!(
            entries.iter().any(|entry| entry.ends_with("/src/")),
            "got {entries:?}"
        );
        assert!(
            entries.iter().any(|entry| entry.ends_with("Cargo.toml")),
            "got {entries:?}"
        );
    }

    #[tokio::test]
    async fn listing_a_missing_directory_is_reported() {
        let (mut e, mut rx) = executor(Path::new("/nonexistent-maxgus-path"));
        let result = run_one(
            &mut e,
            &mut rx,
            Task::ListDirectory {
                path: PathBuf::from("/nonexistent-maxgus-path"),
            },
        )
        .await;
        assert!(result.is_error());
    }

    #[tokio::test]
    async fn a_tree_refresh_returns_a_snapshot() {
        let f = Fixture::new("tree").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let TaskResult::TreeUpdated { nodes, .. } = result else {
            panic!("{result:?}")
        };
        assert!(nodes.iter().any(|n| n.name == "src"), "got {:?}", nodes);
        assert!(nodes.iter().any(|n| n.name == "Cargo.toml"));
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn asking_a_language_server_that_is_not_there_says_so() {
        // The command has already said "Language server: describing..." in
        // the echo area. Returning quietly leaves that there for ever,
        // which is what a file in a language with no server used to do.
        let f = Fixture::new("noserver").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::LspRequest {
                language: "wombat".into(),
                uri: "file:///a.wombat".into(),
                query: LspQuery::Hover(maxgus_lsp::LspPosition::ZERO),
                announced: true,
            },
        )
        .await;
        assert!(result.is_error(), "{result:?}");
        let said = result.message().unwrap_or_default();
        assert!(said.contains("wombat"), "got `{said}`");
        assert!(said.contains("none is configured"), "got `{said}`");
    }

    #[tokio::test]
    async fn expanding_a_directory_leaves_the_cursor_on_it() {
        // The bug this is here for: the result carried the executor's own
        // idea of what was selected, which nothing had ever set, so every
        // expansion sent the editor's cursor back to the root of the tree.
        let f = Fixture::new("treecursor").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Expand(f.path().join("src"))),
        )
        .await;
        let TaskResult::TreeUpdated { select, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(
            select, None,
            "expanding asked the editor to move its cursor to {select:?}"
        );
    }

    #[tokio::test]
    async fn revealing_a_file_does_move_the_cursor_to_it() {
        // The other half: `select` is for the actions that genuinely move
        // the cursor, and emptying it everywhere would break those.
        let f = Fixture::new("treereveal").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let wanted = f.path().join("src").join("main.rs");
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Reveal(wanted.clone())),
        )
        .await;
        let TaskResult::TreeUpdated { select, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(select, Some(wanted));
    }

    #[tokio::test]
    async fn expanding_a_directory_reveals_its_children() {
        let f = Fixture::new("treeexpand").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Tree(TreeAction::Expand(f.path().join("src"))),
        )
        .await;
        let TaskResult::TreeUpdated { nodes, .. } = result else {
            panic!("{result:?}")
        };
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
        let TaskResult::TreeUpdated { select, nodes, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(select, Some(f.path().join("created.txt")));
        assert!(nodes.iter().any(|n| n.name == "created.txt"));
        assert!(
            tokio::fs::try_exists(f.path().join("created.txt"))
                .await
                .unwrap()
        );
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

        let TaskResult::TreeUpdated { select, nodes, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(select, Some(f.path().join("Renamed.toml")));
        assert!(nodes.iter().any(|n| n.name == "Renamed.toml"));
        assert!(
            !tokio::fs::try_exists(f.path().join("Cargo.toml"))
                .await
                .unwrap()
        );
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

        let TaskResult::TreeUpdated { select, nodes, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(
            select, None,
            "nothing is selected after what was selected went"
        );
        assert!(!nodes.iter().any(|n| n.name == "Cargo.toml"));
        assert!(
            !tokio::fs::try_exists(f.path().join("Cargo.toml"))
                .await
                .unwrap()
        );
        assert!(
            tokio::fs::try_exists(f.path().join("src")).await.unwrap(),
            "and nothing else"
        );
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

        let TaskResult::TreeUpdated { select, nodes, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(select, Some(f.path().join("made")));
        assert!(nodes.iter().any(|n| n.name == "made"));
        assert!(
            tokio::fs::metadata(f.path().join("made"))
                .await
                .unwrap()
                .is_dir()
        );
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

        let TaskResult::TreeUpdated { select, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(select, Some(f.path().join("src/Cargo.toml")));
        assert!(
            tokio::fs::try_exists(f.path().join("src/Cargo.toml"))
                .await
                .unwrap()
        );
        assert!(
            !tokio::fs::try_exists(f.path().join("Cargo.toml"))
                .await
                .unwrap()
        );
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn stopping_a_server_that_was_never_started_is_quiet() {
        // The only exercise `stop_server` gets: nothing had been started, so
        // it must return without announcing a server that never existed.
        let f = Fixture::new("treestop").await;
        let (mut e, rx) = executor(f.path());
        e.handle(Task::StopLanguageServer {
            language: "rust".into(),
        })
        .await;
        drop(e);
        let mut rx = rx;
        assert!(
            rx.try_recv().is_err(),
            "it reported stopping something that never ran"
        );
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
                "rename" => TreeAction::Rename {
                    path: unseen.clone(),
                    name: "gone.txt".into(),
                },
                // Into `src`, not the root: the cursor's node already lives
                // in the root, so moving it there would fail as "already
                // exists" and hide the guard being gone.
                "move" => TreeAction::Move {
                    path: unseen.clone(),
                    destination: f.path().join("src"),
                },
                "create-file" => TreeAction::CreateFile {
                    parent: unseen.clone(),
                    name: "made.txt".into(),
                },
                _ => TreeAction::CreateDirectory {
                    parent: unseen.clone(),
                    name: "made".into(),
                },
            };
            run_one(&mut e, &mut rx, Task::Tree(task)).await;

            assert!(
                tokio::fs::try_exists(f.path().join("Cargo.toml"))
                    .await
                    .unwrap(),
                "`{action}` touched the node the cursor was on"
            );
            assert!(
                !tokio::fs::try_exists(f.path().join("made.txt"))
                    .await
                    .unwrap(),
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
        tokio::fs::write(f.path().join(".hidden"), "")
            .await
            .unwrap();
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::Refresh)).await;
        let TaskResult::TreeUpdated {
            nodes, show_hidden, ..
        } = result
        else {
            panic!()
        };
        assert!(!show_hidden);
        assert!(!nodes.iter().any(|n| n.name == ".hidden"));

        let result = run_one(&mut e, &mut rx, Task::Tree(TreeAction::ToggleHidden)).await;
        let TaskResult::TreeUpdated {
            nodes, show_hidden, ..
        } = result
        else {
            panic!()
        };
        assert!(show_hidden);
        assert!(nodes.iter().any(|n| n.name == ".hidden"));
    }

    #[cfg(feature = "full")]
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
        let TaskResult::Reparsed {
            revision,
            highlights,
            ..
        } = result
        else {
            panic!("{result:?}")
        };
        assert_eq!(revision, 7);
        assert!(!highlights.is_empty());
        assert!(highlights.iter().any(|h| h.face == "font-lock-keyword"));
    }

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
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
            let TaskResult::Reparsed { highlights, .. } = result else {
                panic!()
            };
            assert!(!highlights.is_empty(), "`{text}` produced nothing");
        }
        // A parser's worth is the tree it holds, and a tree belongs to one
        // buffer; sharing would mean discarding it at every switch.
        assert_eq!(e.highlighters.len(), 2);
    }

    #[cfg(feature = "full")]
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
    #[cfg(feature = "full")]
    async fn an_edit_between_reparses_still_highlights_correctly() {
        let f = Fixture::new("parseincremental").await;
        let (mut e, mut rx) = executor(f.path());
        let before = "fn main() { let x = 1; }";
        let after = "fn main() { let renamed = 1; }";

        run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: before.into(),
                revision: 1,
                range: 0..usize::MAX,
            },
        )
        .await;
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: after.into(),
                revision: 2,
                range: 0..usize::MAX,
            },
        )
        .await;
        let TaskResult::Reparsed { highlights, .. } = result else {
            panic!("{result:?}")
        };

        // An incremental parse must produce the same answer a full one would.
        let mut fresh = Highlighter::new("rust").unwrap();
        fresh.parse(after).unwrap();
        assert_eq!(highlights, fresh.highlights(after));
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_buffer_that_changes_language_starts_over() {
        let f = Fixture::new("parselanguage").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: "fn a() {}".into(),
                revision: 1,
                range: 0..usize::MAX,
            },
        )
        .await;
        // `write-file` under a new name can change a buffer's language.
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "python".into(),
                text: "def a(): pass".into(),
                revision: 2,
                range: 0..usize::MAX,
            },
        )
        .await;
        let TaskResult::Reparsed { highlights, .. } = result else {
            panic!()
        };
        assert!(!highlights.is_empty(), "the new grammar was used");
        assert_eq!(e.highlighters[&BufferId(1)].language, "python");
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn forgetting_a_buffer_releases_its_parser_and_its_text() {
        let f = Fixture::new("parseforget").await;
        let (mut e, mut rx) = executor(f.path());
        run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "rust".into(),
                text: "fn a() {}".into(),
                revision: 1,
                range: 0..usize::MAX,
            },
        )
        .await;
        assert_eq!(e.highlighters.len(), 1);
        e.handle(Task::ForgetBuffer {
            buffer: BufferId(1),
        })
        .await;
        assert!(
            e.highlighters.is_empty(),
            "a killed buffer must not be held onto"
        );
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
        let TaskResult::ShellOutput { output, status, .. } = result else {
            panic!("{result:?}")
        };
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
        let TaskResult::ShellOutput { output, status, .. } = result else {
            panic!("{result:?}")
        };
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
            Task::Shell {
                command: "pwd".into(),
                directory: f.path().join("src"),
                insert_at: None,
            },
        )
        .await;
        let TaskResult::ShellOutput { output, .. } = result else {
            panic!()
        };
        assert!(output.trim().ends_with("/src"), "got `{output}`");
    }

    #[tokio::test]
    #[cfg(feature = "full")]
    async fn a_request_to_a_server_that_is_not_running_is_a_no_op() {
        let f = Fixture::new("norequest").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::LspRequest {
            language: "rust".into(),
            uri: "file:///a.rs".into(),
            query: LspQuery::DocumentSymbols { for_panel: false },
            // Not announced, so the silence below is the point.
            announced: false,
        })
        .await;
        assert!(rx.try_recv().is_err());
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn starting_a_server_with_no_configuration_does_nothing() {
        let f = Fixture::new("nospec").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::StartLanguageServer {
            language: "rust".into(),
        })
        .await;
        assert!(
            rx.try_recv().is_err(),
            "an unconfigured language is not an error"
        );
    }

    #[cfg(feature = "full")]
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
        e.handle(Task::StartLanguageServer {
            language: "rust".into(),
        })
        .await;
        let result = rx.try_recv().expect("a failure was reported");
        assert!(result.is_error());
    }

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
    #[test]
    fn identical_texts_produce_no_change_to_report() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        assert!(changed_range("same", "same", encoding).is_none());
    }

    #[cfg(feature = "full")]
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
        assert_eq!(
            rebuilt, current,
            "the described change does not reproduce the text"
        );
    }

    #[cfg(feature = "full")]
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

    #[cfg(feature = "full")]
    #[test]
    fn a_changed_region_is_a_fraction_of_a_large_document() {
        let encoding = maxgus_lsp::PositionEncoding::Utf16;
        let previous: String = (0..5_000).map(|n| format!("line {n}\n")).collect();
        let at = previous.len() / 2;
        let at = (at..previous.len())
            .find(|i| previous.is_char_boundary(*i))
            .unwrap();
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

    #[cfg(feature = "full")]
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
        run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-m",
            "x",
        ]);

        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::GitBranch {
                root: f.path().to_path_buf(),
            },
        )
        .await;
        let TaskResult::GitBranch { branch } = result else {
            panic!("{result:?}")
        };
        assert_eq!(branch.as_deref(), Some("trunk"));
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_directory_outside_any_repository_reports_no_branch() {
        let f = Fixture::new("gitnone").await;
        let (mut e, mut rx) = executor(f.path());
        let result = run_one(
            &mut e,
            &mut rx,
            Task::GitBranch {
                root: f.path().to_path_buf(),
            },
        )
        .await;
        let TaskResult::GitBranch { branch } = result else {
            panic!("{result:?}")
        };
        assert_eq!(branch, None, "a plain directory has no branch");
    }

    // `find_upwards` locates a language server's project root.
    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_project_root_is_found_by_walking_upwards() {
        // The workspace this test is compiled in is itself a good fixture.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = find_upwards(here, "Cargo.toml")
            .await
            .expect("this crate has one");
        assert!(found.join("Cargo.toml").exists());
        assert!(find_upwards(here, "no-such-marker-file").await.is_none());
    }
}

/// Parsing costs, which only a build with the grammars in it can measure.
#[cfg(all(test, feature = "full"))]
mod scale {
    use super::*;
    use maxgus_text::BufferId;
    use std::time::Instant;

    /// A file large enough that a full parse is visible to a person.
    fn source(lines: usize) -> String {
        (0..lines)
            .map(|n| {
                format!("fn function_{n}(argument: &str) -> usize {{ argument.len() + {n} }}\n")
            })
            .collect()
    }

    /// Typing a character into the middle of `text`.
    fn typed_into(text: &str) -> String {
        let at = text.len() / 2;
        let at = (at..text.len())
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(text.len());
        let mut edited = text.to_string();
        edited.insert(at, 'x');
        edited
    }

    #[cfg(feature = "full")]
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
            TreeConfig {
                git_status: false,
                ..Default::default()
            },
            Vec::new(),
            tx,
        );

        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        tokio::spawn(async move { flag.store(true, Ordering::SeqCst) });

        let text = source(if cfg!(debug_assertions) {
            3_000
        } else {
            20_000
        });
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

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_reparse_after_typing_costs_far_less_than_the_first_one() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut executor = Executor::new(
            PathBuf::from("/tmp"),
            TreeConfig {
                git_status: false,
                ..Default::default()
            },
            Vec::new(),
            tx,
        );

        let text = source(if cfg!(debug_assertions) {
            3_000
        } else {
            20_000
        });
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

#[cfg(feature = "full")]
/// The shell to start when the configuration does not name one.
fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

#[cfg(feature = "full")]
/// What every diff is asked for.
///
/// The prefixes are forced rather than left to configuration: modern git
/// writes `i/` and `w/` for a worktree diff when `diff.mnemonicPrefix` is on,
/// and the patches built from these have to be predictable.
const DIFF_ARGS: &[&str] = &[
    "diff",
    "--no-ext-diff",
    "--no-color",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

#[cfg(feature = "full")]
/// The `--format` every log is asked for.
const LOG_FORMAT_ARG: &str = "--format=%H%x1f%h%x1f%an%x1f%ar%x1f%D%x1f%s%x1e";

#[cfg(feature = "full")]
/// The arguments, a description, and anything to write to git's input.
fn git_command(action: GitAction) -> Option<(Vec<String>, String, Option<String>)> {
    let words = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let with_paths = |args: &[&str], paths: Vec<PathBuf>| {
        let mut out = words(args);
        // `--` first: a path that looks like an option is still a path.
        out.push("--".into());
        out.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
        out
    };
    Some(match action {
        GitAction::Stage(paths) => (with_paths(&["add", "--"], paths), "Stage".into(), None),
        GitAction::Unstage(paths) => (
            with_paths(&["restore", "--staged"], paths),
            "Unstage".into(),
            None,
        ),
        GitAction::StageAll => (words(&["add", "--all"]), "Stage everything".into(), None),
        GitAction::UnstageAll => (
            words(&["reset", "--quiet", "HEAD", "--"]),
            "Unstage everything".into(),
            None,
        ),
        GitAction::Discard(paths) => (
            with_paths(&["checkout", "--"], paths),
            "Discard".into(),
            None,
        ),
        GitAction::DeleteUntracked(paths) => (
            with_paths(&["clean", "-f", "--"], paths),
            "Delete".into(),
            None,
        ),
        GitAction::ApplyPatch {
            patch,
            arguments,
            describe,
        } => {
            let mut args = words(&["apply"]);
            args.extend(arguments);
            args.push("-".into());
            (args, describe, Some(patch))
        }
        GitAction::Commit {
            message,
            amend,
            arguments,
        } => {
            let mut args = words(&["commit", "--file=-"]);
            if amend {
                args.push("--amend".into());
            }
            args.extend(arguments);
            (args, "Commit".into(), Some(message))
        }
        GitAction::Push { arguments } => {
            let mut args = words(&["push"]);
            args.extend(arguments);
            (args, "Push".into(), None)
        }
        GitAction::Pull { arguments } => {
            let mut args = words(&["pull"]);
            // `--ff-only` unless the menu asked to rebase, so a pull never
            // makes a merge commit nobody asked for.
            if !arguments.iter().any(|flag| flag == "--rebase") {
                args.push("--ff-only".into());
            }
            args.extend(arguments);
            (args, "Pull".into(), None)
        }
        GitAction::Fetch { arguments } => {
            let mut args = words(&["fetch"]);
            args.extend(arguments);
            (args, "Fetch".into(), None)
        }
        GitAction::Checkout(name) => (
            vec!["checkout".into(), name.clone()],
            format!("Checkout {name}"),
            None,
        ),
        GitAction::CreateBranch(name) => (
            vec!["checkout".into(), "-b".into(), name.clone()],
            format!("Create branch {name}"),
            None,
        ),
        GitAction::Merge(name) => (
            vec!["merge".into(), name.clone()],
            format!("Merge {name}"),
            None,
        ),
        GitAction::Stash { message, arguments } => {
            let mut args = words(&["stash", "push"]);
            args.extend(arguments);
            if let Some(message) = message {
                args.push("--message".into());
                args.push(message);
            }
            (args, "Stash".into(), None)
        }
        GitAction::StashPop(name) => (
            vec!["stash".into(), "pop".into(), name],
            "Pop stash".into(),
            None,
        ),
        GitAction::StashApply(name) => (
            vec!["stash".into(), "apply".into(), name],
            "Apply stash".into(),
            None,
        ),
        GitAction::StashDrop(name) => (
            vec!["stash".into(), "drop".into(), name],
            "Drop stash".into(),
            None,
        ),
        GitAction::Run {
            arguments,
            describe,
        } => (arguments, describe, None),
        // These answer with a buffer rather than with a line of output, and
        // are handled before this. Reaching here means `git()` grew a variant
        // and forgot one, so it says so instead of quietly running the wrong
        // command — which is exactly the bug this arm used to hide.
        GitAction::Refresh
        | GitAction::Log { .. }
        | GitAction::Diff { .. }
        | GitAction::Show { .. } => return None,
    })
}

#[cfg(feature = "full")]
/// Runs git and returns its standard output as text, or nothing.
async fn git_output(root: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git_raw(root, args).await).into_owned()
}

#[cfg(feature = "full")]
/// Runs git and returns its standard output as bytes.
///
/// A failure is emptiness rather than an error: half of these commands fail
/// in the ordinary course of things — `@{upstream}` on a branch that has none
/// — and reporting that as a problem would bury the ones that are.
async fn git_raw(root: &Path, args: &[&str]) -> Vec<u8> {
    let mut process = tokio::process::Command::new("git");
    process
        .args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    match process.output().await {
        Ok(output) if output.status.success() => output.stdout,
        _ => Vec::new(),
    }
}

/// Removes files and directories, directories and all.
async fn delete_all(paths: &[PathBuf]) -> std::io::Result<()> {
    for path in paths {
        let metadata = tokio::fs::symlink_metadata(path).await?;
        match metadata.is_dir() {
            true => tokio::fs::remove_dir_all(path).await?,
            false => tokio::fs::remove_file(path).await?,
        }
    }
    Ok(())
}

/// Copies to a destination, which is a directory when there is more than one
/// thing to copy — as `cp` requires and for the same reason.
async fn copy_all(from: &[PathBuf], to: &Path) -> std::io::Result<()> {
    let into_directory = from.len() > 1 || tokio::fs::metadata(to).await.is_ok_and(|m| m.is_dir());
    for path in from {
        let destination = match into_directory {
            true => to.join(path.file_name().unwrap_or_default()),
            false => to.to_path_buf(),
        };
        match tokio::fs::symlink_metadata(path).await?.is_dir() {
            true => copy_directory(path, &destination).await?,
            false => {
                if let Some(parent) = destination.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::copy(path, &destination).await?;
            }
        }
    }
    Ok(())
}

/// Copies a directory, one level of recursion at a time.
///
/// Written with an explicit stack rather than recursively, because an `async
/// fn` that calls itself needs boxing and a stack is clearer than that.
async fn copy_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        tokio::fs::create_dir_all(&destination).await?;
        let mut reader = tokio::fs::read_dir(&source).await?;
        while let Some(entry) = reader.next_entry().await? {
            let target = destination.join(entry.file_name());
            match entry.metadata().await?.is_dir() {
                true => pending.push((entry.path(), target)),
                false => {
                    tokio::fs::copy(entry.path(), target).await?;
                }
            }
        }
    }
    Ok(())
}

async fn rename_all(from: &[PathBuf], to: &Path) -> std::io::Result<()> {
    let into_directory = from.len() > 1 || tokio::fs::metadata(to).await.is_ok_and(|m| m.is_dir());
    for path in from {
        let destination = match into_directory {
            true => to.join(path.file_name().unwrap_or_default()),
            false => to.to_path_buf(),
        };
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(path, &destination).await?;
    }
    Ok(())
}

/// `rwxr-xr-x`, where the platform has such a thing.
fn permissions_of(metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = metadata.permissions().mode();
        let bit = |shift: u32, letter: char| match mode >> shift & 1 {
            1 => letter,
            _ => '-',
        };
        let kind = match metadata.is_dir() {
            true => 'd',
            false => '-',
        };
        [
            kind,
            bit(8, 'r'),
            bit(7, 'w'),
            bit(6, 'x'),
            bit(5, 'r'),
            bit(4, 'w'),
            bit(3, 'x'),
            bit(2, 'r'),
            bit(1, 'w'),
            bit(0, 'x'),
        ]
        .into_iter()
        .collect()
    }
    #[cfg(not(unix))]
    {
        match (metadata.is_dir(), metadata.permissions().readonly()) {
            (true, _) => "d---------".to_string(),
            (false, true) => "-r--------".to_string(),
            (false, false) => "-rw-------".to_string(),
        }
    }
}

/// `Aug 29 15:03`, or the year for anything older than six months, which is
/// what `ls` does and for the same reason: the time stops being the useful
/// half once something is old.
fn modified_of(metadata: &std::fs::Metadata) -> String {
    let Ok(time) = metadata.modified() else {
        return String::new();
    };
    let Ok(since) = time.duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };
    let seconds = since.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let name = MONTHS[(month as usize).clamp(1, 12) - 1];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(seconds);
    if (now - seconds).abs() > 180 * 86_400 {
        return format!("{name} {day:>2}  {year}");
    }
    let minutes = seconds.rem_euclid(86_400) / 60;
    format!("{name} {day:>2} {:02}:{:02}", minutes / 60, minutes % 60)
}

/// Days since the epoch to a calendar date, by Howard Hinnant's algorithm.
///
/// A date is wanted and no dependency is: the whole of what is needed from a
/// calendar here is a month name and a day number.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = match mp < 10 {
        true => mp + 3,
        false => mp - 9,
    } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// Directories a search for somewhere to work should not walk into.
///
/// Dotfiles because a home directory is mostly caches and state, and the
/// rest because they hold thousands of directories nobody is looking for and
/// walking them is most of what a search would cost.
fn skip(name: &str) -> bool {
    const HEAVY: &[&str] = &[
        "node_modules",
        "target",
        "vendor",
        "__pycache__",
        ".venv",
        "venv",
        "dist",
        "build",
    ];
    name.starts_with('.') || HEAVY.contains(&name)
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    /// A little tree with the things a real home directory has in it.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("maxgus-walk-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        for path in [
            "Projects/editor/src",
            "Projects/website",
            "Projects/editor/target/debug",
            "Projects/editor/node_modules/left-pad",
            ".cache/nothing",
            "notes",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        std::fs::write(root.join("notes/a.txt"), "a").unwrap();
        root
    }

    /// The walk, run the way the task runs it.
    async fn walk(root: &Path) -> (Vec<String>, bool) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let executor = Executor::new(root.to_path_buf(), TreeConfig::default(), Vec::new(), tx);
        executor.find_directories(root.to_path_buf()).await;
        match rx.recv().await {
            Some(TaskResult::DirectoriesFound { paths, capped, .. }) => (paths, capped),
            other => panic!("expected a walk, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_walk_finds_directories_by_their_path_below_the_root() {
        let root = fixture("finds");
        let (paths, capped) = walk(&root).await;
        assert!(!capped);
        assert!(
            paths.contains(&"Projects/editor/src".to_string()),
            "got {paths:?}"
        );
        assert!(paths.contains(&"notes".to_string()), "got {paths:?}");
    }

    #[tokio::test]
    async fn the_walk_leaves_out_files_caches_and_build_directories() {
        // A home directory is mostly things nobody is looking for, and
        // walking them is most of what a search would cost.
        let root = fixture("skips");
        let (paths, _) = walk(&root).await;
        assert!(
            !paths.iter().any(|p| p.contains("node_modules")),
            "got {paths:?}"
        );
        assert!(!paths.iter().any(|p| p.contains("target")), "got {paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with('.')), "got {paths:?}");
        assert!(
            !paths.iter().any(|p| p.ends_with("a.txt")),
            "a file was offered as a directory: {paths:?}"
        );
    }

    #[test]
    fn what_a_search_for_somewhere_to_work_walks_past() {
        assert!(skip(".git"));
        assert!(skip(".cache"));
        assert!(skip("node_modules"));
        assert!(skip("target"));
        assert!(!skip("src"));
        assert!(!skip("Projects"));
    }
}

#[cfg(test)]
mod dired_tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("maxgus-dired-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested/deeper")).unwrap();
        std::fs::write(root.join("a.txt"), "alpha").unwrap();
        std::fs::write(root.join("b.txt"), "beta").unwrap();
        std::fs::write(root.join("nested/deeper/c.txt"), "gamma").unwrap();
        root
    }

    #[tokio::test]
    async fn deleting_takes_files_and_whole_directories() {
        let root = fixture("delete");
        delete_all(&[root.join("a.txt"), root.join("nested")])
            .await
            .unwrap();
        assert!(!root.join("a.txt").exists());
        assert!(
            !root.join("nested").exists(),
            "the directory is still there"
        );
        assert!(root.join("b.txt").exists(), "it took something else too");
    }

    #[tokio::test]
    async fn copying_one_file_to_a_name_makes_that_name() {
        let root = fixture("copyone");
        copy_all(&[root.join("a.txt")], &root.join("copy.txt"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("copy.txt")).unwrap(),
            "alpha"
        );
        assert!(root.join("a.txt").exists(), "the original is gone");
    }

    #[tokio::test]
    async fn copying_several_things_puts_them_in_the_directory() {
        let root = fixture("copymany");
        let into = root.join("into");
        std::fs::create_dir_all(&into).unwrap();
        copy_all(&[root.join("a.txt"), root.join("b.txt")], &into)
            .await
            .unwrap();
        assert!(into.join("a.txt").exists());
        assert!(into.join("b.txt").exists());
    }

    #[tokio::test]
    async fn copying_a_directory_takes_what_is_inside_it() {
        let root = fixture("copydir");
        copy_all(&[root.join("nested")], &root.join("clone"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("clone/deeper/c.txt")).unwrap(),
            "gamma",
            "the copy did not go all the way down"
        );
    }

    #[tokio::test]
    async fn renaming_moves_rather_than_copies() {
        let root = fixture("rename");
        rename_all(&[root.join("a.txt")], &root.join("renamed.txt"))
            .await
            .unwrap();
        assert!(!root.join("a.txt").exists(), "the original is still there");
        assert_eq!(
            std::fs::read_to_string(root.join("renamed.txt")).unwrap(),
            "alpha"
        );
    }

    #[tokio::test]
    async fn renaming_several_things_moves_them_into_the_directory() {
        let root = fixture("renamemany");
        let into = root.join("into");
        std::fs::create_dir_all(&into).unwrap();
        rename_all(&[root.join("a.txt"), root.join("b.txt")], &into)
            .await
            .unwrap();
        assert!(into.join("a.txt").exists() && into.join("b.txt").exists());
        assert!(!root.join("a.txt").exists());
    }

    #[test]
    fn permissions_read_as_ls_writes_them() {
        let root = fixture("perms");
        let file = std::fs::metadata(root.join("a.txt")).unwrap();
        let directory = std::fs::metadata(root.join("nested")).unwrap();
        let shown = permissions_of(&file);
        assert_eq!(shown.len(), 10, "got `{shown}`");
        assert!(
            shown.starts_with('-'),
            "a file is not a directory: `{shown}`"
        );
        assert!(
            permissions_of(&directory).starts_with('d'),
            "a directory should say so"
        );
    }

    #[test]
    fn a_date_is_written_the_way_a_listing_writes_one() {
        // Two dates a long way apart: the recent one carries a time, the old
        // one carries a year, as `ls` does.
        let recent = modified_of(&std::fs::metadata(fixture("dates").join("a.txt")).unwrap());
        assert!(
            recent.contains(':'),
            "a file written moments ago should show a time: `{recent}`"
        );
        // Dates checked against a calendar rather than against the same
        // arithmetic written twice: a leap day, a century year, and one
        // either side of the epoch.
        assert_eq!(civil_from_days(0), (1970, 1, 1), "the epoch is wrong");
        assert_eq!(civil_from_days(-1), (1969, 12, 31), "before the epoch");
        assert_eq!(
            civil_from_days(11_017),
            (2000, 3, 1),
            "the day after a leap day"
        );
        assert_eq!(civil_from_days(18_993), (2022, 1, 1));
        assert_eq!(civil_from_days(19_600), (2023, 8, 31));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }
}
