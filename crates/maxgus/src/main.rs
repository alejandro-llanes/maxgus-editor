//! `maxgus` — a lightning-fast Emacs. In a window, or in the terminal.

mod app;
#[cfg(all(windows, feature = "gui"))]
mod console;
mod tasks;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use maxgus_config::Config;
use maxgus_core::{Dispatcher, Editor};
use maxgus_tui::{Rect, Terminal};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Which of the three builds this is, for `--version`.
///
/// They look identical and are not, and which one is running is the first
/// thing to know when one of them does not do what was expected.
static VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let build = if cfg!(feature = "gui") {
        "gui"
    } else if cfg!(feature = "full") {
        "full"
    } else {
        "minimal"
    };
    format!("{} ({build})", env!("CARGO_PKG_VERSION"))
});

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(
    name = "maxgus",
    version = VERSION.as_str(),
    about = "A lightning-fast Emacs, in a window or a terminal"
)]
struct Arguments {
    /// Files to visit on startup.
    files: Vec<PathBuf>,

    /// Read configuration from this file instead of the usual place.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Start with no configuration at all.
    #[arg(long, short = 'Q')]
    no_config: bool,

    /// Open the file tree rooted here, and take it as the project root.
    #[arg(long, value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Open a window, even where one would not be opened by default.
    #[cfg(feature = "gui")]
    #[arg(long)]
    gui: bool,

    /// Open the window filling the screen, this time. Also `-fs`.
    #[cfg(feature = "gui")]
    #[arg(long)]
    fullscreen: bool,

    /// Take over the terminal rather than opening a window. Also `-nw`.
    ///
    /// Every build accepts it, so that the habit works everywhere; only a
    /// `gui` build has a window for it to turn off.
    #[arg(long = "no-window-system", visible_alias = "tty")]
    no_window_system: bool,
}

/// Whether a window can be opened here at all.
///
/// A `gui` build run over ssh, or on a machine with no session, has nothing
/// to draw into. Emacs starts in the terminal in that case rather than
/// failing, and so does this — `--gui` overrides it and lets the failure
/// happen, which is what someone who passed it wants to see.
#[cfg(feature = "gui")]
fn display_available() -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

