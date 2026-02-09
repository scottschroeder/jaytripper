use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jaytripper_core::ids::CharacterId;

use super::types::{AuthSession, LoginRequest};
use crate::{
    EsiError, EsiResult, client::SsoAuthClient, esi_client::ManagedEsiClient,
    token_store::TokenStore,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnsureSessionResult {
    Missing,
    Ready(AuthSession),
    NeedsReauth { reason: String },
}

pub trait Clock {
    fn now_epoch_secs(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch_secs(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(now) => now.as_secs() as i64,
            Err(_) => 0,
        }
    }
}

pub struct AuthService<C, S, T = SystemClock>
where
    C: SsoAuthClient,
    S: TokenStore,
    T: Clock,
{
    client: C,
    store: S,
    required_scopes: Vec<String>,
    clock: T,
    refresh_skew_secs: i64,
}

impl<C, S> AuthService<C, S, SystemClock>
where
    C: SsoAuthClient,
    S: TokenStore,
{
    pub fn new(client: C, store: S, required_scopes: Vec<String>) -> Self {
        Self::with_clock(client, store, required_scopes, SystemClock)
    }
}

impl<C, S, T> AuthService<C, S, T>
where
    C: SsoAuthClient,
    S: TokenStore,
    T: Clock,
{
    pub fn with_clock(client: C, store: S, required_scopes: Vec<String>, clock: T) -> Self {
        Self {
            client,
            store,
            required_scopes,
            clock,
            refresh_skew_secs: 60,
        }
    }

    pub fn with_refresh_skew_secs(mut self, refresh_skew_secs: i64) -> Self {
        self.refresh_skew_secs = refresh_skew_secs;
        self
    }

    pub fn begin_login(&mut self) -> EsiResult<LoginRequest> {
        self.client.begin_login()
    }

    pub async fn complete_login(
        &mut self,
        code: &str,
        callback_state: &str,
    ) -> EsiResult<AuthSession> {
        let now = self.clock.now_epoch_secs();
        let tokens = self.client.exchange_code(code, callback_state).await?;

        let missing_scopes = missing_required_scopes(&tokens.scopes, &self.required_scopes);
        if !missing_scopes.is_empty() {
            self.store.clear_session(tokens.character_id)?;
            return Err(EsiError::MissingRequiredScopes {
                missing: missing_scopes,
            });
        }

        let session = AuthSession {
            character_id: tokens.character_id,
            character_name: tokens.character_name,
            scopes: tokens.scopes,
            access_token: tokens.access_token,
            access_expires_at_epoch_secs: tokens.access_expires_at_epoch_secs,
            refresh_token: tokens.refresh_token,
            updated_at_epoch_secs: now,
        };

        self.store.save_session(&session)?;
        Ok(session)
    }

    pub fn load_session(&self, character_id: CharacterId) -> EsiResult<Option<AuthSession>> {
        self.store.load_session(character_id)
    }

    pub fn logout(&self, character_id: CharacterId) -> EsiResult<()> {
        self.store.clear_session(character_id)
    }

    pub async fn ensure_valid_session(
        &mut self,
        character_id: CharacterId,
    ) -> EsiResult<EnsureSessionResult> {
        let now = self.clock.now_epoch_secs();
        let Some(mut session) = self.store.load_session(character_id)? else {
            return Ok(EnsureSessionResult::Missing);
        };

        let missing_scopes = missing_required_scopes(&session.scopes, &self.required_scopes);
        if !missing_scopes.is_empty() {
            self.store.clear_session(character_id)?;
            return Ok(EnsureSessionResult::NeedsReauth {
                reason: EsiError::MissingRequiredScopes {
                    missing: missing_scopes,
                }
                .to_string(),
            });
        }

        if !session.should_refresh(now, self.refresh_skew_secs) {
            return Ok(EnsureSessionResult::Ready(session));
        }

        match self.client.refresh(&session.refresh_token).await {
            Ok(tokens) => {
                session.access_token = tokens.access_token;
                session.access_expires_at_epoch_secs = tokens.access_expires_at_epoch_secs;
                session.refresh_token = tokens.refresh_token;
                session.updated_at_epoch_secs = now;

                let missing_scopes =
                    missing_required_scopes(&session.scopes, &self.required_scopes);
                if !missing_scopes.is_empty() {
                    self.store.clear_session(character_id)?;
                    return Ok(EnsureSessionResult::NeedsReauth {
                        reason: EsiError::MissingRequiredScopes {
                            missing: missing_scopes,
                        }
                        .to_string(),
                    });
                }

                self.store.save_session(&session)?;
                Ok(EnsureSessionResult::Ready(session))
            }
            Err(err) => Ok(EnsureSessionResult::NeedsReauth {
                reason: err.to_string(),
            }),
        }
    }

    pub fn next_refresh_delay(
        &self,
        character_id: CharacterId,
        floor: Duration,
    ) -> EsiResult<NextRefreshDelay> {
        let Some(session) = self.store.load_session(character_id)? else {
            return Ok(NextRefreshDelay::NeedsReauth {
                reason: "session missing for selected character".to_string(),
            });
        };

        let missing_scopes = missing_required_scopes(&session.scopes, &self.required_scopes);
        if !missing_scopes.is_empty() {
            self.store.clear_session(character_id)?;
            return Ok(NextRefreshDelay::NeedsReauth {
                reason: EsiError::MissingRequiredScopes {
                    missing: missing_scopes,
                }
                .to_string(),
            });
        }

        let now = self.clock.now_epoch_secs();
        let refresh_deadline = session.access_expires_at_epoch_secs - self.refresh_skew_secs;
        let seconds_until_deadline = refresh_deadline - now;
        if seconds_until_deadline <= 0 {
            return Ok(NextRefreshDelay::ReadyNow);
        }

        let delay = Duration::from_secs(seconds_until_deadline as u64);
        Ok(NextRefreshDelay::Wait(delay.max(floor)))
    }
}

impl<C, S, T> AuthService<C, S, T>
where
    C: SsoAuthClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    pub async fn connect_character(
        self,
        character_id: CharacterId,
    ) -> EsiResult<ManagedEsiClient<C, S, T>>
    where
        C: crate::client::EsiApiClient,
    {
        ManagedEsiClient::connect(self, character_id).await
    }
}

