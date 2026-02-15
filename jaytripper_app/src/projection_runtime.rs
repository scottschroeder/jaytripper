use jaytripper_store::{EventRecord, GlobalSeq};

use crate::{
    AppError,
    state::{AppProjection, project_event_record},
};

#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct ProjectionRuntimeState {
    pub(crate) projection: AppProjection,
    pub(crate) last_projected_seq: Option<GlobalSeq>,
}

pub(crate) fn project_records_with_monotonic_guard(
    state: &mut ProjectionRuntimeState,
    records: &[EventRecord],
) -> Result<(), AppError> {
    for record in records {
        if let Some(last_seq) = state.last_projected_seq
            && record.global_seq <= last_seq
        {
            continue;
        }

        project_event_record(&mut state.projection, record)?;
        state.last_projected_seq = Some(record.global_seq);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use jaytripper_core::{
        CHARACTER_MOVED_EVENT_TYPE, CHARACTER_MOVED_SCHEMA_VERSION, CharacterMovedPayload,
        ids::{CharacterId, SolarSystemId},
        time::Timestamp,
    };
    use jaytripper_store::{EventEnvelope, EventRecord, EventSource, GlobalSeq};

    use super::{ProjectionRuntimeState, project_records_with_monotonic_guard};

    #[test]
    fn skips_stale_records_after_newer_record_applies() {
        let mut state = ProjectionRuntimeState::default();
        let newer = movement_record(
            3,
            CharacterId(77),
            Some(SolarSystemId(30002510)),
            SolarSystemId(30002645),
        );
        let stale = movement_record(
            2,
            CharacterId(77),
            Some(SolarSystemId(30000142)),
            SolarSystemId(30002510),
        );

        project_records_with_monotonic_guard(&mut state, &[newer]).expect("apply newer record");
        project_records_with_monotonic_guard(&mut state, &[stale]).expect("skip stale record");

        assert_eq!(state.last_projected_seq, Some(GlobalSeq(3)));
        assert_eq!(
            state
                .projection
                .characters
                .get(&CharacterId(77))
                .expect("character projection")
                .current_system_id,
            SolarSystemId(30002645)
        );
    }

    #[test]
    fn monotonically_advances_sequence_for_in_order_batch() {
        let mut state = ProjectionRuntimeState::default();
        let first = movement_record(1, CharacterId(1), None, SolarSystemId(30000142));
        let second = movement_record(
            2,
            CharacterId(1),
            Some(SolarSystemId(30000142)),
            SolarSystemId(30002510),
        );

        project_records_with_monotonic_guard(&mut state, &[first, second])
            .expect("apply in-order batch");

        assert_eq!(state.last_projected_seq, Some(GlobalSeq(2)));
    }

    fn movement_record(
        seq: i64,
        character_id: CharacterId,
        from_system_id: Option<SolarSystemId>,
        to_system_id: SolarSystemId,
    ) -> EventRecord {
        let payload = CharacterMovedPayload {
            from_system_id,
            to_system_id,
        };

        EventRecord {
            global_seq: GlobalSeq(seq),
            envelope: EventEnvelope {
                event_id: format!("evt-{seq}"),
                event_type: CHARACTER_MOVED_EVENT_TYPE.to_owned(),
                schema_version: CHARACTER_MOVED_SCHEMA_VERSION,
                stream_key: format!("character:{}", character_id.0),
                occurred_at: ts(1_700_000_000 + seq),
                recorded_at: ts(1_700_000_000 + seq),
                attribution_character_id: Some(character_id),
                source: EventSource::Esi,
                payload_json: serde_json::to_string(&payload).expect("serialize payload"),
            },
        }
    }

    fn ts(epoch_secs: i64) -> Timestamp {
        Timestamp::from_epoch_secs(epoch_secs).expect("valid epoch seconds")
    }
}