/// `-nw`, spelled the way Emacs spells it.
///
/// A single dash in front of two letters is not a form any argument parser
/// offers, and it is the form thirty years of muscle memory types, so it is
/// translated before the parser ever sees it.
fn argv_with_emacs_spellings() -> Vec<std::ffi::OsString> {
    std::env::args_os()
        .map(|argument| match argument.to_str() {
            Some("-nw") => std::ffi::OsString::from("--no-window-system"),
            #[cfg(feature = "gui")]
            Some("-fs") => std::ffi::OsString::from("--fullscreen"),
            _ => argument,
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    // Before anything else this process does, so what is reported is the time
    // the editor took rather than the time it took to measure itself. The
    // dynamic loader's work is already done by now and cannot be counted.
    let started = std::time::Instant::now();
    let arguments = Arguments::parse_from(argv_with_emacs_spellings());

    // Traces go to a file: stderr belongs to the editor's own display.
    if let Ok(path) = std::env::var("MAXGUS_LOG")
        && let Ok(file) = std::fs::File::create(&path)
    {
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    }

    let (config, mut problems) = load_config(&arguments);
    // The parser cannot do this itself: which faces exist is knowledge of the
    // faces crate, which is built on top of the config crate rather than under
    // it. A misspelled face would otherwise be accepted in silence and simply
    // never paint.
    problems.extend(
        unknown_face_warnings(&config)
            .iter()
            .map(ToString::to_string),
    );
    problems.extend(unknown_theme_problems(&config));
    let root = project_root(&arguments);

    // A build with a window in it is a desktop program: it opens one unless
    // told not to, or unless there is nothing to open it into. A build
    // without a window in it never has the choice.
    //
    // In a window the frame is decided by the font and the window's size,
    // which are not known until it opens; a nominal one gets the editor built.
    #[cfg(feature = "gui")]
    let windowed = !arguments.no_window_system && (arguments.gui || display_available());
    #[cfg(not(feature = "gui"))]
    let windowed = {
        // Read so that it is not a lie: this build has no window, so the
        // flag asking for no window is already satisfied.
        let _ = arguments.no_window_system;
        false
    };

    // The terminal is claimed before anything can panic inside it: the panic
    // hook is what puts the user's shell back if something goes wrong.
    let terminal = if windowed {
        None
    } else {
        maxgus_tui::terminal::install_panic_hook();
        Some(Terminal::new().context("this program needs a terminal")?)
    };
    let frame = match &terminal {
        Some(terminal) => Rect::from_size(terminal.size()),
        None => Rect::from_size(maxgus_tui::Size::new(100, 30)),
    };

    let theme = maxgus_core::build_theme(&config.themes, &config.settings.theme);
    let mut editor = Editor::new(config.settings.clone(), theme, frame);
    // Kept so `load-theme` can rebuild any theme with the user's own faces
    // still laid over it, exactly as the line above did.
    editor.theme_specs = config.themes.clone();
    // Where a chosen theme would be written, and what is written there now.
    editor.config_path = arguments.config.clone().or_else(default_config_path);
    editor.state_dir = default_state_dir();
    // Snippets live beside the configuration, one directory per mode, the way
    // yasnippet arranges them.
    editor.snippets = load_snippets(editor.config_path.as_deref());
    // The script beside the configuration, read like any other file.
    #[cfg(feature = "full")]
    if let Some(path) = editor
        .config_path
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(|dir| dir.join("init.rhai"))
    {
        editor.spawn(maxgus_core::Task::ReadScript { path });
    }
    editor.config_says_theme = Some(config.settings.theme.clone());

    let registry = maxgus_core::standard_registry();
    editor.command_names = registry.interactive_names();
    editor.command_docs = registry
        .iter()
        .map(|c| (c.name.to_string(), c.doc.to_string()))
        .collect();
    editor.project_dir = Some(root.clone());
    editor.tree_width = config.tree.width as u16;
    editor.tree_follow = config.tree.follow;

    // The keymaps once the commands are known, so a binding can be checked
    // against them. Any problem goes in with the file's own.
    problems.extend(maxgus_core::keymap::apply_configured_keymaps(
        &mut editor,
        &config.keymaps,
    ));
    let dead =
        maxgus_core::keymap::bindings_to_nothing(&config.keymaps, |name| registry.contains(name));
    // A command the script defines is not known until the script has been
    // read, which is when a full build asks after these.
    #[cfg(feature = "full")]
    {
        editor.unchecked_bindings = dead;
    }
    #[cfg(not(feature = "full"))]
    problems.extend(maxgus_core::keymap::describe_bindings_to_nothing(&dead));

    // Configuration problems are reported rather than swallowed, but they
    // never stop the editor starting.
    if !problems.is_empty() {
        editor.error(format!(
            "{}: {}",
            maxgus_core::count(problems.len(), "configuration problem"),
            problems.join("; ")
        ));
    }

    // A session, when the configuration asks for one and no files were named:
    // `maxgus file.rs` means that file, not the last twelve.
    if config.settings.session
        && arguments.files.is_empty()
        && let Some(state) = editor.state_dir.clone()
    {
        editor.spawn(maxgus_core::Task::ReadSession {
            path: maxgus_core::session::path_for(&state, &root),
        });
    }

    // The saved workspaces, always: they are a list of places to work, not
    // a record of where you left off, so they are worth having whether or
    // not a session is being restored and whether or not a file was named.
    if let Some(state) = editor.state_dir.clone() {
        editor.spawn(maxgus_core::Task::ReadWorkspaces {
            path: maxgus_core::workspace::path_for(&state),
        });
    }

    for path in &arguments.files {
        editor.spawn(maxgus_core::Task::ReadFile {
            path: absolute(path),
            reverting: None,
            other_window: false,
        });
    }
    if arguments.directory.is_some() {
        editor.spawn(maxgus_core::Task::Tree(maxgus_core::TreeAction::Refresh));
    }
    #[cfg(feature = "full")]
    editor.spawn(maxgus_core::Task::GitBranch { root: root.clone() });

    // The panel, when the configuration asks for it. Opened last so that the
    // files named on the command line are already on their way and the panel
    // lays out around the window they will land in.
    if config.settings.panel_at_startup
        && let Err(error) = maxgus_core::commands::tree::open(&mut editor, root.clone())
    {
        editor.error(error.to_string());
    }

    // Everything the editor needs to draw its first frame is in place, which
    // is what a startup time means.
    editor.set_startup_time(started.elapsed());

    let (task_tx, task_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = mpsc::unbounded_channel();
    let executor = tasks::Executor::with_grammars(
        root,
        config.tree.clone(),
        config.lsp.clone(),
        config.grammars.clone(),
        grammar_home(),
        result_tx,
    );
    tokio::spawn(executor.run(task_rx));

    #[cfg(feature = "gui")]
    if windowed {
        // Started from Explorer, a console came with the window, and goes.
        #[cfg(windows)]
        console::let_go_of_a_console_of_its_own();
        return gui::run(
            editor,
            Dispatcher::new(registry),
            &config,
            arguments.fullscreen,
            task_tx,
            result_rx,
        );
    }

    App::new(
        editor,
        Dispatcher::new(registry),
        terminal.expect("a terminal, since this is not a window"),
        task_tx,
        result_rx,
    )
    .run()
    .await
}

/// Starting the editor in a window rather than in the terminal.
#[cfg(feature = "gui")]
mod gui {
    use anyhow::Result;
    use maxgus_config::Config;
    use maxgus_core::{Dispatcher, Editor, Task, TaskResult};
    use maxgus_gui::quads::Palette;
    use tokio::sync::mpsc;

    /// Hands the editor to the window, with two threads' worth of plumbing
    /// between it and the executor: the window's loop is not async, and the
    /// executor is.
    pub fn run(
        editor: Editor,
        dispatcher: Dispatcher,
        config: &Config,
        fullscreen: bool,
        tasks: mpsc::UnboundedSender<Task>,
        mut results: mpsc::UnboundedReceiver<TaskResult>,
    ) -> Result<()> {
        // The window's loop takes tasks on a plain channel and forwards them
        // to tokio's, and takes results back the same way.
        let (task_tx, task_rx) = std::sync::mpsc::channel::<Task>();
        std::thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                if tasks.send(task).is_err() {
                    return;
                }
            }
        });
        let (result_tx, result_rx) = std::sync::mpsc::channel::<TaskResult>();
        std::thread::spawn(move || {
            while let Some(result) = results.blocking_recv() {
                if result_tx.send(result).is_err() {
                    return;
                }
            }
        });

        let settings = maxgus_gui::Settings {
            title: "maxgus".into(),
            font: config.settings.gui_font.clone(),
            font_size: config.settings.gui_font_size as f32,
            palette: Palette::of(&editor.theme),
            fullscreen,
        };
        maxgus_gui::run(editor, dispatcher, settings, task_tx, result_rx)
    }
}

