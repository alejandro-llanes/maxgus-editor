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

/// A parse that has finished, on its way back to the executor.
#[cfg(feature = "full")]
struct Parsed {
    buffer: maxgus_text::BufferId,
    revision: u64,
    /// `None` when the parse panicked, which takes the parser with it: the
    /// next request starts a fresh one.
    syntax: Option<BufferSyntax>,
    highlights: Option<(std::ops::Range<usize>, Vec<maxgus_syntax::Highlight>)>,
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
    /// The workspace folders each running server has been told about.
    #[cfg(feature = "full")]
    server_roots: HashMap<String, Vec<PathBuf>>,
    /// The text each open document was last sent as, so a change can be
    /// described as the region that differs rather than the whole file.
    #[cfg(feature = "full")]
    documents: HashMap<String, String>,
    #[cfg(feature = "full")]
    lsp_specs: Vec<LspSpec>,
    /// Shells running on pseudo-terminals, by tab.
    #[cfg(feature = "full")]
    terminals: HashMap<TerminalId, Terminal>,
    /// Where a grammar the editor installs goes, and where the parser list
    /// is cached. Unset in a build that was never told, which is every
    /// caller but the editor itself — and then nothing can be installed.
    #[cfg(feature = "full")]
    grammar_home: Option<PathBuf>,
    /// Languages already reported as having no grammar, so the offer is
    /// made once rather than on every pause in typing.
    #[cfg(feature = "full")]
    announced: std::collections::HashSet<String>,
    /// The parser list as cached on disk, read once and kept.
    ///
    /// `None` until it has been looked for; `Some(None)` when there is no
    /// cache, which is how the editor knows it cannot say whether a parser
    /// for a language exists.
    #[cfg(feature = "full")]
    catalog: Option<Option<maxgus_syntax::Catalog>>,

    reporter: Reporter,
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
    /// The program's output has ended: it has exited, or is about to.
    Ended,
}

/// The configured directories, with the editor's own install directory
/// after them.
///
/// After, not before: a grammar a package manager installed and the user
/// pointed at is the one they meant, and it should not be shadowed by
/// something this editor built earlier.
#[cfg(feature = "full")]
fn with_home(mut directories: Vec<PathBuf>, home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(home) = home {
        directories.push(home.to_path_buf());
    }
    directories
}

/// Somewhere to say how a job went, and nothing else.
///
/// The jobs that need no more than that — a shell command, a project search,
/// a walk of the home directory, a copy, git — are run beside the executor
/// rather than in its queue. In it, `M-! sleep 10`, a search of a large
/// project or a push to a slow remote held up every save and every file read
/// queued behind it, for as long as it took.
#[derive(Clone)]
struct Reporter {
    results: mpsc::UnboundedSender<TaskResult>,
}

impl Reporter {
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
            FileAction::Delete(paths) => {
                let outcome = delete_all(&paths).await;
                // Whatever did go, even when something after it would not:
                // a buffer over a deleted file writes it back on save.
                for path in paths {
                    if !tokio::fs::try_exists(&path).await.unwrap_or(true) {
                        self.send(TaskResult::PathDeleted { path });
                    }
                }
                outcome
            }
            FileAction::Copy { from, to } => copy_all(&from, &to).await,
            FileAction::Rename { from, to } => match rename_all(&from, &to).await {
                Ok(moved) => {
                    for (from, to) in moved {
                        self.send(TaskResult::PathMoved { from, to });
                    }
                    Ok(())
                }
                Err(error) => Err(error),
            },
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
        never_ask_at_the_terminal(&mut process);
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

    /// Searches the project on a blocking thread.
    ///
    /// Walking a tree and reading every file in it is exactly the work tokio
    /// asks not to be done on its own threads, and a large project would stop
    /// every other task while it ran.
    #[cfg(feature = "full")]
    async fn grep(&self, root: PathBuf, search: maxgus_grep::Search) {
        let pattern = search.pattern.clone();
        let searched = root.clone();
        let outcome =
            tokio::task::spawn_blocking(move || maxgus_grep::search(&searched, &search)).await;
        match outcome {
            Ok(Ok(found)) => self.send(TaskResult::GrepFinished {
                pattern,
                root,
                found,
            }),
            Ok(Err(error)) => self.fail("search", error),
            Err(error) => self.fail("search", error),
        }
    }

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
/// Git's jobs, one after another on a queue of their own.
///
/// One at a time, because two gits at once in one repository fight over its
/// index lock, and a status read between a stage and the refresh after it
/// shows the stage undone. On their own queue, because a push to a slow
/// remote is no reason for a save to wait.
fn git_worker(reporter: Reporter) -> mpsc::UnboundedSender<(PathBuf, GitAction)> {
    let (sender, mut jobs) = mpsc::unbounded_channel::<(PathBuf, GitAction)>();
    tokio::spawn(async move {
        while let Some((root, action)) = jobs.recv().await {
            reporter.git(root, action).await;
        }
    });
    sender
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
        Executor::with_grammars(
            root,
            tree_config,
            lsp_specs,
            Default::default(),
            None,
            results,
        )
    }

