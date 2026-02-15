use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;
use jaytripper_app::{
    AppError, AppRuntime, SignatureSnapshotRecordContext, SignatureSnapshotRecordOutcome,
};
use jaytripper_core::{
    SignatureEntry,
    ids::{CharacterId, SolarSystemId},
};
use jaytripper_esi::{CharacterLocation, EsiClient, EsiError, LocationPollConfig};
use jaytripper_store::{EventEnvelope, EventSource};
use tokio::sync::watch;

mod support;

use support::{TestHarness, ts};

struct MockEsiClient {
    character_id: CharacterId,
    responses: Mutex<VecDeque<Result<CharacterLocation, EsiError>>>,
}

#[async_trait]
impl EsiClient for MockEsiClient {
    fn character_id(&self) -> CharacterId {
        self.character_id
    }

    fn requires_reauth(&self) -> bool {
        false
    }

    fn reauth_reason(&self) -> Option<String> {
        None
    }

    async fn get_current_location(&self) -> Result<CharacterLocation, EsiError> {
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .unwrap_or_else(|| Err(EsiError::message("no response configured")))
    }
}

#[tokio::test]
async fn track_latest_system_for_character() {
    let h = TestHarness::new();
    h.append_movement(
        CharacterId(42),
        None,
        SolarSystemId(30000142),
        ts(1_700_000_000),
    )
    .await;
    h.append_movement(
        CharacterId(42),
        Some(SolarSystemId(30000142)),
        SolarSystemId(30002510),
        ts(1_700_000_120),
    )
    .await;

    let app = h.app().await;
    assert_eq!(
        app.character_current_system(CharacterId(42)).await,
        Some(SolarSystemId(30002510))
    );
}

#[tokio::test]
async fn run_ingestion_until_shutdown_returns_when_shutdown_already_signaled() {
    let h = TestHarness::new();
    let app = h.app().await;

    let client = MockEsiClient {
        character_id: CharacterId(4242),
        responses: Mutex::new(VecDeque::new()),
    };
    let config = LocationPollConfig::default();
    let (_shutdown_tx, shutdown_rx) = watch::channel(true);

    app.run_ingestion_until_shutdown(client, config, shutdown_rx)
        .await
        .expect("ingestion run should honor immediate shutdown");

    assert_eq!(h.ordered_events().await.len(), 0);
}

#[tokio::test]
async fn record_signature_snapshot_auto_uses_focused_without_character_location() {
    let h = TestHarness::new();
    let app = h.app().await;

    let outcome = app
        .record_signature_snapshot(
            SignatureSnapshotRecordContext::Auto {
                focused_system_id: SolarSystemId(30000142),
                attribution_character_id: Some(CharacterId(9001)),
            },
            "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
        )
        .await
        .expect("record signatures");

    assert_eq!(
        outcome,
        SignatureSnapshotRecordOutcome::Recorded {
            system_id: SolarSystemId(30000142),
        }
    );
    assert_eq!(h.ordered_events().await.len(), 1);
}

#[tokio::test]
async fn record_signature_snapshot_auto_requests_confirmation_when_mismatch() {
    let h = TestHarness::new();
    h.append_movement(
        CharacterId(42),
        None,
        SolarSystemId(30000142),
        ts(1_700_000_000),
    )
    .await;
    let app = h.app().await;

    let outcome = app
        .record_signature_snapshot(
            SignatureSnapshotRecordContext::Auto {
                focused_system_id: SolarSystemId(30002510),
                attribution_character_id: Some(CharacterId(42)),
            },
            "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
        )
        .await
        .expect("record call should not fail");

    assert_eq!(
        outcome,
        SignatureSnapshotRecordOutcome::NeedsConfirmation {
            focused_system_id: SolarSystemId(30002510),
            character_system_id: SolarSystemId(30000142),
            character_id: CharacterId(42),
        }
    );
    assert_eq!(h.ordered_events().await.len(), 1);
}

