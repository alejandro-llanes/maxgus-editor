//! Tree nodes.

use crate::git::GitStatus;
use std::path::{Path, PathBuf};

/// What a node represents on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeKind {
    Directory,
    File,
    /// A symbolic link; `Node::target_is_dir` says what it points at.
    Symlink,
}

impl NodeKind {
    pub fn is_directory(self) -> bool {
        matches!(self, NodeKind::Directory)
    }
}

/// One entry in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub kind: NodeKind,
    /// True for an expanded directory. Meaningless for files.
    pub expanded: bool,
    /// Children, populated the first time the directory is expanded.
    pub children: Vec<Node>,
    /// True once the directory's children have been read, so an empty
    /// directory is distinguishable from an unread one.
    pub loaded: bool,
    /// Git status, when the tree is inside a repository.
    pub git: Option<GitStatus>,
    /// For symlinks, whether the target is a directory.
    pub target_is_dir: bool,
}

impl Node {
    pub fn new(path: impl Into<PathBuf>, kind: NodeKind) -> Node {
        let path = path.into();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            // The root of the filesystem has no file name.
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Node {
            path,
            name,
            kind,
            expanded: false,
            children: Vec::new(),
            loaded: false,
            git: None,
            target_is_dir: false,
        }
    }

    /// True when this node can be expanded: a directory, or a symlink to one.
    pub fn is_expandable(&self) -> bool {
        self.kind.is_directory() || (self.kind == NodeKind::Symlink && self.target_is_dir)
    }

