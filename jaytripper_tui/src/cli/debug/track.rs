use std::{path::PathBuf, time::Instant};

use clap::{Args, Subcommand};
use jaytripper_app::CharacterTrackerApp;
use jaytripper_esi::LocationPollConfig;
use tokio::{sync::watch, time::Duration};

use crate::cli::debug::common::{build_auth_service, load_esi_config, required_character_id};

#[derive(Debug, Args)]
pub(crate) struct TrackCommand {
    #[command(subcommand)]
    subcmd: TrackSubcommand,
}

#[derive(Debug, Subcommand)]
enum TrackSubcommand {
    /// Run ingestion until Ctrl+C.
    Run(RunCommand),
}

impl TrackCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            TrackSubcommand::Run(cmd) => cmd.run().await,
        }
    }
}

#[derive(Debug, Args)]
struct RunCommand {
    #[arg(long)]
    character_id: Option<u64>,

    #[arg(long, default_value = "jaytripper.sqlite")]
    db: PathBuf,
}

impl RunCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let character_id = required_character_id(self.character_id)?;
        let config = load_esi_config("jaytripper-tui-track/0.1")?;

        println!("Tracking character {character_id}.");
        println!("Persisting events to {}", self.db.display());

        let app = CharacterTrackerApp::connect(&self.db).await?;
        let auth = build_auth_service(&config)?;
        let esi_client = auth.connect_character(character_id).await?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let app_for_task = app.clone();
        let mut ingestion_task = tokio::spawn(async move {
            app_for_task
                .run_ingestion_until_shutdown(
                    esi_client,
                    LocationPollConfig::default(),
                    shutdown_rx,
                )
                .await
        });

        println!("Listening for movement updates. Press Ctrl+C to stop.");

        let mut last_system = None;
        let mut last_wait_log = Instant::now();
        loop {
            tokio::select! {
                outcome = &mut ingestion_task => {
                    match outcome {
                        Ok(Ok(())) => {
                            eprintln!("Ingestion loop exited cleanly.");
                            break;
                        }
                        Ok(Err(error)) => {
                            anyhow::bail!("ingestion loop failed: {error}");
                        }
                        Err(error) => {
                            anyhow::bail!("ingestion task join failed: {error}");
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("Stopping tracker...");
                    let _ = shutdown_tx.send(true);
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let snapshot = app.snapshot();
                    if let Some(status) = snapshot.characters.get(&character_id) {
                        if last_system != Some(status.current_system_id) {
                            last_system = Some(status.current_system_id);
                            println!("character {character_id} -> system {}", status.current_system_id);
                        }
                        last_wait_log = Instant::now();
                    } else if last_wait_log.elapsed() >= Duration::from_secs(5) {
                        eprintln!("waiting for first movement event for character {character_id}...");
                        last_wait_log = Instant::now();
                    }
                }
            }
        }

        if !ingestion_task.is_finished() {
            ingestion_task.await??;
        }

        Ok(())
    }
}
