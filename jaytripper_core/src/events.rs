use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    ids::{CharacterId, SolarSystemId},
    time::Timestamp,
};

pub const CHARACTER_MOVED_EVENT_TYPE: &str = "character_moved";
pub const CHARACTER_MOVED_SCHEMA_VERSION: i64 = 1;
pub const SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE: &str = "system_signatures_observed";
pub const SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION: i64 = 1;

pub fn character_stream_key(character_id: CharacterId) -> String {
    format!("character:{}", character_id.0)
}

pub fn system_stream_key(system_id: SolarSystemId) -> String {
    format!("system:{}", system_id.0)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterMovedPayload {
    pub from_system_id: Option<SolarSystemId>,
    pub to_system_id: SolarSystemId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignatureEntry {
    pub signature_id: String,
    pub group: String,
    pub site_type: Option<String>,
    pub name: Option<String>,
    pub scan_percent: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemSignaturesObservedPayload {
    pub system_id: SolarSystemId,
    pub snapshot_id: String,
    pub entries: Vec<SignatureEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementEventSource {
    Esi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureEventSource {
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovementEvent {
    pub character_id: CharacterId,
    pub from_system_id: Option<SolarSystemId>,
    pub to_system_id: SolarSystemId,
    pub observed_at: Timestamp,
    pub source: MovementEventSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemSignaturesObservedEvent {
    pub system_id: SolarSystemId,
    pub snapshot_id: String,
    pub entries: Vec<SignatureEntry>,
    pub observed_at: Timestamp,
    pub attribution_character_id: Option<CharacterId>,
    pub source: SignatureEventSource,
}

impl MovementEvent {
    pub fn as_character_moved_payload(&self) -> CharacterMovedPayload {
        CharacterMovedPayload {
            from_system_id: self.from_system_id,
            to_system_id: self.to_system_id,
        }
    }
}

impl SystemSignaturesObservedEvent {
    pub fn as_payload(&self) -> SystemSignaturesObservedPayload {
        SystemSignaturesObservedPayload {
            system_id: self.system_id,
            snapshot_id: self.snapshot_id.clone(),
            entries: self.entries.clone(),
        }
    }
}

#[async_trait]
pub trait MovementEventSink {
    type Error: Send + Sync + 'static;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error>;
}
