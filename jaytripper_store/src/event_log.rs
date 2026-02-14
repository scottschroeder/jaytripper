use std::{path::Path, str::FromStr, time::Duration};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use jaytripper_core::{
    CHARACTER_MOVED_EVENT_TYPE, CHARACTER_MOVED_SCHEMA_VERSION, MovementEvent, MovementEventSink,
    MovementEventSource, SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE,
    SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION, SignatureEventSource, SystemSignaturesObservedEvent,
    Timestamp, character_stream_key, ids::CharacterId, system_stream_key,
};
use serde::{Deserialize, Serialize};
use sqlx::{
    FromRow, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::StoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Esi,
    Manual,
    Import,
    Sync,
}

impl EventSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Esi => "esi",
            Self::Manual => "manual",
            Self::Import => "import",
            Self::Sync => "sync",
        }
    }
}

impl FromStr for EventSource {
    type Err = StoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "esi" => Ok(Self::Esi),
            "manual" => Ok(Self::Manual),
            "import" => Ok(Self::Import),
            "sync" => Ok(Self::Sync),
            other => Err(StoreError::InvalidEventSource(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub schema_version: i64,
    pub stream_key: String,
    pub occurred_at: Timestamp,
    pub recorded_at: Timestamp,
    pub attribution_character_id: Option<CharacterId>,
    pub source: EventSource,
    pub payload_json: String,
}

pub type NewEvent = EventEnvelope;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRecord {
    pub global_seq: i64,
    pub envelope: EventEnvelope,
}

#[derive(Clone)]
pub struct EventLogStore {
    pool: SqlitePool,
}

impl EventLogStore {
    pub async fn connect(database_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connect_options = SqliteConnectOptions::new()
            .filename(database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(connect_options)
            .await?;

        sqlx::migrate!().run(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn append_event(&self, event: &NewEvent) -> Result<i64, StoreError> {
        let attribution_character_id = event
            .attribution_character_id
            .map(character_id_to_sqlite)
            .transpose()?;
        let source = event.source.as_str();
        let event_id = &event.event_id;
        let event_type = &event.event_type;
        let stream_key = &event.stream_key;
        let payload_json = &event.payload_json;
        let occurred_at_epoch_millis = event.occurred_at.as_epoch_millis();
        let recorded_at_epoch_millis = event.recorded_at.as_epoch_millis();

        let inserted = sqlx::query!(
            r#"
            INSERT INTO event_log (
                event_id,
                event_type,
                schema_version,
                stream_key,
                occurred_at_epoch_millis,
                recorded_at_epoch_millis,
                attribution_character_id,
                source,
                payload_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            RETURNING global_seq
            "#,
            event_id,
            event_type,
            event.schema_version,
            stream_key,
            occurred_at_epoch_millis,
            recorded_at_epoch_millis,
            attribution_character_id,
            source,
            payload_json,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(inserted.global_seq)
    }

    pub async fn append_movement_event(&self, event: &MovementEvent) -> Result<i64, StoreError> {
        self.append_movement_event_at(event, Timestamp::now()).await
    }

    pub async fn append_movement_event_at(
        &self,
        event: &MovementEvent,
        recorded_at: Timestamp,
    ) -> Result<i64, StoreError> {
        let new_event = NewEvent {
            event_id: uuid::Uuid::now_v7().to_string(),
            event_type: CHARACTER_MOVED_EVENT_TYPE.to_owned(),
            schema_version: CHARACTER_MOVED_SCHEMA_VERSION,
            stream_key: character_stream_key(event.character_id),
            occurred_at: event.observed_at,
            recorded_at,
            attribution_character_id: Some(event.character_id),
            source: map_movement_source(event.source),
            payload_json: serde_json::to_string(&event.as_character_moved_payload())?,
        };

        self.append_event(&new_event).await
    }

    pub async fn append_system_signatures_observed_event(
        &self,
        event: &SystemSignaturesObservedEvent,
    ) -> Result<i64, StoreError> {
        self.append_system_signatures_observed_event_at(event, Timestamp::now())
            .await
    }

    pub async fn append_system_signatures_observed_event_at(
        &self,
        event: &SystemSignaturesObservedEvent,
        recorded_at: Timestamp,
    ) -> Result<i64, StoreError> {
        let new_event = NewEvent {
            event_id: uuid::Uuid::now_v7().to_string(),
            event_type: SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE.to_owned(),
            schema_version: SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION,
            stream_key: system_stream_key(event.system_id),
            occurred_at: event.observed_at,
            recorded_at,
            attribution_character_id: event.attribution_character_id,
            source: map_signature_source(event.source),
            payload_json: serde_json::to_string(&event.as_payload())?,
        };

        self.append_event(&new_event).await
    }

    pub async fn read_ordered_events(&self) -> Result<Vec<EventRecord>, StoreError> {
        let mut rows = sqlx::query_as!(
            DbEventRecord,
            r#"
            SELECT
                global_seq AS "global_seq!",
                event_id AS "event_id!",
                event_type AS "event_type!",
                schema_version AS "schema_version!",
                stream_key AS "stream_key!",
                occurred_at_epoch_millis AS "occurred_at_epoch_millis!",
                recorded_at_epoch_millis AS "recorded_at_epoch_millis!",
                attribution_character_id AS "attribution_character_id?",
                source AS "source!",
                payload_json AS "payload_json!"
            FROM event_log
            ORDER BY global_seq ASC
            "#,
        )
        .fetch(&self.pool);

        let mut records = Vec::new();
        while let Some(row) = rows.try_next().await? {
            records.push(EventRecord::try_from(row)?);
        }

        Ok(records)
    }

    pub async fn read_events_since(&self, since_seq: i64) -> Result<Vec<EventRecord>, StoreError> {
        let mut rows = sqlx::query_as!(
            DbEventRecord,
            r#"
            SELECT
                global_seq AS "global_seq!",
                event_id AS "event_id!",
                event_type AS "event_type!",
                schema_version AS "schema_version!",
                stream_key AS "stream_key!",
                occurred_at_epoch_millis AS "occurred_at_epoch_millis!",
                recorded_at_epoch_millis AS "recorded_at_epoch_millis!",
                attribution_character_id AS "attribution_character_id?",
                source AS "source!",
                payload_json AS "payload_json!"
            FROM event_log
            WHERE global_seq > ?1
            ORDER BY global_seq ASC
            "#,
            since_seq,
        )
        .fetch(&self.pool);

        let mut records = Vec::new();
        while let Some(row) = rows.try_next().await? {
            records.push(EventRecord::try_from(row)?);
        }

        Ok(records)
    }

    pub async fn read_events_by_stream(
        &self,
        stream_key: &str,
    ) -> Result<Vec<EventRecord>, StoreError> {
        let mut rows = sqlx::query_as!(
            DbEventRecord,
            r#"
            SELECT
                global_seq AS "global_seq!",
                event_id AS "event_id!",
                event_type AS "event_type!",
                schema_version AS "schema_version!",
                stream_key AS "stream_key!",
                occurred_at_epoch_millis AS "occurred_at_epoch_millis!",
                recorded_at_epoch_millis AS "recorded_at_epoch_millis!",
                attribution_character_id AS "attribution_character_id?",
                source AS "source!",
                payload_json AS "payload_json!"
            FROM event_log
            WHERE stream_key = ?1
            ORDER BY global_seq ASC
            "#,
            stream_key,
        )
        .fetch(&self.pool);

        let mut records = Vec::new();
        while let Some(row) = rows.try_next().await? {
            records.push(EventRecord::try_from(row)?);
        }

        Ok(records)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl MovementEventSink for EventLogStore {
    type Error = StoreError;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error> {
        self.append_movement_event(&event).await?;
        Ok(())
    }
}

#[derive(Debug, FromRow)]
struct DbEventRecord {
    global_seq: i64,
    event_id: String,
    event_type: String,
    schema_version: i64,
    stream_key: String,
    occurred_at_epoch_millis: i64,
    recorded_at_epoch_millis: i64,
    attribution_character_id: Option<i64>,
    source: String,
    payload_json: String,
}

impl TryFrom<DbEventRecord> for EventRecord {
    type Error = StoreError;

    fn try_from(value: DbEventRecord) -> Result<Self, Self::Error> {
        let attribution_character_id = value
            .attribution_character_id
            .map(character_id_from_sqlite)
            .transpose()?;

        Ok(Self {
            global_seq: value.global_seq,
            envelope: EventEnvelope {
                event_id: value.event_id,
                event_type: value.event_type,
                schema_version: value.schema_version,
                stream_key: value.stream_key,
                occurred_at: Timestamp::from_epoch_millis(value.occurred_at_epoch_millis).ok_or(
                    StoreError::InvalidEpochMillis(value.occurred_at_epoch_millis),
                )?,
                recorded_at: Timestamp::from_epoch_millis(value.recorded_at_epoch_millis).ok_or(
                    StoreError::InvalidEpochMillis(value.recorded_at_epoch_millis),
                )?,
                attribution_character_id,
                source: EventSource::from_str(&value.source)?,
                payload_json: value.payload_json,
            },
        })
    }
}

fn character_id_to_sqlite(character_id: CharacterId) -> Result<i64, StoreError> {
    i64::try_from(character_id.0).map_err(|_| StoreError::CharacterIdOverflow(character_id.0))
}

fn character_id_from_sqlite(raw: i64) -> Result<CharacterId, StoreError> {
    let value = u64::try_from(raw).map_err(|_| StoreError::NegativeCharacterId(raw))?;
    Ok(CharacterId(value))
}

fn map_movement_source(source: MovementEventSource) -> EventSource {
    match source {
        MovementEventSource::Esi => EventSource::Esi,
    }
}

fn map_signature_source(source: SignatureEventSource) -> EventSource {
    match source {
        SignatureEventSource::Manual => EventSource::Manual,
    }
}

#[cfg(test)]
mod tests {
    use jaytripper_core::{
        CHARACTER_MOVED_EVENT_TYPE, CHARACTER_MOVED_SCHEMA_VERSION, MovementEvent,
        MovementEventSource, SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE,
        SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION, SignatureEntry, SignatureEventSource,
        SystemSignaturesObservedEvent, SystemSignaturesObservedPayload, Timestamp,
        ids::{CharacterId, SolarSystemId},
    };
    use tempfile::tempdir;

    use super::{EventLogStore, EventSource, NewEvent};

    #[tokio::test]
    async fn append_and_read_events_round_trip() {
        let temp_dir = tempdir().expect("tempdir");
        let database_path = temp_dir.path().join("events.sqlite");

        let store = EventLogStore::connect(&database_path)
            .await
            .expect("connect store");

        let first_seq = store
            .append_event(&NewEvent {
                event_id: "evt-1".to_owned(),
                event_type: "character_moved".to_owned(),
                schema_version: 1,
                stream_key: "character:42".to_owned(),
                occurred_at: ts_millis(1_700_000_000_123),
                recorded_at: ts_millis(1_700_000_005_123),
                attribution_character_id: Some(CharacterId(42)),
                source: EventSource::Esi,
                payload_json: "{\"to_system_id\":30000142}".to_owned(),
            })
            .await
            .expect("append first event");

        let second_seq = store
            .append_event(&NewEvent {
                event_id: "evt-2".to_owned(),
                event_type: "character_moved".to_owned(),
                schema_version: 1,
                stream_key: "character:42".to_owned(),
                occurred_at: ts_millis(1_700_000_100_456),
                recorded_at: ts_millis(1_700_000_101_456),
                attribution_character_id: Some(CharacterId(42)),
                source: EventSource::Manual,
                payload_json: "{\"from_system_id\":30000142,\"to_system_id\":30002510}".to_owned(),
            })
            .await
            .expect("append second event");

        assert!(second_seq > first_seq);

        let ordered = store.read_ordered_events().await.expect("read ordered");
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].envelope.event_id, "evt-1");
        assert_eq!(ordered[1].envelope.event_id, "evt-2");

        let since = store
            .read_events_since(first_seq)
            .await
            .expect("read since sequence");
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].envelope.event_id, "evt-2");

        let stream = store
            .read_events_by_stream("character:42")
            .await
            .expect("read by stream");
        assert_eq!(stream.len(), 2);
    }

    #[tokio::test]
    async fn migrations_apply_on_reopen() {
        let temp_dir = tempdir().expect("tempdir");
        let database_path = temp_dir.path().join("events.sqlite");

        let store = EventLogStore::connect(&database_path)
            .await
            .expect("connect first");
        drop(store);

        let reopened_store = EventLogStore::connect(&database_path)
            .await
            .expect("connect second");
        let events = reopened_store
            .read_ordered_events()
            .await
            .expect("read ordered after reopen");

        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn append_movement_event_uses_expected_envelope_shape() {
        let temp_dir = tempdir().expect("tempdir");
        let database_path = temp_dir.path().join("events.sqlite");

        let store = EventLogStore::connect(&database_path)
            .await
            .expect("connect store");

        store
            .append_movement_event_at(
                &MovementEvent {
                    character_id: CharacterId(42),
                    from_system_id: Some(SolarSystemId(30000142)),
                    to_system_id: SolarSystemId(30002510),
                    observed_at: ts_secs(1_700_000_000),
                    source: MovementEventSource::Esi,
                },
                ts_millis(1_700_000_000_999),
            )
            .await
            .expect("append movement event");

        let events = store.read_ordered_events().await.expect("read ordered");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].envelope.event_type, CHARACTER_MOVED_EVENT_TYPE);
        assert_eq!(
            events[0].envelope.schema_version,
            CHARACTER_MOVED_SCHEMA_VERSION
        );
        assert_eq!(events[0].envelope.stream_key, "character:42");
        assert_eq!(events[0].envelope.source, EventSource::Esi);
        assert_eq!(
            events[0].envelope.recorded_at.as_epoch_millis(),
            1_700_000_000_999
        );
    }

    #[tokio::test]
    async fn append_signature_event_uses_expected_envelope_shape() {
        let temp_dir = tempdir().expect("tempdir");
        let database_path = temp_dir.path().join("events.sqlite");

        let store = EventLogStore::connect(&database_path)
            .await
            .expect("connect store");

        let entries = vec![SignatureEntry {
            signature_id: "CWT-368".to_owned(),
            group: "Cosmic Signature".to_owned(),
            site_type: Some("Gas Site".to_owned()),
            name: None,
            scan_percent: Some(28.6),
        }];

        store
            .append_system_signatures_observed_event_at(
                &SystemSignaturesObservedEvent {
                    system_id: SolarSystemId(31000001),
                    snapshot_id: "snapshot-01".to_owned(),
                    entries: entries.clone(),
                    observed_at: ts_secs(1_700_000_300),
                    attribution_character_id: Some(CharacterId(42)),
                    source: SignatureEventSource::Manual,
                },
                ts_millis(1_700_000_300_999),
            )
            .await
            .expect("append signature event");

        let events = store.read_ordered_events().await.expect("read ordered");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].envelope.event_type,
            SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE
        );
        assert_eq!(
            events[0].envelope.schema_version,
            SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION
        );
        assert_eq!(events[0].envelope.stream_key, "system:31000001");
        assert_eq!(events[0].envelope.source, EventSource::Manual);

        let payload: SystemSignaturesObservedPayload =
            serde_json::from_str(&events[0].envelope.payload_json).expect("deserialize payload");
        assert_eq!(payload.system_id, SolarSystemId(31000001));
        assert_eq!(payload.snapshot_id, "snapshot-01");
        assert_eq!(payload.entries, entries);
    }

    #[tokio::test]
    async fn movement_and_signature_events_coexist_in_ordered_stream() {
        let temp_dir = tempdir().expect("tempdir");
        let database_path = temp_dir.path().join("events.sqlite");

        let store = EventLogStore::connect(&database_path)
            .await
            .expect("connect store");

        store
            .append_movement_event_at(
                &MovementEvent {
                    character_id: CharacterId(42),
                    from_system_id: None,
                    to_system_id: SolarSystemId(30000142),
                    observed_at: ts_secs(1_700_000_000),
                    source: MovementEventSource::Esi,
                },
                ts_millis(1_700_000_000_500),
            )
            .await
            .expect("append movement event");

        store
            .append_system_signatures_observed_event_at(
                &SystemSignaturesObservedEvent {
                    system_id: SolarSystemId(30000142),
                    snapshot_id: "snapshot-02".to_owned(),
                    entries: vec![SignatureEntry {
                        signature_id: "GJP-344".to_owned(),
                        group: "Cosmic Signature".to_owned(),
                        site_type: None,
                        name: None,
                        scan_percent: Some(0.9),
                    }],
                    observed_at: ts_secs(1_700_000_120),
                    attribution_character_id: Some(CharacterId(42)),
                    source: SignatureEventSource::Manual,
                },
                ts_millis(1_700_000_120_500),
            )
            .await
            .expect("append signature event");

        let events = store.read_ordered_events().await.expect("read ordered");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].envelope.event_type, CHARACTER_MOVED_EVENT_TYPE);
        assert_eq!(
            events[1].envelope.event_type,
            SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE
        );
    }

    fn ts_secs(value: i64) -> Timestamp {
        Timestamp::from_epoch_secs(value).expect("valid epoch seconds")
    }

    fn ts_millis(value: i64) -> Timestamp {
        Timestamp::from_epoch_millis(value).expect("valid epoch milliseconds")
    }
}
