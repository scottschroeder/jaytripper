use async_trait::async_trait;
use jaytripper_core::{MovementEvent, MovementEventSink};

use crate::{AppError, app::AppRuntime};

#[derive(Clone)]
pub(crate) struct AppMovementSink {
    app: AppRuntime,
}

impl AppMovementSink {
    pub(crate) fn new(app: AppRuntime) -> Self {
        Self { app }
    }
}

#[async_trait]
impl MovementEventSink for AppMovementSink {
    type Error = AppError;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error> {
        self.app.store().append_movement_event(&event).await?;
        self.app.catch_up_projection_from_store().await
    }
}

#[cfg(test)]
mod tests {
    use jaytripper_core::{
        MovementEvent, MovementEventSink, MovementEventSource,
        ids::{CharacterId, SolarSystemId},
        time::Timestamp,
    };
    use tempfile::tempdir;

    use super::AppMovementSink;
    use crate::app::AppRuntime;

    #[tokio::test]
    async fn emit_movement_updates_store_and_projection() {
        let temp_dir = tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("events.sqlite");
        let app = AppRuntime::connect(&db_path).await.expect("connect app");
        let sink = AppMovementSink::new(app.clone());

        sink.emit_movement(MovementEvent {
            character_id: CharacterId(1337),
            from_system_id: None,
            to_system_id: SolarSystemId(30002053),
            observed_at: ts(1_700_000_777),
            source: MovementEventSource::Esi,
        })
        .await
        .expect("emit movement");

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

    fn ts(epoch_secs: i64) -> Timestamp {
        Timestamp::from_epoch_secs(epoch_secs).expect("valid epoch seconds")
    }
}
