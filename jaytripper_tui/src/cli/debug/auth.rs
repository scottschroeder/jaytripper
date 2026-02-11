use clap::{Args, Subcommand};
use jaytripper_esi::EnsureSessionResult;

use crate::cli::debug::common::{
    build_auth_service, load_esi_config, print_session_details, required_character_id,
    selected_character_id, wait_for_callback,
};

#[derive(Debug, Args)]
pub(crate) struct AuthCommand {
    #[command(subcommand)]
    subcmd: AuthSubcommand,
}

#[derive(Debug, Subcommand)]
enum AuthSubcommand {
    /// Log in and persist a session in keyring.
    Login(LoginCommand),

    /// Show stored session metadata.
    Status(StatusCommand),

    /// Remove stored keyring session.
    Logout(LogoutCommand),
}

impl AuthCommand {
    pub(crate) async fn run(&self) -> anyhow::Result<()> {
        match &self.subcmd {
            AuthSubcommand::Login(cmd) => cmd.run().await,
            AuthSubcommand::Status(cmd) => cmd.run().await,
            AuthSubcommand::Logout(cmd) => cmd.run().await,
        }
    }
}

#[derive(Debug, Args)]
struct LoginCommand {
    #[arg(long)]
    character_id: Option<u64>,
}

impl LoginCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let config = load_esi_config("jaytripper-tui-auth/0.1")?;
        let mut auth = build_auth_service(&config)?;

        if let Some(character_id) = selected_character_id(self.character_id) {
            match auth.ensure_valid_session(character_id).await? {
                EnsureSessionResult::Ready(session) => {
                    println!("Session already valid for character {character_id}.");
                    print_session_details(&session);
                    return Ok(());
                }
                EnsureSessionResult::NeedsReauth { reason } => {
                    println!("Existing session needs reauth: {reason}");
                }
                EnsureSessionResult::Missing => {
                    println!("No existing session found for {character_id}; starting login flow.");
                }
            }
        } else {
            println!(
                "No character selected; starting login flow. Set --character-id or EVE_CHARACTER_ID to auto-reuse sessions."
            );
        }

        let login = auth.begin_login()?;
        println!(
            "Open this URL in your browser:\n\n{}\n",
            login.authorization_url
        );
        println!("Expected state: {}", login.state);
        println!("Waiting for callback on {}", config.callback_url);

        let (code, callback_state) = wait_for_callback(&config.callback_url)?;
        let session = auth
            .complete_login(code.trim(), callback_state.trim())
            .await?;

        println!(
            "Authenticated {} ({})",
            session.character_name.as_deref().unwrap_or("<unknown>"),
            session.character_id
        );
        print_session_details(&session);
        Ok(())
    }
}

#[derive(Debug, Args)]
struct StatusCommand {
    #[arg(long)]
    character_id: Option<u64>,
}

impl StatusCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let character_id = required_character_id(self.character_id)?;
        let config = load_esi_config("jaytripper-tui-auth/0.1")?;
        let auth = build_auth_service(&config)?;

        match auth.load_session(character_id)? {
            Some(session) => print_session_details(&session),
            None => println!("No session found in keyring for character {character_id}."),
        }

        Ok(())
    }
}

#[derive(Debug, Args)]
struct LogoutCommand {
    #[arg(long)]
    character_id: Option<u64>,
}

impl LogoutCommand {
    async fn run(&self) -> anyhow::Result<()> {
        let character_id = required_character_id(self.character_id)?;
        let config = load_esi_config("jaytripper-tui-auth/0.1")?;
        let auth = build_auth_service(&config)?;

        auth.logout(character_id)?;
        println!("Cleared stored session from keyring for character {character_id}.");
        Ok(())
    }
}
