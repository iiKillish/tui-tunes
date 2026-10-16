use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::{RwLock, RwLockReadGuard};
use std::{fs, process};

use log::{debug, error};
use ncspot::{CONFIGURATION_FILE_NAME, USER_STATE_FILE_NAME};
use platform_dirs::AppDirs;

use crate::command::{SortDirection, SortKey};
use crate::model::playable::Playable;
use crate::queue;
use crate::serialization::{CBOR, Serializer, TOML};

pub const CACHE_VERSION: u16 = 1;
pub const DEFAULT_COMMAND_KEY: char = ':';

/// The playback state when ncspot is started.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Default,
}

/// The focussed library tab when ncspot is started.
#[derive(Clone, Serialize, Deserialize, Debug, Hash, strum_macros::EnumIter)]
#[serde(rename_all = "lowercase")]
pub enum LibraryTab {
    Tracks,
    Albums,
    Artists,
    Playlists,
    Podcasts,
    Browse,
}

/// The format used to represent tracks in a list.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct TrackFormat {
    pub left: Option<String>,
    pub center: Option<String>,
    pub right: Option<String>,
}

impl TrackFormat {
    pub fn default() -> Self {
        Self {
            left: Some(String::from("%artists - %title")),
            center: Some(String::from("%album")),
            right: Some(String::from("%saved %duration")),
        }
    }
}

/// The format used when sending desktop notifications about playback status.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct NotificationFormat {
    pub title: Option<String>,
    pub body: Option<String>,
}

impl NotificationFormat {
    pub fn default() -> Self {
        Self {
            title: Some(String::from("%title")),
            body: Some(String::from("%artists")),
        }
    }
}

/// The configuration of ncspot.
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ConfigValues {
    pub command_key: Option<char>,
    pub initial_screen: Option<String>,
    pub default_keybindings: Option<bool>,
    pub keybindings: Option<HashMap<String, String>>,
    pub theme: Option<ConfigTheme>,
    pub use_nerdfont: Option<bool>,
    pub audio_cache: Option<bool>,
    pub audio_cache_size: Option<u32>,
    pub backend: Option<String>,
    pub backend_device: Option<String>,
    pub volnorm: Option<bool>,
    pub volnorm_pregain: Option<f64>,
    pub notify: Option<bool>,
    pub bitrate: Option<u32>,
    pub gapless: Option<bool>,
    pub shuffle: Option<bool>,
    pub repeat: Option<queue::RepeatSetting>,
    pub playback_state: Option<PlaybackState>,
    pub track_format: Option<TrackFormat>,
    pub notification_format: Option<NotificationFormat>,
    pub library_tabs: Option<Vec<LibraryTab>>,
    pub hide_display_names: Option<bool>,
    pub ap_port: Option<u16>,
    /// Proxy used for all of ncspot's network traffic: the Spotify access
    /// point, the Spotify APIs, OAuth and cover art downloads. Supported
    /// schemes: `http://` (CONNECT proxy) and `socks5h://` (SOCKS5 proxy with
    /// remote DNS resolution). Both accept `user:password@host:port`.
    pub proxy: Option<String>,
    /// Client ID of your own Spotify developer app, used for Web API requests instead of
    /// ncspot's shared one (which is frequently rate limited). The app must have
    /// `http://127.0.0.1:8989/login` registered as a redirect URI.
    pub client_id: Option<String>,
    /// TOML file with theme colors (the keys of `[theme]`, at the top level), relative to the
    /// configuration directory unless absolute. Colors in `[theme]` override it.
    pub theme_file: Option<String>,
    /// How the volume slider maps to loudness: "cubic" (default, like system mixers), "log"
    /// (60 dB range, quiet sooner) or "linear".
    pub volume_curve: Option<String>,
}

/// The ncspot theme.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct ConfigTheme {
    pub background: Option<String>,
    pub primary: Option<String>,
    pub secondary: Option<String>,
    pub title: Option<String>,
    pub playing: Option<String>,
    pub playing_bg: Option<String>,
    pub highlight: Option<String>,
    pub highlight_bg: Option<String>,
    pub highlight_inactive_bg: Option<String>,
    pub error: Option<String>,
    pub error_bg: Option<String>,
    pub statusbar_progress: Option<String>,
    pub statusbar_progress_bg: Option<String>,
    pub statusbar_bg: Option<String>,
    pub cmdline: Option<String>,
    pub cmdline_bg: Option<String>,
}

/// The ordering that is used when representing a playlist.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SortingOrder {
    pub key: SortKey,
    pub direction: SortDirection,
}

