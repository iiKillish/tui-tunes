//! Song lyrics from [LRCLIB](https://lrclib.net), a free lyrics database without API keys. Most
//! songs there have time-synced lyrics (in the LRC format), which the side panel highlights line
//! by line as the song plays.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, error};
use serde::Deserialize;

use crate::model::track::Track;

const API: &str = "https://lrclib.net/api";
/// LRCLIB asks clients to identify themselves.
const USER_AGENT: &str = concat!(
    "ttunes/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/iiKillish/tui-tunes)"
);
/// How long to wait before trying again after a failed request.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Largest difference between a search result's length and the track's for it to count as the
/// same recording.
const MAX_DURATION_DIFFERENCE_SECS: f64 = 5.0;
/// Largest response read. Lyrics are a few kilobytes, a search's results some hundreds.
const MAX_RESPONSE_BYTES: u64 = 5 * 1024 * 1024;

/// The HTTP client for LRCLIB, shared so its connection is reused from song to song.
fn client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(CLIENT.get_or_init(|| client))
}

/// Parse the JSON `body`, failing instead of reading more than `max` bytes of it.
fn read_json<T: serde::de::DeserializeOwned>(
    body: impl std::io::Read,
    max: u64,
) -> Result<T, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    body.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("response too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// One line of synced lyrics: when it starts (milliseconds into the song) and its text. Empty
/// text marks an instrumental break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncedLine {
    pub time: u32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lyrics {
    /// Being downloaded.
    Loading,
    /// Lyrics with a start time for every line.
    Synced(Vec<SyncedLine>),
    /// Lyrics without timing.
    Plain(Vec<String>),
    /// Known to have no lyrics.
    Instrumental,
    /// Not in LRCLIB.
    NotFound,
    /// The request failed; retried after [RETRY_AFTER].
    Failed(String),
}

impl Lyrics {
    /// Index of the line being sung `progress` milliseconds into the song: the last one that has
    /// started. `None` before the first line or for unsynced lyrics.
    pub fn current_line(&self, progress: u32) -> Option<usize> {
        match self {
            Self::Synced(lines) => lines
                .partition_point(|line| line.time <= progress)
                .checked_sub(1),
            _ => None,
        }
    }
}

/// Fetched lyrics per track, downloaded in the background on first use.
#[derive(Clone, Default)]
pub struct LyricsStore {
    cache: Arc<RwLock<HashMap<String, (Lyrics, Instant)>>>,
}

impl LyricsStore {
    /// The lyrics of `track`, starting a download if they aren't known yet (or the last attempt
    /// failed a while ago). `on_update` is called from the download thread when it finishes.
    pub fn get(&self, track: &Track, on_update: impl FnOnce() + Send + 'static) -> Lyrics {
        let key = track.id.clone().unwrap_or_else(|| track.uri.clone());
        if let Some((lyrics, fetched)) = self.cache.read().unwrap().get(&key) {
            let retry = matches!(lyrics, Lyrics::Failed(_)) && fetched.elapsed() > RETRY_AFTER;
            if !retry {
                return lyrics.clone();
            }
        }

        self.cache
            .write()
            .unwrap()
            .insert(key.clone(), (Lyrics::Loading, Instant::now()));
        // Tests stay offline: a real lookup would race the screen being drawn.
        if cfg!(test) {
            return Lyrics::Loading;
        }
        let cache = self.cache.clone();
        let track = track.clone();
        thread::spawn(move || {
            let lyrics = fetch(&track).unwrap_or_else(|e| {
                error!("Failed to fetch lyrics for {}: {e}", track.title);
                Lyrics::Failed(e)
            });
            debug!("fetched lyrics for {}", track.title);
            cache.write().unwrap().insert(key, (lyrics, Instant::now()));
            on_update();
        });
        Lyrics::Loading
    }
}

/// An LRCLIB lyrics record.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    instrumental: bool,
    duration: Option<f64>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Record {
    fn into_lyrics(self) -> Lyrics {
        if let Some(synced) = self.synced_lyrics.as_deref() {
            let lines = parse_lrc(synced);
            if !lines.is_empty() {
                return Lyrics::Synced(lines);
            }
        }
        match self.plain_lyrics {
            Some(plain) if !plain.trim().is_empty() => {
                Lyrics::Plain(plain.lines().map(|l| l.trim().to_string()).collect())
            }
            _ if self.instrumental => Lyrics::Instrumental,
            _ => Lyrics::NotFound,
        }
    }
}

/// Look up `track`: first an exact match on title, artist, album and length, then a search by
/// title and artist (picking the closest length, preferring synced lyrics).
fn fetch(track: &Track) -> Result<Lyrics, String> {
    let client = client()?;
    let artist = track
        .artists
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    let duration_secs = f64::from(track.duration) / 1000.0;

    let mut params = vec![
        ("track_name", track.title.clone()),
        ("artist_name", artist.to_string()),
        ("duration", (track.duration / 1000).to_string()),
    ];
    if let Some(album) = &track.album {
        params.push(("album_name", album.clone()));
    }
    let url =
        url::Url::parse_with_params(&format!("{API}/get"), &params).map_err(|e| e.to_string())?;
    let response = client.get(url).send().map_err(|e| e.to_string())?;
    if response.status().is_success() {
        let record: Record = read_json(response, MAX_RESPONSE_BYTES)?;
        return Ok(record.into_lyrics());
    }
    if response.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(format!("LRCLIB returned {}", response.status()));
    }

    let url = url::Url::parse_with_params(
        &format!("{API}/search"),
        &[
            ("track_name", track.title.as_str()),
            ("artist_name", artist),
        ],
    )
    .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let records: Vec<Record> = read_json(response, MAX_RESPONSE_BYTES)?;
    let best = records
        .into_iter()
        .filter_map(|r| {
            let difference = (r.duration? - duration_secs).abs();
            (difference <= MAX_DURATION_DIFFERENCE_SECS).then_some((r, difference))
        })
        // Synced lyrics first, then the closest length.
        .min_by(|(a, da), (b, db)| {
            b.synced_lyrics
                .is_some()
                .cmp(&a.synced_lyrics.is_some())
                .then(da.total_cmp(db))
        });
    Ok(best.map_or(Lyrics::NotFound, |(record, _)| record.into_lyrics()))
}

