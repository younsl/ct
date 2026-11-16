//! One error type for both backends, classified the way the store needs it.

/// A database failure. The first two variants are the classes callers branch
/// on; everything else carries the driver error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A single-row query matched nothing.
    #[error("query returned no rows")]
    QueryReturnedNoRows,
    /// A constraint rejected the statement. `unique` is set for unique and
    /// primary key violations.
    #[error("constraint violation: {message}")]
    Constraint { unique: bool, message: String },
    /// A column held a value the requested Rust type cannot represent.
    #[error("column conversion: {0}")]
    Conversion(String),
    /// The operation exists on one backend only.
    #[error("{0} is not supported on this database")]
    Unsupported(&'static str),
    #[error(transparent)]
    Sqlite(rusqlite::Error),
    #[error(transparent)]
    Postgres(tokio_postgres::Error),
    /// Connection setup failed before a driver error existed.
    #[error("{0}")]
    Connect(String),
}

/// Result alias for facade calls.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// True for unique and primary key violations.
    pub const fn is_unique_violation(&self) -> bool {
        matches!(self, Self::Constraint { unique: true, .. })
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => Self::QueryReturnedNoRows,
            rusqlite::Error::SqliteFailure(f, ref message)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Self::Constraint {
                    unique: matches!(
                        f.extended_code,
                        rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                            | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                    ),
                    message: message.clone().unwrap_or_else(|| f.to_string()),
                }
            }
            other => Self::Sqlite(other),
        }
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(e: tokio_postgres::Error) -> Self {
        use tokio_postgres::error::SqlState;
        let Some(db) = e.as_db_error() else {
            return Self::Postgres(e);
        };
        // Class 23 is integrity constraint violation.
        if db.code().code().starts_with("23") {
            return Self::Constraint {
                unique: *db.code() == SqlState::UNIQUE_VIOLATION,
                message: db.message().to_string(),
            };
        }
        Self::Postgres(e)
    }
}

/// Turns a missing row into `None`, like rusqlite's extension of the same name.
pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
