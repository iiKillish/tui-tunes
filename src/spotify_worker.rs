use crate::config;
use crate::events::{Event, EventManager};
use crate::model::playable::Playable;
use crate::queue::QueueEvent;
use crate::spotify::{PlayerEvent, Spotify};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use librespot_core::SpotifyUri;
use librespot_core::authentication::Credentials;
use librespot_core::session::Session;
use librespot_metadata::{Artist, Metadata, Playlist, Track};
use librespot_playback::mixer::Mixer;
use librespot_playback::player::{Player, PlayerEvent as LibrespotPlayerEvent};
use librespot_protocol::autoplay_context_request::AutoplayContextRequest;
use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use tokio::sync::mpsc;
use tokio::time;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn reconnect_delay(failed_attempts: u32) -> Duration {
    Duration::from_secs(
        2u64.saturating_pow(failed_attempts.saturating_sub(1))
            .min(30),
    )
}

#[derive(Debug)]
pub(crate) enum WorkerCommand {
    Load(Playable, bool, u32),
    Play,
    Pause,
    Stop,
    Seek(u32),
    SetVolume(u16),
    Preload(Playable),
    /// Ask Spotify's autoplay for tracks to follow a context; the track URIs are sent back.
    Autoplay(AutoplayContextRequest, oneshot::Sender<Vec<String>>),
    /// Load a playlist through the session; see [session_playlist].
    SessionPlaylist(SpotifyUri, oneshot::Sender<Option<Playlist>>),
    /// Load an artist's top tracks (their URIs) through the session; see [session_top_tracks].
    SessionTopTracks(SpotifyUri, oneshot::Sender<Vec<SpotifyUri>>),
    /// Load tracks through the session; see [session_tracks].
    SessionTracks(Vec<SpotifyUri>, oneshot::Sender<Vec<Track>>),
    /// Load artists' pictures through the session; see [session_artist_pictures].
    SessionArtistPictures(Vec<SpotifyUri>, oneshot::Sender<Vec<Option<String>>>),
    Shutdown,
}

enum PlayerStatus {
    Playing,
    Paused,
    Stopped,
}

/// The playlist at `uri`, loaded through the session the way Spotify's own apps do rather than
/// through the Web API, which no longer gives apps the playlists Spotify makes (Daily Mix,
/// Discover Weekly, editorial playlists). Undocumented, so it may stop working.
async fn session_playlist(session: &Session, uri: &SpotifyUri) -> Option<Playlist> {
    let playlist = match Playlist::get(session, uri).await {
        Ok(playlist) => playlist,
        Err(e) => {
            warn!("Couldn't load {uri:?} through the session: {e}");
            return None;
        }
    };
    debug!(
        "Loaded {uri:?} through the session: {:?}, {} items, picture {:?}, picture sizes {:?}",
        playlist.name(),
        playlist.contents.items.len(),
        playlist.attributes.picture,
        playlist.attributes.picture_sizes
    );
    Some(playlist)
}

/// The top tracks in the user's country of the artist at `uri`, loaded through the session: the
/// Web API no longer gives them to apps. Undocumented, so it may stop working.
async fn session_top_tracks(session: &Session, uri: &SpotifyUri) -> Vec<SpotifyUri> {
    match Artist::get(session, uri).await {
        Ok(artist) => artist.top_tracks.for_country(&session.country()).0,
        Err(e) => {
            warn!("Couldn't load {uri:?} through the session: {e}");
            Vec::new()
        }
    }
}

/// The tracks at `uris`, loaded through the session ten at a time, in order; ones that fail are
/// left out.
async fn session_tracks(session: &Session, uris: Vec<SpotifyUri>) -> Vec<Track> {
    let count = uris.len();
    let loading = futures::StreamExt::map(futures::stream::iter(uris), |uri| async move {
        Track::get(session, &uri)
            .await
            .inspect_err(|e| warn!("Couldn't load {uri:?} through the session: {e}"))
            .ok()
    });
    let loaded: Vec<Option<Track>> =
        futures::StreamExt::collect(futures::StreamExt::buffered(loading, 10)).await;
    let tracks: Vec<Track> = loaded.into_iter().flatten().collect();
    debug!(
        "Loaded {} of {count} tracks through the session",
        tracks.len()
    );
    tracks
}

