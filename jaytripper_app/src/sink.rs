use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jaytripper_core::{MovementEvent, MovementEventSink};
use jaytripper_store::EventLogStore;

use crate::{AppError, CharacterTrackerSnapshot, state::apply_record};

#[derive(Clone)]
pub struct StoreAndStateMovementSink {
    pub(crate) store: EventLogStore,
    pub(crate) state: Arc<Mutex<CharacterTrackerSnapshot>>,
}

#[async_trait]
impl MovementEventSink for StoreAndStateMovementSink {
    type Error = AppError;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error> {
        self.store.emit_movement(event).await?;

        let since_seq = self
            .state
            .lock()
            .expect("state lock poisoned")
            .last_applied_global_seq
            .unwrap_or(0);

        let records = self.store.read_events_since(since_seq).await?;

        let mut snapshot = self.state.lock().expect("state lock poisoned");
        for record in &records {
            apply_record(&mut snapshot, record)?;
        }

        Ok(())
    }
}
