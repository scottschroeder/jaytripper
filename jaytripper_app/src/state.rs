use std::collections::HashMap;

use jaytripper_core::{
    CHARACTER_MOVED_EVENT_TYPE, CHARACTER_MOVED_SCHEMA_VERSION, CharacterMovedPayload,
    ProjectedSignature, SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE,
    SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION, SystemSignaturesObservedPayload, Timestamp,
    ids::{CharacterId, SolarSystemId},
    merge_signature_snapshot,
};
use jaytripper_store::{EventRecord, EventSource, GlobalSeq};

use crate::AppError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CharacterLocationProjection {
    pub(crate) current_system_id: SolarSystemId,
    pub(crate) last_movement_observed_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotMetadata {
    pub(crate) snapshot_id: String,
    pub(crate) observed_at: Timestamp,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SystemSignaturesProjection {
    pub(crate) last_snapshot: Option<SnapshotMetadata>,
    pub(crate) signatures_by_id: HashMap<String, ProjectedSignature>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AppProjection {
    pub(crate) characters: HashMap<CharacterId, CharacterLocationProjection>,
    pub(crate) signatures_by_system: HashMap<SolarSystemId, SystemSignaturesProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventMetadata {
    pub(crate) global_seq: GlobalSeq,
    pub(crate) occurred_at: Timestamp,
    pub(crate) event_id: String,
    pub(crate) source: EventSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CharacterMovedProjectionEvent {
    pub(crate) character_id: CharacterId,
    pub(crate) payload: CharacterMovedPayload,
}

pub(crate) trait ProjectionReducer<E> {
    fn project(&mut self, event: &E, metadata: &EventMetadata) -> Result<(), AppError>;
}

impl ProjectionReducer<CharacterMovedProjectionEvent> for AppProjection {
    fn project(
        &mut self,
        event: &CharacterMovedProjectionEvent,
        metadata: &EventMetadata,
    ) -> Result<(), AppError> {
        self.characters.insert(
            event.character_id,
            CharacterLocationProjection {
                current_system_id: event.payload.to_system_id,
                last_movement_observed_at: metadata.occurred_at,
            },
        );
        Ok(())
    }
}

impl ProjectionReducer<SystemSignaturesObservedPayload> for AppProjection {
    fn project(
        &mut self,
        event: &SystemSignaturesObservedPayload,
        metadata: &EventMetadata,
    ) -> Result<(), AppError> {
        let system_projection = self
            .signatures_by_system
            .entry(event.system_id)
            .or_default();

        system_projection.last_snapshot = Some(SnapshotMetadata {
            snapshot_id: event.snapshot_id.clone(),
            observed_at: metadata.occurred_at,
        });
        merge_signature_snapshot(&mut system_projection.signatures_by_id, &event.entries);

        Ok(())
    }
}

pub(crate) fn project_event_record(
    projection: &mut AppProjection,
    record: &EventRecord,
) -> Result<(), AppError> {
    let envelope = &record.envelope;
    let metadata = EventMetadata {
        global_seq: record.global_seq,
        occurred_at: envelope.occurred_at,
        event_id: envelope.event_id.clone(),
        source: envelope.source,
    };

    match envelope.event_type.as_str() {
        CHARACTER_MOVED_EVENT_TYPE => {
            if envelope.schema_version != CHARACTER_MOVED_SCHEMA_VERSION {
                return Err(AppError::UnsupportedSchemaVersion {
                    event_type: envelope.event_type.clone(),
                    schema_version: envelope.schema_version,
                });
            }

            let payload: CharacterMovedPayload = serde_json::from_str(&envelope.payload_json)?;
            let character_id = envelope.attribution_character_id.ok_or_else(|| {
                AppError::MissingCharacterAttribution {
                    event_type: envelope.event_type.clone(),
                    global_seq: record.global_seq,
                }
            })?;

            projection.project(
                &CharacterMovedProjectionEvent {
                    character_id,
                    payload,
                },
                &metadata,
            )?;
        }
        SYSTEM_SIGNATURES_OBSERVED_EVENT_TYPE => {
            if envelope.schema_version != SYSTEM_SIGNATURES_OBSERVED_SCHEMA_VERSION {
                return Err(AppError::UnsupportedSchemaVersion {
                    event_type: envelope.event_type.clone(),
                    schema_version: envelope.schema_version,
                });
            }

            let payload: SystemSignaturesObservedPayload =
                serde_json::from_str(&envelope.payload_json)?;
            projection.project(&payload, &metadata)?;
        }
        _ => {}
    }

    Ok(())
}
