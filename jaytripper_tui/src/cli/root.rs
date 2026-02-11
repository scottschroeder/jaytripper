use clap::{ArgAction, Parser, Subcommand};

use crate::cli::{debug::DebugCommand, tui::TuiCommand};

pub(crate) fn get_args() -> CliOpts {
    CliOpts::parse()
}

#[derive(Debug, Parser)]
#[command(version = clap::crate_version!(), author = "Scott S. <scottschroeder@sent.com>")]
pub(crate) struct CliOpts {
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    subcmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch the full-screen operator interface.
    Tui(TuiCommand),

    /// Debug and operations commands.
    Debug(DebugCommand),
}

impl CliOpts {
    pub(crate) fn verbose(&self) -> u8 {
        self.verbose
    }

    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            Command::Tui(cmd) => cmd.run().await,
            Command::Debug(cmd) => cmd.run().await,
        }
    }
}
