use clap::{Args, Subcommand};
use jaytripper_esi::EsiClient;

use crate::cli::debug::common::{build_auth_service, load_esi_config, required_character_id};

#[derive(Debug, Args)]
pub(crate) struct LocationCommand {
    #[command(subcommand)]
    subcmd: LocationSubcommand,
}

#[derive(Debug, Subcommand)]
enum LocationSubcommand {
    /// Fetch one location sample.
    Once(OnceCommand),
}

impl LocationCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            LocationSubcommand::Once(cmd) => cmd.run().await,
        }
    }
}

#[derive(Debug, Args)]
struct OnceCommand {
    #[arg(long)]
    character_id: Option<u64>,
}

impl OnceCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let character_id = required_character_id(self.character_id)?;
        let config = load_esi_config("jaytripper-tui-location/0.1")?;
        let auth = build_auth_service(&config)?;
        let client = auth.connect_character(character_id).await?;
        let location = client.get_current_location().await?;

        println!("Character: {character_id}");
        println!("Solar system id: {}", location.solar_system_id);
        println!(
            "Station id: {}",
            location
                .station_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<none>".to_owned())
        );
        println!(
            "Structure id: {}",
            location
                .structure_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<none>".to_owned())
        );

        Ok(())
    }
}
