//! `maxgus` — a very small Emacs for the terminal.

mod app;
mod tasks;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use maxgus_config::Config;
use maxgus_core::{Dispatcher, Editor};
use maxgus_tui::{Rect, Terminal};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(name = "maxgus", version, about = "A very small Emacs for the terminal")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();

    // Traces go to a file: stderr belongs to the editor's own display.
    if let Ok(path) = std::env::var("MAXGUS_LOG")
        && let Ok(file) = std::fs::File::create(&path)
    {
        tracing_subscriber::fmt().with_writer(std::sync::Mutex::new(file)).with_ansi(false).init();
    }

    let (config, mut warnings) = load_config(&arguments)?;
    // The parser cannot do this itself: which faces exist is knowledge of the
    // faces crate, which is built on top of the config crate rather than under
    // it. A misspelled face would otherwise be accepted in silence and simply
    // never paint.
    warnings.extend(unknown_face_warnings(&config));
    let root = project_root(&arguments);

    // The terminal is claimed before anything can panic inside it: the panic
    // hook is what puts the user's shell back if something goes wrong.
    maxgus_tui::terminal::install_panic_hook();
    let terminal = Terminal::new().context("this program needs a terminal")?;
    let frame = Rect::from_size(terminal.size());

    let theme = maxgus_core::build_theme(&config.themes, &config.settings.theme);
    let mut editor = Editor::new(config.settings.clone(), theme, frame);
    // Kept so `load-theme` can rebuild any theme with the user's own faces
    // still laid over it, exactly as the line above did.
    editor.theme_specs = config.themes.clone();
    // Where a chosen theme would be written, and what is written there now.
    editor.config_path = arguments.config.clone().or_else(default_config_path);
    editor.config_says_theme = Some(config.settings.theme.clone());
    apply_keymaps(&mut editor, &config);

    let registry = maxgus_core::standard_registry();
    editor.command_names = registry.interactive_names();
    editor.command_docs = registry.iter().map(|c| (c.name.to_string(), c.doc.to_string())).collect();
    editor.tree_root = Some(root.clone());
    editor.tree_width = config.tree.width as u16;
    editor.tree_follow = config.tree.follow;

    // Configuration problems are reported rather than swallowed, but they
    // never stop the editor starting.
    if !warnings.is_empty() {
        editor.error(format!(
            "{} configuration problem(s): {}",
            warnings.len(),
            warnings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        ));
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
    editor.spawn(maxgus_core::Task::GitBranch { root: root.clone() });

    let (task_tx, task_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = mpsc::unbounded_channel();
    let executor = tasks::Executor::new(root, config.tree.clone(), config.lsp.clone(), result_tx);
    tokio::spawn(executor.run(task_rx));

    App::new(editor, Dispatcher::new(registry), terminal, task_tx, result_rx).run().await
}

/// Reads the configuration, returning it and any complaints about it.
fn load_config(arguments: &Arguments) -> Result<(Config, Vec<maxgus_config::Warning>)> {
    if arguments.no_config {
        return Ok((Config::default(), Vec::new()));
    }
    let Some(path) = arguments.config.clone().or_else(default_config_path) else {
        return Ok((Config::default(), Vec::new()));
    };
    let Ok(source) = std::fs::read_to_string(&path) else {
        // No configuration file is the normal case, not a problem.
        return Ok((Config::default(), Vec::new()));
    };
    match Config::parse(&source) {
        Ok(mut config) => {
            let mut warnings = config.warnings.clone();
            warnings.extend(load_theme_directory(&path, &mut config));
            Ok((config, warnings))
        }
        // A file that cannot be parsed at all is worth refusing to start over:
        // silently ignoring it would be more confusing than an error.
        Err(error) => Err(anyhow::anyhow!("{}: {error}", path.display())),
    }
}

/// Reads every `themes/*.kdl` beside the configuration file into `config`.
///
/// A theme is then just a file you drop in: `set theme="nord"` finds it, and
/// `M-x load-theme` offers it, with nothing else to write. Files are taken in
/// name order so a repeated theme resolves the same way on every start.
fn load_theme_directory(
    config_path: &Path,
    config: &mut Config,
) -> Vec<maxgus_config::Warning> {
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
        let shown = file.file_name().unwrap_or(file.as_os_str()).to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(&file) else { continue };
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
    // A file argument implies its directory; otherwise the working directory.
    if let Some(first) = arguments.files.first()
        && let Some(parent) = absolute(first).parent()
    {
        return parent.to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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
            let hint = closest.map(|c| format!(", did you mean `{c}`?")).unwrap_or_default();
            maxgus_config::Warning::new(line, format!("unknown face `{name}`{hint}"))
        })
        .collect()
}

/// Layers the configuration's keymaps over the built-in ones.
fn apply_keymaps(editor: &mut Editor, config: &Config) {
    if let Some(spec) = config.keymap("global")
        && let Err(error) = spec.apply_to(&mut editor.keymaps.global)
    {
        editor.error(format!("global keymap: {error}"));
    }
    // Any other block becomes a minor-mode map, which the editor turns on when
    // that mode is active.
    for spec in &config.keymaps {
        if spec.name == "global" {
            continue;
        }
        match spec.to_keymap() {
            Ok(map) => editor.mode_keymaps.push(map),
            Err(error) => editor.error(format!("{} keymap: {error}", spec.name)),
        }
    }
}
