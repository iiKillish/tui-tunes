//! Commands that work anywhere: playback, volume, the library, saving state.

use std::time::Duration;

use log::{debug, error, info};

use crate::command::{Command, InsertSource, SeekDirection, SleepMode, TargetMode};
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::episode::Episode;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::show::Show;
use crate::model::track::Track;
use crate::queue::{RepeatSetting, SleepTimer};
use crate::spotify::{UriType, VOLUME_PERCENT};
use crate::spotify_url::SpotifyUrl;
use crate::traits::ListItem;
use crate::tui::app::App;
use crate::tui::nav;
use crate::tui::overlays::menu::{Entry, Menu};
use crate::tui::overlays::prompt::Prompt;
use crate::tui::page::{Ctx, Outcome, Overlay};

/// Save the queue and playback position for next time.
pub fn save_state(ctx: &Ctx) {
    let queue = ctx.queue.queue.read().unwrap();
    ctx.config.with_state_mut(|s| {
        debug!(
            "saving state, {} items, current track: {:?}",
            queue.len(),
            ctx.queue.get_current_index()
        );
        s.queuestate.queue.clone_from(&queue);
        s.queuestate.random_order = ctx.queue.get_random_order();
        s.queuestate.sources = ctx.queue.sources();
        s.queuestate.current_track = ctx.queue.get_current_index();
        s.queuestate.track_progress = ctx.spotify.get_current_progress();
    });
    ctx.config.save_state();
}

/// The keybindings, as a menu (`?` or `:help`).
pub fn help(app: &App) -> Box<dyn Overlay> {
    let entries = app
        .binding_list
        .iter()
        .map(|(key, commands)| {
            let text = commands
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            let commands = commands.clone();
            Entry::new(format!("{key:<14} {text}"), move |_| {
                Outcome::Run(commands.clone())
            })
        })
        .collect();
    Box::new(Menu::new("Keys", entries).filterable())
}

/// An item from a Spotify link or URI: open its page, or its menu for tracks and episodes.
fn open_link(url: &SpotifyUrl, ctx: &Ctx) -> Outcome {
    let api = &ctx.spotify.api;
    let item: Option<Box<dyn ListItem>> = match url.uri_type {
        UriType::Track => api
            .track(&url.id)
            .ok()
            .map(|t| Track::from(&t).as_listitem()),
        UriType::Album => api
            .album(&url.id)
            .ok()
            .map(|a| Album::from(&a).as_listitem()),
        UriType::Playlist => api
            .playlist(&url.id)
            .ok()
            .map(|p| Playlist::from(&p).as_listitem()),
        UriType::Artist => api
            .artist(&url.id)
            .ok()
            .map(|a| Artist::from(&a).as_listitem()),
        UriType::Episode => api
            .episode(&url.id)
            .ok()
            .map(|e| Episode::from(&e).as_listitem()),
        UriType::Show => api.show(&url.id).ok().map(|s| Show::from(&s).as_listitem()),
    };
    match item {
        Some(item) => match item.target() {
            Some(target) => Outcome::Open(nav::open_target(target, ctx)),
            None => Outcome::Overlay(nav::context_menu(item, ctx)),
        },
        None => Outcome::Error("Couldn't find that on Spotify".into()),
    }
}

