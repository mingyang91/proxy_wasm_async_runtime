pub(crate) mod radix_tree;
pub(crate) mod trie;
pub mod config;

/// A possible error value occurred when adding a route.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum RouteError {
    /// Invalid path
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// Duplicate path
    #[error("duplicate path: {0}")]
    Duplicate(String),

    /// Invalid regex in path
    #[error("invalid regex in path: {path}")]
    InvalidRegex {
        /// Path
        path: String,

        /// Regex
        regex: String,
    },
}