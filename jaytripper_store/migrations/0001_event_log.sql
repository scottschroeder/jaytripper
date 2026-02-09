CREATE TABLE IF NOT EXISTS event_log (
    global_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    event_type TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    stream_key TEXT NOT NULL,
    occurred_at_epoch_millis INTEGER NOT NULL,
    recorded_at_epoch_millis INTEGER NOT NULL,
    attribution_character_id INTEGER,
    source TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_event_log_stream_key_global_seq
    ON event_log(stream_key, global_seq);

CREATE INDEX IF NOT EXISTS idx_event_log_recorded_at
    ON event_log(recorded_at_epoch_millis);
