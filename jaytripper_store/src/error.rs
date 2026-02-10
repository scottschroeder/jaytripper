#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("invalid event source '{0}'")]
    InvalidEventSource(String),

    #[error("character id {0} does not fit into sqlite INTEGER")]
    CharacterIdOverflow(u64),

    #[error("character id {0} is negative in sqlite record")]
    NegativeCharacterId(i64),

    #[error("invalid unix epoch milliseconds value: {0}")]
    InvalidEpochMillis(i64),

    #[error("payload serialization failed: {0}")]
    PayloadSerialization(#[from] serde_json::Error),
}
