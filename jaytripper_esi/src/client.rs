use rfesi::prelude::{Esi, EsiBuilder, PkceVerifier, TokenClaims};
use serde::Deserialize;

use crate::{EsiError, EsiResult, config::EsiConfig, types::LoginRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialAuthTokens {
    pub character_id: u64,
    pub character_name: Option<String>,
    pub scopes: Vec<String>,
    pub access_token: String,
    pub access_expires_at_epoch_secs: i64,
    pub refresh_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshTokens {
    pub access_token: String,
    pub access_expires_at_epoch_secs: i64,
    pub refresh_token: String,
}

#[allow(async_fn_in_trait)]
pub trait SsoClient {
    fn begin_login(&mut self) -> EsiResult<LoginRequest>;
    async fn exchange_code(
        &mut self,
        code: &str,
        callback_state: &str,
    ) -> EsiResult<InitialAuthTokens>;
    async fn refresh(&mut self, refresh_token: &str) -> EsiResult<RefreshTokens>;
}

#[derive(Debug)]
struct PendingPkceState {
    state: String,
    verifier: Option<PkceVerifier>,
}

pub struct RfesiSsoClient {
    esi: Esi,
    pending: Option<PendingPkceState>,
}

impl RfesiSsoClient {
    pub fn new(config: &EsiConfig) -> EsiResult<Self> {
        config.validate()?;

        let esi = EsiBuilder::new()
            .user_agent(&config.user_agent)
            .client_id(&config.client_id)
            .callback_url(&config.callback_url)
            .enable_application_authentication(true)
            .scope(&config.scopes_for_esi())
            .build()?;

        Ok(Self { esi, pending: None })
    }

    fn read_access_expiry_secs(&self) -> EsiResult<i64> {
        let expiry_ms = self
            .esi
            .access_expiration
            .ok_or(EsiError::MissingAccessExpiration)?;
        Ok(expiry_ms / 1_000)
    }

    fn read_access_token(&self) -> EsiResult<String> {
        self.esi
            .access_token
            .clone()
            .ok_or(EsiError::MissingAccessToken)
    }

    fn read_refresh_token(&self) -> EsiResult<String> {
        self.esi
            .refresh_token
            .clone()
            .ok_or(EsiError::MissingRefreshToken)
    }
}

impl SsoClient for RfesiSsoClient {
    fn begin_login(&mut self) -> EsiResult<LoginRequest> {
        let auth_info = self.esi.get_authorize_url()?;

        self.pending = Some(PendingPkceState {
            state: auth_info.state.clone(),
            verifier: auth_info.pkce_verifier,
        });

        Ok(LoginRequest {
            authorization_url: auth_info.authorization_url,
            state: auth_info.state,
        })
    }

    async fn exchange_code(
        &mut self,
        code: &str,
        callback_state: &str,
    ) -> EsiResult<InitialAuthTokens> {
        let pending = self.pending.take().ok_or(EsiError::LoginNotStarted)?;

        if callback_state != pending.state {
            return Err(EsiError::StateMismatch {
                expected: pending.state,
                got: callback_state.to_string(),
            });
        }

        let claims = self
            .esi
            .authenticate(code, pending.verifier)
            .await?
            .ok_or(EsiError::MissingClaims)?;

        let character_id = parse_character_id(&claims)?;
        let scopes = parse_scopes(&claims)?;

        Ok(InitialAuthTokens {
            character_id,
            character_name: Some(claims.name),
            scopes,
            access_token: self.read_access_token()?,
            access_expires_at_epoch_secs: self.read_access_expiry_secs()?,
            refresh_token: self.read_refresh_token()?,
        })
    }

    async fn refresh(&mut self, refresh_token: &str) -> EsiResult<RefreshTokens> {
        self.esi.refresh_access_token(Some(refresh_token)).await?;

        Ok(RefreshTokens {
            access_token: self.read_access_token()?,
            access_expires_at_epoch_secs: self.read_access_expiry_secs()?,
            refresh_token: self.read_refresh_token()?,
        })
    }
}

fn parse_character_id(claims: &TokenClaims) -> EsiResult<u64> {
    let parts: Vec<&str> = claims.sub.split(':').collect();
    if parts.len() != 3 || parts[0] != "CHARACTER" || parts[1] != "EVE" {
        return Err(EsiError::InvalidTokenSubject(claims.sub.clone()));
    }

    parts[2]
        .parse::<u64>()
        .map_err(|_| EsiError::InvalidTokenSubject(claims.sub.clone()))
}

fn parse_scopes(claims: &TokenClaims) -> EsiResult<Vec<String>> {
    match claims.scp.clone() {
        None => Ok(Vec::new()),
        Some(value) => {
            let parsed: ScopeClaim = serde_json::from_value(value.clone())
                .map_err(|_| EsiError::InvalidScopeClaim(value.to_string()))?;
            Ok(match parsed {
                ScopeClaim::One(single) => vec![single],
                ScopeClaim::Many(many) => many,
            })
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ScopeClaim {
    One(String),
    Many(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::{parse_character_id, parse_scopes};
    use rfesi::prelude::TokenClaims;
    use serde_json::json;

    #[test]
    fn parses_character_id_from_subject() {
        let claims = mock_claims("CHARACTER:EVE:123456789", json!(["a", "b"]));
        let character_id = parse_character_id(&claims).expect("valid character id");
        assert_eq!(character_id, 123456789);
    }

    #[test]
    fn scopes_are_read_from_array() {
        let claims = mock_claims("CHARACTER:EVE:1", json!(["s1", "s2"]));
        let scopes = parse_scopes(&claims).expect("scope parsing should succeed");
        assert_eq!(scopes, vec!["s1", "s2"]);
    }

    #[test]
    fn invalid_scope_format_is_rejected() {
        let claims = mock_claims("CHARACTER:EVE:1", json!(["s1", 42]));
        assert!(parse_scopes(&claims).is_err());
    }

    #[test]
    fn invalid_subject_is_rejected() {
        let claims = mock_claims("BAD:FORMAT", json!(null));
        assert!(parse_character_id(&claims).is_err());
    }

    fn mock_claims(sub: &str, scp: serde_json::Value) -> TokenClaims {
        TokenClaims {
            aud: vec!["client".to_string(), "EVE Online".to_string()],
            azp: "client".to_string(),
            exp: 1000,
            iat: 100,
            iss: "https://login.eveonline.com".to_string(),
            jti: "jti".to_string(),
            kid: "kid".to_string(),
            name: "Pilot".to_string(),
            owner: "owner".to_string(),
            region: "world".to_string(),
            scp: Some(scp),
            sub: sub.to_string(),
            tenant: "tranquility".to_string(),
            tier: "live".to_string(),
        }
    }
}
