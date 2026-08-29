//! Remembering what was open.
//!
//! What is worth restoring is what a person would have to do by hand to carry
//! on: the files they had open, where they were in each, which one they were
//! looking at, and whether the panel was up. Window splits are deliberately
//! not in that list — Emacs' `desktop-save-mode` leaves them out too, and a
//! layout restored into a differently sized terminal is worse than none.
//!
//! Written as KDL, because the configuration is KDL and a person may want to
//! read or delete one.

use std::path::{Path, PathBuf};

/// One file, and where the reader was in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub path: PathBuf,
    /// The character offset point was at.
    pub point: usize,
    /// The first line the window was showing, so a long file comes back
    /// scrolled where it was rather than jumping to put point in the middle.
    pub top_line: usize,
}

/// Everything a session remembers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub root: Option<PathBuf>,
    pub files: Vec<OpenFile>,
    /// Which of `files` was being edited.
    pub current: Option<PathBuf>,
    pub panel_open: bool,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Writes the session as KDL.
    pub fn to_kdl(&self) -> String {
        let mut out = String::from("// Written by maxgus. Delete it to start fresh.\n");
        if let Some(root) = &self.root {
            out.push_str(&format!("root {}\n", quote(&root.to_string_lossy())));
        }
        // KDL 2 spells booleans with a hash; a bare `true` is an identifier.
        out.push_str(&format!(
            "panel {}\n",
            match self.panel_open {
                true => "#true",
                false => "#false",
            }
        ));
        for file in &self.files {
            let current = self.current.as_deref() == Some(file.path.as_path());
            out.push_str(&format!(
                "file {} point={} top={}{}\n",
                quote(&file.path.to_string_lossy()),
                file.point,
                file.top_line,
                match current {
                    true => " current=#true",
                    false => "",
                }
            ));
        }
        out
    }

    /// Reads a session back, ignoring anything it does not recognise.
    ///
    /// A session file is written by the editor and read by it, so the only
    /// way it is wrong is if it was edited by hand or written by an older
    /// version. Neither is a reason to refuse to start: what can be read is
    /// used and the rest is dropped.
    pub fn from_kdl(source: &str) -> Session {
        let mut session = Session::default();
        let Ok(document) = source.parse::<kdl::KdlDocument>() else {
            return session;
        };
        for node in document.nodes() {
            match node.name().value() {
                "root" => {
                    if let Some(text) = node.entries().first().and_then(|e| e.value().as_string()) {
                        session.root = Some(PathBuf::from(text));
                    }
                }
                "panel" => {
                    session.panel_open = node
                        .entries()
                        .first()
                        .and_then(|e| e.value().as_bool())
                        .unwrap_or(false);
                }
                "file" => {
                    let Some(path) = node
                        .entries()
                        .iter()
                        .find(|e| e.name().is_none())
                        .and_then(|e| e.value().as_string())
                    else {
                        continue;
                    };
                    let number = |name: &str| {
                        node.entries()
                            .iter()
                            .find(|e| e.name().is_some_and(|n| n.value() == name))
                            .and_then(|e| e.value().as_integer())
                            .unwrap_or(0)
                            .max(0) as usize
                    };
                    let path = PathBuf::from(path);
                    let current = node
                        .entries()
                        .iter()
                        .find(|e| e.name().is_some_and(|n| n.value() == "current"))
                        .and_then(|e| e.value().as_bool())
                        .unwrap_or(false);
                    if current {
                        session.current = Some(path.clone());
                    }
                    session.files.push(OpenFile {
                        path,
                        point: number("point"),
                        top_line: number("top"),
                    });
                }
                _ => {}
            }
        }
        session
    }
}

/// Where a project's session is kept.
///
/// Under the state directory rather than in the project, because a session is
/// the reader's business and not the repository's: nobody wants to gitignore
/// their editor.
pub fn path_for(state_dir: &Path, root: &Path) -> PathBuf {
    state_dir
        .join("sessions")
        .join(format!("{}.kdl", fingerprint(root)))
}

/// A stable name for a directory, short enough to be a filename.
///
/// The path itself would be one too, but a path with separators in it is not
/// a filename and a path that is escaped into one is unreadable. A digest
/// with the directory's own name in front is both.
fn fingerprint(root: &Path) -> String {
    let text = root.to_string_lossy();
    // FNV-1a: a hash, not a cryptographic one — nothing here is a secret and
    // a collision costs one restored session, not a security property.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "root".into())
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    match name.is_empty() {
        true => format!("{hash:016x}"),
        false => format!("{name}-{hash:016x}"),
    }
}

