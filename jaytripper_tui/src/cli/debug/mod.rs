mod auth;
mod common;
mod events;
mod location;
mod state;
mod track;

use clap::{Args, Subcommand};

use self::{
    auth::AuthCommand, events::EventsCommand, location::LocationCommand, state::StateCommand,
    track::TrackCommand,
};

#[derive(Debug, Args)]
pub(crate) struct DebugCommand {
    #[command(subcommand)]
    subcmd: DebugSubcommand,
}

#[derive(Debug, Subcommand)]
enum DebugSubcommand {
    /// SSO and session operations.
    Auth(AuthCommand),

    /// Fetch current character location.
    Location(LocationCommand),

    /// Run continuous movement tracking ingestion.
    Track(TrackCommand),

    /// Inspect raw events from the local event log.
    Events(EventsCommand),

    /// Inspect derived in-memory state from replay.
    State(StateCommand),
}

impl DebugCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            DebugSubcommand::Auth(cmd) => cmd.run().await,
            DebugSubcommand::Location(cmd) => cmd.run().await,
            DebugSubcommand::Track(cmd) => cmd.run().await,
            DebugSubcommand::Events(cmd) => cmd.run().await,
            DebugSubcommand::State(cmd) => cmd.run().await,
        }
    }
}
