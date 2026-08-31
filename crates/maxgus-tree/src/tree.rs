//! The tree model and its operations.

use crate::{
    Result, TreeError,
    git::{GitStatus, git_status},
    node::{Node, NodeKind},
};
use maxgus_config::TreeConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One line of the rendered tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleNode {
    pub path: PathBuf,
    pub name: String,
    pub kind: NodeKind,
    /// Indentation level; the root is zero.
    pub depth: usize,
    pub expanded: bool,
    pub expandable: bool,
    pub git: Option<GitStatus>,
    pub is_root: bool,
}

impl VisibleNode {
    /// The face the label is drawn in.
    pub fn face(&self) -> &'static str {
        if self.is_root {
            "tree-root"
        } else {
            match self.kind {
                NodeKind::Directory => "tree-directory",
                NodeKind::Symlink => "tree-symlink",
                NodeKind::File => "tree-file",
            }
        }
    }

    /// The arrow prefix.
    pub fn arrow(&self) -> &'static str {
        if !self.expandable {
            "  "
        } else if self.expanded {
            "v "
        } else {
            "> "
        }
    }

    /// The whole line as text, without faces: indentation, arrow, name and the
    /// git indicator.
    pub fn render(&self) -> String {
        let indent = "  ".repeat(self.depth);
        let git = self
            .git
            .map(|g| format!(" {}", g.indicator()))
            .unwrap_or_default();
        format!("{indent}{}{}{git}", self.arrow(), self.name)
    }
}

/// A lazily expanded view of one or more directories.
///
/// More than one because a workspace is usually more than one directory —
/// the library beside the application that uses it, the notes beside the
/// code. treemacs calls them projects and keeps a list of them; so does
/// this, and the first of them is the one the rest of the editor means by
/// "the project": what a language server is told about, what a project
/// search walks. Adding a second directory to look at is not the same as
/// changing which project you are working in.
#[derive(Debug)]
pub struct FileTree {
    /// Never empty. A tree with no roots has nothing to draw and no way to
    /// get a root back, so the last one cannot be removed.
    roots: Vec<Node>,
    config: TreeConfig,
    /// Index into the flattened view.
    cursor: usize,
    visible: Vec<VisibleNode>,
    git: HashMap<PathBuf, GitStatus>,
}

impl FileTree {
    /// Opens `root` and expands it one level.
    pub async fn open(root: impl Into<PathBuf>, config: TreeConfig) -> Result<FileTree> {
        let root = root.into();
        let meta = tokio::fs::metadata(&root)
            .await
            .map_err(|source| TreeError::Io {
                path: root.clone(),
                source,
            })?;
        if !meta.is_dir() {
            return Err(TreeError::NotADirectory(root));
        }
        let mut tree = FileTree {
            roots: vec![Node::new(root, NodeKind::Directory)],
            config,
            cursor: 0,
            visible: Vec::new(),
            git: HashMap::new(),
        };
        tree.refresh().await?;
        Ok(tree)
    }

    /// The first root, which is what the rest of the editor means by "the
    /// project": the directory a language server is told about and a
    /// project search walks. Adding a second directory to look at does not
    /// move it.
    pub fn root_path(&self) -> &Path {
        &self.roots[0].path
    }

    /// Every directory the tree is showing, in the order they were added.
    pub fn roots(&self) -> Vec<&Path> {
        self.roots.iter().map(|root| root.path.as_path()).collect()
    }