#[tokio::test]
async fn record_signature_snapshot_explicit_applies_even_when_character_mismatch() {
    let h = TestHarness::new();
    h.append_movement(
        CharacterId(42),
        None,
        SolarSystemId(30000142),
        ts(1_700_000_000),
    )
    .await;
    let app = h.app().await;

    let outcome = app
        .record_signature_snapshot(
            SignatureSnapshotRecordContext::Explicit {
                system_id: SolarSystemId(30002510),
                attribution_character_id: Some(CharacterId(42)),
            },
            "ABC-123\tCosmic Signature\tGas Site\t\t10.0%\n",
        )
        .await
        .expect("record signatures");

    assert_eq!(
        outcome,
        SignatureSnapshotRecordOutcome::Recorded {
            system_id: SolarSystemId(30002510),
        }
    );
}

#[tokio::test]
async fn record_signature_snapshot_returns_parse_error() {
    let h = TestHarness::new();
    let app = h.app().await;

    let err = app
        .record_signature_snapshot(
            SignatureSnapshotRecordContext::Auto {
                focused_system_id: SolarSystemId(30000142),
                attribution_character_id: None,
            },
            "BAD\tCosmic Signature\tGas Site\t\t10.0%\n",
        )
        .await
        .expect_err("expected parse error");

    assert!(matches!(
        err,
        AppError::SignatureParse(jaytripper_core::SignatureParseError::InvalidSignatureId { .. })
    ));
}

#[tokio::test]
async fn unknown_event_type_is_skipped_and_projection_still_replays() {
    let h = TestHarness::new();
    h.append_movement(
        CharacterId(9001),
        None,
        SolarSystemId(30000142),
        ts(1_700_000_000),
    )
    .await;
    h.store()
        .await
        .append_event(&EventEnvelope {
            event_id: uuid::Uuid::now_v7().to_string(),
            event_type: "some_future_event".to_owned(),
            schema_version: 1,
            stream_key: "future:stream".to_owned(),
            occurred_at: ts(1_700_000_010),
            recorded_at: ts(1_700_000_010),
            attribution_character_id: None,
            source: EventSource::Import,
            payload_json: "{\"hello\":\"world\"}".to_owned(),
        })
        .await
        .expect("append unknown event");

    let app = h.app().await;

    assert_eq!(
        app.character_current_system(CharacterId(9001)).await,
        Some(SolarSystemId(30000142))
    );
}

#[tokio::test]
async fn restart_preserves_character_positions_from_mixed_stream() {
    let h = TestHarness::new();
    h.append_movement(
        CharacterId(42),
        None,
        SolarSystemId(30000142),
        ts(1_700_000_000),
    )
    .await;
    h.append_movement(
        CharacterId(42),
        Some(SolarSystemId(30000142)),
        SolarSystemId(30002510),
        ts(1_700_000_060),
    )
    .await;
    h.append_movement(
        CharacterId(100),
        None,
        SolarSystemId(30005196),
        ts(1_700_000_090),
    )
    .await;
    h.append_signature_snapshot(
        SolarSystemId(30000142),
        "snap-a1",
        vec![SignatureEntry {
            signature_id: "ABC-123".to_owned(),
            group: "Cosmic Signature".to_owned(),
            site_type: Some("Gas Site".to_owned()),
            name: None,
            scan_percent: Some(70.0),
        }],
        Some(CharacterId(42)),
        ts(1_700_000_120),
    )
    .await;

    let app_before_restart = AppRuntime::connect(h.db_path())
        .await
        .expect("connect app before restart");
    let before_char_42 = app_before_restart
        .character_current_system(CharacterId(42))
        .await;
    let before_char_100 = app_before_restart
        .character_current_system(CharacterId(100))
        .await;

    let app_after_restart = AppRuntime::connect(h.db_path())
        .await
        .expect("connect app after restart");
    let after_char_42 = app_after_restart
        .character_current_system(CharacterId(42))
        .await;
    let after_char_100 = app_after_restart
        .character_current_system(CharacterId(100))
        .await;

    assert_eq!(before_char_42, after_char_42);
    assert_eq!(before_char_100, after_char_100);
}
