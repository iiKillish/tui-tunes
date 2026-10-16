use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use librespot_core::authentication::Credentials as RespotCredentials;
use librespot_core::cache::Cache;
use librespot_oauth::OAuthClientBuilder;
use log::{error, info, warn};

use crate::config::{self, Config};
use crate::spotify::Spotify;

pub const SPOTIFY_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

/// Redirect URI for Web API logins. It has to match one registered for the user's Spotify app
/// exactly, so unlike the librespot login it can't use a random free port.
pub const WEB_API_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

static WEB_API_CLIENT_ID: OnceLock<String> = OnceLock::new();

/// Use `client_id` (the user's own Spotify app, see [crate::client_id]) for Web API requests.
/// Must be called before any Web API client is created.
pub fn set_web_api_client_id(client_id: String) {
    info!("Using Web API client ID {client_id}");
    let _ = WEB_API_CLIENT_ID.set(client_id);
}

/// Client ID used for Web API requests; empty until [set_web_api_client_id] was called.
pub fn web_api_client_id() -> &'static str {
    WEB_API_CLIENT_ID.get().map_or("", String::as_str)
}

/// Tokens are tied to the client ID that issued them, so each client ID keeps its own cache file.
fn rspotify_token_path() -> PathBuf {
    config::cache_path(&format!("rspotify_token_{}.json", web_api_client_id()))
}

static OAUTH_SCOPES: &[&str] = &[
    "playlist-modify",
    "playlist-modify-private",
    "playlist-modify-public",
    "playlist-read",
    "playlist-read-collaborative",
    "playlist-read-private",
    "streaming",
    "user-follow-modify",
    "user-follow-read",
    "user-library-modify",
    "user-library-read",
    "user-modify",
    "user-modify-playback-state",
    "user-modify-private",
    "user-personalized",
    "user-read-currently-playing",
    "user-read-email",
    "user-read-play-history",
    "user-read-playback-position",
    "user-read-playback-state",
    "user-read-private",
    "user-read-recently-played",
    "user-top-read",
];

static WEB_API_OAUTH_SCOPES: &[&str] = &[
    "streaming",
    "user-read-email",
    "user-read-private",
    "user-library-read",
    "user-library-modify",
    "user-read-playback-state",
    "user-modify-playback-state",
    "playlist-read-private",
    "playlist-modify-public",
    "playlist-modify-private",
    "user-follow-read",
    "user-follow-modify",
    "user-top-read",
    "user-read-currently-playing",
    "user-read-recently-played",
];

pub fn find_free_port() -> Result<u16, String> {
    let socket = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    socket
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| e.to_string())
}

/// Redirect URI for the librespot login, on a free port.
fn spotify_redirect_uri() -> String {
    let auth_port = find_free_port().expect("Could not find free port");
    format!("http://127.0.0.1:{auth_port}/login")
}

/// Get credentials for use with librespot. This first tries to get cached credentials. If no cached
/// credentials are available it will initiate the OAuth2 login process.
pub fn get_credentials(configuration: &Config) -> Result<RespotCredentials, String> {
    let mut credentials = {
        let cache = Cache::new(Some(config::cache_path("librespot")), None, None, None)
            .expect("Could not create librespot cache");
        let cached_credentials = cache.credentials();
        match cached_credentials {
            Some(c) => {
                info!("Using cached credentials");
                c
            }
            None => {
                info!("Attempting to login via OAuth2");
                credentials_prompt(None)?
            }
        }
    };

    while let Err(error) = Spotify::test_credentials(configuration, credentials.clone()) {
        let error_msg = format!("{error}");
        credentials = credentials_prompt(Some(error_msg))?;
    }
    Ok(credentials)
}

fn credentials_prompt(error_message: Option<String>) -> Result<RespotCredentials, String> {
    if let Some(message) = error_message {
        eprintln!("Connection error: {message}");
    }

    create_credentials()
}

pub fn create_credentials() -> Result<RespotCredentials, String> {
    println!("To login you need to perform OAuth2 authorization using your web browser\n");

    let client_builder = OAuthClientBuilder::new(
        SPOTIFY_CLIENT_ID,
        &spotify_redirect_uri(),
        OAUTH_SCOPES.to_vec(),
    );
    let oauth_client = client_builder.build().map_err(|e| e.to_string())?;

    oauth_client
        .get_access_token()
        .map(|token| RespotCredentials::with_access_token(token.access_token))
        .map_err(|e| e.to_string())
}

