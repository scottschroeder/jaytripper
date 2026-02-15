mod error;
mod event_log;

pub use error::StoreError;
pub use event_log::{EventEnvelope, EventLogStore, EventRecord, EventSource, GlobalSeq, NewEvent};
