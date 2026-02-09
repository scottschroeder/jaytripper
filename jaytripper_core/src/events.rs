use async_trait::async_trait;

use crate::ids::{CharacterId, SolarSystemId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementEventSource {
    Esi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovementEvent {
    pub character_id: CharacterId,
    pub from_system_id: Option<SolarSystemId>,
    pub to_system_id: SolarSystemId,
    pub observed_at_epoch_secs: i64,
    pub source: MovementEventSource,
}

#[async_trait]
pub trait MovementEventSink {
    type Error: Send + Sync + 'static;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error>;
}