impl<C, S, T> AuthService<C, S, T>
where
    C: SsoAuthClient,
    S: TokenStore,
    T: Clock,
{
    pub(crate) fn client_mut(&mut self) -> &mut C {
        &mut self.client
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextRefreshDelay {
    ReadyNow,
    Wait(Duration),
    NeedsReauth { reason: String },
}

fn missing_required_scopes(granted_scopes: &[String], required_scopes: &[String]) -> Vec<String> {
    required_scopes
        .iter()
        .filter(|required| !granted_scopes.iter().any(|granted| granted == *required))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;
    use jaytripper_core::ids::CharacterId;

    use super::AuthSession;
    use crate::{
        EsiError, EsiResult,
        auth::{AuthService, Clock, EnsureSessionResult},
        client::{InitialAuthTokens, RefreshTokens, SsoAuthClient},
        token_store::TokenStore,
    };

    #[derive(Clone, Copy)]
    struct FixedClock {
        now: i64,
    }

    impl Clock for FixedClock {
        fn now_epoch_secs(&self) -> i64 {
            self.now
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        sessions: Mutex<HashMap<CharacterId, AuthSession>>,
    }

    impl TokenStore for MemoryStore {
        fn load_session(&self, character_id: CharacterId) -> EsiResult<Option<AuthSession>> {
            Ok(self
                .sessions
                .lock()
                .expect("lock")
                .get(&character_id)
                .cloned())
        }

        fn save_session(&self, session: &AuthSession) -> EsiResult<()> {
            self.sessions
                .lock()
                .expect("lock")
                .insert(session.character_id, session.clone());
            Ok(())
        }

        fn clear_session(&self, character_id: CharacterId) -> EsiResult<()> {
            self.sessions.lock().expect("lock").remove(&character_id);
            Ok(())
        }
    }

    struct MockClient {
        login_request: Option<crate::auth::LoginRequest>,
        initial_tokens: Option<InitialAuthTokens>,
        refresh_result: Option<EsiResult<RefreshTokens>>,
    }

    #[async_trait]
    impl SsoAuthClient for MockClient {
        fn begin_login(&mut self) -> EsiResult<crate::auth::LoginRequest> {
            self.login_request
                .clone()
                .ok_or_else(|| EsiError::message("no login request configured"))
        }

        async fn exchange_code(
            &mut self,
            _code: &str,
            _callback_state: &str,
        ) -> EsiResult<InitialAuthTokens> {
            self.initial_tokens
                .clone()
                .ok_or_else(|| EsiError::message("no initial tokens configured"))
        }

        async fn refresh(&mut self, _refresh_token: &str) -> EsiResult<RefreshTokens> {
            self.refresh_result
                .take()
                .unwrap_or_else(|| Err(EsiError::message("no refresh result configured")))
        }
    }

    fn sample_session(expires_at: i64) -> AuthSession {
        AuthSession {
            character_id: CharacterId(9001),
            character_name: Some("Pilot".to_string()),
            scopes: vec!["esi-location.read_location.v1".to_string()],
            access_token: "access".to_string(),
            access_expires_at_epoch_secs: expires_at,
            refresh_token: "refresh".to_string(),
            updated_at_epoch_secs: 100,
        }
    }

    #[tokio::test]
    async fn complete_login_persists_session() {
        let client = MockClient {
            login_request: None,
            initial_tokens: Some(InitialAuthTokens {
                character_id: CharacterId(9001),
                character_name: Some("Pilot".to_string()),
                scopes: vec!["esi-location.read_location.v1".to_string()],
                access_token: "new-access".to_string(),
                access_expires_at_epoch_secs: 1000,
                refresh_token: "new-refresh".to_string(),
            }),
            refresh_result: None,
        };
        let store = MemoryStore::default();
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_location.v1".to_string()],
            FixedClock { now: 777 },
        );

        let session = service
            .complete_login("code", "state")
            .await
            .expect("complete login should succeed");

        assert_eq!(session.character_id, CharacterId(9001));
        assert_eq!(session.updated_at_epoch_secs, 777);
        assert_eq!(
            service
                .load_session(CharacterId(9001))
                .expect("load should work"),
            Some(session)
        );
    }

    #[tokio::test]
    async fn complete_login_fails_and_clears_when_required_scope_missing() {
        let client = MockClient {
            login_request: None,
            initial_tokens: Some(InitialAuthTokens {
                character_id: CharacterId(9001),
                character_name: Some("Pilot".to_string()),
                scopes: vec!["publicData".to_string()],
                access_token: "new-access".to_string(),
                access_expires_at_epoch_secs: 1000,
                refresh_token: "new-refresh".to_string(),
            }),
            refresh_result: None,
        };
        let store = MemoryStore::default();
        store
            .save_session(&sample_session(10_000))
            .expect("save should work");
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_location.v1".to_string()],
            FixedClock { now: 777 },
        );

        let err = service
            .complete_login("code", "state")
            .await
            .expect_err("complete login should fail when scope is missing");

        assert!(matches!(err, EsiError::MissingRequiredScopes { .. }));
        assert!(
            service
                .load_session(CharacterId(9001))
                .expect("load should work")
                .is_none()
        );
    }

    #[tokio::test]
    async fn ensure_valid_session_returns_existing_without_refresh() {
        let client = MockClient {
            login_request: None,
            initial_tokens: None,
            refresh_result: None,
        };
        let store = MemoryStore::default();
        store
            .save_session(&sample_session(10_000))
            .expect("save should work");
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_location.v1".to_string()],
            FixedClock { now: 500 },
        )
        .with_refresh_skew_secs(60);

        let result = service
            .ensure_valid_session(CharacterId(9001))
            .await
            .expect("ensure should succeed");

        assert!(matches!(result, EnsureSessionResult::Ready(_)));
    }

    #[tokio::test]
    async fn ensure_valid_session_refreshes_when_expiring() {
        let client = MockClient {
            login_request: None,
            initial_tokens: None,
            refresh_result: Some(Ok(RefreshTokens {
                access_token: "refreshed-access".to_string(),
                access_expires_at_epoch_secs: 10_000,
                refresh_token: "refreshed-refresh".to_string(),
            })),
        };
        let store = MemoryStore::default();
        store
            .save_session(&sample_session(510))
            .expect("save should work");
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_location.v1".to_string()],
            FixedClock { now: 500 },
        )
        .with_refresh_skew_secs(60);

        let result = service
            .ensure_valid_session(CharacterId(9001))
            .await
            .expect("ensure should succeed");

        let EnsureSessionResult::Ready(session) = result else {
            panic!("expected ready session after refresh");
        };
        assert_eq!(session.access_token, "refreshed-access");
        assert_eq!(session.refresh_token, "refreshed-refresh");
    }

    #[tokio::test]
    async fn ensure_valid_session_requests_reauth_when_refresh_fails() {
        let client = MockClient {
            login_request: None,
            initial_tokens: None,
            refresh_result: Some(Err(EsiError::message("refresh token rejected"))),
        };
        let store = MemoryStore::default();
        store
            .save_session(&sample_session(510))
            .expect("save should work");
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_location.v1".to_string()],
            FixedClock { now: 500 },
        )
        .with_refresh_skew_secs(60);

        let result = service
            .ensure_valid_session(CharacterId(9001))
            .await
            .expect("ensure should succeed with needs reauth state");

        let EnsureSessionResult::NeedsReauth { reason } = result else {
            panic!("expected needs reauth");
        };
        assert!(reason.contains("refresh token rejected"));
        assert!(
            service
                .load_session(CharacterId(9001))
                .expect("load should work")
                .is_some()
        );
    }

    #[tokio::test]
    async fn ensure_valid_session_clears_when_required_scope_missing() {
        let client = MockClient {
            login_request: None,
            initial_tokens: None,
            refresh_result: None,
        };
        let store = MemoryStore::default();
        store
            .save_session(&sample_session(10_000))
            .expect("save should work");
        let mut service = AuthService::with_clock(
            client,
            store,
            vec!["esi-location.read_ship_type.v1".to_string()],
            FixedClock { now: 500 },
        )
        .with_refresh_skew_secs(60);

        let result = service
            .ensure_valid_session(CharacterId(9001))
            .await
            .expect("ensure should produce a needs reauth state");

        let EnsureSessionResult::NeedsReauth { reason } = result else {
            panic!("expected needs reauth");
        };
        assert!(reason.contains("missing required scopes"));
        assert!(
            service
                .load_session(CharacterId(9001))
                .expect("load should work")
                .is_none()
        );
    }
}
