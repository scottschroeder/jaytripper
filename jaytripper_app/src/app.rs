use std::{path::Path, sync::Arc};

use jaytripper_core::{
    SignatureEventSource, SystemSignaturesObservedEvent,
    ids::{CharacterId, SolarSystemId},
    parse_signature_snapshot,
    time::Timestamp,
};
use jaytripper_esi::{EsiClient, LocationIngestor, LocationPollConfig};
use jaytripper_store::{EventLogStore, EventRecord, GlobalSeq};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

use crate::{
    AppError,
    sink::AppMovementSink,
    state::{AppProjection, project_event_record},
};

#[derive(Clone, Default, Debug, PartialEq)]
struct ProjectionRuntimeState {
    projection: AppProjection,
    last_projected_seq: Option<GlobalSeq>,
}

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

    #[cfg(test)]
    async fn last_projected_seq(&self) -> Option<GlobalSeq> {
        self.state.lock().await.last_projected_seq
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

enum SignatureTargetSystemResolution {
    Record {
        system_id: SolarSystemId,
        attribution_character_id: Option<CharacterId>,
    },
    NeedsConfirmation {
        focused_system_id: SolarSystemId,
        character_system_id: SolarSystemId,
        character_id: CharacterId,
    },
}

fn resolve_signature_target_system(
    projection: &AppProjection,
    context: SignatureSnapshotRecordContext,
) -> SignatureTargetSystemResolution {
    match context {
        SignatureSnapshotRecordContext::Explicit {
            system_id,
            attribution_character_id,
        } => SignatureTargetSystemResolution::Record {
            system_id,
            attribution_character_id,
        },
        SignatureSnapshotRecordContext::Auto {
            focused_system_id,
            attribution_character_id,
        } => {
            let character_system = attribution_character_id.and_then(|character_id| {
                projection
                    .characters
                    .get(&character_id)
                    .map(|status| status.current_system_id)
            });

            match (attribution_character_id, character_system) {
                (Some(character_id), Some(character_system_id))
                    if character_system_id != focused_system_id =>
                {
                    SignatureTargetSystemResolution::NeedsConfirmation {
                        focused_system_id,
                        character_system_id,
                        character_id,
                    }
                }
                (_, Some(_)) => SignatureTargetSystemResolution::Record {
                    system_id: focused_system_id,
                    attribution_character_id,
                },
                (_, None) => SignatureTargetSystemResolution::Record {
                    system_id: focused_system_id,
                    attribution_character_id,
                },
            }
        }
    }
}

fn project_records_with_monotonic_guard(
    state: &mut ProjectionRuntimeState,
    records: &[EventRecord],
) -> Result<(), AppError> {
    for record in records {
        if let Some(last_seq) = state.last_projected_seq
            && record.global_seq <= last_seq
        {
            continue;
        }

        project_event_record(&mut state.projection, record)?;
        state.last_projected_seq = Some(record.global_seq);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use jaytripper_core::{
        MovementEvent, MovementEventSink, MovementEventSource, SignatureEntry,
        SignatureEventSource, SystemSignaturesObservedEvent,
        ids::{CharacterId, SolarSystemId, StationId},
        time::Timestamp,
    };
    use jaytripper_esi::{CharacterLocation, EsiClient, EsiError, LocationPollConfig};
    use jaytripper_store::{EventEnvelope, EventSource};
    use tempfile::tempdir;
    use tokio::sync::watch;

    use super::{
        AppRuntime, SignatureSnapshotRecordContext, SignatureSnapshotRecordOutcome,
        project_records_with_monotonic_guard,
    };
    use crate::AppError;

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
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append first move");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(42),
                from_system_id: Some(SolarSystemId(30000142)),
                to_system_id: SolarSystemId(30002510),
                observed_at: ts_secs(1_700_000_120),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append second move");

        let app = AppRuntime::from_store(store)
            .await
            .expect("build app from store");
        assert_eq!(
            app.last_projected_seq().await,
            Some(jaytripper_store::GlobalSeq(2))
        );
        assert_eq!(
            app.character_current_system(CharacterId(42)).await,
            Some(SolarSystemId(30002510))
        );
    }

    #[tokio::test]
    async fn sink_append_updates_projection_and_store() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(1337),
                from_system_id: None,
                to_system_id: SolarSystemId(30002053),
                observed_at: ts_secs(1_700_000_777),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        assert_eq!(
            app.last_projected_seq().await,
            Some(jaytripper_store::GlobalSeq(1))
        );
        assert_eq!(
            app.character_current_system(CharacterId(1337)).await,
            Some(SolarSystemId(30002053))
        );

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
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

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

        assert_eq!(
            app.character_current_system(CharacterId(4242)).await,
            Some(SolarSystemId(30000142))
        );
    }

    #[tokio::test]
    async fn replay_restores_signatures_and_characters_from_mixed_stream() {
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
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append move 1");

        store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id: SolarSystemId(30000142),
                snapshot_id: "snapshot-01".to_owned(),
                entries: vec![SignatureEntry {
                    signature_id: "ABC-123".to_owned(),
                    group: "Cosmic Signature".to_owned(),
                    site_type: None,
                    name: None,
                    scan_percent: Some(70.0),
                }],
                observed_at: ts_secs(1_700_000_060),
                attribution_character_id: Some(CharacterId(42)),
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append sigs 1");

        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(42),
                from_system_id: Some(SolarSystemId(30000142)),
                to_system_id: SolarSystemId(30002510),
                observed_at: ts_secs(1_700_000_120),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append move 2");

        store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id: SolarSystemId(30000142),
                snapshot_id: "snapshot-02".to_owned(),
                entries: vec![SignatureEntry {
                    signature_id: "ABC-123".to_owned(),
                    group: "Cosmic Signature".to_owned(),
                    site_type: None,
                    name: None,
                    scan_percent: Some(0.0),
                }],
                observed_at: ts_secs(1_700_000_180),
                attribution_character_id: Some(CharacterId(42)),
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append sigs 2");

        let app = AppRuntime::from_store(store)
            .await
            .expect("build app from store");
        assert_eq!(
            app.last_projected_seq().await,
            Some(jaytripper_store::GlobalSeq(4))
        );
        assert_eq!(
            app.character_current_system(CharacterId(42)).await,
            Some(SolarSystemId(30002510))
        );

        let state = app.state.lock().await;
        let system_signatures = state
            .projection
            .signatures_by_system
            .get(&SolarSystemId(30000142))
            .expect("system signatures");
        let signature = system_signatures
            .signatures_by_id
            .get("ABC-123")
            .expect("signature projection");
        assert_eq!(signature.latest_scan_percent, Some(0.0));
        assert_eq!(signature.highest_scan_percent_seen, Some(70.0));
    }

    #[tokio::test]
    async fn record_signature_snapshot_auto_uses_focused_without_character_location() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        let outcome = app
            .record_signature_snapshot(
                SignatureSnapshotRecordContext::Auto {
                    focused_system_id: SolarSystemId(30000142),
                    attribution_character_id: Some(CharacterId(9001)),
                },
                "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
            )
            .await
            .expect("record signatures");

        assert_eq!(
            outcome,
            SignatureSnapshotRecordOutcome::Recorded {
                system_id: SolarSystemId(30000142),
            }
        );

        let state = app.state.lock().await;
        let system_signatures = state
            .projection
            .signatures_by_system
            .get(&SolarSystemId(30000142))
            .expect("system signatures");
        assert!(system_signatures.signatures_by_id.contains_key("ABC-123"));
    }

    #[tokio::test]
    async fn record_signature_snapshot_auto_requests_confirmation_when_mismatch() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(42),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        let outcome = app
            .record_signature_snapshot(
                SignatureSnapshotRecordContext::Auto {
                    focused_system_id: SolarSystemId(30002510),
                    attribution_character_id: Some(CharacterId(42)),
                },
                "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
            )
            .await
            .expect("record call should not fail");

        assert_eq!(
            outcome,
            SignatureSnapshotRecordOutcome::NeedsConfirmation {
                focused_system_id: SolarSystemId(30002510),
                character_system_id: SolarSystemId(30000142),
                character_id: CharacterId(42),
            }
        );

        let events = jaytripper_store::EventLogStore::connect(&db_path)
            .await
            .expect("reconnect store")
            .read_ordered_events()
            .await
            .expect("read ordered events");
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn record_signature_snapshot_auto_uses_focused_when_character_matches() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(42),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        let outcome = app
            .record_signature_snapshot(
                SignatureSnapshotRecordContext::Auto {
                    focused_system_id: SolarSystemId(30000142),
                    attribution_character_id: Some(CharacterId(42)),
                },
                "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
            )
            .await
            .expect("record signatures");

        assert_eq!(
            outcome,
            SignatureSnapshotRecordOutcome::Recorded {
                system_id: SolarSystemId(30000142),
            }
        );
    }

    #[tokio::test]
    async fn record_signature_snapshot_explicit_applies_even_when_character_mismatch() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(42),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        let outcome = app
            .record_signature_snapshot(
                SignatureSnapshotRecordContext::Explicit {
                    system_id: SolarSystemId(30002510),
                    attribution_character_id: Some(CharacterId(42)),
                },
                "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
            )
            .await
            .expect("record signatures");

        assert_eq!(
            outcome,
            SignatureSnapshotRecordOutcome::Recorded {
                system_id: SolarSystemId(30002510),
            }
        );
    }

    #[tokio::test]
    async fn catch_up_projection_from_store_skips_already_applied_records() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.movement_sink()
            .emit_movement(MovementEvent {
                character_id: CharacterId(1),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("emit movement");

        let seq_before = app.last_projected_seq().await;
        app.catch_up_projection_from_store()
            .await
            .expect("catch up should be idempotent");
        assert_eq!(app.last_projected_seq().await, seq_before);
    }

    #[tokio::test]
    async fn monotonic_guard_rejects_late_older_overlap_batch() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        app.store()
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(77),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append movement 1");
        app.catch_up_projection_from_store()
            .await
            .expect("catch up 1");

        app.store()
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(77),
                from_system_id: Some(SolarSystemId(30000142)),
                to_system_id: SolarSystemId(30002510),
                observed_at: ts_secs(1_700_000_100),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append movement 2");
        app.store()
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(77),
                from_system_id: Some(SolarSystemId(30002510)),
                to_system_id: SolarSystemId(30002645),
                observed_at: ts_secs(1_700_000_200),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append movement 3");

        let records = app
            .store()
            .read_ordered_events()
            .await
            .expect("read records");
        let event_2 = records
            .iter()
            .find(|record| record.global_seq == jaytripper_store::GlobalSeq(2))
            .expect("event 2 exists")
            .clone();
        let event_3 = records
            .iter()
            .find(|record| record.global_seq == jaytripper_store::GlobalSeq(3))
            .expect("event 3 exists")
            .clone();

        {
            let mut state = app.state.lock().await;
            project_records_with_monotonic_guard(&mut state, &[event_3])
                .expect("apply newer overlap batch");
            project_records_with_monotonic_guard(&mut state, &[event_2])
                .expect("apply stale overlap batch");
        }

        assert_eq!(
            app.last_projected_seq().await,
            Some(jaytripper_store::GlobalSeq(3))
        );
        assert_eq!(
            app.character_current_system(CharacterId(77)).await,
            Some(SolarSystemId(30002645))
        );
    }

    #[tokio::test]
    async fn record_signature_snapshot_returns_parse_error() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");

        let err = app
            .record_signature_snapshot(
                SignatureSnapshotRecordContext::Auto {
                    focused_system_id: SolarSystemId(30000142),
                    attribution_character_id: None,
                },
                "BAD\tCosmic Signature\tGas Site\t\t10.0%\n",
            )
            .await
            .expect_err("expected parse error");

        assert!(matches!(
            err,
            AppError::SignatureParse(
                jaytripper_core::SignatureParseError::InvalidSignatureId { .. }
            )
        ));
    }

    #[tokio::test]
    async fn unknown_event_type_is_skipped_and_sequence_advances() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");

        let store = jaytripper_store::EventLogStore::connect(&db_path)
            .await
            .expect("connect store");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(9001),
                from_system_id: None,
                to_system_id: SolarSystemId(30000142),
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append movement");
        store
            .append_event(&EventEnvelope {
                event_id: uuid::Uuid::now_v7().to_string(),
                event_type: "some_future_event".to_owned(),
                schema_version: 1,
                stream_key: "future:stream".to_owned(),
                occurred_at: ts_secs(1_700_000_010),
                recorded_at: ts_secs(1_700_000_010),
                attribution_character_id: None,
                source: EventSource::Import,
                payload_json: "{\"hello\":\"world\"}".to_owned(),
            })
            .await
            .expect("append unknown event");

        let app = AppRuntime::from_store(store)
            .await
            .expect("build app from store");

        assert_eq!(
            app.character_current_system(CharacterId(9001)).await,
            Some(SolarSystemId(30000142))
        );
        assert_eq!(
            app.last_projected_seq().await,
            Some(jaytripper_store::GlobalSeq(2))
        );
    }

    #[tokio::test]
    async fn restart_preserves_mixed_projection_with_overwrites() {
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
                observed_at: ts_secs(1_700_000_000),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append move 1");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(42),
                from_system_id: Some(SolarSystemId(30000142)),
                to_system_id: SolarSystemId(30002510),
                observed_at: ts_secs(1_700_000_060),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append move 2 overwrite");
        store
            .append_movement_event(&MovementEvent {
                character_id: CharacterId(100),
                from_system_id: None,
                to_system_id: SolarSystemId(30005196),
                observed_at: ts_secs(1_700_000_090),
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append move second character");

        store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id: SolarSystemId(30000142),
                snapshot_id: "snap-a1".to_owned(),
                entries: vec![
                    SignatureEntry {
                        signature_id: "ABC-123".to_owned(),
                        group: "Cosmic Signature".to_owned(),
                        site_type: Some("Gas Site".to_owned()),
                        name: None,
                        scan_percent: Some(70.0),
                    },
                    SignatureEntry {
                        signature_id: "DEF-999".to_owned(),
                        group: "Cosmic Signature".to_owned(),
                        site_type: Some("Relic Site".to_owned()),
                        name: None,
                        scan_percent: Some(35.0),
                    },
                ],
                observed_at: ts_secs(1_700_000_120),
                attribution_character_id: Some(CharacterId(42)),
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append signature snapshot a1");
        store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id: SolarSystemId(30000142),
                snapshot_id: "snap-a2".to_owned(),
                entries: vec![SignatureEntry {
                    signature_id: "ABC-123".to_owned(),
                    group: "Cosmic Signature".to_owned(),
                    site_type: Some("Gas Site".to_owned()),
                    name: Some("Cloud Ring".to_owned()),
                    scan_percent: Some(10.0),
                }],
                observed_at: ts_secs(1_700_000_180),
                attribution_character_id: Some(CharacterId(42)),
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append signature snapshot a2 overwrite");

        store
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id: SolarSystemId(30002510),
                snapshot_id: "snap-b1".to_owned(),
                entries: vec![SignatureEntry {
                    signature_id: "QWE-777".to_owned(),
                    group: "Cosmic Anomaly".to_owned(),
                    site_type: Some("Ore Site".to_owned()),
                    name: None,
                    scan_percent: Some(100.0),
                }],
                observed_at: ts_secs(1_700_000_200),
                attribution_character_id: Some(CharacterId(42)),
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append signature snapshot b1");

        store
            .append_event(&EventEnvelope {
                event_id: uuid::Uuid::now_v7().to_string(),
                event_type: "some_future_event".to_owned(),
                schema_version: 1,
                stream_key: "future:stream".to_owned(),
                occurred_at: ts_secs(1_700_000_220),
                recorded_at: ts_secs(1_700_000_220),
                attribution_character_id: None,
                source: EventSource::Import,
                payload_json: "{}".to_owned(),
            })
            .await
            .expect("append unknown event");

        let app_before_restart = AppRuntime::connect(&db_path)
            .await
            .expect("connect app before restart");

        let before_state = app_before_restart.state.lock().await.clone();

        assert_eq!(
            app_before_restart
                .character_current_system(CharacterId(42))
                .await,
            Some(SolarSystemId(30002510))
        );
        assert_eq!(
            app_before_restart
                .character_current_system(CharacterId(100))
                .await,
            Some(SolarSystemId(30005196))
        );

        let system_a = before_state
            .projection
            .signatures_by_system
            .get(&SolarSystemId(30000142))
            .expect("system A signatures");
        let abc = system_a
            .signatures_by_id
            .get("ABC-123")
            .expect("ABC present");
        assert_eq!(abc.latest_scan_percent, Some(10.0));
        assert_eq!(abc.highest_scan_percent_seen, Some(70.0));
        let def = system_a
            .signatures_by_id
            .get("DEF-999")
            .expect("DEF present");
        assert!(def.missing_from_latest_snapshot);

        let app_after_restart = AppRuntime::connect(&db_path)
            .await
            .expect("connect app after restart");
        let after_state = app_after_restart.state.lock().await.clone();

        assert_eq!(before_state, after_state);
    }

    fn ts_secs(value: i64) -> Timestamp {
        Timestamp::from_epoch_secs(value).expect("valid epoch seconds")
    }
}