/// The URLs of the pictures of the artists at `uris`, in order, loaded through the session ten at
/// a time: `None` where there's no picture or the artist couldn't be loaded. Undocumented, so it
/// may stop working.
async fn session_artist_pictures(session: &Session, uris: Vec<SpotifyUri>) -> Vec<Option<String>> {
    let loading = futures::StreamExt::map(futures::stream::iter(uris), |uri| async move {
        let artist = Artist::get(session, &uri)
            .await
            .inspect_err(|e| warn!("Couldn't load {uri:?} through the session: {e}"))
            .ok()?;
        artist
            .portraits
            .iter()
            .chain(artist.portrait_group.iter())
            .max_by_key(|image| image.width)
            .and_then(|image| image.id.to_base16().ok())
            .map(|hex| format!("https://i.scdn.co/image/{hex}"))
    });
    futures::StreamExt::collect(futures::StreamExt::buffered(loading, 10)).await
}

pub struct Worker {
    events: EventManager,
    player_events: UnboundedReceiverStream<LibrespotPlayerEvent>,
    commands: UnboundedReceiverStream<WorkerCommand>,
    session: Session,
    player: Arc<Player>,
    player_status: PlayerStatus,
    mixer: Arc<dyn Mixer>,
    cfg: Arc<config::Config>,
    credentials: Credentials,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        events: EventManager,
        player_events: mpsc::UnboundedReceiver<LibrespotPlayerEvent>,
        commands: mpsc::UnboundedReceiver<WorkerCommand>,
        session: Session,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
        cfg: Arc<config::Config>,
        credentials: Credentials,
    ) -> Self {
        Self {
            events,
            player_events: UnboundedReceiverStream::new(player_events),
            commands: UnboundedReceiverStream::new(commands),
            player,
            session,
            player_status: PlayerStatus::Stopped,
            mixer,
            cfg,
            credentials,
        }
    }

    pub async fn run_loop(&mut self) {
        let mut ui_refresh = time::interval(Duration::from_millis(400));
        let mut reconnect: Option<BoxFuture<'static, Result<Session, librespot_core::Error>>> =
            None;
        let mut failed_attempts = 0;
        let mut loading = None;
        let mut load_failed = false;
        let mut preload = None;
        let mut preload_retried = false;
        // The play request of the track playing, and whether it has played to its end.
        let mut playing_request = None;
        let mut ended = false;
        // The play request paused to make way for another track (see the Load command), whose
        // Paused event isn't passed on.
        let mut interrupted = None;

        loop {
            if self.session.is_invalid() && reconnect.is_none() {
                let delay = reconnect_delay(failed_attempts);
                warn!("Librespot session invalidated, reconnecting in {delay:?}");
                let cache = self.session.cache();
                // a login's access token expires, the stored credentials don't
                let credentials = cache
                    .and_then(|cache| cache.credentials())
                    .unwrap_or_else(|| self.credentials.clone());
                let session = Session::new(
                    Spotify::session_config(&self.cfg),
                    cache.map(|cache| cache.as_ref().clone()),
                );
                reconnect = Some(Box::pin(async move {
                    time::sleep(delay).await;
                    time::timeout(CONNECT_TIMEOUT, session.connect(credentials, true))
                        .await
                        .unwrap_or_else(|e| Err(librespot_core::Error::deadline_exceeded(e)))
                        .map(|_| session)
                }));
            }

            tokio::select! {
                result = async { reconnect.as_mut().unwrap().await }, if reconnect.is_some() => {
                    reconnect = None;
                    match result {
                        Ok(session) => {
                            info!("Librespot session reconnected");
                            self.player.set_session(session.clone());
                            self.session = session;
                            failed_attempts = 0;
                            load_failed = false;
                            preload_retried = false;
                            if let Some((uri, start_playing, position_ms)) = loading.clone() {
                                self.player.load(uri, start_playing, position_ms);
                            }
                            if let Some(uri) = preload.clone() {
                                self.player.preload(uri);
                            }
                        }
                        Err(e) => {
                            failed_attempts += 1;
                            error!("Reconnecting failed: {e}");
                        }
                    }
                },
                cmd = self.commands.next() => match cmd {
                    Some(WorkerCommand::Load(playable, start_playing, position_ms)) => {
                        loading = None;
                        preload = None;
                        match SpotifyUri::from_uri(&playable.uri()) {
                            Ok(uri) => {
                                info!("player loading track: {uri:?}");
                                if !uri.is_playable() {
                                    warn!("track is not playable");
                                    self.events.send(Event::Player(PlayerEvent::FinishedTrack));
                                } else {
                                    // What the sink has of the track before isn't to be heard,
                                    // unless that played to its end and runs into this one.
                                    let playing =
                                        matches!(self.player_status, PlayerStatus::Playing);
                                    if !(playing && ended) {
                                        crate::audio::discard_buffered();
                                    }
                                    // Loading leaves the sink running (for gapless playback), so
                                    // what it has buffered of a track still playing would be
                                    // heard first: pausing stops it.
                                    if playing && !ended {
                                        interrupted = playing_request;
                                        self.player.pause();
                                    }
                                    self.player.load(uri.clone(), start_playing, position_ms);
                                    loading = Some((uri, start_playing, position_ms));
                                    load_failed = false;
                                }
                            }
                            Err(e) => {
                                error!("error parsing uri: {e:?}");
                                self.events.send(Event::Player(PlayerEvent::FinishedTrack));
                            }
                        }
                    }
                    Some(WorkerCommand::Play) => {
                        if let Some((uri, start_playing, position_ms)) = loading.as_mut() {
                            *start_playing = true;
                            if load_failed && !self.session.is_invalid() {
                                self.player.load(uri.clone(), true, *position_ms);
                            }
                        }
                        self.player.play();
                    }
                    Some(WorkerCommand::Pause) => {
                        if let Some((_, start_playing, _)) = loading.as_mut() {
                            *start_playing = false;
                        }
                        self.player.pause();
                    }
                    Some(WorkerCommand::Stop) => {
                        crate::audio::discard_buffered();
                        loading = None;
                        preload = None;
                        self.player.stop();
                    }
                    Some(WorkerCommand::Seek(pos)) => {
                        // A stopped sink keeps audio from before the seek.
                        if !matches!(self.player_status, PlayerStatus::Playing) {
                            crate::audio::discard_buffered();
                        }
                        if let Some((_, _, position_ms)) = loading.as_mut() {
                            *position_ms = pos;
                        }
                        self.player.seek(pos);
                    }
                    Some(WorkerCommand::SetVolume(volume)) => {
                        self.mixer.set_volume(volume);
                    }
                    Some(WorkerCommand::Preload(playable)) => {
                        if let Ok(uri) = SpotifyUri::from_uri(&playable.uri()) {
                            debug!("Preloading {uri:?}");
                            self.player.preload(uri.clone());
                            preload = Some(uri);
                            preload_retried = false;
                        }
                    }
                    Some(WorkerCommand::Autoplay(request, reply)) => {
                        let session = self.session.clone();
                        tokio::spawn(async move {
                            let uris = match session.spclient().get_autoplay_context(&request).await {
                                Ok(context) => context
                                    .pages
                                    .into_iter()
                                    .flat_map(|page| page.tracks)
                                    .filter_map(|track| track.uri)
                                    .filter(|uri| uri.starts_with("spotify:track:"))
                                    .collect(),
                                Err(e) => {
                                    warn!("Autoplay failed: {e}");
                                    Vec::new()
                                }
                            };
                            let _ = reply.send(uris);
                        });
                    }
                    Some(WorkerCommand::SessionPlaylist(uri, reply)) => {
                        let session = self.session.clone();
                        tokio::spawn(async move {
                            let _ = reply.send(session_playlist(&session, &uri).await);
                        });
                    }
                    Some(WorkerCommand::SessionTopTracks(uri, reply)) => {
                        let session = self.session.clone();
                        tokio::spawn(async move {
                            let _ = reply.send(session_top_tracks(&session, &uri).await);
                        });
                    }
                    Some(WorkerCommand::SessionTracks(uris, reply)) => {
                        let session = self.session.clone();
                        tokio::spawn(async move {
                            let _ = reply.send(session_tracks(&session, uris).await);
                        });
                    }
                    Some(WorkerCommand::SessionArtistPictures(uris, reply)) => {
                        let session = self.session.clone();
                        tokio::spawn(async move {
                            let _ = reply.send(session_artist_pictures(&session, uris).await);
                        });
                    }
                    Some(WorkerCommand::Shutdown) => {
                        info!("Librespot session shut down, terminating worker");
                        self.player.stop();
                        self.session.shutdown();
                        self.events.send(Event::Player(PlayerEvent::Stopped));
                        break;
                    }
                    None => info!("empty stream")
                },
                event = self.player_events.next() => match event {
                    Some(LibrespotPlayerEvent::Playing {
                        play_request_id,
                        track_id,
                        position_ms,
                    }) => {
                        playing_request = Some(play_request_id);
                        ended = false;
                        loading.take_if(|(uri, ..)| *uri == track_id);
                        let position = Duration::from_millis(position_ms as u64);
                        let playback_start = SystemTime::now() - position;
                        self.events
                            .send(Event::Player(PlayerEvent::Playing(playback_start)));
                        self.player_status = PlayerStatus::Playing;
                    }
                    Some(LibrespotPlayerEvent::Paused {
                        play_request_id,
                        track_id,
                        position_ms,
                    }) => {
                        if interrupted.take_if(|id| *id == play_request_id).is_some() {
                            continue;
                        }
                        loading.take_if(|(uri, ..)| *uri == track_id);
                        let position = Duration::from_millis(position_ms as u64);
                        self.events
                            .send(Event::Player(PlayerEvent::Paused(position)));
                        self.player_status = PlayerStatus::Paused;
                    }
                    Some(LibrespotPlayerEvent::Stopped { .. }) => {
                        self.events.send(Event::Player(PlayerEvent::Stopped));
                        self.player_status = PlayerStatus::Stopped;
                    }
                    Some(LibrespotPlayerEvent::EndOfTrack { .. }) => {
                        ended = true;
                        self.events.send(Event::Player(PlayerEvent::FinishedTrack));
                    }
                    Some(LibrespotPlayerEvent::TrackChanged { audio_item }) => {
                        if let Some(cache) = self.session.cache() {
                            let paths = audio_item
                                .files
                                .values()
                                .filter_map(|file| cache.file_path(*file))
                                .collect();
                            tokio::task::spawn_blocking(move || mark_used(paths));
                        }
                    }
                    Some(LibrespotPlayerEvent::TimeToPreloadNextTrack { .. }) => {
                        self.events
                            .send(Event::Queue(QueueEvent::PreloadTrackRequest));
                    }
                    Some(LibrespotPlayerEvent::Seeked { play_request_id: _, track_id: _, position_ms}) => {
                        let position = Duration::from_millis(position_ms as u64);
                        let event = match self.player_status {
                            PlayerStatus::Playing => {
                                let playback_start = SystemTime::now() - position;
                                PlayerEvent::Playing(playback_start)
                            },
                            PlayerStatus::Paused => PlayerEvent::Paused(position),
                            PlayerStatus::Stopped => PlayerEvent::Stopped,
                        };
                        self.events.send(Event::Player(event));
                    }
                    Some(LibrespotPlayerEvent::Unavailable { track_id, .. }) => {
                        if let Some((uri, start_playing, position_ms)) =
                            loading.clone().filter(|(uri, ..)| *uri == track_id)
                        {
                            if load_failed || self.session.is_invalid() {
                                warn!("Unable to load {uri:?}, holding it");
                                let position = Duration::from_millis(position_ms as u64);
                                self.events
                                    .send(Event::Player(PlayerEvent::Paused(position)));
                                self.player_status = PlayerStatus::Paused;
                            } else {
                                warn!("Unable to load {uri:?}, retrying");
                                self.player.load(uri, start_playing, position_ms);
                            }
                            load_failed = true;
                        } else if preload.as_ref() == Some(&track_id)
                            && !preload_retried
                            && !self.session.is_invalid()
                        {
                            preload_retried = true;
                            self.player.preload(track_id);
                        }
                    }
                    Some(event) => {
                        debug!("Unhandled player event: {event:?}");
                    }
                    None => {
                        warn!("Librespot player event channel died, terminating worker");
                        break
                    },
                },
                // Update animated parts of the UI (e.g. statusbar during playback).
                _ = ui_refresh.tick() => {
                    if !matches!(self.player_status, PlayerStatus::Stopped) {
                        self.events.trigger();
                    }
                },
            }
        }
    }
}

/// Mark the cached audio files at `paths` (those there are) as just used. When the cache is full,
/// librespot deletes the files used longest ago, judged across restarts by their access times,
/// which reading them doesn't update on filesystems mounted `noatime`: a song played again would
/// be deleted as if it hadn't been.
fn mark_used(paths: Vec<std::path::PathBuf>) {
    let used = std::fs::FileTimes::new().set_accessed(SystemTime::now());
    for path in paths {
        if let Ok(file) = std::fs::File::open(&path)
            && let Err(e) = file.set_times(used)
        {
            debug!("Couldn't mark {} as used: {e}", path.display());
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        debug!("Worker thread is shutting down, stopping player");
        self.player.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::reconnect_delay;
    use std::time::Duration;

    #[test]
    fn reconnect_delay_doubles_up_to_half_a_minute() {
        let delays: Vec<_> = (0..=7).map(reconnect_delay).collect();
        let expected: Vec<_> = [1, 1, 2, 4, 8, 16, 30, 30].map(Duration::from_secs).into();
        assert_eq!(delays, expected);
    }
}
