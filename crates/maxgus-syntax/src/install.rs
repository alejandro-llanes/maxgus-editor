//! Fetching the parser list, and building a parser from its repository.
//!
//! Everything here runs external programs and blocks: `git` to clone, and a
//! C compiler to build. None of it may run on the runtime — the executor
//! calls it from `spawn_blocking`, the same way it opens a shared library.
//!
//! # What this trusts, and what the user is agreeing to
//!
//! [`crate::dynamic`] explains why loading a grammar is `unsafe` and why the
//! editor never goes looking for one. Installing raises the stake: the
//! editor clones a repository named by a wiki anybody can edit, compiles the
//! C in it with the user's own compiler, and then `dlopen`s the result.
//! Compiling C is running arbitrary code once removed, and the produced
//! library is loaded into the editor's own process.
//!
//! So nothing here happens without the user answering a question first. The
//! offer names the language and the repository it would clone, the whole
//! command line is recorded in [`Report::log`] so it can be read afterwards,
//! and `git` is run with `GIT_TERMINAL_PROMPT=0` so a repository that has
//! moved or gone private fails rather than sitting there waiting for a
//! password nobody can see.
//!
//! The result goes to one directory the editor owns — the one passed as
//! `into`, which is where it looks for grammars it installed itself. Nothing
//! is written anywhere else, and `sudo` is never involved: a directory under
//! the user's own home does not need it, and using it would leave
//! root-owned files in a home directory.

use crate::dynamic::library_names;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The wiki, as the git repository it is.
pub const CATALOG_REPOSITORY: &str = "https://github.com/tree-sitter/tree-sitter.wiki.git";

/// The page inside it.
pub const CATALOG_FILE: &str = "List-of-parsers.md";

/// How old a cached list may be before it is fetched again, in seconds.
/// A week: the wiki gains a few rows a month, and a fetch is a network round
/// trip in front of a menu the user is waiting on.
pub const CATALOG_MAX_AGE: u64 = 7 * 24 * 60 * 60;

/// What went wrong installing a grammar.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("`{tool}` is not installed, and building a grammar needs it")]
    ToolMissing { tool: String },
    #[error("{what} failed: {message}")]
    Ran { what: String, message: String },
    #[error("{url} has no src/parser.c in it, so there is nothing to compile")]
    NoParser { url: String },
    #[error(
        "{url} holds several grammars ({}); install one by name from M-x install-grammar",
        .found.join(", ")
    )]
    Ambiguous { url: String, found: Vec<String> },
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// What to build, and where to put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The language the editor will look this up as. The library is named
    /// for it, which is what makes it findable, and the symbol the loader
    /// asks for is derived from it — so `c-sharp` finds a grammar exporting
    /// `tree_sitter_c_sharp`, punctuation and all.
    pub language: String,
    /// The repository to clone, as the catalog gives it.
    pub url: String,
    /// The directory the editor searches for grammars it installed.
    pub into: PathBuf,
}

/// What an install did, in enough detail to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub language: String,
    pub url: String,
    pub library: PathBuf,
    /// Where the queries went, when the repository had any.
    pub queries: Option<PathBuf>,
    /// Every command run, with its output. Shown in a buffer, so that a
    /// compile that says something useful on the way past is not lost.
    pub log: String,
    /// Things that worked but are worth saying — a grammar with no
    /// `highlights.scm` being the one that matters.
    pub warnings: Vec<String>,
}

/// The parser list, from the cache when it is recent enough and from the
/// wiki otherwise.
///
/// `cache` is a file the editor owns. A fetch that fails when a cached copy
/// exists uses the cached copy: an editor with no network should still open
/// the menu it opened yesterday.
pub fn catalog(cache: &Path, refresh: bool) -> Result<String, InstallError> {
    if !refresh && let Some(cached) = fresh_cache(cache) {
        return Ok(cached);
    }
    match fetch_catalog() {
        Ok(text) => {
            if let Some(parent) = cache.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(cache, &text);
            Ok(text)
        }
        Err(error) => match std::fs::read_to_string(cache) {
            Ok(stale) => Ok(stale),
            Err(_) => Err(error),
        },
    }
}

