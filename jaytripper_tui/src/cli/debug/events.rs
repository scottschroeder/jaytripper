use std::path::PathBuf;

use clap::Args;
use jaytripper_store::EventLogStore;

#[derive(Debug, Args)]
pub(crate) struct EventsCommand {
    #[arg(long, default_value = "jaytripper.sqlite")]
    db: PathBuf,

    #[arg(long)]
    since: Option<i64>,

    #[arg(long)]
    stream: Option<String>,

    #[arg(long)]
    limit: Option<usize>,
}

impl EventsCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        let store = EventLogStore::connect(&self.db).await?;

        let mut records = if let Some(stream_key) = &self.stream {
            store.read_events_by_stream(stream_key).await?
        } else if let Some(since_seq) = self.since {
            store.read_events_since(since_seq).await?
        } else {
            store.read_ordered_events().await?
        };

        if self.stream.is_some()
            && let Some(since_seq) = self.since
        {
            records.retain(|record| record.global_seq > since_seq);
        }

        let total = records.len();
        if let Some(limit) = self.limit
            && records.len() > limit
        {
            let keep_from = records.len() - limit;
            records = records.split_off(keep_from);
        }

        println!(
            "Showing {} event(s) from {} (matched {total} before limit)",
            records.len(),
            self.db.display()
        );

        for record in records {
            let attribution = record
                .envelope
                .attribution_character_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "<none>".to_owned());

            println!(
                "seq={} type={} stream={} source={:?} occurred={} character={} id={}",
                record.global_seq,
                record.envelope.event_type,
                record.envelope.stream_key,
                record.envelope.source,
                record.envelope.occurred_at.as_epoch_secs(),
                attribution,
                record.envelope.event_id,
            );
        }

        Ok(())
    }
}
