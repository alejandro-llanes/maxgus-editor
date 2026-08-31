//! Named sets of directories, kept between sessions.
//!
//! The tree can show several directories at once. A workspace is that list
//! given a name and written down, so the set you work in is one command to
//! come back to rather than several to rebuild. treemacs has the same idea
//! and the same word for it.
//!
//! Distinct from a *session*, which is the files you had open and where you
//! were in each: a session is where you left off, and a workspace is what
//! you are working on. One is restored for you and the other is chosen.
//!
//! Written as KDL, because the configuration is KDL and a person may want to
//! read one, edit one, or delete the lot.

use std::path::{Path, PathBuf};

/// One named set of directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    /// In the order the tree should show them. The first is the project.
    pub directories: Vec<PathBuf>,
}

/// Every workspace that has been saved, in the order they were saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspaces(Vec<Workspace>);

impl Workspaces {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Workspace> {
        self.0.iter()
    }

    /// The names, for a prompt to offer.
    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|w| w.name.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&Workspace> {
        self.0.iter().find(|w| w.name == name)
    }

    /// Saves `directories` under `name`, replacing a workspace of that name.
    ///
    /// Replacing rather than refusing: saving over one is how a workspace is
    /// edited, and a second entry with the same name would be a list with
    /// two answers in it.
    pub fn save(&mut self, name: impl Into<String>, directories: Vec<PathBuf>) {
        let name = name.into();
        match self.0.iter_mut().find(|w| w.name == name) {
            Some(existing) => existing.directories = directories,
            None => self.0.push(Workspace { name, directories }),
        }
    }

    /// Forgets one. Says whether there was one to forget.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|w| w.name != name);
        self.0.len() != before
    }

    /// Writes them as KDL.
    pub fn to_kdl(&self) -> String {
        let mut out =
            String::from("// Written by maxgus. Saved sets of directories for the file tree.\n");
        for workspace in &self.0 {
            out.push_str(&format!("workspace {} {{\n", quote(&workspace.name)));
            for directory in &workspace.directories {
                out.push_str(&format!(
                    "    directory {}\n",
                    quote(&directory.to_string_lossy())
                ));
            }
            out.push_str("}\n");
        }
        out
    }

    /// Reads them back, ignoring anything unrecognised.
    ///
    /// The same rule the session file follows: this is written by the editor
    /// and read by it, so the only ways it is wrong are a hand edit or an
    /// older version, and neither is a reason to refuse to start. What can
    /// be read is used and the rest is dropped.
    pub fn from_kdl(source: &str) -> Workspaces {
        let Ok(document) = source.parse::<kdl::KdlDocument>() else {
            return Workspaces::default();
        };
        let mut out = Workspaces::default();
        for node in document.nodes() {
            if node.name().value() != "workspace" {
                continue;
            }
            let Some(name) = node.entries().first().and_then(|e| e.value().as_string()) else {
                continue;
            };
            let mut directories = Vec::new();
            if let Some(children) = node.children() {
                for child in children.nodes() {
                    if child.name().value() != "directory" {
                        continue;
                    }
                    if let Some(path) = child.entries().first().and_then(|e| e.value().as_string())
                    {
                        directories.push(PathBuf::from(path));
                    }
                }
            }
            // A workspace with nothing in it would open a tree with no
            // directories, which the tree does not allow anyway.
            if !directories.is_empty() {
                out.save(name, directories);
            }
        }
        out
    }
}

/// Where the list is kept.
///
/// Under the state directory beside the sessions, and not in the
/// configuration: it is written by the editor rather than by hand, and a
/// file the editor rewrites is a poor place for anyone's comments.
pub fn path_for(state_dir: &Path) -> PathBuf {
    state_dir.join("workspaces.kdl")
}

/// A KDL string, with the quotes and escapes it needs.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Workspaces {
        let mut workspaces = Workspaces::default();
        workspaces.save("editor", vec!["/src/maxgus".into(), "/src/notes".into()]);
        workspaces.save("website", vec!["/src/site".into()]);
        workspaces
    }

    #[test]
    fn a_workspace_is_a_name_and_the_directories_under_it() {
        let workspaces = saved();
        assert_eq!(workspaces.names(), ["editor", "website"]);
        assert_eq!(
            workspaces.get("editor").unwrap().directories,
            [PathBuf::from("/src/maxgus"), PathBuf::from("/src/notes")]
        );
        assert_eq!(workspaces.get("nothing"), None);
    }

    #[test]
    fn saving_over_one_replaces_it_rather_than_adding_a_second() {
        // Saving over a workspace is how it is edited, and two entries with
        // the same name would be a list with two answers in it.
        let mut workspaces = saved();
        workspaces.save("editor", vec!["/elsewhere".into()]);
        assert_eq!(
            workspaces.names(),
            ["editor", "website"],
            "it was added twice"
        );
        assert_eq!(
            workspaces.get("editor").unwrap().directories,
            [PathBuf::from("/elsewhere")]
        );
    }

    #[test]
    fn one_can_be_forgotten_and_says_whether_there_was_one() {
        let mut workspaces = saved();
        assert!(workspaces.remove("website"));
        assert_eq!(workspaces.names(), ["editor"]);
        assert!(!workspaces.remove("website"), "it was removed twice");
    }

    #[test]
    fn what_is_written_is_what_comes_back() {
        let workspaces = saved();
        let read = Workspaces::from_kdl(&workspaces.to_kdl());
        assert_eq!(read, workspaces);
    }

    #[test]
    fn a_path_with_quotes_or_backslashes_in_it_survives_the_round_trip() {
        // Rare, and the sort of thing that corrupts a whole file when it is
        // not handled: an unescaped quote ends the string early and every
        // workspace after it is lost.
        let mut workspaces = Workspaces::default();
        let awkward = PathBuf::from(r#"/src/one "two"\three"#);
        workspaces.save("odd", vec![awkward.clone()]);
        let read = Workspaces::from_kdl(&workspaces.to_kdl());
        assert_eq!(read.get("odd").unwrap().directories, [awkward]);
    }

    #[test]
    fn a_file_that_will_not_parse_is_no_workspaces_rather_than_no_editor() {
        assert!(Workspaces::from_kdl("workspace { { {").is_empty());
        assert!(Workspaces::from_kdl("").is_empty());
    }

    #[test]
    fn anything_unrecognised_is_dropped_and_the_rest_is_kept() {
        // An older version, or a hand edit. What can be read is used.
        let read = Workspaces::from_kdl(
            r#"
            nonsense "here"
            workspace "good" {
                directory "/a"
                something-else "ignored"
            }
            workspace {
                directory "/nameless"
            }
            "#,
        );
        assert_eq!(read.names(), ["good"], "a nameless one was kept");
        assert_eq!(read.get("good").unwrap().directories, [PathBuf::from("/a")]);
    }

    #[test]
    fn a_workspace_with_no_directories_is_not_kept() {
        // It would open a tree with nothing in it, which the tree refuses
        // anyway — better to not offer it than to offer it and fail.
        let read = Workspaces::from_kdl("workspace \"empty\" {\n}\n");
        assert!(read.is_empty());
    }

    #[test]
    fn the_list_lives_beside_the_sessions_rather_than_in_the_configuration() {
        let path = path_for(Path::new("/state"));
        assert_eq!(path, PathBuf::from("/state/workspaces.kdl"));
    }
}
