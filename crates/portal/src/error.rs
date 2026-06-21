//! Store-level errors and their mapping onto ConnectRPC error codes.

use connectrpc::ConnectError;

/// Errors produced by the resource stores.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The requested resource does not exist.
    #[error("{0} not found")]
    NotFound(String),

    /// A resource with the same identity already exists.
    #[error("{0} already exists")]
    AlreadyExists(String),

    /// The request was malformed (missing/invalid fields).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

impl From<StoreError> for ConnectError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::NotFound(msg) => ConnectError::not_found(msg),
            StoreError::AlreadyExists(msg) => ConnectError::already_exists(msg),
            StoreError::InvalidArgument(msg) => ConnectError::invalid_argument(msg),
        }
    }
}

/// Convenience alias for store operations.
pub type StoreResult<T> = Result<T, StoreError>;