    /// True when the node is hidden by convention: a leading dot.
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    /// The face this node is drawn in.
    pub fn face(&self, is_root: bool) -> &'static str {
        if is_root {
            return "tree-root";
        }
        match self.kind {
            NodeKind::Directory => "tree-directory",
            NodeKind::Symlink => "tree-symlink",
            NodeKind::File => "tree-file",
        }
    }

    /// The face for the git indicator, when there is one.
    pub fn git_face(&self) -> Option<&'static str> {
        self.git.map(GitStatus::face)
    }

    /// The arrow shown before an expandable node.
    pub fn arrow(&self) -> &'static str {
        if !self.is_expandable() {
            "  "
        } else if self.expanded {
            "v "
        } else {
            "> "
        }
    }

    /// Finds the child whose path is `path`, at any depth.
    pub fn find(&self, path: &Path) -> Option<&Node> {
        if self.path == path {
            return Some(self);
        }
        // Only descend where the path could possibly live.
        if !path.starts_with(&self.path) {
            return None;
        }
        self.children.iter().find_map(|c| c.find(path))
    }

    /// Mutable counterpart of [`Node::find`].
    pub fn find_mut(&mut self, path: &Path) -> Option<&mut Node> {
        if self.path == path {
            return Some(self);
        }
        if !path.starts_with(&self.path) {
            return None;
        }
        self.children.iter_mut().find_map(|c| c.find_mut(path))
    }

    /// Sort key placing directories before files, then case-insensitive by
    /// name — the order treemacs uses.
    fn sort_key(&self, directories_first: bool) -> (u8, String) {
        let group = u8::from(directories_first && !self.is_expandable());
        (group, self.name.to_lowercase())
    }

    /// Sorts this node's children in place.
    pub fn sort_children(&mut self, directories_first: bool) {
        self.children.sort_by_key(|c| c.sort_key(directories_first));
    }

    /// Collapses this node and everything under it.
    pub fn collapse_recursively(&mut self) {
        self.expanded = false;
        for child in &mut self.children {
            child.collapse_recursively();
        }
    }

    /// Number of nodes in this subtree, counting itself.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Node::count).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str) -> Node {
        Node::new(path, NodeKind::Directory)
    }

    fn file(path: &str) -> Node {
        Node::new(path, NodeKind::File)
    }

    #[test]
    fn a_node_takes_its_name_from_its_path() {
        assert_eq!(file("/a/b/c.rs").name, "c.rs");
        assert_eq!(dir("/a/b").name, "b");
        assert_eq!(dir("/").name, "/", "the filesystem root has no file name");
    }

    #[test]
    fn directories_and_directory_symlinks_are_expandable() {
        assert!(dir("/a").is_expandable());
        assert!(!file("/a/b.txt").is_expandable());
        let mut link = Node::new("/a/link", NodeKind::Symlink);
        assert!(!link.is_expandable(), "a link to a file is not");
        link.target_is_dir = true;
        assert!(link.is_expandable());
    }

    #[test]
    fn dotfiles_are_hidden_by_convention() {
        assert!(file("/a/.gitignore").is_hidden());
        assert!(!file("/a/README").is_hidden());
    }

    #[test]
    fn faces_distinguish_the_node_kinds() {
        assert_eq!(dir("/a").face(true), "tree-root");
        assert_eq!(dir("/a").face(false), "tree-directory");
        assert_eq!(file("/a/b").face(false), "tree-file");
        assert_eq!(Node::new("/a/l", NodeKind::Symlink).face(false), "tree-symlink");
    }

    #[test]
    fn arrows_reflect_expansion_state() {
        let mut d = dir("/a");
        assert_eq!(d.arrow(), "> ");
        d.expanded = true;
        assert_eq!(d.arrow(), "v ");
        assert_eq!(file("/a/b").arrow(), "  ", "files have no arrow");
    }

    #[test]
    fn the_git_face_follows_the_status() {
        let mut n = file("/a/b");
        assert_eq!(n.git_face(), None);
        n.git = Some(GitStatus::Modified);
        assert_eq!(n.git_face(), Some("tree-git-modified"));
    }

    #[test]
    fn find_locates_a_node_at_any_depth() {
        let mut root = dir("/a");
        let mut sub = dir("/a/b");
        sub.children.push(file("/a/b/c.rs"));
        root.children.push(sub);

        assert_eq!(root.find(Path::new("/a/b/c.rs")).unwrap().name, "c.rs");
        assert_eq!(root.find(Path::new("/a/b")).unwrap().name, "b");
        assert_eq!(root.find(Path::new("/a")).unwrap().name, "a");
        assert!(root.find(Path::new("/a/missing")).is_none());
        assert!(root.find(Path::new("/elsewhere")).is_none());
    }

    #[test]
    fn find_mut_allows_editing_in_place() {
        let mut root = dir("/a");
        root.children.push(dir("/a/b"));
        root.find_mut(Path::new("/a/b")).unwrap().expanded = true;
        assert!(root.find(Path::new("/a/b")).unwrap().expanded);
    }

    #[test]
    fn children_sort_directories_first_then_case_insensitively() {
        let mut root = dir("/a");
        root.children = vec![file("/a/zeta.rs"), dir("/a/Src"), file("/a/Alpha.rs"), dir("/a/bin")];
        root.sort_children(true);
        let names: Vec<&str> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["bin", "Src", "Alpha.rs", "zeta.rs"]);
    }

    #[test]
    fn sorting_can_ignore_the_directory_grouping() {
        let mut root = dir("/a");
        root.children = vec![file("/a/a.rs"), dir("/a/z")];
        root.sort_children(false);
        let names: Vec<&str> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a.rs", "z"]);
    }

    #[test]
    fn collapsing_recursively_closes_the_whole_subtree() {
        let mut root = dir("/a");
        root.expanded = true;
        let mut sub = dir("/a/b");
        sub.expanded = true;
        sub.children.push({
            let mut d = dir("/a/b/c");
            d.expanded = true;
            d
        });
        root.children.push(sub);

        root.collapse_recursively();
        assert!(!root.expanded);
        assert!(!root.find(Path::new("/a/b")).unwrap().expanded);
        assert!(!root.find(Path::new("/a/b/c")).unwrap().expanded);
    }

    #[test]
    fn count_includes_the_node_itself() {
        let mut root = dir("/a");
        assert_eq!(root.count(), 1);
        root.children.push(file("/a/x"));
        root.children.push({
            let mut d = dir("/a/b");
            d.children.push(file("/a/b/y"));
            d
        });
        assert_eq!(root.count(), 4);
    }
}