/// The runtime state of the music queue.
#[derive(Serialize, Default, Deserialize, Debug, Clone)]
pub struct QueueState {
    pub current_track: Option<usize>,
    pub random_order: Option<Vec<usize>>,
    pub track_progress: std::time::Duration,
    pub queue: Vec<Playable>,
    /// Where each queue item was queued from ("Next from: …" in the queue panel), parallel to
    /// `queue`. Missing from state saved by older versions.
    #[serde(default)]
    pub sources: Vec<Option<String>>,
}

/// Runtime state that should be persisted accross sessions.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserState {
    pub volume: u16,
    pub shuffle: bool,
    pub repeat: queue::RepeatSetting,
    pub queuestate: QueueState,
    pub playlist_orders: HashMap<String, SortingOrder>,
    pub cache_version: u16,
    pub playback_state: PlaybackState,
}

impl Default for UserState {
    fn default() -> Self {
        Self {
            volume: u16::MAX,
            shuffle: false,
            repeat: queue::RepeatSetting::None,
            queuestate: QueueState::default(),
            playlist_orders: HashMap::new(),
            cache_version: 0,
            playback_state: PlaybackState::Default,
        }
    }
}

/// Configuration files are read/written relative to this directory.
static BASE_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

/// The complete configuration (state + user configuration) of ncspot.
pub struct Config {
    /// The configuration file path.
    filename: String,
    /// Configuration set by the user, read only.
    values: RwLock<ConfigValues>,
    /// Runtime state which can't be edited by the user, read/write.
    state: RwLock<UserState>,
}

impl Config {
    /// Create a default configuration from in-memory defaults, without touching the filesystem.
    #[cfg(test)]
    pub fn new_for_test() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            filename: String::new(),
            values: RwLock::new(ConfigValues::default()),
            state: RwLock::new(UserState::default()),
        })
    }

    /// Generate the configuration from the user configuration file and the runtime state file.
    /// `filename` can be used to look for a differently named configuration file.
    pub fn new(filename: Option<String>) -> Self {
        let filename = filename.unwrap_or(CONFIGURATION_FILE_NAME.to_owned());
        let values = load(&filename).unwrap_or_else(|e| {
            eprint!(
                "There is an error in your configuration file at {}:\n\n{e}",
                user_configuration_directory()
                    .map(|ref mut path| {
                        path.push(CONFIGURATION_FILE_NAME);
                        path.to_string_lossy().to_string()
                    })
                    .expect("configuration directory expected but not found")
            );
            process::exit(1);
        });

        let mut userstate = {
            let path = config_path(USER_STATE_FILE_NAME);
            CBOR.load_or_generate_default(path, || Ok(UserState::default()), true)
                .expect("could not load user state")
        };

        if let Some(shuffle) = values.shuffle {
            userstate.shuffle = shuffle;
        }

        if let Some(repeat) = values.repeat {
            userstate.repeat = repeat;
        }

        if let Some(playback_state) = values.playback_state.clone() {
            userstate.playback_state = playback_state;
        }

        Self {
            filename,
            values: RwLock::new(values),
            state: RwLock::new(userstate),
        }
    }

    /// Get the user configuration values.
    pub fn values(&self) -> RwLockReadGuard<'_, ConfigValues> {
        self.values.read().unwrap()
    }

    /// Get the runtime user state values.
    pub fn state(&self) -> RwLockReadGuard<'_, UserState> {
        self.state.read().unwrap()
    }

    /// Modify the internal user state through a shared reference using a closure.
    pub fn with_state_mut<F>(&self, cb: F)
    where
        F: Fn(&mut UserState),
    {
        let mut state_guard = self.state.write().unwrap();
        cb(&mut state_guard);
    }

    /// Update the version number of the runtime user state. This should be done before saving it to
    /// disk.
    fn update_state_cache_version(&self) {
        self.with_state_mut(|state| state.cache_version = CACHE_VERSION);
    }

    /// Save runtime state to the user configuration directory.
    pub fn save_state(&self) {
        self.update_state_cache_version();

        let path = config_path(USER_STATE_FILE_NAME);
        debug!("saving user state to {}", path.display());
        if let Err(e) = CBOR.write(path, &*self.state()) {
            error!("Could not save user state: {e}");
        }
    }

    /// The theme's colors, from the configuration file (and its theme file).
    pub fn build_theme(&self) -> crate::theme::Palette {
        crate::theme::load(&self.values().theme)
    }

    /// Attempt to reload the configuration from the configuration file.
    ///
    /// This only updates the values stored in memory but doesn't perform any additional actions
    /// like updating active keybindings.
    pub fn reload(&self) -> Result<(), Box<dyn Error>> {
        let cfg = load(&self.filename)?;
        *self.values.write().unwrap() = cfg;
        Ok(())
    }
}

