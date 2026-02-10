use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use jaytripper_core::ids::CharacterId;
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    AuthService, EnsureSessionResult, EsiError, EsiResult,
    api::CharacterLocation,
    auth::{Clock, NextRefreshDelay},
    client::{EsiApiClient, SsoAuthClient},
    token_store::TokenStore,
};

const DEFAULT_REFRESH_FLOOR: Duration = Duration::from_secs(5);

#[async_trait]
pub trait EsiClient {
    fn character_id(&self) -> CharacterId;
    fn requires_reauth(&self) -> bool;
    fn reauth_reason(&self) -> Option<String>;
    async fn get_current_location(&self) -> EsiResult<CharacterLocation>;
}

struct ManagedState<C, S, T>
where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    auth: AuthService<C, S, T>,
}

pub struct ManagedEsiClient<C, S, T = crate::auth::SystemClock>
where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    character_id: CharacterId,
    state: Arc<AsyncMutex<ManagedState<C, S, T>>>,
    needs_reauth: Arc<AtomicBool>,
    reauth_reason: Arc<Mutex<Option<String>>>,
    shutdown_tx: watch::Sender<bool>,
    refresh_task: JoinHandle<()>,
}

impl<C, S, T> ManagedEsiClient<C, S, T>
where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    pub async fn connect(
        mut auth_service: AuthService<C, S, T>,
        character_id: CharacterId,
    ) -> EsiResult<Self> {
        match auth_service.ensure_valid_session(character_id).await? {
            EnsureSessionResult::Ready(_) => {}
            EnsureSessionResult::Missing => {
                return Err(EsiError::NeedsReauth {
                    reason: "session missing for selected character".to_string(),
                });
            }
            EnsureSessionResult::NeedsReauth { reason } => {
                return Err(EsiError::NeedsReauth { reason });
            }
        }

        auth_service.client_mut().ensure_api_ready().await?;

        let state = Arc::new(AsyncMutex::new(ManagedState { auth: auth_service }));
        let needs_reauth = Arc::new(AtomicBool::new(false));
        let reauth_reason = Arc::new(Mutex::new(None));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let refresh_task = tokio::spawn(refresh_loop(
            Arc::clone(&state),
            character_id,
            DEFAULT_REFRESH_FLOOR,
            Arc::clone(&needs_reauth),
            Arc::clone(&reauth_reason),
            shutdown_rx,
        ));

        Ok(Self {
            character_id,
            state,
            needs_reauth,
            reauth_reason,
            shutdown_tx,
            refresh_task,
        })
    }
}

#[async_trait]
impl<C, S, T> EsiClient for ManagedEsiClient<C, S, T>
where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    fn character_id(&self) -> CharacterId {
        self.character_id
    }

    fn requires_reauth(&self) -> bool {
        self.needs_reauth.load(Ordering::Relaxed)
    }

    fn reauth_reason(&self) -> Option<String> {
        match self.reauth_reason.lock() {
            Ok(reason) => reason.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    async fn get_current_location(&self) -> EsiResult<CharacterLocation> {
        if self.requires_reauth() {
            let reason = self
                .reauth_reason()
                .unwrap_or_else(|| "reauthentication required".to_string());
            return Err(EsiError::NeedsReauth { reason });
        }

        let mut state = self.state.lock().await;
        state
            .auth
            .client_mut()
            .get_current_location(self.character_id)
            .await
    }
}

impl<C, S, T> Drop for ManagedEsiClient<C, S, T>
where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        self.refresh_task.abort();
    }
}

async fn refresh_loop<C, S, T>(
    state: Arc<AsyncMutex<ManagedState<C, S, T>>>,
    character_id: CharacterId,
    refresh_floor: Duration,
    needs_reauth: Arc<AtomicBool>,
    reauth_reason: Arc<Mutex<Option<String>>>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    C: SsoAuthClient + EsiApiClient + Send + 'static,
    S: TokenStore + Send + Sync + 'static,
    T: Clock + Send + Sync + 'static,
{
    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let next_delay = {
            let state = state.lock().await;
            state.auth.next_refresh_delay(character_id, refresh_floor)
        };

        let wait_duration = match next_delay {
            Ok(NextRefreshDelay::ReadyNow) => Duration::from_secs(0),
            Ok(NextRefreshDelay::Wait(duration)) => duration,
            Ok(NextRefreshDelay::NeedsReauth { reason }) => {
                mark_needs_reauth(&needs_reauth, &reauth_reason, reason);
                break;
            }
            Err(_) => refresh_floor,
        };

        if wait_duration > Duration::from_secs(0) {
            tokio::select! {
                _ = sleep(wait_duration) => {}
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        let refresh_outcome = {
            let mut state = state.lock().await;
            state.auth.ensure_valid_session(character_id).await
        };

        match refresh_outcome {
            Ok(EnsureSessionResult::Ready(_)) => {}
            Ok(EnsureSessionResult::Missing) => {
                mark_needs_reauth(
                    &needs_reauth,
                    &reauth_reason,
                    "session missing for selected character".to_string(),
                );
                break;
            }
            Ok(EnsureSessionResult::NeedsReauth { reason }) => {
                mark_needs_reauth(&needs_reauth, &reauth_reason, reason);
                break;
            }
            Err(_) => sleep(refresh_floor).await,
        }
    }
}

fn mark_needs_reauth(
    needs_reauth: &AtomicBool,
    reauth_reason: &Mutex<Option<String>>,
    reason: String,
) {
    needs_reauth.store(true, Ordering::Relaxed);
    match reauth_reason.lock() {
        Ok(mut guard) => *guard = Some(reason),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = Some(reason);
        }
    }
}