    /// The same, told where to look for grammars the editor was not built
    /// with. Nothing is looked for unless this says where.
    pub fn with_grammars(
        root: PathBuf,
        tree_config: TreeConfig,
        #[cfg_attr(not(feature = "full"), allow(unused_variables))] lsp_specs: Vec<LspSpec>,
        #[cfg_attr(not(feature = "full"), allow(unused_variables))]
        grammars: maxgus_config::GrammarConfig,
        // `grammar_home` is the directory the editor installs grammars into.
        // It is searched as well as the configured ones, because everything
        // in it was put there by an install the user agreed to.
        #[cfg_attr(not(feature = "full"), allow(unused_variables))] grammar_home: Option<PathBuf>,
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
                libraries: with_home(grammars.search, grammar_home.as_deref()),
                queries: with_home(grammars.queries, grammar_home.as_deref()),
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
            server_roots: HashMap::new(),
            #[cfg(feature = "full")]
            terminals: HashMap::new(),
            #[cfg(feature = "full")]
            documents: HashMap::new(),
            #[cfg(feature = "full")]
            lsp_specs,
            #[cfg(feature = "full")]
            grammar_home,
            #[cfg(feature = "full")]
            announced: std::collections::HashSet::new(),
            #[cfg(feature = "full")]
            catalog: None,
            reporter: Reporter { results },
        }
    }

    /// Runs until the task channel closes.
    pub async fn run(mut self, mut tasks: mpsc::UnboundedReceiver<Task>) {
        #[cfg(feature = "full")]
        let git = git_worker(self.reporter.clone());
        // Parses run beside the loop rather than in it. In it, every task
        // behind a parse waited for it — a file read, a language server's
        // request — and a parse that has to recover from a syntax error
        // near the top of a large file takes seconds. Each buffer has at
        // most one parse running and one request waiting behind it, the
        // newest: typing through a slow parse used to queue a parse a key.
        #[cfg(feature = "full")]
        let (parsed_tx, mut parsed_rx) = mpsc::unbounded_channel::<Parsed>();
        #[cfg(feature = "full")]
        let mut parsing: HashMap<maxgus_text::BufferId, Option<Task>> = HashMap::new();
        loop {
            #[cfg(feature = "full")]
            let task = tokio::select! {
                task = tasks.recv() => task,
                Some(parsed) = parsed_rx.recv() => {
                    // A buffer forgotten while it was being parsed is not
                    // put back.
                    let Some(waiting) = parsing.remove(&parsed.buffer) else {
                        continue;
                    };
                    self.finish_parse(parsed);
                    if let Some(next) = waiting {
                        self.start_parse(next, &parsed_tx, &mut parsing).await;
                    }
                    continue;
                }
            };
            #[cfg(not(feature = "full"))]
            let task = tasks.recv().await;
            let Some(task) = task else {
                break;
            };
            let reporter = self.reporter.clone();
            match task {
                #[cfg(feature = "full")]
                Task::Reparse { buffer, .. } => match parsing.get_mut(&buffer) {
                    Some(waiting) => *waiting = Some(task),
                    None => self.start_parse(task, &parsed_tx, &mut parsing).await,
                },
                #[cfg(feature = "full")]
                Task::ForgetBuffer { buffer } => {
                    parsing.remove(&buffer);
                    self.handle(task).await;
                }
                Task::Shell {
                    command,
                    directory,
                    insert_at,
                } => {
                    tokio::spawn(
                        async move { reporter.shell(command, directory, insert_at).await },
                    );
                }
                Task::FindDirectories { root } => {
                    tokio::spawn(async move { reporter.find_directories(root).await });
                }
                Task::DiredAct { action } => {
                    tokio::spawn(async move { reporter.dired_act(action).await });
                }
                #[cfg(feature = "full")]
                Task::Grep { root, search } => {
                    tokio::spawn(async move { reporter.grep(root, search).await });
                }
                #[cfg(feature = "full")]
                Task::Git { root, action } => {
                    let _ = git.send((root, action));
                }
                task => self.handle(task).await,
            }
        }
        // Leaving without shutting servers down would orphan the processes.
        self.shutdown().await;
    }

    fn send(&self, result: TaskResult) {
        self.reporter.send(result);
    }

    /// Reports a failure to the editor rather than swallowing it.
    fn fail(&self, context: &str, error: impl std::fmt::Display) {
        self.reporter.fail(context, error);
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
            Task::InsertFile { path, buffer } => self.insert_file(path, buffer).await,
            #[cfg(feature = "full")]
            Task::ReadFileForEdits { path } => self.read_file_for_edits(path).await,
            Task::RestoreFile { path } => {
                match tokio::fs::try_exists(&path).await.unwrap_or(false) {
                    true => self.read_file(path, None, false).await,
                    false => self.send(TaskResult::Said(format!(
                        "{} is gone, so the session left it out",
                        path.display()
                    ))),
                }
            }
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
            #[cfg(feature = "full")]
            Task::GrammarCatalog { refresh, language } => {
                self.grammar_catalog(refresh, language).await
            }
            #[cfg(feature = "full")]
            Task::InstallGrammar { language, url } => self.install_grammar(language, url).await,
            Task::Dired { path } => self.dired(path).await,
            Task::Browse { path } => self.browse(path).await,
            Task::FindDirectories { root } => self.reporter.find_directories(root).await,
            Task::DiredAct { action } => self.reporter.dired_act(action).await,
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
            Task::StartLanguageServer { language, file } => {
                self.start_server(&language, file).await
            }
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
                self.reporter.shell(command, directory, insert_at).await;
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
            Task::Git { root, action } => self.reporter.git(root, action).await,
            #[cfg(feature = "full")]
            Task::Grep { root, search } => self.reporter.grep(root, search).await,
            #[cfg(feature = "full")]
            Task::ApplyGrep {
                replacements,
                unsaved,
            } => self.apply_grep(replacements, unsaved).await,
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
        // What kind of thing it is, before any of it is read.
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            // A directory named at a file prompt is one to look at, as Emacs'
            // `find-file` opens dired on it. It was an "Is a directory".
            if metadata.is_dir() && reverting.is_none() {
                self.dired(path).await;
                return;
            }
            // A pipe or a device is read by waiting for whatever writes to
            // it, and every other job the editor has queued would wait too.
            if !metadata.is_file() {
                self.fail(
                    "find-file",
                    format!("{} is not a regular file", path.display()),
                );
                return;
            }
        }
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                // A picture is decoded rather than shown as the bytes it is
                // made of. One that will not decode falls through and is
                // read as the bytes are, the way any file is.
                #[cfg(feature = "full")]
                if maxgus_core::picture::is_picture(&path) {
                    let decoded = {
                        let bytes = bytes.clone();
                        let path = path.clone();
                        tokio::task::spawn_blocking(move || decode_picture(&bytes, &path))
                            .await
                            .ok()
                            .flatten()
                    };
                    if let Some(picture) = decoded {
                        self.send(TaskResult::PictureRead {
                            path,
                            picture: std::sync::Arc::new(picture),
                            reverting,
                            other_window,
                        });
                        return;
                    }
                }
                // Invalid UTF-8 is shown rather than refused, so a stray byte
                // in an otherwise readable file does not stop it being read.
                // What it cannot be is *saved*: the replacement characters
                // would go to disk over the bytes they stand in for, so the
                // buffer is opened read-only and says why.
                let lossy = std::str::from_utf8(&bytes).is_err();
                let contents = String::from_utf8_lossy(&bytes).into_owned();
                let metadata = tokio::fs::metadata(&path).await.ok();
                let read_only = lossy || metadata.as_ref().is_some_and(|m| !may_write(m));
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

    /// Reads a file a language server's edit is waiting to be made in.
    #[cfg(feature = "full")]
    async fn read_file_for_edits(&self, path: PathBuf) {
        let why = "the language server's change to it was not made";
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                return self.fail("rename", format!("{} is not a file; {why}", path.display()));
            }
            Err(error) => {
                return self.fail("rename", format!("{}: {error}; {why}", path.display()));
            }
        };
        match tokio::fs::read(&path).await {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => self.send(TaskResult::FileReadForEdits {
                    read_only: !may_write(&metadata),
                    disk_time: metadata.modified().ok(),
                    path,
                    contents,
                }),
                Err(_) => self.fail("rename", format!("{} is not text; {why}", path.display())),
            },
            Err(error) => self.fail("rename", format!("{}: {error}; {why}", path.display())),
        }
    }

    /// Reads a file for `insert-file`.
    ///
    /// Text only: bytes that are not UTF-8 would be inserted as replacement
    /// characters and saved as them, which is the change the read-only guard
    /// on visiting such a file exists to prevent.
    async fn insert_file(&self, path: PathBuf, buffer: maxgus_text::BufferId) {
        if tokio::fs::metadata(&path).await.is_ok_and(|m| !m.is_file()) {
            self.fail(
                "insert-file",
                format!("{} is not a regular file", path.display()),
            );
            return;
        }
        match tokio::fs::read(&path).await {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => self.send(TaskResult::FileInserted {
                    path,
                    buffer,
                    contents,
                }),
                Err(_) => self.fail(
                    "insert-file",
                    format!("{} is not text; nothing was inserted", path.display()),
                ),
            },
            Err(error) => self.fail("insert-file", format!("{}: {error}", path.display())),
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
            // Through a link to what it points at: a link to a directory is
            // somewhere `RET` goes, and was listed as a file `RET` could not
            // open. A link to nothing is still listed, as the link it is.
            let metadata = match tokio::fs::metadata(entry.path()).await {
                Ok(metadata) => metadata,
                Err(_) => match entry.metadata().await {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                },
            };
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

    /// Reads the script file. A project with none is the usual case and not
    /// a failure.
    #[cfg(feature = "full")]
    async fn read_script(&self, path: PathBuf) {
        match tokio::fs::read_to_string(&path).await {
            Ok(source) => self.send(TaskResult::ScriptRead { source, path }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.send(TaskResult::ScriptMissing { path });
            }
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
        match write_safely(&path, contents.as_bytes()).await {
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

    /// Opens the tree for `directory`, or keeps it when it already shows it.
    ///
    /// Returns the directory it was rooted at, when this rooted it. The tree
    /// used to open wherever the editor had started — from an application
    /// menu, the home directory — whatever the file on the screen was, and
    /// kept doing so however many projects were visited after.
    async fn show_tree(&mut self, directory: &Path) -> maxgus_tree::Result<Option<PathBuf>> {
        let project = project_directory(directory).await;
        match self.tree.as_mut() {
            None => {
                let root = project.unwrap_or_else(|| directory.to_path_buf());
                let tree = match FileTree::open(root.clone(), self.tree_config.clone()).await {
                    Ok(tree) => tree,
                    // A buffer for a file not written yet, in a directory
                    // that does not exist yet: the tree still has to open
                    // somewhere.
                    Err(_) => FileTree::open(self.root.clone(), self.tree_config.clone()).await?,
                };
                let home = tree.root_path().to_path_buf();
                self.tree = Some(tree);
                Ok(Some(home))
            }
            Some(tree) if tree.root_holding(directory).is_some() => {
                tree.refresh().await?;
                Ok(None)
            }
            // A file in another repository: the tree goes to that project,
            // as Doom's treemacs goes to the project being worked in.
            Some(tree) if project.is_some() => {
                let root = project.expect("checked by the guard");
                tree.set_roots(vec![root.clone()]).await?;
                Ok(Some(root))
            }
            // Somewhere that is no project at all — a dotfile in the home
            // directory — is not worth losing what the tree was showing.
            Some(tree) => {
                tree.refresh().await?;
                Ok(None)
            }
        }
    }

    async fn tree_action(&mut self, action: TreeAction) {
        // What the editor should take as where the tree lives now, when the
        // action rooted it somewhere.
        let mut home: Option<PathBuf> = None;
        match &action {
            TreeAction::Close => {
                self.tree = None;
                return;
            }
            TreeAction::Show(directory) => match self.show_tree(directory).await {
                Ok(rooted) => home = rooted,
                Err(error) => {
                    self.fail("File tree", error);
                    return;
                }
            },
            _ => {}
        }
        if let Err(error) = self.ensure_tree().await {
            self.fail("File tree", error);
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
        // What to say about it, and what happened to files a buffer may be
        // visiting.
        let mut said: Option<String> = None;
        let mut moved: Option<(PathBuf, PathBuf)> = None;
        let mut deleted: Option<PathBuf> = None;
        let outcome: Result<(), maxgus_tree::TreeError> = match action {
            TreeAction::Show(_) | TreeAction::Close => Ok(()),
            TreeAction::Refresh => tree.refresh().await,
            TreeAction::Toggle(path) => tree.toggle(&path).await.map(|_| ()),
            TreeAction::Expand(path) => tree.expand(&path).await,
            TreeAction::Collapse(path) => {
                tree.collapse(&path);
                Ok(())
            }
            TreeAction::ExpandRecursively(path) => {
                tree.expand_recursively(&path).await.map(|expansion| {
                    if expansion.stopped {
                        said = Some(format!(
                            "Opened the first {} directories and stopped; open deeper ones \
                             one at a time",
                            expansion.directories
                        ));
                    }
                })
            }
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
                    Ok(dropped) => {
                        if !dropped.is_empty() {
                            let names: Vec<String> = dropped
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect();
                            said = Some(format!("Not readable, left out: {}", names.join(", ")));
                        }
                        home = Some(tree.root_path().to_path_buf());
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            TreeAction::ToggleHidden => tree.toggle_show_hidden().await.map(|()| {
                said = Some(
                    match tree.config().show_hidden {
                        true => "Dotfiles shown",
                        false => "Dotfiles hidden",
                    }
                    .into(),
                );
            }),
            TreeAction::ToggleDirectoriesFirst => {
                let on = !tree.config().directories_first;
                self.tree_config.directories_first = on;
                tree.set_directories_first(on).await.map(|()| {
                    said = Some(
                        match on {
                            true => "Directories first",
                            false => "Directories sorted in among the files",
                        }
                        .into(),
                    );
                })
            }
            TreeAction::ToggleGitStatus => {
                let on = !tree.config().git_status;
                self.tree_config.git_status = on;
                tree.set_git_status(on).await.map(|()| {
                    said = Some(
                        match on {
                            true => "Git status shown",
                            false => "Git status hidden",
                        }
                        .into(),
                    );
                })
            }
            TreeAction::CreateFile { parent, name } => match Self::at(tree, &parent) {
                Ok(()) => tree.create_file(&name).await.map(|path| {
                    said = Some(format!("Created {}", tree.shown(&path)));
                    select = Some(path);
                }),
                Err(error) => Err(error),
            },
            TreeAction::CreateDirectory { parent, name } => match Self::at(tree, &parent) {
                Ok(()) => tree.create_directory(&name).await.map(|path| {
                    said = Some(format!("Created {}/", tree.shown(&path)));
                    select = Some(path);
                }),
                Err(error) => Err(error),
            },
            TreeAction::Delete(path) => match Self::at(tree, &path) {
                Ok(()) => {
                    // Named before it goes: afterwards there is nothing in the
                    // tree to name it from.
                    let shown = tree.shown(&path);
                    tree.delete_selected().await.map(|gone| {
                        said = Some(format!("Deleted {shown}"));
                        deleted = Some(gone);
                        select = None;
                    })
                }
                Err(error) => Err(error),
            },
            TreeAction::Rename { path, name } => match Self::at(tree, &path) {
                Ok(()) => tree.rename_selected(&name).await.map(|new| {
                    said = Some(format!(
                        "Renamed {} to {}",
                        tree.shown(&path),
                        new.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    moved = Some((path, new.clone()));
                    select = Some(new);
                }),
                Err(error) => Err(error),
            },
            TreeAction::Move { path, destination } => match Self::at(tree, &path) {
                Ok(()) => {
                    let shown = tree.shown(&path);
                    tree.move_selected(&destination).await.map(|new| {
                        said = Some(format!("Moved {shown} to {}", tree.shown(&destination)));
                        moved = Some((path, new.clone()));
                        select = Some(new);
                    })
                }
                Err(error) => Err(error),
            },
        };
        if let Err(error) = outcome {
            let message = tree.explain(&error);
            self.fail("File tree", message);
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        self.send(TaskResult::TreeUpdated {
            nodes: tree.visible().to_vec(),
            select,
            show_hidden: tree.config().show_hidden,
            roots: tree.roots().into_iter().map(Path::to_path_buf).collect(),
            home,
        });
        if let Some((from, to)) = moved {
            self.send(TaskResult::PathMoved { from, to });
        }
        if let Some(path) = deleted {
            self.send(TaskResult::PathDeleted { path });
        }
        if let Some(said) = said {
            self.send(TaskResult::Said(said));
        }
    }

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

    /// Says once that a language has no grammar, so the editor can offer to
    /// fetch one.
    ///
    /// Once per language per session. Re-highlighting happens on every lull
    /// in typing, and a question that came back every few seconds would make
    /// the file unusable rather than helpful.
    ///
    /// What is sent depends on what can be known without going to the
    /// network, which is where the names compiled into
    /// [`maxgus_syntax::is_known`] earn their place. A cached parser list is
    /// consulted first and its rows go with the message. Failing that, the
    /// shipped names say whether anyone has written a parser for this
    /// language at all — and if nobody has, **nothing is sent**. `txt`,
    /// `log` and `bak` are languages as far as a file extension is
    /// concerned, and a question about installing a `txt` grammar in front
    /// of a file being typed into is the feature making itself unusable.
    #[cfg(feature = "full")]
    async fn announce_missing(&mut self, language: &str) {
        // Nowhere to install to means nothing to offer.
        if self.grammar_home.is_none() || !self.announced.insert(language.to_string()) {
            return;
        }
        let candidates = match self.cached_catalog().await {
            Some(catalog) => match catalog.for_language(language) {
                found if found.is_empty() => return,
                found => found.into_iter().cloned().collect(),
            },
            // No list on disk yet. The names say whether it is worth asking
            // to fetch one; the repository can only be named afterwards.
            None => match maxgus_syntax::is_known(language) {
                true => Vec::new(),
                false => return,
            },
        };
        self.send(TaskResult::GrammarMissing {
            language: language.to_string(),
            candidates,
        });
    }

    /// The parser list as already cached, read once per session and never
    /// fetched. Nothing here goes to the network: an editor that phoned home
    /// on opening a file would be doing it without being asked.
    #[cfg(feature = "full")]
    async fn cached_catalog(&mut self) -> Option<&maxgus_syntax::Catalog> {
        if self.catalog.is_none() {
            let cache = Executor::catalog_cache(self.grammar_home.as_ref()?);
            let read =
                tokio::task::spawn_blocking(move || maxgus_syntax::install::cached_catalog(&cache))
                    .await
                    .ok()
                    .flatten();
            self.catalog = Some(read.map(|text| maxgus_syntax::Catalog::parse(&text)));
        }
        self.catalog.as_ref()?.as_ref()
    }

    /// The file the parser list is cached in, beside the grammars rather
    /// than among them.
    #[cfg(feature = "full")]
    fn catalog_cache(home: &Path) -> PathBuf {
        home.parent().unwrap_or(home).join("parser-list.md")
    }

    /// Reads tree-sitter's list of parsers, from the cache or the wiki.
    ///
    /// Cloning a repository blocks, so it goes to the blocking pool for the
    /// same reason opening a shared library does.
    #[cfg(feature = "full")]
    async fn grammar_catalog(&mut self, refresh: bool, language: Option<String>) {
        let Some(home) = self.grammar_home.clone() else {
            self.send(TaskResult::GrammarCatalog {
                language,
                parsers: Vec::new(),
                error: Some("this build has nowhere to install grammars".to_string()),
            });
            return;
        };
        let cache = Executor::catalog_cache(&home);
        let read =
            tokio::task::spawn_blocking(move || maxgus_syntax::install::catalog(&cache, refresh))
                .await;
        let (parsers, error) = match read {
            Ok(Ok(text)) => {
                let catalog = maxgus_syntax::Catalog::parse(&text);
                // Asked about a language: the parsers that would colour it,
                // best first. Asked about nothing: all of them, in the
                // order the wiki lists them, which is alphabetical.
                let parsers = match &language {
                    Some(language) => catalog
                        .for_language(language)
                        .into_iter()
                        .cloned()
                        .collect(),
                    None => catalog.entries().to_vec(),
                };
                (parsers, None)
            }
            Ok(Err(error)) => (Vec::new(), Some(error.to_string())),
            Err(_) => (Vec::new(), Some("the fetch did not finish".to_string())),
        };
        self.send(TaskResult::GrammarCatalog {
            language,
            parsers,
            error,
        });
    }

    /// Clones, builds and installs one grammar, then loads it.
    ///
    /// Loading it here rather than leaving it for the next keystroke is what
    /// turns "the files are on disk" into an answer: a grammar built against
    /// a different tree-sitter, or exporting a symbol under another name,
    /// fails at `dlopen` and the user finds out now, next to the install
    /// that caused it.
    #[cfg(feature = "full")]
    async fn install_grammar(&mut self, language: String, url: String) {
        let Some(home) = self.grammar_home.clone() else {
            self.send(TaskResult::GrammarInstalled {
                language,
                summary: "This build has nowhere to install grammars".to_string(),
                log: String::new(),
                failed: true,
            });
            return;
        };
        let request = maxgus_syntax::InstallRequest {
            language: language.clone(),
            url: url.clone(),
            into: home,
        };
        let built =
            tokio::task::spawn_blocking(move || maxgus_syntax::install::install(&request)).await;
        let report = match built {
            Ok(Ok(report)) => report,
            Ok(Err(error)) => {
                self.send(TaskResult::GrammarInstalled {
                    language,
                    summary: format!("{url} would not install: {error}"),
                    log: error.to_string(),
                    failed: true,
                });
                return;
            }
            Err(_) => {
                self.send(TaskResult::GrammarInstalled {
                    language,
                    summary: format!("The install of {url} did not finish"),
                    log: String::new(),
                    failed: true,
                });
                return;
            }
        };

        // It was looked for once and found missing; that answer is now out
        // of date, and so is having said so.
        self.grammars.forget(&language);
        self.announced.remove(&language);
        // The list was just fetched to get here, so what was read from the
        // cache before is out of date.
        self.catalog = None;
        let loaded = self.grammar_for(&language).await.is_some();

        let mut log = report.log.clone();
        for warning in &report.warnings {
            log.push_str(warning);
            log.push('\n');
        }
        let summary = match (loaded, report.warnings.first()) {
            (true, None) => format!(
                "{language}: installed {} from {url}",
                report.library.display()
            ),
            (true, Some(warning)) => format!("{language}: installed, but {warning}"),
            (false, _) => format!(
                "{language}: built, but it would not load: {}",
                self.grammars
                    .failure(&language)
                    .unwrap_or("no grammar for it after all")
            ),
        };
        self.send(TaskResult::GrammarInstalled {
            language,
            summary,
            log,
            failed: !loaded,
        });
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
                            match self.grammars.caveat(name) {
                                Some(caveat) => {
                                    let _ = writeln!(out, "  {name} — {caveat}");
                                }
                                None => {
                                    let _ = writeln!(out, "  {name}");
                                }
                            }
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
        if let Some(home) = &self.grammar_home {
            let _ = write!(
                out,
                "\nInstalled by this editor into\n  {}\n                   M-x install-grammar chooses one to fetch and build.\n",
                home.display()
            );
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
        let Some(syntax) = self.syntax_for(buffer, language).await else {
            return;
        };
        // Parsing a large file is a quarter of a second of solid CPU with
        // nothing in it to await. Run on a runtime thread it would stop tokio
        // polling anything else for that whole time — the language server's
        // transport and the terminal's input among them — so it goes to the
        // blocking pool and the workers stay free.
        let parsed =
            tokio::task::spawn_blocking(move || parse(buffer, revision, syntax, text, range)).await;
        if let Ok(parsed) = parsed {
            self.finish_parse(parsed);
        }
    }

    /// Starts the parse a [`Task::Reparse`] asks for, off the loop, marking
    /// its buffer busy until it comes back.
    #[cfg(feature = "full")]
    async fn start_parse(
        &mut self,
        task: Task,
        done: &mpsc::UnboundedSender<Parsed>,
        parsing: &mut HashMap<maxgus_text::BufferId, Option<Task>>,
    ) {
        let Task::Reparse {
            buffer,
            language,
            text,
            revision,
            range,
        } = task
        else {
            return;
        };
        let Some(syntax) = self.syntax_for(buffer, &language).await else {
            return;
        };
        parsing.insert(buffer, None);
        let done = done.clone();
        tokio::task::spawn_blocking(move || {
            // A panic is caught so that it still comes back: a buffer whose
            // parse never answered would stay busy, and never be coloured
            // again.
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse(buffer, revision, syntax, text, range)
            }))
            .unwrap_or(Parsed {
                buffer,
                revision,
                syntax: None,
                highlights: None,
            });
            let _ = done.send(parsed);
        });
    }

    /// The parser a buffer's next parse uses: the one it has, or a new one
    /// for its language. `None` for a language with no grammar.
    #[cfg(feature = "full")]
    async fn syntax_for(
        &mut self,
        buffer: maxgus_text::BufferId,
        language: &str,
    ) -> Option<BufferSyntax> {
        // A buffer whose language changed — after `write-file`, say — starts
        // over with the right grammar.
        if self
            .highlighters
            .get(&buffer)
            .is_some_and(|s| s.language != language)
        {
            self.highlighters.remove(&buffer);
        }
        // Taken out of the map rather than borrowed, because the parse
        // leaves this thread and needs to own it.
        if let Some(syntax) = self.highlighters.remove(&buffer) {
            return Some(syntax);
        }
        // Compiled in, or loaded from where the configuration said. A
        // language with neither is not an error; it simply goes
        // unhighlighted, as it did before there was a grammar for anything.
        let Some(grammar) = self.grammar_for(language).await else {
            self.announce_missing(language).await;
            return None;
        };
        let highlighter = match Highlighter::with_grammar(language, grammar) {
            Ok(highlighter) => highlighter,
            Err(error) => {
                // Remembered so `describe-grammars` can say why the file is
                // plain, and so it is not tried again on every pause in
                // typing.
                self.grammars.remember_failure(language, error.to_string());
                return None;
            }
        };
        let left_out = highlighter.left_out();
        if let Some(first) = left_out.first() {
            self.grammars.note(
                language,
                format!(
                    "{} of its query's patterns name nodes the grammar does not have \
                     and were left out, the first being `{}`",
                    left_out.len(),
                    first.lines().next().unwrap_or_default()
                ),
            );
        }
        Some(BufferSyntax {
            language: language.to_string(),
            highlighter,
            text: String::new(),
        })
    }

    /// Keeps the parser a parse gave back, and sends on what it found.
    #[cfg(feature = "full")]
    fn finish_parse(&mut self, parsed: Parsed) {
        // A parse that panicked must not take the buffer's grammar with it;
        // the next edit starts a fresh highlighter instead.
        let Some(syntax) = parsed.syntax else {
            return;
        };
        self.highlighters.insert(parsed.buffer, syntax);
        if let Some((range, highlights)) = parsed.highlights {
            self.send(TaskResult::Reparsed {
                buffer: parsed.buffer,
                revision: parsed.revision,
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

        // The controlling half's orders, which the reading half also uses to
        // say the program has gone.
        let (commands, orders) = std::sync::mpsc::channel();

        // The reading half. A pty read blocks until the program writes, which
        // may be never, so it gets a thread rather than a slice of the runtime.
        let results = self.reporter.results.clone();
        let ended = commands.clone();
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
            // The end of the output is the program ending. The controlling
            // half was left waiting for orders that would never come, so the
            // shell was never waited for — a zombie per `exit` — and the tab
            // never heard that it had gone.
            let _ = ended.send(PtyCommand::Ended);
        });

        // The controlling half, which owns everything that can block.
        let results = self.reporter.results.clone();
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
                    PtyCommand::Ended => break,
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
    async fn start_server(&mut self, language: &str, file: Option<PathBuf>) {
        let Some(spec) = self.spec_for(language).cloned() else {
            // Nothing configured. Quiet on purpose: a server is started
            // whenever a file is opened, so complaining here would put a
            // message on the screen for every buffer in a language nobody
            // has configured one for. A request that needed a server says
            // so instead — see `lsp_request`.
            return;
        };
        // Looked for from the file, not from wherever the editor was started:
        // started from an application menu that is the home directory, and a
        // language server told to index a home directory is still at it an
        // hour later.
        let from = file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        let root = self.server_root(&spec, &from).await;
        if let Some(client) = self.servers.get(language).cloned() {
            // Running already. A file from another project is another folder
            // of the workspace, where the server can take one.
            let known = self.server_roots.entry(language.to_string()).or_default();
            if !known.iter().any(|folder| from.starts_with(folder))
                && client.can_add_workspace_folders().await
                && client.add_workspace_folder(&root).is_ok()
            {
                known.push(root);
            }
            return;
        }

        match Client::spawn(&spec.command, &spec.args, &root).await {
            Ok((client, events)) => {
                if let Err(error) = client.initialize(&root).await {
                    self.fail("language server", error);
                    return;
                }
                self.servers
                    .insert(language.to_string(), Arc::clone(&client));
                self.server_roots
                    .insert(language.to_string(), vec![root.clone()]);
                // Diagnostics and messages arrive on their own schedule.
                tokio::spawn(forward_events(
                    events,
                    self.reporter.results.clone(),
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
        self.server_roots.remove(language);
        let _ = client.shutdown().await;
        self.send(TaskResult::LanguageServerStopped {
            language: language.to_string(),
        });
    }

    #[cfg(feature = "full")]
    /// Where a server for a file under `from` should be rooted: the nearest
    /// directory above it holding one of the configured markers, in the order
    /// they were configured.
    ///
    /// The home directory never counts — a dotfiles repository there would
    /// make every file under it one project. With no marker, the project the
    /// editor was started in when the file is inside it, and otherwise the
    /// file's own directory.
    async fn server_root(&self, spec: &LspSpec, from: &Path) -> PathBuf {
        let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        for marker in &spec.root_markers {
            if let Some(found) = find_upwards(from, marker).await
                && (home.as_deref() != Some(found.as_path()) || from == found)
            {
                return found;
            }
        }
        match from.starts_with(&self.root) {
            true => self.root.clone(),
            false => from.to_path_buf(),
        }
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
            let why = match self.spec_for(&language).is_some() {
                true => format!("the server for `{language}` is not running yet"),
                false => format!("none is configured for `{language}`"),
            };
            match announced {
                true => self.fail(&format!("language server: {}", query.description()), why),
                false => self.send(TaskResult::LspNoAnswer {
                    uri,
                    query,
                    message: why,
                }),
            }
            return;
        };
        let results = self.reporter.results.clone();
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
                // Asked out loud, so the failure is said out loud.
                Err(error) if announced => TaskResult::Failed {
                    context: format!("language server: {}", query.description()),
                    message: error.to_string(),
                },
                // Asked while the cursor rested: a server that says "content
                // modified" to a hover mid-edit is not news, and an error in
                // the echo area on every pause in typing was.
                Err(error) => TaskResult::LspNoAnswer {
                    uri,
                    query,
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

    /// Writes edited result lines back to their files.
    ///
    /// Every file is read and checked before any is written, and each is
    /// written the way a save writes, so a disk that fills halfway through
    /// leaves the file as it was rather than half of it.
    #[cfg(feature = "full")]
    async fn apply_grep(
        &self,
        replacements: Vec<maxgus_grep::Replacement>,
        unsaved: Vec<maxgus_grep::Replacement>,
    ) {
        let asked = replacements.clone();
        let prepared =
            tokio::task::spawn_blocking(move || maxgus_grep::prepare(&replacements)).await;
        let mut written = Vec::new();
        let failure = match prepared {
            Ok(Ok(files)) => {
                let mut failure = None;
                for file in files {
                    if let Err(error) = write_safely(&file.path, file.contents.as_bytes()).await {
                        failure = Some(format!("{}: {error}", file.path.display()));
                        break;
                    }
                    let disk_time = tokio::fs::metadata(&file.path)
                        .await
                        .ok()
                        .and_then(|m| m.modified().ok());
                    let lines = asked
                        .iter()
                        .filter(|line| line.path == file.path)
                        .cloned()
                        .collect();
                    written.push(maxgus_core::grep::WrittenFile {
                        path: file.path,
                        lines,
                        disk_time,
                    });
                }
                failure
            }
            Ok(Err(error)) => Some(error.to_string()),
            Err(error) => Some(error.to_string()),
        };
        self.send(TaskResult::GrepApplied {
            written,
            failure,
            unsaved,
        });
    }

    // ---- shell ---------------------------------------------------------
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

/// Decodes a picture for the buffer that will stand in for it. The pixels
/// kept are no more than a window could want: a photograph straight off a
/// camera is cut down to `LONGEST` on its longer side, which is more than
/// any window is tall and a fraction of the memory.
#[cfg(feature = "full")]
fn decode_picture(bytes: &[u8], path: &std::path::Path) -> Option<maxgus_core::picture::Picture> {
    const LONGEST: u32 = 2048;
    let format = image::guess_format(bytes)
        .ok()
        .or_else(|| image::ImageFormat::from_path(path).ok())?;
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 {
        return None;
    }
    let rgba = match width.max(height) > LONGEST {
        true => decoded
            .resize(LONGEST, LONGEST, image::imageops::FilterType::Triangle)
            .to_rgba8(),
        false => decoded.to_rgba8(),
    };
    let (kept_width, kept_height) = rgba.dimensions();
    Some(maxgus_core::picture::Picture {
        width,
        height,
        format: match format {
            image::ImageFormat::Jpeg => "JPEG".to_string(),
            image::ImageFormat::WebP => "WebP".to_string(),
            other => other.extensions_str()[0].to_ascii_uppercase(),
        },
        bytes: bytes.len() as u64,
        pixels: maxgus_core::picture::Pixels {
            width: kept_width,
            height: kept_height,
            rgba: std::sync::Arc::from(rgba.into_raw()),
        },
    })
}

#[cfg(all(test, feature = "full"))]
mod picture_tests {
    use super::decode_picture;
    use std::path::Path;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn a_png_is_decoded_to_its_pixels_and_described() {
        let bytes = png(16, 8);
        let picture = decode_picture(&bytes, Path::new("a.png")).unwrap();
        assert_eq!((picture.width, picture.height), (16, 8));
        assert_eq!(picture.format, "PNG");
        assert_eq!(picture.bytes, bytes.len() as u64);
        assert_eq!(picture.pixels.rgba.len(), 16 * 8 * 4);
        assert_eq!(&picture.pixels.rgba[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn a_huge_picture_is_kept_smaller_but_described_at_its_full_size() {
        let bytes = png(4096, 1024);
        let picture = decode_picture(&bytes, Path::new("wide.png")).unwrap();
        assert_eq!((picture.width, picture.height), (4096, 1024));
        assert_eq!((picture.pixels.width, picture.pixels.height), (2048, 512));
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_not_one() {
        assert!(decode_picture(b"fn main() {}", Path::new("a.png")).is_none());
    }
}

/// The top of the repository `directory` is in, when it is in one.
///
/// The home directory never counts: plenty of people keep their dotfiles in
/// a repository there, and every directory under it would otherwise be one
/// and the same project.
async fn project_directory(directory: &Path) -> Option<PathBuf> {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let mut at = Some(directory);
    while let Some(candidate) = at {
        if home.as_deref() == Some(candidate) {
            return None;
        }
        for marker in [".git", ".hg", ".jj"] {
            if tokio::fs::try_exists(candidate.join(marker))
                .await
                .unwrap_or(false)
            {
                return Some(candidate.to_path_buf());
            }
        }
        at = candidate.parent();
    }
    None
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
        // Installing a grammar is a clone and a C compile: two subprocesses
        // and the files they leave behind. Blocking is what it is, and it
        // too is reached only through `spawn_blocking` — held to by the
        // test below.
        "maxgus-syntax/src/install.rs",
        // The window's remembered size: one small file read before the
        // window exists and written after it has gone. Neither end has a
        // runtime to block.
        "maxgus-gui/src/geometry.rs",
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

    /// The install exception above is only safe while it holds.
    #[cfg(feature = "full")]
    #[test]
    fn a_grammar_is_only_ever_fetched_or_built_off_a_blocking_thread() {
        let source = include_str!("tasks.rs");
        let ships = source
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"))
            .collect::<Vec<_>>();
        let calls: Vec<(usize, &str)> = ships
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.contains("install::install") || line.contains("install::catalog")
            })
            .map(|(n, line)| (n + 1, *line))
            .collect();
        assert!(
            !calls.is_empty(),
            "nothing installs a grammar any more; this test has nothing to hold"
        );
        for (n, line) in calls {
            assert!(
                line.contains("spawn_blocking"),
                "line {n}: `{}` runs git or a compiler on the runtime",
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
                // `tokio::fs::File::open` is the asynchronous one.
                let code = code
                    .replace("tokio::fs::File::", "tokio::fs::")
                    .replace("tokio::fs::OpenOptions", "tokio::fs::");
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

    #[cfg(feature = "full")]
    /// A repository with one committed file, changed since.
    async fn changed_repository(tag: &str) -> Option<(Fixture, PathBuf)> {
        let f = Fixture::new(tag).await;
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(f.path())
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .is_ok_and(|out| out.status.success())
        };
        if !git(&["init", "-q"]) {
            return None;
        }
        let file = f.path().join("tracked.txt");
        tokio::fs::write(&file, "one\n").await.unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-qm", "first"]);
        tokio::fs::write(&file, "one\ntwo\n").await.unwrap();
        Some((f, file))
    }

    #[cfg(feature = "full")]
    /// Runs one git action to the end and says whether it reported a failure.
    async fn run_git(root: &Path, action: GitAction) -> Vec<TaskResult> {
        let (mut e, mut rx) = executor(root);
        e.handle(Task::Git {
            root: root.to_path_buf(),
            action,
        })
        .await;
        let mut results = Vec::new();
        while let Ok(result) = rx.try_recv() {
            results.push(result);
        }
        results
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn staging_unstaging_discarding_and_deleting_really_happen() {
        let Some((f, file)) = changed_repository("gitpaths").await else {
            return;
        };
        let status = || async {
            git_output(f.path(), &["status", "--porcelain", "--", "tracked.txt"]).await
        };

        let results = run_git(f.path(), GitAction::Stage(vec![file.clone()])).await;
        assert!(
            !results.iter().any(TaskResult::is_error),
            "stage: {results:?}"
        );
        assert_eq!(status().await, "M  tracked.txt\n");

        let results = run_git(f.path(), GitAction::Unstage(vec![file.clone()])).await;
        assert!(
            !results.iter().any(TaskResult::is_error),
            "unstage: {results:?}"
        );
        assert_eq!(status().await, " M tracked.txt\n");

        let results = run_git(f.path(), GitAction::Discard(vec![file.clone()])).await;
        assert!(
            !results.iter().any(TaskResult::is_error),
            "discard: {results:?}"
        );
        assert_eq!(status().await, "");
        assert_eq!(tokio::fs::read_to_string(&file).await.unwrap(), "one\n");

        let stray = f.path().join("stray.txt");
        tokio::fs::write(&stray, "x").await.unwrap();
        let results = run_git(f.path(), GitAction::DeleteUntracked(vec![stray.clone()])).await;
        assert!(
            !results.iter().any(TaskResult::is_error),
            "clean: {results:?}"
        );
        assert!(!tokio::fs::try_exists(&stray).await.unwrap());
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn discarding_staged_work_goes_back_to_the_last_commit() {
        let Some((f, file)) = changed_repository("gitdiscardstaged").await else {
            return;
        };
        git_output(f.path(), &["add", "tracked.txt"]).await;
        let results = run_git(f.path(), GitAction::DiscardStaged(vec![file.clone()])).await;
        assert!(!results.iter().any(TaskResult::is_error), "{results:?}");
        assert_eq!(tokio::fs::read_to_string(&file).await.unwrap(), "one\n");
        assert_eq!(
            git_output(f.path(), &["status", "--porcelain", "--", "tracked.txt"]).await,
            ""
        );

        // A staged rename goes back whole: the old name returns, the new
        // one goes.
        git_output(f.path(), &["mv", "tracked.txt", "renamed.txt"]).await;
        let results = run_git(
            f.path(),
            GitAction::DiscardStaged(vec![f.path().join("renamed.txt"), file.clone()]),
        )
        .await;
        assert!(!results.iter().any(TaskResult::is_error), "{results:?}");
        assert!(tokio::fs::try_exists(&file).await.unwrap());
        assert!(
            !tokio::fs::try_exists(f.path().join("renamed.txt"))
                .await
                .unwrap()
        );
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_push_that_wants_a_password_fails_rather_than_asking() {
        let Some((f, _)) = changed_repository("gitnoprompt").await else {
            return;
        };
        // Somewhere that would ask for credentials, and never answers.
        git_output(
            f.path(),
            &["remote", "add", "origin", "https://127.0.0.1:9/nobody.git"],
        )
        .await;
        let results = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            run_git(
                f.path(),
                GitAction::Push {
                    arguments: vec!["origin".into(), "HEAD".into()],
                },
            ),
        )
        .await
        .expect("git waited for someone to type a password");
        assert!(results.iter().any(TaskResult::is_error), "got {results:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_slow_shell_command_does_not_hold_up_reading_a_file() {
        // The README's promise: nothing is waited on. A queue that ran one
        // job at a time made `M-! sleep 10` hold every save and read for ten
        // seconds.
        let f = Fixture::new("notwaiting").await;
        let (tasks, queue) = mpsc::unbounded_channel();
        let (e, mut rx) = executor(f.path());
        tokio::spawn(e.run(queue));
        tasks
            .send(Task::Shell {
                command: "sleep 5".into(),
                directory: f.path().to_path_buf(),
                insert_at: None,
            })
            .unwrap();
        tasks
            .send(Task::ReadFile {
                path: f.path().join("Cargo.toml"),
                reverting: None,
                other_window: false,
            })
            .unwrap();
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("the read waited for the shell command")
            .expect("an answer");
        assert!(
            matches!(first, TaskResult::FileRead { .. }),
            "got {first:?}"
        );
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn a_parse_holds_up_neither_other_work_nor_the_newest_parse() {
        // A parse ran inside the loop, so a slow one — a syntax error near
        // the top of a large file makes tree-sitter take seconds — held up
        // every read and server request behind it, and typing through it
        // queued one more parse a key.
        let f = Fixture::new("parsebeside").await;
        let (tasks, queue) = mpsc::unbounded_channel();
        let (e, mut rx) = executor(f.path());
        tokio::spawn(e.run(queue));
        let line = "fn function(argument: u32) -> u32 { let value = argument * 2; value + 1 }\n";
        let text = format!("x{}", line.repeat(2_000));
        for revision in 1..=3 {
            tasks
                .send(Task::Reparse {
                    buffer: BufferId(1),
                    language: "rust".into(),
                    text: text.clone(),
                    revision,
                    range: 0..4096,
                })
                .unwrap();
        }
        tasks
            .send(Task::ReadFile {
                path: f.path().join("Cargo.toml"),
                reverting: None,
                other_window: false,
            })
            .unwrap();
        let first = tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
            .await
            .expect("an answer in time")
            .expect("an answer");
        assert!(
            matches!(first, TaskResult::FileRead { .. }),
            "the read waited for the parse: got {first:?}"
        );
        let mut revisions = Vec::new();
        while revisions.len() < 2 {
            match tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv()).await {
                Ok(Some(TaskResult::Reparsed { revision, .. })) => revisions.push(revision),
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert_eq!(
            revisions,
            [1, 3],
            "the request superseded while the first ran was parsed anyway"
        );
    }

    #[tokio::test]
    async fn visiting_a_directory_lists_it_the_way_dired_does() {
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
        assert!(
            matches!(result, TaskResult::DiredListed { .. }),
            "got {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pipe_is_refused_rather_than_waited_on() {
        let f = Fixture::new("fifo").await;
        let fifo = f.path().join("pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|status| status.success()) {
            return;
        }
        let (mut e, mut rx) = executor(f.path());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_one(
                &mut e,
                &mut rx,
                Task::ReadFile {
                    path: fifo,
                    reverting: None,
                    other_window: false,
                },
            ),
        )
        .await
        .expect("reading a pipe must not wait for a writer");
        assert!(result.is_error(), "got {result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_save_keeps_the_mode_and_writes_through_a_link() {
        use std::os::unix::fs::PermissionsExt as _;
        let f = Fixture::new("safesave").await;
        let script = f.path().join("run.sh");
        tokio::fs::write(&script, "echo old\n").await.unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        let link = f.path().join("link.sh");
        tokio::fs::symlink(&script, &link).await.unwrap();

        write_safely(&link, b"echo new\n").await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&script).await.unwrap(),
            "echo new\n"
        );
        let mode = tokio::fs::metadata(&script)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "the executable bit went");
        assert!(
            tokio::fs::symlink_metadata(&link)
                .await
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link was replaced by a file"
        );
        let mut leftovers = tokio::fs::read_dir(f.path()).await.unwrap();
        while let Some(entry) = leftovers.next_entry().await.unwrap() {
            assert!(
                !entry.file_name().to_string_lossy().contains("maxgus-save"),
                "a temporary file was left behind"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_without_its_write_bit_is_not_writable_even_to_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join("maxgus-maywrite");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("locked.txt");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        let metadata = std::fs::metadata(&file).unwrap();
        if rustix::process::geteuid().is_root() {
            assert!(may_write(&metadata));
        } else {
            assert!(!may_write(&metadata));
        }
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(may_write(&std::fs::metadata(&file).unwrap()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn copying_or_moving_onto_something_that_exists_is_refused() {
        let f = Fixture::new("noclobber").await;
        let from = f.path().join("from.txt");
        let to = f.path().join("to.txt");
        tokio::fs::write(&from, "new").await.unwrap();
        tokio::fs::write(&to, "precious").await.unwrap();

        assert!(copy_all(std::slice::from_ref(&from), &to).await.is_err());
        assert!(rename_all(std::slice::from_ref(&from), &to).await.is_err());
        assert_eq!(tokio::fs::read_to_string(&to).await.unwrap(), "precious");
        assert!(tokio::fs::try_exists(&from).await.unwrap());

        // Into itself is refused too, before a byte is copied.
        let dir = f.path().join("src");
        assert!(
            copy_all(std::slice::from_ref(&dir), &dir.join("inner"))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn copying_a_directory_copies_its_links_as_links() {
        let f = Fixture::new("copylinks").await;
        let dir = f.path().join("tree");
        tokio::fs::create_dir_all(dir.join("a")).await.unwrap();
        tokio::fs::symlink("..", dir.join("a/up")).await.unwrap();
        tokio::fs::write(dir.join("a/file"), "x").await.unwrap();
        let copy = f.path().join("copy");
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            copy_all(std::slice::from_ref(&dir), &copy),
        )
        .await
        .expect("a link loop must not make the copy endless")
        .unwrap();
        assert!(
            tokio::fs::symlink_metadata(copy.join("a/up"))
                .await
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            tokio::fs::read_to_string(copy.join("a/file"))
                .await
                .unwrap(),
            "x"
        );
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
            // Nested names are fine; leaving the directory is not.
            name: "../escape".into(),
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

    /// The whole of installing a grammar, against the real wiki and the
    /// real compiler: fetch the list, clone a repository, build it, install
    /// it, load it, and colour a buffer with it.
    ///
    /// Ignored by default because it needs the network, `git` and a C
    /// compiler, and takes seconds rather than milliseconds. `cargo test --
    /// --ignored` runs it. Nothing else covers the seam between the pieces,
    /// each of which is unit-tested on its own.
    #[cfg(feature = "full")]
    #[tokio::test]
    #[ignore = "clones a repository and runs a C compiler"]
    async fn a_grammar_is_fetched_built_installed_and_then_colours_a_buffer() {
        let f = Fixture::new("grammarinstall").await;
        let home = f.path().join("state/grammars");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut e = Executor::with_grammars(
            f.path().to_path_buf(),
            TreeConfig {
                git_status: false,
                ..Default::default()
            },
            Vec::new(),
            Default::default(),
            Some(home.clone()),
            tx,
        );

        // KDL: not compiled in, and this editor's own configuration language.
        let result = run_one(
            &mut e,
            &mut rx,
            Task::GrammarCatalog {
                refresh: true,
                language: Some("kdl".into()),
            },
        )
        .await;
        let TaskResult::GrammarCatalog { parsers, error, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(error, None, "the wiki should be readable");
        // Asked about a language, so the answer is that language's parsers
        // rather than all five hundred, best first.
        let kdl = parsers.first().cloned().expect("the wiki lists kdl");
        assert_eq!(kdl.name, "kdl", "narrowed to the language asked about");

        let result = run_one(
            &mut e,
            &mut rx,
            Task::InstallGrammar {
                language: "kdl".into(),
                url: kdl.url.clone(),
            },
        )
        .await;
        let TaskResult::GrammarInstalled {
            summary,
            log,
            failed,
            ..
        } = result
        else {
            panic!("{result:?}")
        };
        assert!(!failed, "{summary}\n{log}");
        assert!(
            home.join(&maxgus_syntax::dynamic::library_names("kdl")[0])
                .is_file(),
            "the library is where the loader looks"
        );
        assert!(
            home.join("kdl/highlights.scm").is_file(),
            "and so is the query"
        );

        // The point of all of it: a buffer in that language is coloured.
        let result = run_one(
            &mut e,
            &mut rx,
            Task::Reparse {
                buffer: BufferId(1),
                language: "kdl".into(),
                text: "node \"argument\" key=1\n".into(),
                revision: 1,
                range: 0..usize::MAX,
            },
        )
        .await;
        let TaskResult::Reparsed { highlights, .. } = result else {
            panic!("{result:?}")
        };
        assert!(!highlights.is_empty(), "the grammar just built is in use");
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
        // Answered quietly — nothing for the echo area — so whatever asked
        // can stop saying it is waiting.
        let result = rx.try_recv().expect("an answer, however quiet");
        assert!(
            matches!(result, TaskResult::LspNoAnswer { .. }),
            "got {result:?}"
        );
        assert_eq!(result.message(), None, "and nothing to say about it");
        assert!(!result.is_error());
    }

    #[cfg(feature = "full")]
    #[tokio::test]
    async fn starting_a_server_with_no_configuration_does_nothing() {
        let f = Fixture::new("nospec").await;
        let (mut e, mut rx) = executor(f.path());
        e.handle(Task::StartLanguageServer {
            language: "rust".into(),
            file: None,
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
            file: None,
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
        // `with_paths` puts the `--` in. Written here as well, it went to git
        // twice, and git took the second as a file called `--`: staging a
        // file failed with "pathspec '--' did not match any files".
        GitAction::Stage(paths) => (with_paths(&["add"], paths), "Stage".into(), None),
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
        GitAction::Discard(paths) => (with_paths(&["checkout"], paths), "Discard".into(), None),
        GitAction::DiscardStaged(paths) => (
            with_paths(
                &["restore", "--source=HEAD", "--staged", "--worktree"],
                paths,
            ),
            "Discard".into(),
            None,
        ),
        GitAction::DeleteUntracked(paths) => {
            (with_paths(&["clean", "-f"], paths), "Delete".into(), None)
        }
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
/// Keeps git from asking anything at the terminal.
///
/// A push to a remote that wants a password, or an ssh key with a passphrase,
/// had git open the terminal the editor is drawn on and wait there for an
/// answer typed into the middle of the screen — while every other job queued
/// behind it waited too. Asked this way, it fails and says why instead.
fn never_ask_at_the_terminal(process: &mut tokio::process::Command) {
    process
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_SSH_COMMAND", ssh_without_prompts())
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS");
}

#[cfg(feature = "full")]
/// ssh as the user has it set up, told never to stop and ask.
fn ssh_without_prompts() -> String {
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .or_else(|_| std::env::var("GIT_SSH"))
        .unwrap_or_else(|_| "ssh".to_string());
    format!("{ssh} -o BatchMode=yes")
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
    never_ask_at_the_terminal(&mut process);
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
        // Not following a link: deleting a link to a directory deletes the
        // link, never what is in the directory it names.
        let metadata = tokio::fs::symlink_metadata(path).await?;
        match metadata.is_dir() {
            true => tokio::fs::remove_dir_all(path).await?,
            false => tokio::fs::remove_file(path).await?,
        }
    }
    Ok(())
}

/// Where each of `from` ends up when put at `to`: inside it when it is a
/// directory or there is more than one thing, which `cp` and `mv` require
/// for the same reason; otherwise at `to` itself.
async fn destinations(from: &[PathBuf], to: &Path) -> Vec<(PathBuf, PathBuf)> {
    let into_directory = from.len() > 1 || tokio::fs::metadata(to).await.is_ok_and(|m| m.is_dir());
    from.iter()
        .map(|path| {
            let destination = match into_directory {
                true => to.join(path.file_name().unwrap_or_default()),
                false => to.to_path_buf(),
            };
            (path.clone(), destination)
        })
        .collect()
}

/// Refuses, before anything is touched, a copy or a move that would destroy
/// something or never finish.
///
/// Both went ahead: `cp` onto an existing file replaced it, `mv` onto one
/// replaced it, and a directory copied into itself grew until the disk was
/// full. Dired in Emacs asks before overwriting; this names what is in the
/// way and does nothing, which is the editor's rule for destructive work.
async fn check_destinations(pairs: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
    for (from, to) in pairs {
        if tokio::fs::symlink_metadata(to).await.is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists; nothing was done", to.display()),
            ));
        }
        if to.starts_with(from) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} cannot go inside itself", from.display()),
            ));
        }
    }
    Ok(())
}

async fn copy_all(from: &[PathBuf], to: &Path) -> std::io::Result<()> {
    let pairs = destinations(from, to).await;
    check_destinations(&pairs).await?;
    for (path, destination) in &pairs {
        copy_one(path, destination).await?;
    }
    Ok(())
}

/// Copies one file, link or directory to a place where nothing is.
///
/// A link is copied as a link. Followed, a link to a directory above it —
/// `a/up -> ..` — made the copy walk into itself for ever.
async fn copy_one(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        let metadata = tokio::fs::symlink_metadata(&source).await?;
        if metadata.file_type().is_symlink() {
            let target = tokio::fs::read_link(&source).await?;
            copy_link(&target, &destination).await?;
        } else if metadata.is_dir() {
            tokio::fs::create_dir_all(&destination).await?;
            let mut reader = tokio::fs::read_dir(&source).await?;
            while let Some(entry) = reader.next_entry().await? {
                pending.push((entry.path(), destination.join(entry.file_name())));
            }
        } else {
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&source, &destination).await?;
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn copy_link(target: &Path, at: &Path) -> std::io::Result<()> {
    tokio::fs::symlink(target, at).await
}

#[cfg(not(unix))]
async fn copy_link(target: &Path, at: &Path) -> std::io::Result<()> {
    // Where a link cannot simply be made, what it names is copied instead.
    tokio::fs::copy(target, at).await.map(|_| ())
}

/// Moves each of `from` to `to`, and says where each went.
async fn rename_all(from: &[PathBuf], to: &Path) -> std::io::Result<Vec<(PathBuf, PathBuf)>> {
    let pairs = destinations(from, to).await;
    check_destinations(&pairs).await?;
    let mut moved = Vec::new();
    for (path, destination) in pairs {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        match tokio::fs::rename(&path, &destination).await {
            Ok(()) => {}
            // Onto another disk a rename is not possible, and `mv` copies
            // and deletes instead — which is what this does, rather than
            // saying "Invalid cross-device link".
            Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
                copy_one(&path, &destination).await?;
                delete_all(std::slice::from_ref(&path)).await?;
            }
            Err(error) => return Err(error),
        }
        moved.push((path, destination));
    }
    Ok(moved)
}

/// Whether this process may write a file with this metadata.
///
/// What `access(W_OK)` answers, worked out from who owns the file and its
/// mode. The mode alone — what this asked before — said a root-owned file
/// with `rw-r--r--` was writable, so it opened for editing and every save
/// failed with "Permission denied".
#[cfg(unix)]
fn may_write(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let euid = rustix::process::geteuid();
    if euid.is_root() {
        return true;
    }
    let mode = metadata.mode();
    if metadata.uid() == euid.as_raw() {
        return mode & 0o200 != 0;
    }
    let gid = metadata.gid();
    let in_group = rustix::process::getegid().as_raw() == gid
        || rustix::process::getgroups()
            .unwrap_or_default()
            .iter()
            .any(|group| group.as_raw() == gid);
    match in_group {
        true => mode & 0o020 != 0,
        false => mode & 0o002 != 0,
    }
}

#[cfg(not(unix))]
fn may_write(metadata: &std::fs::Metadata) -> bool {
    !metadata.permissions().readonly()
}

/// Writes `contents` to `path` so that a failure part-way leaves the file as
/// it was.
///
/// The new contents go into a file beside the old one, which is renamed over
/// it once they are all on the disk — the one step a filesystem takes all at
/// once. Truncating the file and writing into it, which is what this did,
/// left half a file behind when the disk filled up or the machine stopped
/// mid-save.
///
/// The rename is not used where it would change something about the file
/// besides its contents: one with other hard links would be split from them,
/// and one owned by somebody else, or by another group, would change hands.
/// Those are written in place as before, and so is a file in a directory
/// that cannot be written to.
async fn write_safely(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    // Through a link to what it names, so the link stays a link.
    let target = tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf());
    let existing = tokio::fs::metadata(&target).await.ok();
    #[cfg(unix)]
    if existing.as_ref().is_some_and(|m| {
        use std::os::unix::fs::MetadataExt as _;
        m.nlink() > 1
    }) {
        return tokio::fs::write(&target, contents).await;
    }
    let (Some(directory), Some(name)) = (target.parent(), target.file_name()) else {
        return tokio::fs::write(&target, contents).await;
    };
    let temporary = directory.join(format!(
        ".{}.maxgus-save-{}~",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = match tokio::fs::File::create(&temporary).await {
        Ok(file) => file,
        // A directory that cannot be written to may still hold a file that
        // can be.
        Err(_) => return tokio::fs::write(&target, contents).await,
    };
    #[cfg(unix)]
    if let (Some(existing), Ok(fresh)) = (&existing, file.metadata().await) {
        use std::os::unix::fs::MetadataExt as _;
        if (existing.uid(), existing.gid()) != (fresh.uid(), fresh.gid()) {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return tokio::fs::write(&target, contents).await;
        }
    }
    let written = async {
        file.write_all(contents).await?;
        file.sync_all().await?;
        drop(file);
        if let Some(existing) = &existing {
            tokio::fs::set_permissions(&temporary, existing.permissions()).await?;
        }
        tokio::fs::rename(&temporary, &target).await
    }
    .await;
    if written.is_err() {
        // The old file is untouched; only the half-written new one goes.
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    written
}

/// Parses `text` with a buffer's parser and highlights `range` of it.
#[cfg(feature = "full")]
fn parse(
    buffer: maxgus_text::BufferId,
    revision: u64,
    mut syntax: BufferSyntax,
    text: String,
    range: std::ops::Range<usize>,
) -> Parsed {
    // Telling the parser which region changed is what lets it keep the rest
    // of the tree.
    if syntax.highlighter.has_tree()
        && let Some(edit) = maxgus_syntax::InputEdit::between(&syntax.text, &text)
    {
        syntax.highlighter.edit(edit, &syntax.text, &text);
    }
    if syntax.highlighter.parse(&text).is_err() {
        return Parsed {
            buffer,
            revision,
            syntax: Some(syntax),
            highlights: None,
        };
    }
    // Only the requested region is queried: running the highlight query over
    // a whole large file costs far more than parsing it, and the answer
    // beyond the window would never be drawn.
    let range = range.start..range.end.min(text.len());
    let highlights = syntax.highlighter.highlights_in(&text, range.clone());
    syntax.text = text;
    Parsed {
        buffer,
        revision,
        syntax: Some(syntax),
        highlights: Some((range, highlights)),
    }
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
        executor.reporter.find_directories(root.to_path_buf()).await;
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