/// Parse the configuration file with name `filename` at the configuration base path.
fn load(filename: &str) -> Result<ConfigValues, String> {
    let path = config_path(filename);
    let mut values: ConfigValues =
        TOML.load_or_generate_default(path, || Ok(ConfigValues::default()), false)?;
    if let Some(theme_file) = values.theme_file.clone() {
        values.theme = Some(load_theme_file(&theme_file, values.theme.as_ref())?);
    }
    Ok(values)
}

/// Read the theme in `theme_file` (see [ConfigValues::theme_file]), with the colors set in
/// `overrides` (the configuration file's own `[theme]`) taking precedence.
fn load_theme_file(
    theme_file: &str,
    overrides: Option<&ConfigTheme>,
) -> Result<ConfigTheme, String> {
    let path = PathBuf::from(theme_file);
    let path = if path.is_absolute() {
        path
    } else {
        config_path(theme_file)
    };
    let contents = fs::read_to_string(&path)
        .map_err(|e| format!("can't read theme file {}: {e}", path.display()))?;
    let mut theme: toml::Table = contents
        .parse()
        .map_err(|e| format!("error in theme file {}: {e}", path.display()))?;
    if let Some(overrides) = overrides {
        let overrides = toml::Table::try_from(overrides).map_err(|e| e.to_string())?;
        theme.extend(overrides);
    }
    toml::Value::Table(theme)
        .try_into()
        .map_err(|e| format!("error in theme file {}: {e}", path.display()))
}

/// Read the configuration file without any side effects (no error reporting,
/// no user state). Used at startup, before the full [Config] is loaded.
fn read_startup_values(filename: Option<String>) -> Option<ConfigValues> {
    let filename = filename.unwrap_or(CONFIGURATION_FILE_NAME.to_owned());
    let contents = std::fs::read_to_string(config_path(&filename)).ok()?;
    toml::from_str(&contents).ok()
}

/// Export the environment variables ncspot relies on, derived from the
/// configuration file:
///
/// * The configured `proxy`, so that every HTTP client stack used by ncspot
///   and its dependencies picks it up:
///   * `ALL_PROXY` is read by the reqwest stacks (OAuth token exchange and
///     cover art downloads); reqwest understands `socks5h://` natively.
///   * `all_proxy` is read by ureq (rspotify Web API). ureq does not know the
///     `socks5h` scheme, but its `socks5` implementation already hands the
///     hostname to the proxy, so the schemes are equivalent there.
///
///   The librespot session reads its proxy straight from the configuration
///   instead (see `Spotify::session_config`).
/// * The `PULSE_PROP_*` stream properties when the PulseAudio backend will be
///   used (see `spotify::pulseaudio_environment`). librespot's PulseAudio sink
///   and libpulse itself only take these from the environment, there is no
///   API to pass them explicitly.
///
/// This must be called from the top of `main`, before any other thread is
/// spawned; see the SAFETY comment below.
pub fn preload_environment(filename: Option<String>) {
    let values = read_startup_values(filename);
    let mut vars: Vec<(&str, String)> = Vec::new();

    let proxy = values
        .as_ref()
        .and_then(|v| v.proxy.as_deref())
        .map(str::trim)
        .filter(|proxy| !proxy.is_empty());
    if let Some(proxy) = proxy {
        log::info!("Setting proxy {proxy}");
        let ureq_proxy = match proxy.split_once("://") {
            Some(("socks5h", rest)) => format!("socks5://{rest}"),
            _ => proxy.to_owned(),
        };
        vars.push(("ALL_PROXY", proxy.to_owned()));
        vars.push(("all_proxy", ureq_proxy));
    }

    let backend = values.as_ref().and_then(|v| v.backend.as_deref());
    vars.extend(
        crate::spotify::pulseaudio_environment(backend)
            .iter()
            .map(|&(key, value)| (key, value.to_owned())),
    );

    for (key, value) in vars {
        // SAFETY: `set_var` is only unsound if another thread reads or writes
        // the environment concurrently (libc `getenv`/`setenv` are not
        // synchronised with Rust's environment lock). This function is called
        // exactly once, from `main`, before the tokio runtime, librespot,
        // MPRIS, IPC, image loading or any other thread is created: up to this
        // point `main` has only installed a panic hook, parsed arguments,
        // configured the file logger (fern, which spawns no threads) and set
        // the base path. The process is therefore still single-threaded.
        unsafe { std::env::set_var(key, value) };
    }
}