/// Run a global command. [Outcome::Ignored] if `cmd` isn't one.
pub fn run(cmd: &Command, ctx: &Ctx) -> Outcome {
    let queue = &ctx.queue;
    let spotify = &ctx.spotify;
    match cmd {
        Command::Noop => {}
        Command::Stop => queue.stop(),
        Command::Previous => {
            if spotify.get_current_progress() < Duration::from_secs(5) {
                queue.previous();
            } else {
                spotify.seek(0);
            }
        }
        Command::Next => queue.next(true),
        Command::Clear => {
            return Outcome::Overlay(Box::new(Menu::confirm("Clear the queue?", |ctx| {
                ctx.queue.clear();
                Outcome::Consumed
            })));
        }
        Command::UpdateLibrary => ctx.library.update_library(true),
        Command::TogglePlay => queue.toggleplayback(),
        Command::Shuffle(mode) => queue.set_shuffle(mode.unwrap_or_else(|| !queue.get_shuffle())),
        Command::Repeat(mode) => {
            queue.set_repeat(mode.unwrap_or_else(|| match queue.get_repeat() {
                RepeatSetting::None => RepeatSetting::RepeatPlaylist,
                RepeatSetting::RepeatPlaylist => RepeatSetting::RepeatTrack,
                RepeatSetting::RepeatTrack => RepeatSetting::None,
            }))
        }
        Command::Seek(direction) => match *direction {
            SeekDirection::Relative(rel) => spotify.seek_relative(rel),
            SeekDirection::Absolute(abs) => spotify.seek(abs),
        },
        Command::VolumeUp(amount) => {
            let volume = spotify
                .volume()
                .saturating_add(VOLUME_PERCENT.saturating_mul(*amount));
            spotify.set_volume(volume, true);
        }
        Command::VolumeDown(amount) => {
            let volume = spotify
                .volume()
                .saturating_sub(VOLUME_PERCENT.saturating_mul(*amount));
            spotify.set_volume(volume, true);
        }
        Command::SaveQueue => {
            return Outcome::Overlay(Box::new(Prompt::new(
                "Save the queue as a playlist",
                |name, ctx| {
                    let tracks = ctx.queue.queue.read().unwrap().clone();
                    ctx.library.save_playlist(name, &tracks);
                    Outcome::Message(format!("Saved the queue as {name}"))
                },
            )));
        }
        Command::NewPlaylist(name) if name.trim().is_empty() => {
            return Outcome::Overlay(Box::new(Prompt::new("New playlist", |name, ctx| match ctx
                .spotify
                .api
                .create_playlist(name, None, None)
            {
                Ok(_) => {
                    ctx.library.update_library(false);
                    Outcome::Message(format!("Created {name}"))
                }
                Err(()) => Outcome::Error(format!("Couldn't create the playlist {name}")),
            })));
        }
        Command::NewPlaylist(name) => match spotify.api.create_playlist(name, None, None) {
            Ok(_) => ctx.library.update_library(false),
            Err(()) => return Outcome::Error(format!("Couldn't create the playlist {name}")),
        },
        Command::Logout => {
            spotify.shutdown();
            let mut credentials = crate::config::cache_path("librespot");
            credentials.push("credentials.json");
            let _ = std::fs::remove_file(credentials);
            return Outcome::Run(vec![Command::Quit]);
        }
        Command::Execute(command) => {
            info!("Executing command: {command}");
            let Ok(command) = std::ffi::CString::new(command.clone()) else {
                return Outcome::Error("The command contains a NUL byte".into());
            };
            let result = unsafe { libc::system(command.as_ptr()) };
            info!("Exit code: {result}");
        }
        Command::Reconnect => spotify.shutdown(),
        Command::Sleep(mode) => {
            let timer = match mode {
                SleepMode::Toggle if queue.sleep_timer() == Some(SleepTimer::EndOfTrack) => None,
                SleepMode::Toggle | SleepMode::EndOfTrack => Some(SleepTimer::EndOfTrack),
                SleepMode::After(seconds) => Some(SleepTimer::At(
                    std::time::Instant::now() + Duration::from_secs(*seconds),
                )),
                SleepMode::Off => None,
            };
            queue.set_sleep_timer(timer);
            return Outcome::Message(match (timer, mode) {
                (Some(SleepTimer::EndOfTrack), _) => "Pausing when this track ends".into(),
                (Some(SleepTimer::At(_)), SleepMode::After(seconds)) => format!(
                    "Pausing in {}",
                    crate::tui::style::duration((*seconds).saturating_mul(1000) as u32)
                ),
                _ => "Sleep timer off".into(),
            });
        }
        Command::Like => {
            if let Some(Playable::Track(track)) = queue.get_current() {
                ctx.library.toggle_saved_track_in_background(&track);
            }
        }
        Command::SaveCurrent => {
            if let Some(mut track) = queue.get_current() {
                track.save(&ctx.library);
            }
        }
        Command::AddCurrent => {
            if let Some(track) = queue.get_current().and_then(|t| t.track()) {
                return Outcome::Overlay(nav::add_to_playlist(vec![track], ctx));
            }
        }
        Command::Open(TargetMode::Current) => {
            if let Some(item) = queue.get_current() {
                return Outcome::Overlay(nav::context_menu(item.as_listitem(), ctx));
            }
        }
        #[cfg(feature = "share_clipboard")]
        Command::Share(TargetMode::Current) => {
            if let Some(url) = queue
                .get_current()
                .and_then(|t| t.as_listitem().share_url())
            {
                let _ = crate::sharing::write_share(url);
                return Outcome::Message("Link copied".into());
            }
        }
        Command::Insert(source) => {
            let url = match source {
                InsertSource::Input(url) => Some(url.clone()),
                #[cfg(feature = "share_clipboard")]
                InsertSource::Clipboard => crate::sharing::read_share()
                    .ok()
                    .and_then(SpotifyUrl::from_url),
            };
            return match url {
                Some(url) => open_link(&url, ctx),
                None => Outcome::Error("That isn't a Spotify link".into()),
            };
        }
        _ => {
            error!("unhandled command: {cmd}");
            return Outcome::Ignored;
        }
    }
    Outcome::Consumed
}
