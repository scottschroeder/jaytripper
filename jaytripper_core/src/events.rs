use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::ids::{CharacterId, SolarSystemId};

pub const CHARACTER_MOVED_EVENT_TYPE: &str = "character_moved";
pub const CHARACTER_MOVED_SCHEMA_VERSION: i64 = 1;

pub fn character_stream_key(character_id: CharacterId) -> String {
    format!("character:{}", character_id.0)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterMovedPayload {
    pub from_system_id: Option<SolarSystemId>,
    pub to_system_id: SolarSystemId,
}

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

impl MovementEvent {
    pub fn as_character_moved_payload(&self) -> CharacterMovedPayload {
        CharacterMovedPayload {
            from_system_id: self.from_system_id,
            to_system_id: self.to_system_id,
        }
    }
}

#[async_trait]
pub trait MovementEventSink {
    type Error: Send + Sync + 'static;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error>;
}
