use std::path::{Path, PathBuf};

use jaytripper_app::AppRuntime;
use jaytripper_core::{
    MovementEvent, MovementEventSource, SignatureEntry, SignatureEventSource,
    SystemSignaturesObservedEvent,
    ids::{CharacterId, SolarSystemId},
    time::Timestamp,
};
use jaytripper_store::{EventLogStore, EventRecord};
use tempfile::TempDir;

pub struct TestHarness {
    _temp_dir: TempDir,
    db_path: PathBuf,
}

impl TestHarness {
    pub fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        Self {
            _temp_dir: temp_dir,
            db_path,
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub async fn app(&self) -> AppRuntime {
        AppRuntime::connect(self.db_path())
            .await
            .expect("connect app")
    }

    pub async fn store(&self) -> EventLogStore {
        EventLogStore::connect(self.db_path())
            .await
            .expect("connect store")
    }

    pub async fn append_movement(
        &self,
        character_id: CharacterId,
        from_system_id: Option<SolarSystemId>,
        to_system_id: SolarSystemId,
        observed_at: Timestamp,
    ) {
        self.store()
            .await
            .append_movement_event(&MovementEvent {
                character_id,
                from_system_id,
                to_system_id,
                observed_at,
                source: MovementEventSource::Esi,
            })
            .await
            .expect("append movement");
    }

    pub async fn append_signature_snapshot(
        &self,
        system_id: SolarSystemId,
        snapshot_id: &str,
        entries: Vec<SignatureEntry>,
        attribution_character_id: Option<CharacterId>,
        observed_at: Timestamp,
    ) {
        self.store()
            .await
            .append_system_signatures_observed_event(&SystemSignaturesObservedEvent {
                system_id,
                snapshot_id: snapshot_id.to_owned(),
                entries,
                observed_at,
                attribution_character_id,
                source: SignatureEventSource::Manual,
            })
            .await
            .expect("append signature snapshot");
    }

    pub async fn ordered_events(&self) -> Vec<EventRecord> {
        self.store()
            .await
            .read_ordered_events()
            .await
            .expect("read ordered events")
    }
}

pub fn ts(epoch_secs: i64) -> Timestamp {
    Timestamp::from_epoch_secs(epoch_secs).expect("valid epoch seconds")
}
