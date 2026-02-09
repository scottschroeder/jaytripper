use std::{path::Path, str::FromStr, time::Duration};

use futures_util::TryStreamExt;
use jaytripper_core::ids::CharacterId;
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
    pub occurred_at_epoch_millis: i64,
    pub recorded_at_epoch_millis: i64,
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
            event.occurred_at_epoch_millis,
            event.recorded_at_epoch_millis,
            attribution_character_id,
            source,
            payload_json,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(inserted.global_seq)
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
                occurred_at_epoch_millis: value.occurred_at_epoch_millis,
                recorded_at_epoch_millis: value.recorded_at_epoch_millis,
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

#[cfg(test)]
mod tests {
    use jaytripper_core::ids::CharacterId;
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
                occurred_at_epoch_millis: 1_700_000_000_123,
                recorded_at_epoch_millis: 1_700_000_005_123,
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
                occurred_at_epoch_millis: 1_700_000_100_456,
                recorded_at_epoch_millis: 1_700_000_101_456,
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
}