/// Quotes a string for KDL.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            root: Some(PathBuf::from("/home/someone/project")),
            files: vec![
                OpenFile {
                    path: PathBuf::from("/home/someone/project/src/main.rs"),
                    point: 420,
                    top_line: 12,
                },
                OpenFile {
                    path: PathBuf::from("/home/someone/project/README.md"),
                    point: 0,
                    top_line: 0,
                },
            ],
            current: Some(PathBuf::from("/home/someone/project/src/main.rs")),
            panel_open: true,
        }
    }

    #[test]
    fn a_session_survives_being_written_and_read_back() {
        let written = session().to_kdl();
        assert_eq!(Session::from_kdl(&written), session());
    }

    #[test]
    fn the_written_form_is_readable_kdl() {
        let written = session().to_kdl();
        assert!(
            written.parse::<kdl::KdlDocument>().is_ok(),
            "it does not parse as KDL:\n{written}"
        );
        assert!(written.contains("src/main.rs"), "no file in it");
        assert!(written.contains("point=420"), "no point");
        assert!(written.contains("current=#true"), "nothing marked current");
    }

    #[test]
    fn a_path_with_a_quote_in_it_comes_back_whole() {
        let odd = Session {
            root: None,
            files: vec![OpenFile {
                path: PathBuf::from(r#"/tmp/a "quoted" \name.rs"#),
                point: 1,
                top_line: 0,
            }],
            current: None,
            panel_open: false,
        };
        assert_eq!(Session::from_kdl(&odd.to_kdl()), odd);
    }

    #[test]
    fn a_file_that_cannot_be_parsed_is_no_session_rather_than_an_error() {
        assert!(Session::from_kdl("this is not kdl {{{").is_empty());
    }

    #[test]
    fn a_node_it_does_not_know_is_ignored() {
        let read = Session::from_kdl(
            "root \"/p\"\nsomething-from-the-future 3\nfile \"/p/a.rs\" point=1\n",
        );
        assert_eq!(read.files.len(), 1);
        assert_eq!(read.root, Some(PathBuf::from("/p")));
    }

    #[test]
    fn a_file_with_no_point_starts_at_the_beginning() {
        let read = Session::from_kdl("file \"/p/a.rs\"\n");
        assert_eq!(read.files[0].point, 0);
        assert_eq!(read.files[0].top_line, 0);
    }

    #[test]
    fn two_projects_get_two_session_files() {
        let state = Path::new("/state");
        let one = path_for(state, Path::new("/home/someone/alpha"));
        let two = path_for(state, Path::new("/home/someone/beta"));
        assert_ne!(one, two);
        assert!(one.starts_with("/state/sessions"));
    }

    #[test]
    fn two_projects_of_the_same_name_are_still_told_apart() {
        // The interesting collision: `~/work/editor` and `~/play/editor` are
        // different projects with the same name, and the name is only half of
        // what the filename is made from.
        let state = Path::new("/state");
        assert_ne!(
            path_for(state, Path::new("/home/someone/work/editor")),
            path_for(state, Path::new("/home/someone/play/editor"))
        );
    }

    #[test]
    fn the_same_project_always_gets_the_same_file() {
        let state = Path::new("/state");
        assert_eq!(
            path_for(state, Path::new("/home/someone/alpha")),
            path_for(state, Path::new("/home/someone/alpha"))
        );
    }

    #[test]
    fn a_session_file_is_named_after_the_project_it_is_for() {
        let named = path_for(Path::new("/state"), Path::new("/home/someone/my-project"));
        assert!(
            named.to_string_lossy().contains("my-project"),
            "unreadable name: {named:?}"
        );
    }

    #[test]
    fn a_project_whose_name_is_awkward_still_gets_a_filename() {
        let odd = path_for(
            Path::new("/state"),
            Path::new("/home/someone/../weird name!"),
        );
        let name = odd.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains('/'), "a separator in a filename: {name}");
        assert!(!name.contains('!'), "punctuation in a filename: {name}");
        assert!(name.ends_with(".kdl"));
    }
}
