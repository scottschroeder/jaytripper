use crate::{EsiError, EsiResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsiConfig {
    pub client_id: String,
    pub callback_url: String,
    pub scopes: Vec<String>,
    pub user_agent: String,
}

impl EsiConfig {
    pub fn validate(&self) -> EsiResult<()> {
        if self.client_id.trim().is_empty() {
            return Err(EsiError::InvalidConfig("EVE_CLIENT_ID must be set"));
        }
        if self.callback_url.trim().is_empty() {
            return Err(EsiError::InvalidConfig("EVE_CALLBACK_URL must be set"));
        }
        if self.scopes.is_empty() {
            return Err(EsiError::InvalidConfig(
                "at least one ESI scope must be configured",
            ));
        }
        if self.user_agent.trim().is_empty() {
            return Err(EsiError::InvalidConfig("user_agent must be set"));
        }
        Ok(())
    }

    pub fn scopes_for_esi(&self) -> String {
        self.scopes.join(" ")
    }
}