/// Returns the plaform app directories for ncspot if they could be determined,
/// or an error otherwise.
pub fn try_proj_dirs() -> Result<AppDirs, String> {
    match *BASE_PATH
        .read()
        .map_err(|_| String::from("Poisoned RWLock"))?
    {
        Some(ref basepath) => Ok(AppDirs {
            cache_dir: basepath.join(".cache"),
            config_dir: basepath.join(".config"),
            data_dir: basepath.join(".local/share"),
            state_dir: basepath.join(".local/state"),
        }),
        None => AppDirs::new(Some("ncspot"), true)
            .ok_or_else(|| String::from("Couldn't determine platform standard directories")),
    }
}

/// Return the path to the current user's configuration directory, or None if it couldn't be found.
/// This function does not guarantee correct permissions or ownership of the directory!
pub fn user_configuration_directory() -> Option<PathBuf> {
    let project_directories = try_proj_dirs().ok()?;
    Some(project_directories.config_dir)
}

/// Return the path to the current user's cache directory, or None if one couldn't be found. This
/// function does not guarantee correct permissions or ownership of the directory!
pub fn user_cache_directory() -> Option<PathBuf> {
    let project_directories = try_proj_dirs().ok()?;
    Some(project_directories.cache_dir)
}

/// Force create the configuration directory at the default project location, removing anything that
/// isn't a directory but has the same name. Return the path to the configuration file inside the
/// directory.
///
/// This doesn't create the file, only the containing directory.
pub fn config_path(file: &str) -> PathBuf {
    let cfg_dir = user_configuration_directory().unwrap();
    if cfg_dir.exists() && !cfg_dir.is_dir() {
        fs::remove_file(&cfg_dir).expect("unable to remove old config file");
    }
    if !cfg_dir.exists() {
        fs::create_dir_all(&cfg_dir).expect("can't create config folder");
    }
    let mut cfg = cfg_dir.to_path_buf();
    cfg.push(file);
    cfg
}

/// Create the cache directory at the default project location, preserving it if it already exists,
/// and return the path to the cache file inside the directory.
///
/// This doesn't create the file, only the containing directory.
pub fn cache_path(file: &str) -> PathBuf {
    let cache_dir = user_cache_directory().unwrap();
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).expect("can't create cache folder");
    }
    let mut pb = cache_dir.to_path_buf();
    pb.push(file);
    pb
}

/// Set the configuration base path. All configuration files are read/written relative to this path.
pub fn set_configuration_base_path(base_path: Option<PathBuf>) {
    if let Some(basepath) = base_path {
        if !basepath.exists() {
            fs::create_dir_all(&basepath).expect("could not create basepath directory");
        }
        *BASE_PATH.write().unwrap() = Some(basepath);
    }
}

#[cfg(test)]
mod queue_state_tests {
    use super::QueueState;

    #[test]
    fn test_queue_state_without_sources_still_loads() {
        // State saved before sources existed.
        let old = r#"{"current_track":null,"random_order":null,
            "track_progress":{"secs":0,"nanos":0},"queue":[]}"#;
        let state: QueueState = serde_json::from_str(old).unwrap();
        assert!(state.sources.is_empty());
    }
}

#[cfg(test)]
mod theme_file_tests {
    use super::{ConfigTheme, load_theme_file};

    #[test]
    fn test_theme_file_with_overrides() {
        let path = std::env::temp_dir().join(format!("ncspot-theme-{}.toml", std::process::id()));
        std::fs::write(&path, "background = \"#111111\"\nprimary = \"#222222\"\n").unwrap();

        let overrides = ConfigTheme {
            primary: Some("#333333".into()),
            ..Default::default()
        };
        let theme = load_theme_file(path.to_str().unwrap(), Some(&overrides)).unwrap();
        assert_eq!(theme.background.as_deref(), Some("#111111"));
        assert_eq!(
            theme.primary.as_deref(),
            Some("#333333"),
            "[theme] overrides the file"
        );

        let theme = load_theme_file(path.to_str().unwrap(), None).unwrap();
        assert_eq!(theme.primary.as_deref(), Some("#222222"));

        std::fs::remove_file(&path).unwrap();
        assert!(load_theme_file(path.to_str().unwrap(), None).is_err());
    }
}
