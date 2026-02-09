use jaytripper_core::ids::CharacterId;
use keyring::Entry;

use crate::{EsiResult, auth::AuthSession};

pub trait TokenStore {
    fn load_session(&self, character_id: CharacterId) -> EsiResult<Option<AuthSession>>;
    fn save_session(&self, session: &AuthSession) -> EsiResult<()>;
    fn clear_session(&self, character_id: CharacterId) -> EsiResult<()>;
}

#[derive(Clone, Debug)]
pub struct KeyringTokenStore {
    service: String,
    account_prefix: String,
}

impl KeyringTokenStore {
    pub fn new(service: impl Into<String>, account_prefix: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account_prefix: account_prefix.into(),
        }
    }

    fn account_for_character(&self, character_id: CharacterId) -> String {
        format!("{}:character:{character_id}", self.account_prefix)
    }

    fn entry_for_character(&self, character_id: CharacterId) -> EsiResult<Entry> {
        Ok(Entry::new(
            &self.service,
            &self.account_for_character(character_id),
        )?)
    }
}

impl TokenStore for KeyringTokenStore {
    fn load_session(&self, character_id: CharacterId) -> EsiResult<Option<AuthSession>> {
        let entry = self.entry_for_character(character_id)?;
        match entry.get_password() {
            Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn save_session(&self, session: &AuthSession) -> EsiResult<()> {
        let entry = self.entry_for_character(session.character_id)?;
        let raw = serde_json::to_string(session)?;
        entry.set_password(&raw)?;
        Ok(())
    }

    fn clear_session(&self, character_id: CharacterId) -> EsiResult<()> {
        let entry = self.entry_for_character(character_id)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}
