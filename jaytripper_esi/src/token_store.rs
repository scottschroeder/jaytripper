use keyring::Entry;

use crate::{EsiResult, types::AuthSession};

pub trait TokenStore {
    fn load_session(&self) -> EsiResult<Option<AuthSession>>;
    fn save_session(&self, session: &AuthSession) -> EsiResult<()>;
    fn clear_session(&self) -> EsiResult<()>;
}

#[derive(Clone, Debug)]
pub struct KeyringTokenStore {
    service: String,
    account: String,
}

impl KeyringTokenStore {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    fn entry(&self) -> EsiResult<Entry> {
        Ok(Entry::new(&self.service, &self.account)?)
    }
}

impl TokenStore for KeyringTokenStore {
    fn load_session(&self) -> EsiResult<Option<AuthSession>> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn save_session(&self, session: &AuthSession) -> EsiResult<()> {
        let entry = self.entry()?;
        let raw = serde_json::to_string(session)?;
        entry.set_password(&raw)?;
        Ok(())
    }

    fn clear_session(&self) -> EsiResult<()> {
        let entry = self.entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}
