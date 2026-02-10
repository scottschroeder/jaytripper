use std::collections::HashMap;

use jaytripper_core::{
    CHARACTER_MOVED_EVENT_TYPE, CHARACTER_MOVED_SCHEMA_VERSION, CharacterMovedPayload, Timestamp,
    ids::{CharacterId, SolarSystemId},
};
use jaytripper_store::EventRecord;

use crate::AppError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterStatus {
    pub current_system_id: SolarSystemId,
    pub last_movement_observed_at: Timestamp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterTrackerSnapshot {
    pub last_applied_global_seq: Option<i64>,
    pub characters: HashMap<CharacterId, CharacterStatus>,
}

pub(crate) fn apply_record(
    snapshot: &mut CharacterTrackerSnapshot,
    record: &EventRecord,
) -> Result<(), AppError> {
    let envelope = &record.envelope;

    if envelope.event_type != CHARACTER_MOVED_EVENT_TYPE {
        snapshot.last_applied_global_seq = Some(record.global_seq);
        return Ok(());
    }

    if envelope.schema_version != CHARACTER_MOVED_SCHEMA_VERSION {
        return Err(AppError::UnsupportedSchemaVersion {
            event_type: envelope.event_type.clone(),
            schema_version: envelope.schema_version,
        });
    }

    let payload: CharacterMovedPayload = serde_json::from_str(&envelope.payload_json)?;
    let character_id =
        envelope
            .attribution_character_id
            .ok_or_else(|| AppError::MissingCharacterAttribution {
                event_type: envelope.event_type.clone(),
                global_seq: record.global_seq,
            })?;

    snapshot.characters.insert(
        character_id,
        CharacterStatus {
            current_system_id: payload.to_system_id,
            last_movement_observed_at: envelope.occurred_at,
        },
    );
    snapshot.last_applied_global_seq = Some(record.global_seq);

    Ok(())
}
