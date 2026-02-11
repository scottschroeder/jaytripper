use std::path::PathBuf;

use clap::{Args, Subcommand};
use jaytripper_app::CharacterTrackerApp;

#[derive(Debug, Args)]
pub(crate) struct StateCommand {
    #[command(subcommand)]
    subcmd: StateSubcommand,
}

#[derive(Debug, Subcommand)]
enum StateSubcommand {
    /// Print character tracker snapshot from replayed store.
    Snapshot(SnapshotCommand),
}

impl StateCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            StateSubcommand::Snapshot(cmd) => cmd.run().await,
        }
    }
}

#[derive(Debug, Args)]
struct SnapshotCommand {
    #[arg(long, default_value = "jaytripper.sqlite")]
    db: PathBuf,
}

impl SnapshotCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let app = CharacterTrackerApp::connect(&self.db).await?;
        let snapshot = app.snapshot();

        println!("DB: {}", self.db.display());
        println!(
            "last_applied_global_seq: {}",
            snapshot
                .last_applied_global_seq
                .map(|seq| seq.to_string())
                .unwrap_or_else(|| "<none>".to_owned())
        );
        println!("characters: {}", snapshot.characters.len());

        let mut rows: Vec<_> = snapshot.characters.into_iter().collect();
        rows.sort_by_key(|(character_id, _)| character_id.0);
        for (character_id, status) in rows {
            println!(
                "character={} current_system={} observed_at={}",
                character_id,
                status.current_system_id,
                status.last_movement_observed_at.as_epoch_secs(),
            );
        }

        Ok(())
    }
}
