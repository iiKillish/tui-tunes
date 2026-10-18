//! Listening history kept by ttunes for the stats page: a line for each song listened to for 30
//! seconds or more (as Spotify counts a play), in `listening.jsonl` in the cache directory.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};
use log::error;
use serde::{Deserialize, Serialize};

use crate::model::track::Track;

const FILE: &str = "listening.jsonl";

/// Shorter listens than this aren't plays.
const MIN_LISTEN: Duration = Duration::from_secs(30);

/// A song listened to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Play {
    /// When it stopped, in seconds since 1970.
    pub at: i64,
    /// How far into it the listening got, in milliseconds.
    pub ms: u64,
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
}

/// Note that `track` was listened to `listened` far, if that counts as a play: 30 seconds, or
/// most of a shorter track.
pub fn record(track: &Track, listened: Duration) {
    // Tests mustn't write to the real history.
    if cfg!(test) {
        return;
    }
    let length = Duration::from_millis(u64::from(track.duration));
    if listened < MIN_LISTEN.min(length * 4 / 5) || listened.is_zero() {
        return;
    }
    let play = Play {
        at: Local::now().timestamp(),
        ms: if length.is_zero() {
            listened
        } else {
            listened.min(length)
        }
        .as_millis() as u64,
        uri: track.uri.clone(),
        title: track.title.clone(),
        artists: track.artists.clone(),
    };
    let path = crate::config::cache_path(FILE);
    let line = match serde_json::to_string(&play) {
        Ok(line) => line,
        Err(e) => return error!("Couldn't note a play: {e}"),
    };
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(e) = written {
        error!("Couldn't note a play in {}: {e}", path.display());
    }
}

/// Every play noted so far, oldest first. Lines that can't be read are skipped.
pub fn load() -> Vec<Play> {
    std::fs::read_to_string(crate::config::cache_path(FILE))
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// What the stats page shows of the plays.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub today: Duration,
    /// The last seven days, today included.
    pub week: Duration,
    pub total: Duration,
    pub plays: usize,
    /// Days in a row with listening, up to today (or yesterday, if not yet today).
    pub streak: u32,
    /// Listening by hour of the day (local time).
    pub hours: [Duration; 24],
    /// The most played songs of the last 30 days: title, artists, plays.
    pub top_songs: Vec<(String, String, usize)>,
    /// The most listened to artists of the last 30 days, and for how long.
    pub top_artists: Vec<(String, Duration)>,
    /// When the first play was noted.
    pub since: Option<NaiveDate>,
}

/// Sum up `plays` as of `now`, with the `top` most played songs and artists.
pub fn summarize(plays: &[Play], now: DateTime<Local>, top: usize) -> Summary {
    let mut summary = Summary::default();
    let today = now.date_naive();
    let local = |at: i64| Local.timestamp_opt(at, 0).single();
    let mut days = std::collections::BTreeSet::new();
    let mut songs: HashMap<&str, (&str, String, usize)> = HashMap::new();
    let mut artists: HashMap<&str, Duration> = HashMap::new();
    for play in plays {
        let Some(at) = local(play.at) else {
            continue;
        };
        let listened = Duration::from_millis(play.ms);
        let day = at.date_naive();
        let age = (today - day).num_days();
        summary.total += listened;
        summary.plays += 1;
        summary.hours[at.hour() as usize] += listened;
        days.insert(day);
        summary.since = Some(summary.since.map_or(day, |since| since.min(day)));
        if age == 0 {
            summary.today += listened;
        }
        if (0..7).contains(&age) {
            summary.week += listened;
        }
        if (0..30).contains(&age) {
            songs
                .entry(&play.uri)
                .or_insert_with(|| (&play.title, play.artists.join(", "), 0))
                .2 += 1;
            for artist in &play.artists {
                *artists.entry(artist).or_default() += listened;
            }
        }
    }

    // The streak: back from today, or from yesterday if nothing's played yet today.
    let mut day = if days.contains(&today) {
        today
    } else {
        today.pred_opt().unwrap_or(today)
    };
    while days.contains(&day) {
        summary.streak += 1;
        day = match day.pred_opt() {
            Some(previous) => previous,
            None => break,
        };
    }

    let mut songs: Vec<_> = songs
        .into_values()
        .map(|(title, artists, count)| (title.to_string(), artists, count))
        .collect();
    songs.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    songs.truncate(top);
    summary.top_songs = songs;
    let mut artists: Vec<_> = artists
        .into_iter()
        .map(|(name, time)| (name.to_string(), time))
        .collect();
    artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    artists.truncate(top);
    summary.top_artists = artists;
    summary
}

/// `duration` as "2h 5m", "5m" or "0m".
pub fn hours_minutes(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// The weekday and date of `day`, like "Mon 3 Mar".
pub fn short_date(day: NaiveDate) -> String {
    format!("{} {} {}", day.weekday(), day.day(), day.format("%b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(at: DateTime<Local>, minutes: u64, title: &str, artist: &str) -> Play {
        Play {
            at: at.timestamp(),
            ms: minutes * 60_000,
            uri: format!("spotify:track:{title}"),
            title: title.into(),
            artists: vec![artist.into()],
        }
    }

    #[test]
    fn test_summary() {
        let now = Local.with_ymd_and_hms(2026, 10, 18, 21, 0, 0).unwrap();
        let day = chrono::Duration::days(1);
        let plays = vec![
            play(now - day * 40, 3, "Old", "Someone"),
            play(now - day * 2, 4, "Misery Business", "Paramore"),
            play(now - day, 4, "Misery Business", "Paramore"),
            play(now - day, 5, "Decode", "Paramore"),
            play(now, 3, "Misery Business", "Paramore"),
            play(now, 2, "good 4 u", "Olivia Rodrigo"),
        ];
        let summary = summarize(&plays, now, 5);
        assert_eq!(summary.today, Duration::from_secs(5 * 60));
        assert_eq!(summary.week, Duration::from_secs(18 * 60));
        assert_eq!(summary.total, Duration::from_secs(21 * 60));
        assert_eq!(summary.plays, 6);
        assert_eq!(summary.streak, 3);
        // By hour, where daylight saving time may move the old play.
        assert_eq!(summary.hours.iter().sum::<Duration>(), summary.total);
        assert_eq!(
            summary.top_songs[0],
            ("Misery Business".to_string(), "Paramore".to_string(), 3)
        );
        assert_eq!(summary.top_artists[0].0, "Paramore");
        assert_eq!(
            summary.top_artists.len(),
            2,
            "the 40-day-old play is left out"
        );
    }

    #[test]
    fn test_hours_minutes() {
        assert_eq!(hours_minutes(Duration::from_secs(59)), "0m");
        assert_eq!(hours_minutes(Duration::from_secs(5 * 60)), "5m");
        assert_eq!(hours_minutes(Duration::from_secs(2 * 3600 + 300)), "2h 5m");
        assert_eq!(hours_minutes(Duration::from_secs(3600)), "1h");
    }
}
