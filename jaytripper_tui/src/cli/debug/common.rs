use std::{
    env,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
};

use anyhow::Context;
use jaytripper_core::{Timestamp, ids::CharacterId};
use jaytripper_esi::{AuthService, AuthSession, EsiConfig, KeyringTokenStore, RfesiSsoClient};
use url::Url;

const DEFAULT_SCOPES: &str = "publicData,esi-location.read_location.v1";
const KEYRING_SERVICE: &str = "jaytripper";
const KEYRING_ACCOUNT_PREFIX: &str = "esi-session";

pub(crate) fn load_esi_config(default_user_agent: &'static str) -> anyhow::Result<EsiConfig> {
    Ok(EsiConfig {
        client_id: required_env("EVE_CLIENT_ID")?,
        callback_url: required_env("EVE_CALLBACK_URL")?,
        scopes: scopes_from_env(),
        user_agent: env::var("JAYTRIPPER_USER_AGENT").unwrap_or_else(|_| default_user_agent.into()),
    })
}

pub(crate) fn build_auth_service(
    config: &EsiConfig,
) -> anyhow::Result<AuthService<RfesiSsoClient, KeyringTokenStore>> {
    let client = RfesiSsoClient::new(config).context("failed to create ESI SSO client")?;
    let store = KeyringTokenStore::new(KEYRING_SERVICE, KEYRING_ACCOUNT_PREFIX);
    Ok(AuthService::new(client, store, config.scopes.clone()))
}

pub(crate) fn selected_character_id(explicit: Option<u64>) -> Option<CharacterId> {
    explicit.map(CharacterId).or_else(|| {
        env::var("EVE_CHARACTER_ID")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(CharacterId)
    })
}

pub(crate) fn required_character_id(explicit: Option<u64>) -> anyhow::Result<CharacterId> {
    selected_character_id(explicit)
        .context("character id is required; provide --character-id or set EVE_CHARACTER_ID")
}

pub(crate) fn print_session_details(session: &AuthSession) {
    let now = Timestamp::now();
    let valid_for = session
        .access_expires_at
        .signed_duration_since(now)
        .num_seconds();

    println!("Character: {}", session.character_id);
    println!(
        "Name: {}",
        session.character_name.as_deref().unwrap_or("<unknown>")
    );
    println!("Scopes: {}", session.scopes.join(","));
    println!("Updated at (epoch): {}", session.updated_at.as_epoch_secs());
    println!(
        "Valid until (epoch): {} ({})",
        session.access_expires_at.as_epoch_secs(),
        if valid_for >= 0 {
            format!("in {valid_for}s")
        } else {
            format!("expired {}s ago", -valid_for)
        }
    );
}

pub(crate) fn wait_for_callback(callback_url: &str) -> anyhow::Result<(String, String)> {
    let parsed = Url::parse(callback_url).context("invalid callback URL")?;
    if parsed.scheme() != "http" {
        anyhow::bail!("callback URL must use http for local callback server");
    }

    let host = parsed
        .host_str()
        .context("callback URL must include host")?;
    let port = parsed
        .port_or_known_default()
        .context("callback URL must include a valid port")?;
    let path = parsed.path().to_owned();

    let bind_addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&bind_addr)
        .with_context(|| format!("failed to bind callback listener on {bind_addr}"))?;
    listener
        .set_nonblocking(false)
        .context("failed to configure callback listener")?;

    let (mut stream, _) = listener.accept().context("failed to accept callback")?;
    let request = read_http_request(&mut stream).context("failed to read callback request")?;
    let request_line = request
        .lines()
        .next()
        .context("empty callback HTTP request")?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        write_http_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "Only GET is supported.",
        )
        .context("failed writing method not allowed response")?;
        anyhow::bail!("callback request must be GET");
    }

    let full_target = format!("http://{bind_addr}{target}");
    let target_url = Url::parse(&full_target).context("invalid callback request target URL")?;
    if target_url.path() != path {
        write_http_response(&mut stream, 404, "Not Found", "Unexpected callback path.")
            .context("failed writing callback path mismatch response")?;
        anyhow::bail!("callback path does not match configured callback URL");
    }

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for (key, value) in target_url.query_pairs() {
        if key == "code" {
            code = Some(value.into_owned());
        } else if key == "state" {
            state = Some(value.into_owned());
        }
    }

    match (code, state) {
        (Some(code), Some(state)) => {
            write_http_response(
                &mut stream,
                200,
                "OK",
                "Authentication captured. You can close this tab.",
            )
            .context("failed writing success callback response")?;
            Ok((code, state))
        }
        _ => {
            write_http_response(
                &mut stream,
                400,
                "Bad Request",
                "Missing code/state query parameters.",
            )
            .context("failed writing bad request callback response")?;
            anyhow::bail!("callback query is missing code and/or state");
        }
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    env::var(name).map_err(|_| anyhow::anyhow!("missing required env var `{name}`"))
}

fn scopes_from_env() -> Vec<String> {
    let raw = env::var("EVE_SCOPES").unwrap_or_else(|_| DEFAULT_SCOPES.to_owned());
    raw.split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, io::Error> {
    let mut buffer = [0_u8; 8192];
    let size = stream.read(&mut buffer)?;
    String::from_utf8(buffer[..size].to_vec())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

fn write_http_response(
    stream: &mut TcpStream,
    code: u16,
    reason: &str,
    body: &str,
) -> Result<(), io::Error> {
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}
