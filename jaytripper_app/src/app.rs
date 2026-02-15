use std::{path::Path, sync::Arc};

use jaytripper_core::{
    SignatureEventSource, SystemSignaturesObservedEvent,
    ids::{CharacterId, SolarSystemId},
    parse_signature_snapshot,
    time::Timestamp,
};
use jaytripper_esi::{EsiClient, LocationIngestor, LocationPollConfig};
use jaytripper_store::{EventLogStore, GlobalSeq};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

use crate::{
    AppError,
    projection_runtime::{ProjectionRuntimeState, project_records_with_monotonic_guard},
    signature_resolution::{SignatureTargetSystemResolution, resolve_signature_target_system},
    sink::AppMovementSink,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureSnapshotRecordContext {
    Auto {
        focused_system_id: SolarSystemId,
        attribution_character_id: Option<CharacterId>,
    },
    Explicit {
        system_id: SolarSystemId,
        attribution_character_id: Option<CharacterId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureSnapshotRecordOutcome {
    Recorded {
        system_id: SolarSystemId,
    },
    NeedsConfirmation {
        focused_system_id: SolarSystemId,
        character_system_id: SolarSystemId,
        character_id: CharacterId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterLocationView {
    pub character_id: CharacterId,
    pub current_system_id: SolarSystemId,
    pub last_movement_observed_at: Timestamp,
}

#[derive(Clone)]
pub struct AppRuntime {
    store: EventLogStore,
    state: Arc<Mutex<ProjectionRuntimeState>>,
}

impl AppRuntime {
    pub async fn connect(database_path: impl AsRef<Path>) -> Result<Self, AppError> {
        let store = EventLogStore::connect(database_path).await?;
        Self::from_store(store).await
    }

    pub async fn from_store(store: EventLogStore) -> Result<Self, AppError> {
        let app = Self {
            store,
            state: Arc::new(Mutex::new(ProjectionRuntimeState::default())),
        };
        app.initialize_from_event_log().await?;
        Ok(app)
    }

    fn movement_sink(&self) -> AppMovementSink {
        AppMovementSink::new(self.clone())
    }

    pub async fn character_locations(&self) -> Vec<CharacterLocationView> {
        self.state
            .lock()
            .await
            .projection
            .characters
            .iter()
            .map(|(character_id, status)| CharacterLocationView {
                character_id: *character_id,
                current_system_id: status.current_system_id,
                last_movement_observed_at: status.last_movement_observed_at,
            })
            .collect()
    }

    pub async fn character_current_system(
        &self,
        character_id: CharacterId,
    ) -> Option<SolarSystemId> {
        self.state
            .lock()
            .await
            .projection
            .characters
            .get(&character_id)
            .map(|status| status.current_system_id)
    }

    pub async fn initialize_from_event_log(&self) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        *state = ProjectionRuntimeState::default();

        let records = self.store.read_ordered_events().await?;
        project_records_with_monotonic_guard(&mut state, &records)?;

        Ok(())
    }

    pub async fn run_ingestion_until_shutdown<C>(
        &self,
        client: C,
        config: LocationPollConfig,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<(), AppError>
    where
        C: EsiClient + Send + Sync,
    {
        let sink = self.movement_sink();
        let mut ingestor = LocationIngestor::new(client, sink, config);
        ingestor.run_until_shutdown(shutdown_rx).await?;
        Ok(())
    }

    pub async fn record_signature_snapshot(
        &self,
        context: SignatureSnapshotRecordContext,
        snapshot_text: &str,
    ) -> Result<SignatureSnapshotRecordOutcome, AppError> {
        let entries = parse_signature_snapshot(snapshot_text)?;

        let mut state = self.state.lock().await;
        let resolution = resolve_signature_target_system(&state.projection, context);

        let (system_id, attribution_character_id) = match resolution {
            SignatureTargetSystemResolution::Record {
                system_id,
                attribution_character_id,
            } => (system_id, attribution_character_id),
            SignatureTargetSystemResolution::NeedsConfirmation {
                focused_system_id,
                character_system_id,
                character_id,
            } => {
                return Ok(SignatureSnapshotRecordOutcome::NeedsConfirmation {
                    focused_system_id,
                    character_system_id,
                    character_id,
                });
            }
        };

        self.store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id,
                snapshot_id: Uuid::now_v7().to_string(),
                entries,
                observed_at: jaytripper_core::Timestamp::now(),
                attribution_character_id,
                source: SignatureEventSource::Manual,
            })
            .await?;

        self.catch_up_projection_from_store_locked(&mut state)
            .await?;

        Ok(SignatureSnapshotRecordOutcome::Recorded { system_id })
    }

    pub(crate) async fn catch_up_projection_from_store(&self) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        self.catch_up_projection_from_store_locked(&mut state).await
    }

    async fn catch_up_projection_from_store_locked(
        &self,
        state: &mut ProjectionRuntimeState,
    ) -> Result<(), AppError> {
        let since_seq = state.last_projected_seq.unwrap_or(GlobalSeq(0));
        let records = self.store.read_events_since(since_seq).await?;
        project_records_with_monotonic_guard(state, &records)
    }

    pub(crate) fn store(&self) -> &EventLogStore {
        &self.store
    }
}