/// The cached list whatever its age, or nothing.
///
/// What the editor consults before offering to install anything: it says
/// whether a parser for a language exists without going near the network.
/// A language nobody has written a grammar for — `txt`, `log`, `bak` — is
/// then never asked about, which is the difference between a useful offer
/// and a prompt in the way of every file that is not code.
pub fn cached_catalog(cache: &Path) -> Option<String> {
    std::fs::read_to_string(cache).ok()
}

/// The cached list, if it was written recently enough to use unasked.
fn fresh_cache(cache: &Path) -> Option<String> {
    let age = cache.metadata().ok()?.modified().ok()?.elapsed().ok()?;
    match age.as_secs() < CATALOG_MAX_AGE {
        true => std::fs::read_to_string(cache).ok(),
        false => None,
    }
}

/// Clones the wiki and reads the page out of it.
///
/// A shallow clone of four markdown files, which is a great deal less than
/// the rendered page would be, and it needs no HTTP client of our own.
fn fetch_catalog() -> Result<String, InstallError> {
    let workspace = TempDir::new("maxgus-parser-list")?;
    let wiki = workspace.path().join("wiki");
    run(
        "git",
        &[
            "clone",
            "--depth=1",
            "--quiet",
            CATALOG_REPOSITORY,
            &wiki.display().to_string(),
        ],
        None,
        "cloning the parser list",
    )?;
    let page = wiki.join(CATALOG_FILE);
    std::fs::read_to_string(&page).map_err(|source| InstallError::Io {
        path: page.display().to_string(),
        source,
    })
}

/// Clones, compiles and installs one grammar.
///
/// Blocking from beginning to end, and slow: a clone and a C compile.
pub fn install(request: &Request) -> Result<Report, InstallError> {
    let mut log = String::new();
    let workspace = TempDir::new(&format!("maxgus-grammar-{}", request.language))?;
    let repository = workspace.path().join("source");

    log.push_str(&run(
        "git",
        &[
            "clone",
            "--depth=1",
            "--quiet",
            &request.url,
            &repository.display().to_string(),
        ],
        None,
        "clone",
    )?);

    // A repository is usually one grammar at its root, but a few hold
    // several — `tree-sitter-sfapex` has `apex/`, `soql/` and `sosl/` — and
    // then the one wanted is the one named like the language.
    let source = grammar_directory(&repository, &request.language, &request.url)?;
    log.push_str(&build(&source, &request.language, &request.into)?);

    let library = request.into.join(&library_names(&request.language)[0]);
    let mut warnings = Vec::new();
    let queries = match install_queries(&repository, &source, &request.language, &request.into)? {
        Some(path) => {
            // `path` is `<into>/<language>`, which is where the query
            // that may need the directive actually is.
            if let Some(base) = note_inheritance(&repository, &source, &request.language, &path) {
                let _ = writeln!(
                    log,
                    "note: {} extends {base}, whose query is read first",
                    request.language
                );
            }
            Some(path)
        }
        None => {
            warnings.push(format!(
                "{} has no queries/highlights.scm, so {} will parse but not colour. \
                 Put a highlights.scm in {} to fix that.",
                request.url,
                request.language,
                request.into.join(&request.language).display()
            ));
            None
        }
    };
    Ok(Report {
        language: request.language.clone(),
        url: request.url.clone(),
        library,
        queries,
        log,
        warnings,
    })
}

