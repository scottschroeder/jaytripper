use std::time::{SystemTime, UNIX_EPOCH};

use jaytripper_esi::{AuthSession, KeyringTokenStore, TokenStore};

const TEST_SERVICE: &str = "jaytripper-keyring-integration-tests";

#[test]
fn keyring_round_trip_save_load_clear() {
    let account = format!(
        "session-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    );
    let store = KeyringTokenStore::new(TEST_SERVICE, account.clone());

    store
        .clear_session()
        .expect("cleanup before test should succeed");

    let session = AuthSession {
        character_id: 123_456_789,
        character_name: Some("Integration Pilot".to_string()),
        scopes: vec![
            "publicData".to_string(),
            "esi-location.read_location.v1".to_string(),
        ],
        access_token: "access-token-test".to_string(),
        access_expires_at_epoch_secs: 2_000_000_000,
        refresh_token: "refresh-token-test".to_string(),
        updated_at_epoch_secs: 1_900_000_000,
    };

    store
        .save_session(&session)
        .expect("saving session in keyring should succeed");

    let loaded = store
        .load_session()
        .expect("loading session from keyring should succeed");
    assert_eq!(loaded, Some(session));

    store
        .clear_session()
        .expect("clearing session in keyring should succeed");

    let loaded_after_clear = store
        .load_session()
        .expect("loading after clear should succeed");
    assert_eq!(loaded_after_clear, None);
}
