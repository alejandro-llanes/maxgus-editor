//! The file tree.
//!
//! A side window listing the project, modelled on treemacs: a lazily expanded
//! tree, git status decoration, and the same keymap. Directory reads and git
//! invocations run on tokio, so a slow filesystem never blocks redisplay.

pub mod git;
pub mod keymap;
pub mod node;
pub mod tree;

pub use git::{GitStatus, git_status};
pub use keymap::{TREEMACS_BINDINGS, treemacs_keymap};
pub use node::{Node, NodeKind};
pub use tree::{FileTree, VisibleNode};

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error("io error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a directory")]
    NotADirectory(std::path::PathBuf),
    #[error("{0} already exists")]
    AlreadyExists(std::path::PathBuf),
    #[error("no node is selected")]
    NoSelection,
    #[error("{0} is not shown in the tree")]
    NotInTree(std::path::PathBuf),
    #[error("`{0}` is not a valid file name")]
    InvalidName(String),
}

pub type Result<T> = std::result::Result<T, TreeError>;
