//! Error types for the wiki server.
//!
//! The MCP surface distinguishes *tool* errors, which come back as a readable
//! `is_error` result the agent can act on, from *startup* errors, which abort
//! the process. Both live here so the two paths never get confused.

use std::path::PathBuf;

use crate::config::MAX_QUERY_LENGTH;

/// A failure that prevents the server from starting or running.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The `--wiki-path` did not resolve to a directory.
    #[error("wiki pages directory not found: {0}")]
    PagesNotFound(PathBuf),

    /// A directory was given that is not usable as a pages root.
    #[error("wiki pages path is not a directory: {0}")]
    NotADirectory(PathBuf),

    /// The configured host or port could not be bound.
    #[error("failed to bind {host}:{port}: {source}")]
    Bind {
        /// Host that was requested.
        host: String,
        /// Port that was requested.
        port: u16,
        /// Underlying I/O failure.
        source: std::io::Error,
    },

    /// The HTTP server stopped with an error.
    #[error("http server failed: {0}")]
    Serve(#[source] std::io::Error),

    /// The stdio transport failed to start or ended in error.
    #[error("stdio transport failed: {0}")]
    Transport(String),

    /// The stdio transport's worker task ended unexpectedly.
    #[error("stdio server task failed: {0}")]
    StdioTask(#[from] tokio::task::JoinError),

    /// Logging initialization failed.
    #[error("failed to initialize logging: {0}")]
    Logging(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A failure inside one tool call, reported to the caller as tool output.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The pages directory disappeared or is unreadable.
    #[error("Wiki pages directory not found")]
    PagesNotFound,

    /// A required argument was missing or empty.
    #[error("Error: {0} is required")]
    MissingArgument(&'static str),

    /// An argument had the wrong JSON type.
    #[error("Error: {0} must be a string")]
    NotAString(&'static str),

    /// The query was longer than the configured cap.
    #[error("Error: query exceeds maximum length of {MAX_QUERY_LENGTH} characters")]
    QueryTooLong,

    /// The named page is absent or resolves outside the pages root.
    #[error("Error: page not found or path outside wiki directory: {0}")]
    PageNotFound(String),

    /// The requested tool does not exist.
    #[error("Error: Unknown tool {0}")]
    UnknownTool(String),

    /// Anything unexpected, matching the reference implementation's catch-all.
    #[error("Error: Wiki operation failed")]
    OperationFailed,
}

impl From<std::io::Error> for ToolError {
    fn from(_: std::io::Error) -> Self {
        ToolError::OperationFailed
    }
}

/// Result alias for tool calls.
pub type ToolResult<T> = std::result::Result<T, ToolError>;
