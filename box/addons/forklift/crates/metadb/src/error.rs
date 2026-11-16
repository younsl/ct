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
    #[error("{}", postgres_message(.0))]
    Postgres(#[source] tokio_postgres::Error),
    /// Connection setup failed before a driver error existed.
    #[error("{0}")]
    Connect(String),
}

/// The driver prints a server error as "db error" and keeps the reason in its
/// source. An operator needs the reason, so it is spelled out here.
fn postgres_message(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} ({})", db.message(), db.code().code()),
        None => match std::error::Error::source(e) {
            Some(cause) => format!("{e}: {cause}"),
            None => e.to_string(),
        },
    }
}

/// Result alias for facade calls.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// True for unique and primary key violations.
    pub const fn is_unique_violation(&self) -> bool {
        matches!(self, Self::Constraint { unique: true, .. })
    }

    /// Why a connection attempt cannot succeed on a retry, or `None` when the
    /// failure may pass by itself (server still starting, network not up).
    pub fn permanent_connect_failure(&self) -> Option<&'static str> {
        let Self::Postgres(e) = self else {
            return None;
        };
        if let Some(db) = e.as_db_error() {
            return permanent_sqlstate(db.code().code());
        }
        rejected_certificate(e).then_some("server certificate rejected")
    }
}

fn permanent_sqlstate(code: &str) -> Option<&'static str> {
    match code {
        // Class 28 is invalid authorization: wrong password, unknown role, or
        // a pg_hba rule refusing this user, database or encryption.
        c if c.starts_with("28") => Some("authentication failed"),
        "3D000" => Some("database does not exist"),
        _ => None,
    }
}

/// True when the TLS handshake failed on the server certificate. The TLS
/// layer reports it as an I/O error wrapping the rustls one.
fn rejected_certificate(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(e);
    while let Some(err) = current {
        let tls = err.downcast_ref::<rustls::Error>().or_else(|| {
            err.downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::get_ref)
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        });
        if matches!(tls, Some(rustls::Error::InvalidCertificate(_))) {
            return true;
        }
        current = err.source();
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_sqlstates_are_login_and_database_errors() {
        assert_eq!(permanent_sqlstate("28P01"), Some("authentication failed"));
        assert_eq!(permanent_sqlstate("28000"), Some("authentication failed"));
        assert_eq!(permanent_sqlstate("3D000"), Some("database does not exist"));
        // 57P03: the database system is starting up.
        assert_eq!(permanent_sqlstate("57P03"), None);
        assert_eq!(permanent_sqlstate("53300"), None);
    }

    #[test]
    fn rejected_certificate_finds_the_rustls_error_in_the_chain() {
        let bad = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        let wrapped = std::io::Error::new(std::io::ErrorKind::InvalidData, bad.clone());
        assert!(rejected_certificate(&bad));
        assert!(rejected_certificate(&wrapped));
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert!(!rejected_certificate(&refused));
        assert!(!rejected_certificate(&rustls::Error::HandshakeNotComplete));
    }

    #[test]
    fn only_driver_errors_can_be_permanent_connect_failures() {
        assert_eq!(Error::Connect("x".into()).permanent_connect_failure(), None);
        assert_eq!(Error::QueryReturnedNoRows.permanent_connect_failure(), None);
    }
}
