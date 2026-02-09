use jaytripper_core::ids::CharacterId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRequest {
    pub authorization_url: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSession {
    pub character_id: CharacterId,
    pub character_name: Option<String>,
    pub scopes: Vec<String>,
    pub access_token: String,
    pub access_expires_at_epoch_secs: i64,
    pub refresh_token: String,
    pub updated_at_epoch_secs: i64,
}

impl AuthSession {
    pub fn should_refresh(&self, now_epoch_secs: i64, refresh_skew_secs: i64) -> bool {
        self.access_expires_at_epoch_secs <= now_epoch_secs + refresh_skew_secs
    }
}
