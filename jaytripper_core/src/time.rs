use std::{fmt, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub fn from_epoch_secs(epoch_secs: i64) -> Option<Self> {
        DateTime::from_timestamp(epoch_secs, 0).map(Self)
    }

    pub fn from_epoch_millis(epoch_millis: i64) -> Option<Self> {
        DateTime::from_timestamp_millis(epoch_millis).map(Self)
    }

    pub fn as_epoch_secs(self) -> i64 {
        self.0.timestamp()
    }

    pub fn as_epoch_millis(self) -> i64 {
        self.0.timestamp_millis()
    }

    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let chrono_duration = chrono::Duration::from_std(duration).ok()?;
        self.0.checked_add_signed(chrono_duration).map(Self)
    }

    pub fn signed_duration_since(self, earlier: Self) -> chrono::Duration {
        self.0.signed_duration_since(earlier.0)
    }
}

impl std::fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
