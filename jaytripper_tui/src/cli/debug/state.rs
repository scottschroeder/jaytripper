use std::path::PathBuf;

use clap::{Args, Subcommand};
use jaytripper_app::AppRuntime;

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
        let app = AppRuntime::connect(&self.db).await?;
        let mut rows = app.character_locations().await;
        rows.sort_by_key(|row| row.character_id.0);

        println!("DB: {}", self.db.display());
        println!("characters: {}", rows.len());

        for row in rows {
            println!(
                "character={} current_system={} observed_at={}",
                row.character_id,
                row.current_system_id,
                row.last_movement_observed_at.as_epoch_secs(),
            );
        }

        Ok(())
    }
}
