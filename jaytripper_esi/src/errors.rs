use thiserror::Error;

pub type EsiResult<T> = Result<T, EsiError>;

#[derive(Debug, Error)]
pub enum EsiError {
    #[error("invalid config: {0}")]
    InvalidConfig(&'static str),
    #[error("system clock error")]
    SystemClock(#[from] std::time::SystemTimeError),
    #[error("rfesi operation failed")]
    Rfesi(#[from] rfesi::prelude::EsiError),
    #[error("keyring operation failed")]
    Keyring(#[from] keyring::Error),
    #[error("session serialization failed")]
    SessionSerialization(#[from] serde_json::Error),
    #[error("login was not started before code exchange")]
    LoginNotStarted,
    #[error("state mismatch: expected {expected}, got {got}")]
    StateMismatch { expected: String, got: String },
    #[error("token claims are missing from the authentication response")]
    MissingClaims,
    #[error("rfesi did not provide an access token")]
    MissingAccessToken,
    #[error("rfesi did not provide access token expiration")]
    MissingAccessExpiration,
    #[error("rfesi did not provide a refresh token")]
    MissingRefreshToken,
    #[error("invalid token subject format: {0}")]
    InvalidTokenSubject(String),
    #[error("invalid token scope claim format: {0}")]
    InvalidScopeClaim(String),
    #[error("missing required scopes: {missing:?}")]
    MissingRequiredScopes { missing: Vec<String> },
    #[error("{0}")]
    Message(String),
}

impl EsiError {
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}