/// The directory holding `src/parser.c`: the repository root, or the one
/// subdirectory that is a grammar, or the one named for the language.
fn grammar_directory(
    repository: &Path,
    language: &str,
    url: &str,
) -> Result<PathBuf, InstallError> {
    if repository.join("src/parser.c").is_file() {
        return Ok(repository.to_path_buf());
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(repository)
        .map_err(|source| InstallError::Io {
            path: repository.display().to_string(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("src/parser.c").is_file())
        .collect();
    found.sort();
    if let Some(named) = found
        .iter()
        .find(|path| file_name(path).contains(&language.replace('-', "_")))
    {
        return Ok(named.clone());
    }
    match found.len() {
        0 => Err(InstallError::NoParser {
            url: url.to_string(),
        }),
        1 => Ok(found.remove(0)),
        _ => Err(InstallError::Ambiguous {
            url: url.to_string(),
            found: found.iter().map(|p| file_name(p)).collect(),
        }),
    }
}

/// Compiles `src/parser.c` and any external scanner into a shared library.
///
/// The command is the one from the guide this feature was asked for —
/// `cc -shared -fPIC -Isrc …` — with the two things it cannot assume filled
/// in: a scanner may be C or C++, and macOS spells a shared library
/// differently.
fn build(source: &Path, language: &str, into: &Path) -> Result<String, InstallError> {
    std::fs::create_dir_all(into).map_err(|source| InstallError::Io {
        path: into.display().to_string(),
        source,
    })?;
    let src = source.join("src");
    let parser = src.join("parser.c");
    if !parser.is_file() {
        return Err(InstallError::NoParser {
            url: source.display().to_string(),
        });
    }
    let scanner = ["scanner.c", "scanner.cc", "scanner.cpp"]
        .into_iter()
        .map(|name| src.join(name))
        .find(|path| path.is_file());
    // A C++ scanner has to be compiled and linked by the C++ driver, or the
    // standard library it uses is missing at load time rather than at build
    // time — which is a `dlopen` failure with an undefined symbol in it,
    // long after the thing that caused it.
    let cplusplus = scanner
        .as_ref()
        .is_some_and(|path| file_name(path).ends_with(".cc") || file_name(path).ends_with(".cpp"));
    let compiler = match cplusplus {
        true => std::env::var("CXX").unwrap_or_else(|_| "c++".to_string()),
        false => std::env::var("CC").unwrap_or_else(|_| "cc".to_string()),
    };

    let output = into.join(&library_names(language)[0]);
    let mut arguments: Vec<String> = vec![
        match cfg!(target_os = "macos") {
            true => "-dynamiclib".to_string(),
            false => "-shared".to_string(),
        },
        "-fPIC".to_string(),
        "-O2".to_string(),
        match cplusplus {
            true => "-std=c++14".to_string(),
            false => "-std=c11".to_string(),
        },
    ];
    if !cplusplus {
        // Scanners written years ago call `iswspace` without including
        // `<wctype.h>`, which every compiler accepted until GCC 14 and
        // clang 16 made an implicit declaration an error. The function is in
        // libc and the call links and works; refusing to build a third of
        // the grammars on the list over a missing include would be the
        // editor enforcing a rule the grammar's author never had to meet.
        // The tree-sitter CLI does the same for the same reason.
        arguments.push("-Wno-implicit-function-declaration".to_string());
    }
    arguments.extend([
        "-I".to_string() + &src.display().to_string(),
        parser.display().to_string(),
    ]);
    if let Some(scanner) = &scanner {
        arguments.push(scanner.display().to_string());
    }
    arguments.extend(["-o".to_string(), output.display().to_string()]);

    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run(&compiler, &borrowed, Some(source), "compile").map_err(|error| match error {
        // `cc` missing is the one failure with an obvious remedy, and the
        // raw "No such file or directory" does not suggest it.
        InstallError::ToolMissing { .. } => InstallError::ToolMissing {
            tool: compiler.clone(),
        },
        other => other,
    })
}

/// Copies the repository's highlight queries in beside the library.
///
/// `<into>/<language>/highlights.scm` is what [`crate::dynamic`] looks for.
/// Every `.scm` beside it comes too: a `highlights.scm` that `; inherits` or
/// includes another file is no use on its own.
fn install_queries(
    repository: &Path,
    source: &Path,
    language: &str,
    into: &Path,
) -> Result<Option<PathBuf>, InstallError> {
    let name = language.replace('-', "_");
    let candidates = [
        source.join("queries"),
        source.join("queries").join(&name),
        repository.join("queries"),
        repository.join("queries").join(&name),
        repository.join("queries").join(language),
    ];
    let Some(from) = candidates
        .iter()
        .find(|directory| directory.join("highlights.scm").is_file())
    else {
        return Ok(None);
    };
    let to = into.join(language);
    std::fs::create_dir_all(&to).map_err(|source| InstallError::Io {
        path: to.display().to_string(),
        source,
    })?;
    for entry in std::fs::read_dir(from)
        .map_err(|source| InstallError::Io {
            path: from.display().to_string(),
            source,
        })?
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "scm") {
            let target = to.join(entry.file_name());
            std::fs::copy(&path, &target).map_err(|source| InstallError::Io {
                path: target.display().to_string(),
                source,
            })?;
        }
    }
    Ok(Some(to))
}

/// The grammar this one is written on top of, as its own `grammar.js` says.
///
/// A grammar that extends another says so in one line — C++ opens with
/// `const C = require('tree-sitter-c/grammar')` — and that line is the only
/// place it is written down: `grammar.json` records the result rather than
/// where it came from.
///
/// It matters because such a repository's `highlights.scm` covers only what
/// it added. Installed by itself it colours `namespace` and `template` and
/// leaves every comment, string and `int` in the file plain, which reads as
/// a grammar that does not work rather than as half a query.
fn base_grammar(directories: &[&Path]) -> Option<String> {
    let source = directories
        .iter()
        .map(|directory| directory.join("grammar.js"))
        .find_map(|path| std::fs::read_to_string(path).ok())?;
    source.lines().find_map(|line| {
        let (_, required) = line.split_once("require(")?;
        let name = required
            .trim_start()
            .trim_start_matches(['\'', '"'])
            .strip_prefix("tree-sitter-")?
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();
        // `tree-sitter-cli` is the tool, not a parent.
        match name.is_empty() || name == "cli" {
            true => None,
            false => Some(name),
        }
    })
}

/// Writes `; inherits: <base>` into an installed query that needs one.
///
/// Only when the repository says it extends another grammar and the query
/// does not already say so itself — Neovim's and Helix's query trees do, and
/// a directive written twice would read the parent twice.
///
/// The line is a comment: the file stays a query anyone else can use, and
/// [`crate::dynamic`] is what acts on it.
fn note_inheritance(
    repository: &Path,
    source: &Path,
    language: &str,
    installed: &Path,
) -> Option<String> {
    let query = installed.join("highlights.scm");
    let text = std::fs::read_to_string(&query).ok()?;
    if text.contains("inherits:") {
        return None;
    }
    let base = base_grammar(&[source, repository])?;
    if base == language || base == language.replace('-', "_") {
        return None;
    }
    let noted = format!(
        "; inherits: {base}\n\
         ; ^ written by maxgus: this grammar extends {base}, and its query\n\
         ; covers only what it adds. Delete the line to use it on its own.\n\
         {text}"
    );
    std::fs::write(&query, noted).ok()?;
    Some(base)
}

/// Runs one program to completion and returns what it printed./// Runs one program to completion and returns what it printed.
///
/// `GIT_TERMINAL_PROMPT=0` is the important part of this: a repository that
/// has been renamed or made private asks for a username, and there is no
/// terminal to type one into. Without it the editor would appear to hang on
/// a clone that is really waiting for a password.
fn run(
    program: &str,
    arguments: &[&str],
    directory: Option<&Path>,
    what: &str,
) -> Result<String, InstallError> {
    let mut command = std::process::Command::new(program);
    command.args(arguments).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let printed = format!("$ {program} {}\n", arguments.join(" "));
    let output = command.output().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => InstallError::ToolMissing {
            tool: program.to_string(),
        },
        _ => InstallError::Ran {
            what: what.to_string(),
            message: error.to_string(),
        },
    })?;
    let said = format!(
        "{}{}{}",
        printed,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match output.status.success() {
        true => Ok(said),
        false => Err(InstallError::Ran {
            what: what.to_string(),
            message: said,
        }),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A directory that removes itself, so a failed clone does not leave a
/// repository in `/tmp` for every attempt.
struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<TempDir, InstallError> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|source| InstallError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Ok(TempDir(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository shaped like a grammar's, without cloning one.
    fn fake_repository(root: &Path, sources: &[(&str, &str)]) {
        for (path, contents) in sources {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a file has a parent")).unwrap();
            std::fs::write(path, contents).unwrap();
        }
    }

    #[test]
    fn a_grammar_at_the_root_is_the_one_to_build() {
        let dir = TempDir::new("maxgus-test-root").unwrap();
        fake_repository(dir.path(), &[("src/parser.c", "")]);
        assert_eq!(
            grammar_directory(dir.path(), "zig", "url").unwrap(),
            dir.path()
        );
    }

    #[test]
    fn a_repository_of_several_grammars_takes_the_one_named_for_the_language() {
        let dir = TempDir::new("maxgus-test-many").unwrap();
        fake_repository(
            dir.path(),
            &[
                ("apex/src/parser.c", ""),
                ("soql/src/parser.c", ""),
                ("sosl/src/parser.c", ""),
            ],
        );
        assert_eq!(
            grammar_directory(dir.path(), "soql", "url").unwrap(),
            dir.path().join("soql")
        );
        // With nothing to go on it refuses rather than picking one, because
        // the wrong grammar is worse than none.
        let error = grammar_directory(dir.path(), "wombat", "url").unwrap_err();
        assert!(
            matches!(error, InstallError::Ambiguous { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn a_single_grammar_in_a_subdirectory_needs_no_name_to_be_found() {
        let dir = TempDir::new("maxgus-test-one-sub").unwrap();
        fake_repository(dir.path(), &[("tree-sitter-markdown/src/parser.c", "")]);
        assert_eq!(
            grammar_directory(dir.path(), "markdown", "url").unwrap(),
            dir.path().join("tree-sitter-markdown")
        );
    }

    #[test]
    fn a_repository_with_no_parser_c_says_so_rather_than_running_a_compiler() {
        let dir = TempDir::new("maxgus-test-empty").unwrap();
        fake_repository(dir.path(), &[("README.md", "")]);
        let error = grammar_directory(dir.path(), "zig", "https://example/x").unwrap_err();
        assert!(
            matches!(error, InstallError::NoParser { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn queries_land_where_the_loader_looks_and_bring_their_neighbours() {
        let repository = TempDir::new("maxgus-test-queries").unwrap();
        let into = TempDir::new("maxgus-test-into").unwrap();
        fake_repository(
            repository.path(),
            &[
                ("src/parser.c", ""),
                ("queries/highlights.scm", "(comment) @comment"),
                ("queries/injections.scm", ""),
                ("queries/README.md", "not a query"),
            ],
        );
        let where_to = install_queries(repository.path(), repository.path(), "zig", into.path())
            .unwrap()
            .expect("there is a highlights.scm");
        assert_eq!(where_to, into.path().join("zig"));
        assert!(where_to.join("highlights.scm").is_file());
        assert!(
            where_to.join("injections.scm").is_file(),
            "a query that includes another needs it"
        );
        assert!(!where_to.join("README.md").exists(), "only queries");
    }

    #[test]
    fn queries_kept_per_language_inside_the_repository_are_found_too() {
        let repository = TempDir::new("maxgus-test-nested").unwrap();
        let into = TempDir::new("maxgus-test-nested-into").unwrap();
        fake_repository(
            repository.path(),
            &[("queries/c_sharp/highlights.scm", "(comment) @comment")],
        );
        let where_to =
            install_queries(repository.path(), repository.path(), "c-sharp", into.path())
                .unwrap()
                .expect("nested under the grammar's own name");
        assert!(where_to.join("highlights.scm").is_file());
    }

    #[test]
    fn a_repository_without_queries_is_not_an_error_here() {
        let repository = TempDir::new("maxgus-test-noq").unwrap();
        let into = TempDir::new("maxgus-test-noq-into").unwrap();
        fake_repository(repository.path(), &[("src/parser.c", "")]);
        assert_eq!(
            install_queries(repository.path(), repository.path(), "zig", into.path()).unwrap(),
            None,
            "the install reports it as a warning instead"
        );
    }

    #[test]
    fn a_grammar_that_extends_another_is_recognised_from_its_own_source() {
        let dir = TempDir::new("maxgus-test-base").unwrap();
        fake_repository(
            dir.path(),
            &[(
                "grammar.js",
                "// @ts-check\nconst C = require('tree-sitter-c/grammar');\n\nmodule.exports = grammar(C, {});\n",
            )],
        );
        assert_eq!(base_grammar(&[dir.path()]), Some("c".to_string()));

        // A repository that requires nothing but the tooling extends nothing.
        let plain = TempDir::new("maxgus-test-plain").unwrap();
        fake_repository(
            plain.path(),
            &[(
                "grammar.js",
                "/// <reference types=\"tree-sitter-cli/dsl\" />\nmodule.exports = grammar({});\n",
            )],
        );
        assert_eq!(base_grammar(&[plain.path()]), None);
        assert_eq!(base_grammar(&[Path::new("/nowhere/at/all")]), None);
    }

    #[test]
    fn a_delta_query_is_told_which_query_to_be_read_after() {
        let repository = TempDir::new("maxgus-test-inherit").unwrap();
        let into = TempDir::new("maxgus-test-inherit-into").unwrap();
        fake_repository(
            repository.path(),
            &[
                (
                    "grammar.js",
                    "const C = require('tree-sitter-c/grammar');\n",
                ),
                ("queries/highlights.scm", "(template_function) @function\n"),
            ],
        );
        let installed = install_queries(repository.path(), repository.path(), "cpp", into.path())
            .unwrap()
            .expect("there is a highlights.scm");
        assert_eq!(
            note_inheritance(repository.path(), repository.path(), "cpp", &installed),
            Some("c".to_string())
        );
        let written = std::fs::read_to_string(installed.join("highlights.scm")).unwrap();
        assert!(written.starts_with("; inherits: c\n"), "{written}");
        assert!(
            written.ends_with("(template_function) @function\n"),
            "the query itself is untouched below the note"
        );

        // Said once. A second install must not stack the directive up.
        assert_eq!(
            note_inheritance(repository.path(), repository.path(), "cpp", &installed),
            None
        );
    }

    #[test]
    fn a_query_that_already_says_what_it_inherits_is_left_alone() {
        let repository = TempDir::new("maxgus-test-said").unwrap();
        let into = TempDir::new("maxgus-test-said-into").unwrap();
        fake_repository(
            repository.path(),
            &[
                (
                    "grammar.js",
                    "const C = require('tree-sitter-c/grammar');\n",
                ),
                ("queries/highlights.scm", "; inherits: c,preproc\n(x) @y\n"),
            ],
        );
        let installed = install_queries(repository.path(), repository.path(), "cpp", into.path())
            .unwrap()
            .expect("there is a highlights.scm");
        assert_eq!(
            note_inheritance(repository.path(), repository.path(), "cpp", &installed),
            None,
            "Neovim's and Helix's queries say it themselves"
        );
    }

    #[test]
    fn a_missing_program_is_named_rather_than_reported_as_a_file_not_found() {
        let error = run("maxgus-no-such-program", &[], None, "test").unwrap_err();
        match error {
            InstallError::ToolMissing { tool } => assert_eq!(tool, "maxgus-no-such-program"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn a_program_that_fails_carries_what_it_said() {
        let error = run("false", &[], None, "test").unwrap_err();
        assert!(matches!(error, InstallError::Ran { .. }), "got {error:?}");
    }

    #[test]
    fn a_temporary_directory_takes_its_contents_with_it() {
        let path = {
            let dir = TempDir::new("maxgus-test-temp").unwrap();
            std::fs::write(dir.path().join("file"), "x").unwrap();
            dir.path().to_path_buf()
        };
        assert!(!path.exists(), "a failed clone leaves nothing behind");
    }

    #[test]
    fn a_recent_cache_is_used_and_a_missing_one_is_not() {
        let dir = TempDir::new("maxgus-test-cache").unwrap();
        let cache = dir.path().join("parsers.md");
        assert_eq!(fresh_cache(&cache), None, "nothing cached yet");
        std::fs::write(&cache, "| x | y | z | 1 | yes | no |").unwrap();
        assert!(fresh_cache(&cache).is_some(), "just written");
    }
}