pub fn get_rspotify_token() -> Result<rspotify::Token, String> {
    let path = rspotify_token_path();
    let token = if let Ok(token_json) = fs::read_to_string(&path) {
        serde_json::from_str::<rspotify::Token>(&token_json).ok()
    } else {
        None
    };

    if let Some(t) = token {
        if !t.is_expired() {
            return Ok(t);
        }

        // Token is expired, try to refresh if we have a refresh token. Spotify's refresh
        // responses may omit the refresh token, in which case it must be reused, so an
        // empty/missing value here means we don't actually have a usable refresh token.
        let refresh_token = t.refresh_token.as_deref().filter(|s| !s.is_empty());
        if let Some(refresh_token) = refresh_token {
            info!("Access token expired, attempting to refresh..");
            let client_builder = OAuthClientBuilder::new(
                web_api_client_id(),
                WEB_API_REDIRECT_URI,
                WEB_API_OAUTH_SCOPES.to_vec(),
            );
            if let Ok(oauth_client) = client_builder.build() {
                match oauth_client.refresh_token(refresh_token) {
                    Ok(new_token) => {
                        let mapped = map_token(new_token, Some(refresh_token));
                        write_token(&path, &mapped);
                        return Ok(mapped);
                    }
                    Err(e) => {
                        error!("Failed to refresh token: {e}");
                    }
                }
            }
        }
    }

    let t = create_rspotify_token()?;
    write_token(&path, &t);
    Ok(t)
}

pub fn create_rspotify_token() -> Result<rspotify::Token, String> {
    println!(
        "To fully enable Web API features, you need to perform a second OAuth2 authorization\n"
    );

    let client_builder = OAuthClientBuilder::new(
        web_api_client_id(),
        WEB_API_REDIRECT_URI,
        WEB_API_OAUTH_SCOPES.to_vec(),
    );
    let oauth_client = client_builder.build().map_err(|e| e.to_string())?;

    oauth_client
        .get_access_token()
        .map(|token| map_token(token, None))
        .map_err(|e| e.to_string())
}

/// Write `token` to `path`, logging (rather than silently discarding) any failure, and
/// restricting file permissions since the file holds a long-lived credential.
fn write_token(path: &Path, token: &rspotify::Token) {
    let json = match serde_json::to_string_pretty(token) {
        Ok(json) => json,
        Err(e) => {
            error!("Failed to serialize rspotify token: {e}");
            return;
        }
    };

    if let Err(e) = fs::write(path, json) {
        error!("Failed to write rspotify token cache: {e}");
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            warn!("Failed to restrict permissions on rspotify token cache: {e}");
        }
    }
}

/// Map an OAuth token obtained from librespot into the `rspotify::Token` type used for Web
/// API calls.
///
/// Spotify's refresh-token responses may omit `refresh_token` entirely, in which case the
/// previously issued refresh token must be reused (it has not been rotated or invalidated).
/// `librespot_oauth` maps that omission to an empty string, so `fallback_refresh_token` is
/// substituted whenever the token returned by librespot is empty, to avoid ever persisting an
/// unusable refresh token.
fn map_token(
    token: librespot_oauth::OAuthToken,
    fallback_refresh_token: Option<&str>,
) -> rspotify::Token {
    let duration = if token.expires_at > std::time::Instant::now() {
        token.expires_at.duration_since(std::time::Instant::now())
    } else {
        std::time::Duration::from_secs(0)
    };
    let expires_in = chrono::Duration::from_std(duration).unwrap_or(chrono::Duration::seconds(0));

    let refresh_token = if token.refresh_token.is_empty() {
        fallback_refresh_token.map(str::to_string)
    } else {
        Some(token.refresh_token)
    };

    rspotify::Token {
        access_token: token.access_token,
        expires_in,
        scopes: std::collections::HashSet::new(),
        expires_at: Some(chrono::Utc::now() + expires_in),
        refresh_token,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn oauth_token(refresh_token: &str) -> librespot_oauth::OAuthToken {
        librespot_oauth::OAuthToken {
            access_token: "access".to_string(),
            refresh_token: refresh_token.to_string(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            token_type: "Bearer".to_string(),
            scopes: Vec::new(),
        }
    }

    #[test]
    fn map_token_keeps_new_refresh_token_when_present() {
        let mapped = map_token(oauth_token("new-refresh-token"), Some("old-refresh-token"));
        assert_eq!(mapped.refresh_token, Some("new-refresh-token".to_string()));
    }

    #[test]
    fn map_token_falls_back_to_previous_refresh_token_when_omitted() {
        let mapped = map_token(oauth_token(""), Some("old-refresh-token"));
        assert_eq!(mapped.refresh_token, Some("old-refresh-token".to_string()));
    }

    #[test]
    fn map_token_yields_no_refresh_token_when_omitted_without_fallback() {
        let mapped = map_token(oauth_token(""), None);
        assert_eq!(mapped.refresh_token, None);
    }
}
