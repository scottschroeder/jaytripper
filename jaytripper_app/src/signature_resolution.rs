use jaytripper_core::ids::{CharacterId, SolarSystemId};

use crate::{app::SignatureSnapshotRecordContext, state::AppProjection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignatureTargetSystemResolution {
    Record {
        system_id: SolarSystemId,
        attribution_character_id: Option<CharacterId>,
    },
    NeedsConfirmation {
        focused_system_id: SolarSystemId,
        character_system_id: SolarSystemId,
        character_id: CharacterId,
    },
}

pub(crate) fn resolve_signature_target_system(
    projection: &AppProjection,
    context: SignatureSnapshotRecordContext,
) -> SignatureTargetSystemResolution {
    match context {
        SignatureSnapshotRecordContext::Explicit {
            system_id,
            attribution_character_id,
        } => SignatureTargetSystemResolution::Record {
            system_id,
            attribution_character_id,
        },
        SignatureSnapshotRecordContext::Auto {
            focused_system_id,
            attribution_character_id,
        } => {
            let character_system = attribution_character_id.and_then(|character_id| {
                projection
                    .characters
                    .get(&character_id)
                    .map(|status| status.current_system_id)
            });

            match (attribution_character_id, character_system) {
                (Some(character_id), Some(character_system_id))
                    if character_system_id != focused_system_id =>
                {
                    SignatureTargetSystemResolution::NeedsConfirmation {
                        focused_system_id,
                        character_system_id,
                        character_id,
                    }
                }
                (_, _) => SignatureTargetSystemResolution::Record {
                    system_id: focused_system_id,
                    attribution_character_id,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use jaytripper_core::{
        ids::{CharacterId, SolarSystemId},
        time::Timestamp,
    };

    use super::{SignatureTargetSystemResolution, resolve_signature_target_system};
    use crate::{
        app::SignatureSnapshotRecordContext,
        state::{AppProjection, CharacterLocationProjection},
    };

    #[test]
    fn explicit_context_always_records_explicit_system() {
        let projection = AppProjection::default();

        let result = resolve_signature_target_system(
            &projection,
            SignatureSnapshotRecordContext::Explicit {
                system_id: SolarSystemId(30002510),
                attribution_character_id: Some(CharacterId(42)),
            },
        );

        assert_eq!(
            result,
            SignatureTargetSystemResolution::Record {
                system_id: SolarSystemId(30002510),
                attribution_character_id: Some(CharacterId(42)),
            }
        );
    }

    #[test]
    fn auto_context_requests_confirmation_when_character_location_mismatches_focus() {
        let mut projection = AppProjection::default();
        projection.characters.insert(
            CharacterId(42),
            CharacterLocationProjection {
                current_system_id: SolarSystemId(30000142),
                last_movement_observed_at: ts(1_700_000_000),
            },
        );

        let result = resolve_signature_target_system(
            &projection,
            SignatureSnapshotRecordContext::Auto {
                focused_system_id: SolarSystemId(30002510),
                attribution_character_id: Some(CharacterId(42)),
            },
        );

        assert_eq!(
            result,
            SignatureTargetSystemResolution::NeedsConfirmation {
                focused_system_id: SolarSystemId(30002510),
                character_system_id: SolarSystemId(30000142),
                character_id: CharacterId(42),
            }
        );
    }

    #[test]
    fn auto_context_records_focused_system_when_character_unknown() {
        let projection = AppProjection::default();

        let result = resolve_signature_target_system(
            &projection,
            SignatureSnapshotRecordContext::Auto {
                focused_system_id: SolarSystemId(30000142),
                attribution_character_id: Some(CharacterId(9001)),
            },
        );

        assert_eq!(
            result,
            SignatureTargetSystemResolution::Record {
                system_id: SolarSystemId(30000142),
                attribution_character_id: Some(CharacterId(9001)),
            }
        );
    }

    fn ts(epoch_secs: i64) -> Timestamp {
        Timestamp::from_epoch_secs(epoch_secs).expect("valid epoch seconds")
    }
}
