#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("store error: {0}")]
    Store(#[from] jaytripper_store::StoreError),

    #[error("esi error: {0}")]
    Esi(#[from] jaytripper_esi::EsiError),

    #[error("payload serialization error: {0}")]
    PayloadSerialization(#[from] serde_json::Error),

    #[error("signature parse error: {0}")]
    SignatureParse(#[from] jaytripper_core::SignatureParseError),

    #[error("unsupported schema version {schema_version} for event type '{event_type}'")]
    UnsupportedSchemaVersion {
        event_type: String,
        schema_version: i64,
    },

    #[error("missing character attribution for event type '{event_type}' at sequence {global_seq}")]
    MissingCharacterAttribution {
        event_type: String,
        global_seq: jaytripper_store::GlobalSeq,
    },
}