    /// True when `path` is one of the roots rather than something inside one.
    pub fn is_root(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| root.path == path)
    }

    /// The root `path` belongs to, if any.
    fn root_of(&self, path: &Path) -> Option<&Node> {
        self.roots.iter().find(|root| path.starts_with(&root.path))
    }

    /// Finds a node anywhere in any root.
    fn find(&self, path: &Path) -> Option<&Node> {
        self.roots.iter().find_map(|root| root.find(path))
    }

    fn find_mut(&mut self, path: &Path) -> Option<&mut Node> {
        self.roots.iter_mut().find_map(|root| root.find_mut(path))
    }

    /// Adds another directory to look at, below the ones already there.
    ///
    /// treemacs' `treemacs-add-project-to-workspace`. A directory already
    /// on the list is not added twice — the second copy would expand and
    /// collapse independently of the first, which is two answers to the
    /// same question.
    pub async fn add_root(&mut self, path: impl Into<PathBuf>) -> Result<()> {
        let path = path.into();
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|source| TreeError::Io {
                path: path.clone(),
                source,
            })?;
        if !meta.is_dir() {
            return Err(TreeError::NotADirectory(path));
        }
        if self.roots.iter().any(|root| root.path == path) {
            return Ok(());
        }
        self.roots.push(Node::new(path, NodeKind::Directory));
        self.refresh().await
    }

    /// Swaps one of the roots for another directory, leaving the rest.
    ///
    /// What `root-down` and `root-up` do. Replacing the whole tree would
    /// throw away every other directory on the list, which is a surprising
    /// amount to lose for a command that says it moves *the* root.
    pub async fn replace_root(&mut self, old: &Path, new: impl Into<PathBuf>) -> Result<()> {
        let new = new.into();
        let meta = tokio::fs::metadata(&new)
            .await
            .map_err(|source| TreeError::Io {
                path: new.clone(),
                source,
            })?;
        if !meta.is_dir() {
            return Err(TreeError::NotADirectory(new));
        }
        let Some(at) = self.roots.iter().position(|root| root.path == old) else {
            return Err(TreeError::NotARoot(old.to_path_buf()));
        };
        self.roots[at] = Node::new(new, NodeKind::Directory);
        self.refresh().await
    }

    /// Stops looking at one of them.
    ///
    /// The last one stays: a tree with nothing in it has no row to put a
    /// cursor on and no way to ask for a directory back.
    pub fn remove_root(&mut self, path: &Path) -> Result<()> {
        if self.roots.len() <= 1 {
            return Err(TreeError::LastRoot);
        }
        let Some(at) = self.roots.iter().position(|root| root.path == path) else {
            return Err(TreeError::NotARoot(path.to_path_buf()));
        };
        self.roots.remove(at);
        self.rebuild_visible();
        Ok(())
    }

    pub fn config(&self) -> &TreeConfig {
        &self.config
    }

    /// The flattened, renderable view.
    pub fn visible(&self) -> &[VisibleNode] {
        &self.visible
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The node under the cursor.
    pub fn selected(&self) -> Option<&VisibleNode> {
        self.visible.get(self.cursor)
    }

    /// The path under the cursor.
    pub fn selected_path(&self) -> Option<&Path> {
        self.selected().map(|n| n.path.as_path())
    }

    /// Moves the cursor to an absolute line, clamped into range.
    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.visible.len().saturating_sub(1));
    }

    // ---- reading -------------------------------------------------------

    /// True when `name` is filtered out by the ignore list or the hidden rule.
    fn is_filtered(&self, name: &str) -> bool {
        if !self.config.show_hidden && name.starts_with('.') {
            return true;
        }
        self.config.ignore.iter().any(|i| i == name)
    }

    /// Reads one directory into a list of nodes.
    async fn read_dir(&self, path: &Path) -> Result<Vec<Node>> {
        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|source| TreeError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        let mut nodes = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|source| TreeError::Io {
            path: path.to_path_buf(),
            source,
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.is_filtered(&name) {
                continue;
            }
            // `file_type` does not follow links, which is what we want here.
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            let child_path = entry.path();
            let kind = if file_type.is_symlink() {
                NodeKind::Symlink
            } else if file_type.is_dir() {
                NodeKind::Directory
            } else {
                NodeKind::File
            };
            let mut node = Node::new(child_path.clone(), kind);
            if kind == NodeKind::Symlink {
                // Resolve just enough to know whether the link is expandable.
                node.target_is_dir = tokio::fs::metadata(&child_path)
                    .await
                    .is_ok_and(|m| m.is_dir());
            }
            node.git = self.git.get(&child_path).copied();
            nodes.push(node);
        }
        let directories_first = self.config.directories_first;
        nodes.sort_by_key(|n| {
            let group = if directories_first && n.is_expandable() {
                0
            } else {
                1
            };
            (group, n.name.to_lowercase())
        });
        Ok(nodes)
    }

    /// Re-reads every expanded directory and refreshes git status, preserving
    /// which directories are open and where the cursor sits.
    pub async fn refresh(&mut self) -> Result<()> {
        let selected = self.selected_path().map(Path::to_path_buf);
        if self.config.git_status {
            // One reading per root: they may be different repositories, or
            // no repository at all, and a status read from one of them says
            // nothing about the others.
            self.git.clear();
            for path in self
                .roots
                .iter()
                .map(|r| r.path.clone())
                .collect::<Vec<_>>()
            {
                self.git.extend(git_status(&path, false).await);
            }
        }
        let expanded = self.expanded_paths();
        for root in &mut self.roots {
            // A root is always open. Collapsing one would leave a heading
            // with no way to get back into it.
            root.expanded = true;
            root.children.clear();
            root.loaded = false;
        }
        self.load_recursively(&expanded).await?;
        self.rebuild_visible();
        if let Some(path) = selected {
            self.goto_path(&path);
        }
        Ok(())
    }

    /// Every currently expanded directory, so a refresh can restore them.
    fn expanded_paths(&self) -> Vec<PathBuf> {
        fn walk(node: &Node, out: &mut Vec<PathBuf>) {
            if node.expanded {
                out.push(node.path.clone());
            }
            for child in &node.children {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        for root in &self.roots {
            walk(root, &mut out);
        }
        out
    }

    /// Reads the root and every directory in `expanded`.
    async fn load_recursively(&mut self, expanded: &[PathBuf]) -> Result<()> {
        // Breadth-first, so each directory is read once.
        let mut queue: Vec<PathBuf> = self.roots.iter().map(|r| r.path.clone()).collect();
        while let Some(path) = queue.pop() {
            let children = self.read_dir(&path).await?;
            for child in &children {
                if child.is_expandable() && expanded.contains(&child.path) {
                    queue.push(child.path.clone());
                }
            }
            // Read before the node is borrowed: the map and the tree are
            // both `self`, and only one of them can be held at a time.
            let status = self.git.get(&path).copied();
            let Some(node) = self.find_mut(&path) else {
                continue;
            };
            node.children = children;
            node.loaded = true;
            node.expanded = true;
            // Directories show the strongest status among their descendants.
            node.git = status;
        }
        self.roll_up_git();
        Ok(())
    }

    /// Gives each directory the most significant status beneath it.
    fn roll_up_git(&mut self) {
        fn walk(node: &mut Node) -> Option<GitStatus> {
            if !node.is_expandable() {
                return node.git;
            }
            let mut statuses: Vec<GitStatus> = node.children.iter_mut().filter_map(walk).collect();
            statuses.extend(node.git);
            node.git = GitStatus::rollup(statuses);
            node.git
        }
        for root in &mut self.roots {
            walk(root);
        }
    }

    /// Recomputes the flattened view from the tree.
    fn rebuild_visible(&mut self) {
        fn walk(node: &Node, depth: usize, is_root: bool, out: &mut Vec<VisibleNode>) {
            out.push(VisibleNode {
                path: node.path.clone(),
                name: node.name.clone(),
                kind: node.kind,
                depth,
                expanded: node.expanded,
                expandable: node.is_expandable(),
                git: node.git,
                is_root,
            });
            if node.expanded {
                for child in &node.children {
                    walk(child, depth + 1, false, out);
                }
            }
        }
        let mut out = Vec::new();
        for root in &self.roots {
            walk(root, 0, true, &mut out);
        }
        self.visible = out;
        self.cursor = self.cursor.min(self.visible.len().saturating_sub(1));
    }

    // ---- expansion -----------------------------------------------------

    /// Expands `path`, reading its children if this is the first time.
    pub async fn expand(&mut self, path: &Path) -> Result<()> {
        let needs_read = match self.find(path) {
            Some(node) if node.is_expandable() => !node.loaded,
            // Expanding a file is a no-op rather than an error: `RET` on a file
            // visits it, and the caller decides which happened.
            Some(_) => return Ok(()),
            None => return Ok(()),
        };
        if needs_read {
            let children = self.read_dir(path).await?;
            let Some(node) = self.find_mut(path) else {
                return Ok(());
            };
            node.children = children;
            node.loaded = true;
        }
        if let Some(node) = self.find_mut(path) {
            node.expanded = true;
        }
        self.roll_up_git();
        self.rebuild_visible();
        Ok(())
    }

    /// Collapses `path` and everything under it.
    pub fn collapse(&mut self, path: &Path) {
        // A root stays open; collapsing one would leave a heading with no
        // way back into it.
        if self.is_root(path) {
            return;
        }
        if let Some(node) = self.find_mut(path) {
            node.collapse_recursively();
        }
        self.rebuild_visible();
    }

    /// `treemacs-TAB-action`: expands a collapsed directory, collapses an
    /// expanded one. Returns true when something changed.
    pub async fn toggle(&mut self, path: &Path) -> Result<bool> {
        let Some(node) = self.find(path) else {
            return Ok(false);
        };
        if !node.is_expandable() {
            return Ok(false);
        }
        if node.expanded {
            self.collapse(path);
        } else {
            self.expand(path).await?;
        }
        Ok(true)
    }

    /// Toggles the node under the cursor.
    pub async fn toggle_selected(&mut self) -> Result<bool> {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return Err(TreeError::NoSelection);
        };
        self.toggle(&path).await
    }

    /// Expands every directory beneath `path`, as `treemacs-expand-all` does.
    pub async fn expand_recursively(&mut self, path: &Path) -> Result<()> {
        let mut queue = vec![path.to_path_buf()];
        while let Some(current) = queue.pop() {
            self.expand(&current).await?;
            let Some(node) = self.find(&current) else {
                continue;
            };
            for child in &node.children {
                if child.is_expandable() {
                    queue.push(child.path.clone());
                }
            }
        }
        Ok(())
    }

    /// `treemacs-toggle-show-dotfiles`.
    pub async fn toggle_show_hidden(&mut self) -> Result<()> {
        self.config.show_hidden = !self.config.show_hidden;
        self.refresh().await
    }

    /// `treemacs-set-width`.
    pub fn set_width(&mut self, width: usize) {
        self.config.width = width.max(8);
    }

    pub fn width(&self) -> usize {
        self.config.width
    }

    // ---- navigation ----------------------------------------------------

    /// `treemacs-next-line`.
    pub fn next_line(&mut self, n: usize) {
        self.cursor = (self.cursor + n).min(self.visible.len().saturating_sub(1));
    }

    /// `treemacs-previous-line`.
    pub fn previous_line(&mut self, n: usize) {
        self.cursor = self.cursor.saturating_sub(n);
    }

    pub fn goto_first(&mut self) {
        self.cursor = 0;
    }

    pub fn goto_last(&mut self) {
        self.cursor = self.visible.len().saturating_sub(1);
    }

    /// `treemacs-next-neighbour`: the next sibling at the same depth.
    pub fn next_neighbour(&mut self) -> bool {
        let Some(depth) = self.visible.get(self.cursor).map(|n| n.depth) else {
            return false;
        };
        for i in (self.cursor + 1)..self.visible.len() {
            match self.visible[i].depth.cmp(&depth) {
                std::cmp::Ordering::Equal => {
                    self.cursor = i;
                    return true;
                }
                // A shallower node means the parent ended: no more siblings.
                std::cmp::Ordering::Less => return false,
                std::cmp::Ordering::Greater => {}
            }
        }
        false
    }

    /// `treemacs-previous-neighbour`.
    pub fn previous_neighbour(&mut self) -> bool {
        let Some(depth) = self.visible.get(self.cursor).map(|n| n.depth) else {
            return false;
        };
        for i in (0..self.cursor).rev() {
            match self.visible[i].depth.cmp(&depth) {
                std::cmp::Ordering::Equal => {
                    self.cursor = i;
                    return true;
                }
                std::cmp::Ordering::Less => return false,
                std::cmp::Ordering::Greater => {}
            }
        }
        false
    }

    /// `treemacs-goto-parent-node`.
    pub fn goto_parent(&mut self) -> bool {
        let Some(depth) = self.visible.get(self.cursor).map(|n| n.depth) else {
            return false;
        };
        if depth == 0 {
            return false;
        }
        for i in (0..self.cursor).rev() {
            if self.visible[i].depth < depth {
                self.cursor = i;
                return true;
            }
        }
        false
    }

    /// `treemacs-collapse-parent-node`: moves to the parent and closes it.
    pub fn collapse_parent(&mut self) -> bool {
        if !self.goto_parent() {
            return false;
        }
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return false;
        };
        self.collapse(&path);
        self.goto_path(&path);
        true
    }

    /// Moves the cursor to `path` if it is visible. Returns whether it was.
    pub fn goto_path(&mut self, path: &Path) -> bool {
        match self.visible.iter().position(|n| n.path == path) {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }

    /// `treemacs-follow-mode`: reveals `path`, expanding whatever it takes,
    /// and puts the cursor on it. Returns false when `path` is outside the
    /// tree.
    pub async fn reveal(&mut self, path: &Path) -> Result<bool> {
        // Whichever of the roots holds it — a file is revealed in the
        // directory it is actually under, not in the first one on the list.
        let Some(root) = self.root_of(path).map(|root| root.path.clone()) else {
            return Ok(false);
        };
        // Expand each ancestor from that root down.
        let mut current = root.clone();
        let Ok(relative) = path.strip_prefix(&root) else {
            return Ok(false);
        };
        for component in relative.components() {
            self.expand(&current).await?;
            current = current.join(component);
        }
        self.rebuild_visible();
        Ok(self.goto_path(path))
    }

    // ---- file operations -----------------------------------------------

    /// The directory new entries are created in: the selected node when it is
    /// a directory, otherwise its parent.
    pub fn target_directory(&self) -> Option<PathBuf> {
        let node = self.selected()?;
        if node.expandable {
            Some(node.path.clone())
        } else {
            node.path.parent().map(Path::to_path_buf)
        }
    }

    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err(TreeError::InvalidName(name.to_string()));
        }
        Ok(())
    }

    /// `treemacs-create-file`.
    pub async fn create_file(&mut self, name: &str) -> Result<PathBuf> {
        Self::validate_name(name)?;
        let dir = self.target_directory().ok_or(TreeError::NoSelection)?;
        let path = dir.join(name);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(TreeError::AlreadyExists(path));
        }
        tokio::fs::write(&path, b"")
            .await
            .map_err(|source| TreeError::Io {
                path: path.clone(),
                source,
            })?;
        self.refresh().await?;
        self.goto_path(&path);
        Ok(path)
    }

    /// `treemacs-create-dir`.
    pub async fn create_directory(&mut self, name: &str) -> Result<PathBuf> {
        Self::validate_name(name)?;
        let dir = self.target_directory().ok_or(TreeError::NoSelection)?;
        let path = dir.join(name);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(TreeError::AlreadyExists(path));
        }
        tokio::fs::create_dir(&path)
            .await
            .map_err(|source| TreeError::Io {
                path: path.clone(),
                source,
            })?;
        self.refresh().await?;
        self.goto_path(&path);
        Ok(path)
    }

    /// `treemacs-delete-file`: removes the selected node, recursively for a
    /// directory. The caller is responsible for confirming first.
    pub async fn delete_selected(&mut self) -> Result<PathBuf> {
        let node = self.selected().ok_or(TreeError::NoSelection)?;
        let path = node.path.clone();
        if self.is_root(&path) {
            return Err(TreeError::NoSelection);
        }
        let result = if node.kind == NodeKind::Directory {
            tokio::fs::remove_dir_all(&path).await
        } else {
            tokio::fs::remove_file(&path).await
        };
        result.map_err(|source| TreeError::Io {
            path: path.clone(),
            source,
        })?;
        self.refresh().await?;
        Ok(path)
    }

    /// `treemacs-rename-file`.
    pub async fn rename_selected(&mut self, new_name: &str) -> Result<PathBuf> {
        Self::validate_name(new_name)?;
        let old = self
            .selected_path()
            .map(Path::to_path_buf)
            .ok_or(TreeError::NoSelection)?;
        if self.is_root(&old) {
            return Err(TreeError::NoSelection);
        }
        let parent = old.parent().ok_or(TreeError::NoSelection)?;
        let new = parent.join(new_name);
        if tokio::fs::try_exists(&new).await.unwrap_or(false) {
            return Err(TreeError::AlreadyExists(new));
        }
        tokio::fs::rename(&old, &new)
            .await
            .map_err(|source| TreeError::Io {
                path: old.clone(),
                source,
            })?;
        self.refresh().await?;
        self.goto_path(&new);
        Ok(new)
    }

    /// `treemacs-move-file`: moves the selection into `destination`.
    pub async fn move_selected(&mut self, destination: &Path) -> Result<PathBuf> {
        let old = self
            .selected_path()
            .map(Path::to_path_buf)
            .ok_or(TreeError::NoSelection)?;
        if self.is_root(&old) {
            return Err(TreeError::NoSelection);
        }
        let name = old.file_name().ok_or(TreeError::NoSelection)?;
        let new = destination.join(name);
        if tokio::fs::try_exists(&new).await.unwrap_or(false) {
            return Err(TreeError::AlreadyExists(new));
        }
        tokio::fs::rename(&old, &new)
            .await
            .map_err(|source| TreeError::Io {
                path: old.clone(),
                source,
            })?;
        self.refresh().await?;
        self.goto_path(&new);
        Ok(new)
    }

    /// `treemacs-copy-absolute-path-at-point`.
    pub fn absolute_path(&self) -> Option<String> {
        Some(self.selected()?.path.to_string_lossy().into_owned())
    }

    /// `treemacs-copy-relative-path-at-point`: relative to the tree root.
    pub fn relative_path(&self) -> Option<String> {
        let path = &self.selected()?.path;
        let root = self.root_of(path).map(|root| root.path.as_path());
        let relative = root.and_then(|root| path.strip_prefix(root).ok());
        Some(relative.unwrap_or(path).to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory tree, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        async fn new(tag: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!("maxgus-tree-{tag}"));
            tokio::fs::remove_dir_all(&dir).await.ok();
            tokio::fs::create_dir_all(dir.join("src/inner"))
                .await
                .unwrap();
            tokio::fs::create_dir_all(dir.join("target")).await.unwrap();
            tokio::fs::create_dir_all(dir.join(".hidden"))
                .await
                .unwrap();
            tokio::fs::write(dir.join("Cargo.toml"), "[package]")
                .await
                .unwrap();
            tokio::fs::write(dir.join("README.md"), "# hi")
                .await
                .unwrap();
            tokio::fs::write(dir.join(".gitignore"), "target")
                .await
                .unwrap();
            tokio::fs::write(dir.join("src/main.rs"), "fn main() {}")
                .await
                .unwrap();
            tokio::fs::write(dir.join("src/lib.rs"), "").await.unwrap();
            tokio::fs::write(dir.join("src/inner/deep.rs"), "")
                .await
                .unwrap();
            tokio::fs::write(dir.join("target/artifact"), "")
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

    /// Git is off in tests so they do not depend on the ambient repository.
    fn config() -> TreeConfig {
        TreeConfig {
            git_status: false,
            ..Default::default()
        }
    }

    async fn open(f: &Fixture) -> FileTree {
        FileTree::open(f.path(), config()).await.unwrap()
    }

    fn names(tree: &FileTree) -> Vec<&str> {
        tree.visible().iter().map(|n| n.name.as_str()).collect()
    }

    #[tokio::test]
    async fn opening_a_tree_expands_the_root_one_level() {
        let f = Fixture::new("open").await;
        let tree = open(&f).await;
        // `target` and dotfiles are filtered by the default config.
        assert_eq!(
            names(&tree),
            vec![
                f.path().file_name().unwrap().to_str().unwrap(),
                "src",
                "Cargo.toml",
                "README.md"
            ]
        );
        assert_eq!(tree.cursor(), 0);
        assert!(tree.selected().unwrap().is_root);
    }

    #[tokio::test]
    async fn a_second_directory_can_be_added_and_shows_below_the_first() {
        // A workspace is usually more than one directory. treemacs calls
        // them projects and keeps a list; so does this.
        let one = Fixture::new("roots-one").await;
        let two = Fixture::new("roots-two").await;
        let mut tree = open(&one).await;
        assert_eq!(tree.roots(), vec![one.path()]);

        tree.add_root(two.path()).await.unwrap();
        assert_eq!(tree.roots(), vec![one.path(), two.path()]);

        // Both headings are there, each with its own contents under it.
        let heads: Vec<&str> = tree
            .visible()
            .iter()
            .filter(|n| n.is_root)
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(heads.len(), 2, "got {heads:?}");
        assert!(
            tree.visible().iter().filter(|n| n.name == "src").count() == 2,
            "each root should have brought its own `src`"
        );
    }

    #[tokio::test]
    async fn the_first_directory_stays_the_project_however_many_are_added() {
        // What a language server is told about and what a project search
        // walks. Looking at a second directory is not changing project.
        let one = Fixture::new("roots-project").await;
        let two = Fixture::new("roots-project-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();
        assert_eq!(tree.root_path(), one.path());
    }

    #[tokio::test]
    async fn the_same_directory_is_not_added_twice() {
        // Two copies would expand and collapse independently, which is two
        // answers to the same question.
        let one = Fixture::new("roots-dup").await;
        let mut tree = open(&one).await;
        tree.add_root(one.path()).await.unwrap();
        assert_eq!(tree.roots(), vec![one.path()]);
    }

    #[tokio::test]
    async fn a_directory_that_is_not_one_cannot_be_added() {
        let f = Fixture::new("roots-notdir").await;
        let mut tree = open(&f).await;
        assert!(tree.add_root(f.path().join("README.md")).await.is_err());
        assert!(tree.add_root(f.path().join("nothing-here")).await.is_err());
        assert_eq!(tree.roots(), vec![f.path()], "a failure still changed it");
    }

    #[tokio::test]
    async fn a_directory_can_be_taken_off_the_list_again() {
        let one = Fixture::new("roots-remove").await;
        let two = Fixture::new("roots-remove-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();
        tree.remove_root(two.path()).unwrap();
        assert_eq!(tree.roots(), vec![one.path()]);
        assert!(
            tree.visible()
                .iter()
                .all(|n| n.path.starts_with(one.path())),
            "something from the removed directory is still showing"
        );
    }

    #[tokio::test]
    async fn the_last_directory_cannot_be_taken_off() {
        // A tree with nothing in it has no row to put a cursor on and no
        // way to ask for a directory back.
        let f = Fixture::new("roots-last").await;
        let mut tree = open(&f).await;
        assert!(tree.remove_root(f.path()).is_err());
        assert!(
            tree.remove_root(&f.path().join("src")).is_err(),
            "not a root"
        );
        assert_eq!(tree.roots(), vec![f.path()]);
    }

    #[tokio::test]
    async fn every_root_stays_open() {
        // Collapsing one would leave a heading with no way back into it.
        let one = Fixture::new("roots-collapse").await;
        let two = Fixture::new("roots-collapse-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();
        for root in [one.path(), two.path()] {
            tree.collapse(root);
            let node = tree
                .visible()
                .iter()
                .find(|n| n.path == root)
                .expect("the heading is still there");
            assert!(node.expanded, "{} collapsed", root.display());
        }
    }

    #[tokio::test]
    async fn a_file_is_revealed_in_the_directory_it_is_actually_under() {
        // Not in the first one on the list, which is what a tree that only
        // ever looked at one root would do.
        let one = Fixture::new("roots-reveal").await;
        let two = Fixture::new("roots-reveal-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();

        let deep = two.path().join("src/inner/deep.rs");
        assert!(tree.reveal(&deep).await.unwrap(), "it was not found");
        assert_eq!(tree.selected_path(), Some(deep.as_path()));
    }

    #[tokio::test]
    async fn a_path_under_no_root_is_not_revealed() {
        let f = Fixture::new("roots-outside").await;
        let mut tree = open(&f).await;
        assert!(
            !tree
                .reveal(Path::new("/nowhere-at-all/x.rs"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn a_relative_path_is_relative_to_the_root_that_holds_it() {
        let one = Fixture::new("roots-relative").await;
        let two = Fixture::new("roots-relative-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();
        tree.reveal(&two.path().join("src/main.rs")).await.unwrap();
        assert_eq!(tree.relative_path().as_deref(), Some("src/main.rs"));
    }

    #[tokio::test]
    async fn moving_one_root_leaves_the_others_where_they_were() {
        // `root-down` says it moves *the* root. Rebuilding the whole tree
        // around the new one — which is what it used to do — would take
        // away every other directory somebody had asked to see.
        let one = Fixture::new("roots-replace").await;
        let two = Fixture::new("roots-replace-2").await;
        let mut tree = open(&one).await;
        tree.add_root(two.path()).await.unwrap();

        tree.replace_root(one.path(), one.path().join("src"))
            .await
            .unwrap();
        assert_eq!(
            tree.roots(),
            vec![one.path().join("src").as_path(), two.path()],
            "the other directory did not survive"
        );
    }

    #[tokio::test]
    async fn a_root_that_is_not_one_cannot_be_moved() {
        let f = Fixture::new("roots-replace-bad").await;
        let mut tree = open(&f).await;
        assert!(
            tree.replace_root(&f.path().join("src"), f.path())
                .await
                .is_err(),
            "`src` is not one of the roots"
        );
        assert!(
            tree.replace_root(f.path(), f.path().join("README.md"))
                .await
                .is_err(),
            "a file cannot be a root"
        );
        assert_eq!(tree.roots(), vec![f.path()]);
    }

    #[tokio::test]
    async fn opening_a_file_instead_of_a_directory_is_an_error() {
        let f = Fixture::new("notdir").await;
        let err = FileTree::open(f.path().join("README.md"), config())
            .await
            .unwrap_err();
        assert!(matches!(err, TreeError::NotADirectory(_)));
    }

    #[tokio::test]
    async fn opening_a_missing_directory_is_an_error() {
        let err = FileTree::open("/nonexistent-maxgus-tree-path", config())
            .await
            .unwrap_err();
        assert!(matches!(err, TreeError::Io { .. }));
    }

    #[tokio::test]
    async fn directories_sort_before_files() {
        let f = Fixture::new("sort").await;
        let tree = open(&f).await;
        let listed = names(&tree);
        let src = listed.iter().position(|n| *n == "src").unwrap();
        let cargo = listed.iter().position(|n| *n == "Cargo.toml").unwrap();
        assert!(src < cargo);
    }

    #[tokio::test]
    async fn ignored_directories_and_dotfiles_are_filtered() {
        let f = Fixture::new("filter").await;
        let tree = open(&f).await;
        assert!(!names(&tree).contains(&"target"), "ignore list applied");
        assert!(!names(&tree).contains(&".gitignore"), "dotfiles hidden");
    }

    #[tokio::test]
    async fn toggling_show_hidden_reveals_dotfiles() {
        let f = Fixture::new("hidden").await;
        let mut tree = open(&f).await;
        tree.toggle_show_hidden().await.unwrap();
        assert!(names(&tree).contains(&".gitignore"));
        assert!(names(&tree).contains(&".hidden"));
        assert!(
            !names(&tree).contains(&"target"),
            "the ignore list still applies"
        );
        tree.toggle_show_hidden().await.unwrap();
        assert!(!names(&tree).contains(&".gitignore"));
    }

    #[tokio::test]
    async fn expanding_a_directory_reads_it_lazily() {
        let f = Fixture::new("expand").await;
        let mut tree = open(&f).await;
        assert!(!names(&tree).contains(&"main.rs"));
        tree.expand(&f.path().join("src")).await.unwrap();
        assert!(names(&tree).contains(&"main.rs"));
        assert!(names(&tree).contains(&"inner"));
        assert!(!names(&tree).contains(&"deep.rs"), "one level only");
    }

    #[tokio::test]
    async fn collapsing_hides_the_whole_subtree() {
        let f = Fixture::new("collapse").await;
        let mut tree = open(&f).await;
        tree.expand_recursively(f.path()).await.unwrap();
        assert!(names(&tree).contains(&"deep.rs"));
        tree.collapse(&f.path().join("src"));
        assert!(!names(&tree).contains(&"main.rs"));
        // Re-expanding does not restore the inner expansion.
        tree.expand(&f.path().join("src")).await.unwrap();
        assert!(names(&tree).contains(&"inner"));
        assert!(!names(&tree).contains(&"deep.rs"));
    }

    #[tokio::test]
    async fn the_root_cannot_be_collapsed() {
        let f = Fixture::new("rootcollapse").await;
        let mut tree = open(&f).await;
        let before = tree.len();
        tree.collapse(f.path());
        assert_eq!(tree.len(), before);
    }

    #[tokio::test]
    async fn toggle_expands_then_collapses() {
        let f = Fixture::new("toggle").await;
        let mut tree = open(&f).await;
        let src = f.path().join("src");
        assert!(tree.toggle(&src).await.unwrap());
        assert!(names(&tree).contains(&"main.rs"));
        assert!(tree.toggle(&src).await.unwrap());
        assert!(!names(&tree).contains(&"main.rs"));
        // A file cannot be toggled.
        assert!(!tree.toggle(&f.path().join("README.md")).await.unwrap());
    }

    #[tokio::test]
    async fn toggling_the_selection_needs_a_selection() {
        let f = Fixture::new("togglesel").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("src"));
        assert!(tree.toggle_selected().await.unwrap());
        assert!(names(&tree).contains(&"main.rs"));
    }

    #[tokio::test]
    async fn expand_recursively_opens_everything() {
        let f = Fixture::new("recursive").await;
        let mut tree = open(&f).await;
        tree.expand_recursively(f.path()).await.unwrap();
        assert!(names(&tree).contains(&"deep.rs"));
    }

    #[tokio::test]
    async fn line_navigation_clamps_at_both_ends() {
        let f = Fixture::new("nav").await;
        let mut tree = open(&f).await;
        tree.previous_line(5);
        assert_eq!(tree.cursor(), 0);
        tree.next_line(100);
        assert_eq!(tree.cursor(), tree.len() - 1);
        tree.goto_first();
        assert_eq!(tree.cursor(), 0);
        tree.goto_last();
        assert_eq!(tree.cursor(), tree.len() - 1);
    }

    #[tokio::test]
    async fn neighbour_navigation_stays_at_one_depth() {
        let f = Fixture::new("neighbour").await;
        let mut tree = open(&f).await;
        tree.expand(&f.path().join("src")).await.unwrap();
        // Depth 1: src, Cargo.toml, README.md.
        assert!(tree.goto_path(&f.path().join("src")));
        assert!(tree.next_neighbour());
        assert_eq!(
            tree.selected().unwrap().name,
            "Cargo.toml",
            "skips src's children"
        );
        assert!(tree.next_neighbour());
        assert_eq!(tree.selected().unwrap().name, "README.md");
        assert!(!tree.next_neighbour(), "no sibling after the last one");
        assert!(tree.previous_neighbour());
        assert_eq!(tree.selected().unwrap().name, "Cargo.toml");
    }

    #[tokio::test]
    async fn goto_parent_walks_up_a_level() {
        let f = Fixture::new("parent").await;
        let mut tree = open(&f).await;
        tree.expand_recursively(f.path()).await.unwrap();
        assert!(tree.goto_path(&f.path().join("src/inner/deep.rs")));
        assert!(tree.goto_parent());
        assert_eq!(tree.selected().unwrap().name, "inner");
        assert!(tree.goto_parent());
        assert_eq!(tree.selected().unwrap().name, "src");
        assert!(tree.goto_parent());
        assert!(tree.selected().unwrap().is_root);
        assert!(!tree.goto_parent(), "the root has no parent");
    }

    #[tokio::test]
    async fn collapse_parent_closes_the_enclosing_directory() {
        let f = Fixture::new("collapseparent").await;
        let mut tree = open(&f).await;
        tree.expand(&f.path().join("src")).await.unwrap();
        assert!(tree.goto_path(&f.path().join("src/main.rs")));
        assert!(tree.collapse_parent());
        assert_eq!(tree.selected().unwrap().name, "src");
        assert!(!tree.selected().unwrap().expanded);
        assert!(!names(&tree).contains(&"main.rs"));
    }

    #[tokio::test]
    async fn reveal_expands_the_path_to_a_deep_file() {
        let f = Fixture::new("reveal").await;
        let mut tree = open(&f).await;
        let deep = f.path().join("src/inner/deep.rs");
        assert!(tree.reveal(&deep).await.unwrap());
        assert_eq!(tree.selected_path().unwrap(), deep);
    }

    #[tokio::test]
    async fn revealing_a_path_outside_the_tree_fails_cleanly() {
        let f = Fixture::new("revealout").await;
        let mut tree = open(&f).await;
        assert!(!tree.reveal(Path::new("/etc/hosts")).await.unwrap());
    }

    #[tokio::test]
    async fn creating_a_file_puts_it_in_the_right_directory() {
        let f = Fixture::new("create").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("src"));
        tree.expand(&f.path().join("src")).await.unwrap();
        tree.goto_path(&f.path().join("src"));
        let created = tree.create_file("new.rs").await.unwrap();
        assert_eq!(created, f.path().join("src/new.rs"));
        assert!(tokio::fs::try_exists(&created).await.unwrap());
        assert_eq!(tree.selected_path().unwrap(), created);
    }

    #[tokio::test]
    async fn creating_from_a_file_selection_uses_its_parent() {
        let f = Fixture::new("createsibling").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("README.md"));
        let created = tree.create_file("SIBLING.md").await.unwrap();
        assert_eq!(created, f.path().join("SIBLING.md"));
    }

    #[tokio::test]
    async fn creating_a_directory_works_and_refuses_duplicates() {
        let f = Fixture::new("createdir").await;
        let mut tree = open(&f).await;
        let created = tree.create_directory("docs").await.unwrap();
        assert!(tokio::fs::metadata(&created).await.unwrap().is_dir());
        // Creating leaves the cursor on the new directory, so a second `docs`
        // there would nest. Go back to the root to hit the duplicate.
        tree.goto_first();
        assert!(matches!(
            tree.create_directory("docs").await,
            Err(TreeError::AlreadyExists(_))
        ));
    }

    #[tokio::test]
    async fn invalid_names_are_rejected() {
        let f = Fixture::new("badname").await;
        let mut tree = open(&f).await;
        for name in ["", "a/b", ".", ".."] {
            assert!(
                matches!(tree.create_file(name).await, Err(TreeError::InvalidName(_))),
                "`{name}` should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn deleting_removes_a_file_and_a_directory_tree() {
        let f = Fixture::new("delete").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("README.md"));
        tree.delete_selected().await.unwrap();
        assert!(
            !tokio::fs::try_exists(f.path().join("README.md"))
                .await
                .unwrap()
        );
        assert!(!names(&tree).contains(&"README.md"));

        tree.goto_path(&f.path().join("src"));
        tree.delete_selected().await.unwrap();
        assert!(!tokio::fs::try_exists(f.path().join("src")).await.unwrap());
    }

    #[tokio::test]
    async fn the_root_cannot_be_deleted_or_renamed() {
        let f = Fixture::new("rootops").await;
        let mut tree = open(&f).await;
        tree.goto_first();
        assert!(matches!(
            tree.delete_selected().await,
            Err(TreeError::NoSelection)
        ));
        assert!(matches!(
            tree.rename_selected("x").await,
            Err(TreeError::NoSelection)
        ));
    }

    #[tokio::test]
    async fn renaming_moves_the_cursor_to_the_new_name() {
        let f = Fixture::new("rename").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("README.md"));
        let new = tree.rename_selected("GUIDE.md").await.unwrap();
        assert_eq!(new, f.path().join("GUIDE.md"));
        assert_eq!(tree.selected_path().unwrap(), new);
        assert!(
            !tokio::fs::try_exists(f.path().join("README.md"))
                .await
                .unwrap()
        );
        // Renaming onto an existing name is refused.
        assert!(matches!(
            tree.rename_selected("Cargo.toml").await,
            Err(TreeError::AlreadyExists(_))
        ));
    }

    #[tokio::test]
    async fn moving_relocates_into_another_directory() {
        let f = Fixture::new("move").await;
        let mut tree = open(&f).await;
        tree.goto_path(&f.path().join("README.md"));
        let moved = tree.move_selected(&f.path().join("src")).await.unwrap();
        assert_eq!(moved, f.path().join("src/README.md"));
        assert!(tokio::fs::try_exists(&moved).await.unwrap());
    }

    #[tokio::test]
    async fn paths_are_available_absolute_and_relative() {
        let f = Fixture::new("paths").await;
        let mut tree = open(&f).await;
        tree.expand(&f.path().join("src")).await.unwrap();
        tree.goto_path(&f.path().join("src/main.rs"));
        assert_eq!(tree.relative_path().unwrap(), "src/main.rs");
        assert_eq!(
            tree.absolute_path().unwrap(),
            f.path().join("src/main.rs").to_string_lossy()
        );
    }

    #[tokio::test]
    async fn refreshing_picks_up_changes_made_outside_the_editor() {
        let f = Fixture::new("refresh").await;
        let mut tree = open(&f).await;
        tokio::fs::write(f.path().join("EXTERNAL.md"), "")
            .await
            .unwrap();
        assert!(!names(&tree).contains(&"EXTERNAL.md"));
        tree.refresh().await.unwrap();
        assert!(names(&tree).contains(&"EXTERNAL.md"));
    }

    #[tokio::test]
    async fn refreshing_preserves_expansion_and_the_cursor() {
        let f = Fixture::new("refreshstate").await;
        let mut tree = open(&f).await;
        tree.expand_recursively(f.path()).await.unwrap();
        let target = f.path().join("src/inner/deep.rs");
        tree.goto_path(&target);
        tree.refresh().await.unwrap();
        assert!(names(&tree).contains(&"deep.rs"), "expansion survived");
        assert_eq!(tree.selected_path().unwrap(), target, "cursor survived");
    }

    #[tokio::test]
    async fn rendering_a_line_shows_indentation_and_the_arrow() {
        let f = Fixture::new("render").await;
        let mut tree = open(&f).await;
        tree.expand(&f.path().join("src")).await.unwrap();
        let src = tree.visible().iter().find(|n| n.name == "src").unwrap();
        assert_eq!(src.render(), "  v src");
        assert_eq!(src.face(), "tree-directory");
        let main = tree.visible().iter().find(|n| n.name == "main.rs").unwrap();
        assert_eq!(main.render(), "      main.rs");
        assert_eq!(main.face(), "tree-file");
        assert_eq!(tree.visible()[0].face(), "tree-root");
    }

    #[tokio::test]
    async fn the_width_has_a_floor() {
        let f = Fixture::new("width").await;
        let mut tree = open(&f).await;
        tree.set_width(40);
        assert_eq!(tree.width(), 40);
        tree.set_width(1);
        assert_eq!(tree.width(), 8);
    }

    #[tokio::test]
    async fn a_git_indicator_is_rendered_when_present() {
        let node = VisibleNode {
            path: PathBuf::from("/a/b.rs"),
            name: "b.rs".into(),
            kind: NodeKind::File,
            depth: 1,
            expanded: false,
            expandable: false,
            git: Some(GitStatus::Modified),
            is_root: false,
        };
        assert_eq!(node.render(), "    b.rs M");
    }
}
