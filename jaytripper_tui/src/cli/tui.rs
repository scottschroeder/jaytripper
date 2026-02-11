use clap::Args;

#[derive(Debug, Args)]
pub(crate) struct TuiCommand {}

impl TuiCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        println!("TUI mode is not implemented yet. Use `debug` commands for now.");
        Ok(())
    }
}
