//! Configuration: parsing `config.kdl` into the values the editor consumes.
//!
//! The language is KDL (see `docs/configuration.md`). Parsing is deliberately
//! forgiving: a node the running version does not understand produces a
//! [`Warning`] carrying a line number rather than aborting the load, so a
//! config written for a newer `maxgus` still starts an older one.

pub mod error;
pub mod parse;
pub mod settings;
pub mod spec;

pub use error::{ConfigError, Warning};
pub use parse::Config;
pub use settings::Settings;
pub use spec::{FaceSpec, KeymapSpec, LspSpec, ThemeSpec, TreeConfig};

pub type Result<T> = std::result::Result<T, ConfigError>;