/// Reads the configuration, returning it and any complaints about it.
fn load_config(arguments: &Arguments) -> (Config, Vec<String>) {
    if arguments.no_config {
        return (Config::default(), Vec::new());
    }
    let Some(path) = arguments.config.clone().or_else(default_config_path) else {
        return (Config::default(), Vec::new());
    };
    let Ok(source) = std::fs::read_to_string(&path) else {
        // No configuration file is the normal case, not a problem.
        return (Config::default(), Vec::new());
    };
    match Config::parse(&source) {
        Ok(mut config) => {
            let mut problems: Vec<String> =
                config.warnings.iter().map(ToString::to_string).collect();
            problems.extend(
                load_theme_directory(&path, &mut config)
                    .iter()
                    .map(ToString::to_string),
            );
            (config, problems)
        }
        // Not a reason to refuse to start. The editor would not open, said
        // only "Failed to parse KDL document", and left some other editor to
        // find the mistake with; now it opens on its defaults and says where
        // the mistake is, and `C-c f p` is the way to it.
        Err(error) => (
            Config::default(),
            vec![format!(
                "{} is not valid KDL, {error}; none of it is in use until that is fixed \
                 (C-c f p opens it)",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            )],
        ),
    }
}

/// A theme the configuration names that there is none of.
///
/// Asked for by `set theme` or by a theme's `base`, it was the default
/// theme that appeared, with nothing said about why.
fn unknown_theme_problems(config: &Config) -> Vec<String> {
    let known: Vec<&str> = maxgus_faces::defaults::BUILTIN_THEMES
        .iter()
        .copied()
        .chain(config.themes.iter().map(|theme| theme.name.as_str()))
        .collect();
    let hint = |name: &str, among: &[&str]| {
        maxgus_config::settings::closest_among(name, among.iter().copied())
            .map(|closest| format!(", did you mean `{closest}`?"))
            .unwrap_or_default()
    };
    let mut problems = Vec::new();
    let theme = config.settings.theme.as_str();
    if !known.contains(&theme) {
        problems.push(format!(
            "there is no theme called `{theme}`{}",
            hint(theme, &known)
        ));
    }
    for spec in &config.themes {
        if let Some(base) = spec.base.as_deref()
            && !maxgus_faces::defaults::BUILTIN_THEMES.contains(&base)
        {
            problems.push(format!(
                "theme `{}` is based on `{base}`, which is not a built-in theme{}",
                spec.name,
                hint(base, maxgus_faces::defaults::BUILTIN_THEMES)
            ));
        }
    }
    problems
}

