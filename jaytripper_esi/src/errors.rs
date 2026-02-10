use std::fmt;

use jaytripper_core::ids::CharacterId;
use thiserror::Error;

pub type EsiResult<T> = Result<T, EsiError>;

#[derive(Debug, Error)]
pub enum EsiError {
    #[error("invalid config: {0}")]
    InvalidConfig(&'static str),
    #[error("esi operation failed")]
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
    #[error("esi did not provide an access token")]
    MissingAccessToken,
    #[error("esi did not provide access token expiration")]
    MissingAccessExpiration,
    #[error("esi did not provide a refresh token")]
    MissingRefreshToken,
    #[error("invalid token subject format: {0}")]
    InvalidTokenSubject(String),
    #[error("invalid token scope claim format: {0}")]
    InvalidScopeClaim(String),
    #[error("character id {0:?} does not fit ESI integer bounds")]
    InvalidCharacterId(CharacterId),
    #[error("missing required scopes: {missing:?}")]
    MissingRequiredScopes { missing: Vec<String> },
    #[error("reauthentication required: {reason}")]
    NeedsReauth { reason: String },
    #[error("{0}")]
    Message(String),
}

impl EsiError {
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }

    pub fn display_chain(&self) -> DisplayChainedError<'_> {
        DisplayChainedError { inner: self }
    }
}

pub struct DisplayChainedError<'a> {
    inner: &'a (dyn std::error::Error + 'static),
}

impl fmt::Debug for DisplayChainedError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(self.inner);

        while let Some(err) = current {
            if first {
                first = false;
            } else {
                write!(f, " -> ")?;
            }

            write!(f, "{err}")?;
            current = err.source();
        }

        Ok(())
    }
}

impl fmt::Display for DisplayChainedError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