/// Parse LRC lyrics: lines like `[01:23.45]text`, possibly with several timestamps for a repeated
/// line (`[00:10.00][01:10.00]chorus`). Metadata tags (`[ar:Artist]`) and untimed lines are
/// skipped. The result is sorted by time.
pub fn parse_lrc(text: &str) -> Vec<SyncedLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some(end) = tag.find(']') else { break };
            let Some(time) = parse_timestamp(&tag[..end]) else {
                break;
            };
            times.push(time);
            rest = &tag[end + 1..];
        }
        let text = rest.trim();
        lines.extend(times.into_iter().map(|time| SyncedLine {
            time,
            text: text.to_string(),
        }));
    }
    lines.sort_by_key(|line| line.time);
    lines
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx` in milliseconds.
fn parse_timestamp(tag: &str) -> Option<u32> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u32 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.trim().parse().ok()?;
    let millis = (seconds * 1000.0).round();
    // The text comes from LRCLIB, so a time too large for a u32 (or NaN or infinity, which
    // parse as numbers too) makes the tag invalid rather than overflowing.
    if seconds < 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&millis) {
        return None;
    }
    minutes.checked_mul(60_000)?.checked_add(millis as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_read_up_to_a_limit() {
        let body = br#"[1, 2, 3]"#;
        assert_eq!(read_json::<Vec<u8>>(&body[..], 9), Ok(vec![1, 2, 3]));
        assert!(read_json::<Vec<u8>>(&body[..], 8).is_err());
    }

    fn line(time: u32, text: &str) -> SyncedLine {
        SyncedLine {
            time,
            text: text.to_string(),
        }
    }

    #[test]
    fn test_parse_timestamp() {
        assert_eq!(parse_timestamp("01:02.345"), Some(62345));
        assert_eq!(parse_timestamp("71582:00"), Some(4_294_920_000));
        assert_eq!(parse_timestamp("00:-1"), None);
        assert_eq!(parse_timestamp("00:NaN"), None);
        assert_eq!(parse_timestamp("00:inf"), None);
    }

    #[test]
    fn test_parse_timestamp_overflow_is_invalid() {
        // Each of these overflowed a u32 (panicking in debug builds).
        assert_eq!(parse_timestamp("71583:00"), None);
        assert_eq!(parse_timestamp("71582:47.296"), None);
        assert_eq!(parse_timestamp("4294967295:00"), None);
        assert_eq!(parse_timestamp("00:99999999999"), None);
        assert_eq!(
            parse_lrc("[71583:00]Too late\n[00:01]Fine"),
            vec![line(1000, "Fine")]
        );
    }

    #[test]
    fn test_parse_lrc() {
        let lrc = "[ar:Someone]\n[00:01.50] First\n[00:03.25]\n[01:02.345]Third";
        assert_eq!(
            parse_lrc(lrc),
            vec![line(1500, "First"), line(3250, ""), line(62345, "Third")]
        );
    }

    #[test]
    fn test_parse_lrc_repeated_lines_are_sorted() {
        let lrc = "[00:10.00][00:30.00]Chorus\n[00:20.00]Verse";
        assert_eq!(
            parse_lrc(lrc),
            vec![
                line(10000, "Chorus"),
                line(20000, "Verse"),
                line(30000, "Chorus")
            ]
        );
    }

    #[test]
    fn test_current_line() {
        let lyrics = Lyrics::Synced(vec![line(1000, "a"), line(2000, "b"), line(3000, "c")]);
        assert_eq!(lyrics.current_line(500), None);
        assert_eq!(lyrics.current_line(1000), Some(0));
        assert_eq!(lyrics.current_line(2999), Some(1));
        assert_eq!(lyrics.current_line(10000), Some(2));
        assert_eq!(Lyrics::Plain(vec![]).current_line(1000), None);
    }

    #[test]
    fn test_record_prefers_synced_and_handles_instrumentals() {
        let record = |synced: Option<&str>, plain: Option<&str>, instrumental| Record {
            instrumental,
            duration: None,
            plain_lyrics: plain.map(str::to_string),
            synced_lyrics: synced.map(str::to_string),
        };
        assert_eq!(
            record(Some("[00:01.00]x"), Some("x"), false).into_lyrics(),
            Lyrics::Synced(vec![line(1000, "x")])
        );
        assert_eq!(
            record(None, Some("a\nb"), false).into_lyrics(),
            Lyrics::Plain(vec!["a".into(), "b".into()])
        );
        assert_eq!(record(None, None, true).into_lyrics(), Lyrics::Instrumental);
        assert_eq!(record(None, None, false).into_lyrics(), Lyrics::NotFound);
    }
}