/// Reads every `themes/*.kdl` beside the configuration file into `config`.
///
/// A theme is then just a file you drop in: `set theme="nord"` finds it, and
/// `M-x load-theme` offers it, with nothing else to write. Files are taken in
/// name order so a repeated theme resolves the same way on every start.
fn load_theme_directory(config_path: &Path, config: &mut Config) -> Vec<maxgus_config::Warning> {
    let Some(dir) = config_path.parent().map(|p| p.join("themes")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No themes directory is the normal case, not a problem.
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "kdl"))
        .collect();
    files.sort();

    let mut warnings = Vec::new();
    for file in files {
        let shown = file
            .file_name()
            .unwrap_or(file.as_os_str())
            .to_string_lossy()
            .into_owned();
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        match Config::parse(&source) {
            Ok(theme_config) => {
                // A theme file that is not about themes is worth saying so
                // about rather than reading in silence.
                if theme_config.themes.is_empty() {
                    warnings.push(maxgus_config::Warning::new(
                        1,
                        format!("`themes/{shown}` defines no theme"),
                    ));
                }
                for warning in &theme_config.warnings {
                    warnings.push(maxgus_config::Warning::new(
                        warning.line,
                        format!("themes/{shown}: {}", warning.message),
                    ));
                }
                config.merge_themes(theme_config.themes);
            }
            Err(error) => warnings.push(maxgus_config::Warning::new(
                1,
                format!("`themes/{shown}` could not be read: {error}"),
            )),
        }
    }
    warnings
}

/// Reads `snippets/<mode>/<name>` from beside the configuration file.
///
/// A directory per mode and a file per snippet, as yasnippet arranges them,
/// so a set copied from an Emacs configuration works unchanged. Files
/// directly in `snippets/` belong to every mode.
fn load_snippets(config: Option<&std::path::Path>) -> Vec<maxgus_core::snippet::Snippet> {
    let Some(root) = config
        .and_then(|path| path.parent())
        .map(|d| d.join("snippets"))
    else {
        return Vec::new();
    };
    let mut snippets = Vec::new();
    read_snippets(&root, None, &mut snippets);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return snippets;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            let mut mode = entry.file_name().to_string_lossy().to_string();
            // yasnippet names the directory `rust-mode`; a directory called
            // `rust` means the same thing, since the mode is the language.
            if !mode.ends_with("-mode") {
                mode.push_str("-mode");
            }
            read_snippets(&entry.path(), Some(mode), &mut snippets);
        }
    }
    snippets
}

fn read_snippets(
    directory: &std::path::Path,
    mode: Option<String>,
    out: &mut Vec<maxgus_core::snippet::Snippet>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(maxgus_core::snippet::Snippet::parse_file(
            &source,
            &name,
            mode.clone(),
        ));
    }
}

/// Where the editor keeps what is its own business: sessions, for now.
fn default_state_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "maxgus").map(|dirs| dirs.data_local_dir().to_path_buf())
}

/// Where a grammar the editor installs for itself goes.
///
/// `~/.local/share/maxgus/grammars` on Linux, and whatever the platform puts
/// in its place elsewhere — the same directory the editor already keeps its
/// sessions beside. It is searched without being configured, because nothing
/// gets into it except by an install the user was asked about first.
fn grammar_home() -> Option<PathBuf> {
    default_state_dir().map(|dir| dir.join("grammars"))
}

/// `~/.config/maxgus/config.kdl`, or wherever the platform puts it.
fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "maxgus")
        .map(|dirs| dirs.config_dir().join("config.kdl"))
}

/// The directory the tree is rooted at and language servers are started in.
fn project_root(arguments: &Arguments) -> PathBuf {
    if let Some(directory) = &arguments.directory {
        return absolute(directory);
    }
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // A file argument implies its project: the repository around it, else
    // the directory it was named from when it is under that, else its own.
    // `maxgus src/editor.rs` from the top of a checkout used to root the
    // tree and the project search at `src`.
    if let Some(first) = arguments.files.first()
        && let Some(parent) = absolute(first).parent()
    {
        let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        if let Some(repository) = repository_around(parent, home.as_deref()) {
            return repository;
        }
        if parent.starts_with(&here) && Some(here.as_path()) != home.as_deref() {
            return here;
        }
        return parent.to_path_buf();
    }
    here
}

/// The top of the repository `directory` is in, stopping short of the home
/// directory — a dotfiles repository there would make everything under it
/// one project.
fn repository_around(directory: &Path, home: Option<&Path>) -> Option<PathBuf> {
    directory
        .ancestors()
        .take_while(|candidate| Some(*candidate) != home)
        .find(|candidate| {
            [".git", ".hg", ".jj"]
                .iter()
                .any(|marker| candidate.join(marker).exists())
        })
        .map(Path::to_path_buf)
}

fn absolute(path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().unwrap_or_default().join(path)
}

/// Complaints about face names in the configuration that the editor does not
/// have, with a suggestion where the name is close to a real one.
fn unknown_face_warnings(config: &Config) -> Vec<maxgus_config::Warning> {
    config
        .themes
        .iter()
        .flat_map(|theme| maxgus_faces::names::unknown_in(theme))
        .map(|(line, name, closest)| {
            let hint = closest
                .map(|c| format!(", did you mean `{c}`?"))
                .unwrap_or_default();
            maxgus_config::Warning::new(line, format!("unknown face `{name}`{hint}"))
        })
        .collect()
}
