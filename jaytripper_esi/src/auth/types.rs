use std::time::Duration;

use jaytripper_core::{ids::CharacterId, time::Timestamp};
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
    pub access_expires_at: Timestamp,
    pub refresh_token: String,
    pub updated_at: Timestamp,
}

impl AuthSession {
    pub fn should_refresh(&self, now: Timestamp, refresh_skew: Duration) -> bool {
        match now.checked_add(refresh_skew) {
            Some(deadline) => self.access_expires_at <= deadline,
            None => true,
        }
    }
}
