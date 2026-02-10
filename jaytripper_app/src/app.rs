use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use jaytripper_esi::{EsiClient, LocationIngestor, LocationPollConfig};
use jaytripper_store::EventLogStore;
use tokio::sync::watch;

use crate::{AppError, CharacterTrackerSnapshot, StoreAndStateMovementSink, state::apply_record};

#[derive(Clone)]
pub struct CharacterTrackerApp {
    store: EventLogStore,
    state: Arc<Mutex<CharacterTrackerSnapshot>>,
}

impl CharacterTrackerApp {
    pub async fn connect(database_path: impl AsRef<Path>) -> Result<Self, AppError> {
        let store = EventLogStore::connect(database_path).await?;
        Self::from_store(store).await
    }

    pub async fn from_store(store: EventLogStore) -> Result<Self, AppError> {
        let app = Self {
            store,
            state: Arc::new(Mutex::new(CharacterTrackerSnapshot::default())),
        };
        app.replay().await?;
        Ok(app)
    }

    pub fn movement_sink(&self) -> StoreAndStateMovementSink {
        StoreAndStateMovementSink {
            store: self.store.clone(),
            state: Arc::clone(&self.state),
        }
    }

    pub fn snapshot(&self) -> CharacterTrackerSnapshot {
        self.state.lock().expect("state lock poisoned").clone()
    }

    pub async fn replay(&self) -> Result<(), AppError> {
        let mut snapshot = CharacterTrackerSnapshot::default();
        for record in self.store.read_ordered_events().await? {
            apply_record(&mut snapshot, &record)?;
        }

        *self.state.lock().expect("state lock poisoned") = snapshot;
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
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use jaytripper_core::{
        MovementEvent, MovementEventSink, MovementEventSource,
        ids::{CharacterId, SolarSystemId, StationId},
    };
    use jaytripper_esi::{CharacterLocation, EsiClient, EsiError, LocationPollConfig};
    use tempfile::tempdir;
    use tokio::sync::watch;

    use super::CharacterTrackerApp;

    struct MockEsiClient {
        character_id: CharacterId,
        responses: Mutex<VecDeque<Result<CharacterLocation, EsiError>>>,
    }

    #[async_trait]
    impl EsiClient for MockEsiClient {
        fn character_id(&self) -> CharacterId {
            self.character_id
        }

        fn requires_reauth(&self) -> bool {
            false
        }

        fn reauth_reason(&self) -> Option<String> {
            None
        }

        async fn get_current_location(&self) -> Result<CharacterLocation, EsiError> {
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .unwrap_or_else(|| Err(EsiError::message("no response configured")))
        }
    }

    fn location(system_id: i32) -> CharacterLocation {
        CharacterLocation {
            solar_system_id: SolarSystemId(system_id),
            station_id: Some(StationId(1)),
            structure_id: None,
        }
    }

    #[tokio::test]
    async fn replay_restores_character_position() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");

        let store = jaytripper_store::EventLogStore::connect(&db_path)
            .await
            .expect("connect store");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(42),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at_epoch_secs: 1_700_000_000,
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append first move");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(42),
                from_system_id: Some(SolarSystemId(30000142)),
                to_system_id: SolarSystemId(30002510),
                observed_at_epoch_secs: 1_700_000_120,
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append second move");

        let app = CharacterTrackerApp::from_store(store)
            .await
            .expect("build app from store");
        let snapshot = app.snapshot();

        assert_eq!(snapshot.last_applied_global_seq, Some(2));
        let status = snapshot
            .characters
            .get(&CharacterId(42))
            .expect("character state");
        assert_eq!(status.current_system_id, SolarSystemId(30002510));
    }

    #[tokio::test]
    async fn sink_append_updates_state_and_store() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = CharacterTrackerApp::connect(&db_path)
            .await
            .expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(1337),
                from_system_id: None,
                to_system_id: SolarSystemId(30002053),
                observed_at_epoch_secs: 1_700_000_777,
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        let snapshot = app.snapshot();
        assert_eq!(snapshot.last_applied_global_seq, Some(1));
        let status = snapshot
            .characters
            .get(&CharacterId(1337))
            .expect("character state");
        assert_eq!(status.current_system_id, SolarSystemId(30002053));

        let events = jaytripper_store::EventLogStore::connect(&db_path)
            .await
            .expect("reconnect store")
            .read_ordered_events()
            .await
            .expect("read ordered events");
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn run_ingestion_until_shutdown_writes_to_store() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = CharacterTrackerApp::connect(&db_path)
            .await
            .expect("connect app");

        let client = MockEsiClient {
            character_id: CharacterId(4242),
            responses: Mutex::new(VecDeque::from(vec![Ok(location(30000142))])),
        };
        let config = LocationPollConfig {
            base_interval: Duration::from_secs(60),
            jitter_factor: 0.0,
            api_failure_backoff_initial: Duration::from_secs(1),
            api_failure_backoff_max: Duration::from_secs(5),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let stop_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = shutdown_tx.send(true);
        });

        app.run_ingestion_until_shutdown(client, config, shutdown_rx)
            .await
            .expect("ingestion run should succeed");
        stop_handle.await.expect("stop handle complete");

        let snapshot = app.snapshot();
        assert!(snapshot.characters.contains_key(&CharacterId(4242)));
    }
}
