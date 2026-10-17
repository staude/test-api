pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors of the service layer. Messages are user-facing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Anmeldung erforderlich")]
    Unauthenticated,
    #[error("E-Mail oder Passwort falsch")]
    InvalidCredentials,
    #[error("Keine Berechtigung: {0}")]
    Forbidden(String),
    #[error("Nicht gefunden")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Zu viele Anmeldeversuche, bitte später erneut versuchen")]
    RateLimited,
    #[error("Interner Fehler")]
    Internal(String),
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        Error::Internal(err.to_string())
    }
}

pub(crate) fn validation(msg: impl Into<String>) -> Error {
    Error::Validation(msg.into())
}
