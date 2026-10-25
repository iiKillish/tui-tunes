//! Music files on this computer, played through librespot: it indexes the files in the
//! `music_dirs` folders when the player starts (see [directories]) and plays a file given its
//! `spotify:local:` URI, made from the file's tags and length. [scan] reads the same folders the
//! same way, so the URIs it makes are the ones librespot knows, and keeps their folders for
//! browsing.

use std::collections::HashSet;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librespot_core::SpotifyUri;
use log::{debug, warn};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, StandardVisualKey, Visual};
use symphonia::core::probe::Hint;

use crate::config::Config;
use crate::model::folder::{Entry, Folder};
use crate::model::track::Track;

/// The files librespot looks for. It takes .mp4 and .m4p too, but can't play them: symphonia
/// 0.5's MP4 reader leaves out AAC's channel count, which librespot's decoder needs.
const EXTENSIONS: &[&str] = &["mp3", "flac"];

/// What librespot's player plays: it has no resampling, so it turns down other files.
const SAMPLE_RATE: u32 = librespot_playback::SAMPLE_RATE;
const CHANNELS: usize = librespot_playback::NUM_CHANNELS as usize;

/// The folders to find music in: the `music_dirs` setting (`~` for the home folder), or the
/// system's music folder (`~/Music`, `$XDG_MUSIC_DIR`) if there is one.
pub fn directories(cfg: &Config) -> Vec<PathBuf> {
    match &cfg.values().music_dirs {
        Some(dirs) => dirs.iter().map(|dir| expand_home(dir)).collect(),
        None => platform_dirs::UserDirs::new()
            .map(|dirs| dirs.music_dir)
            .filter(|dir| dir.is_dir())
            .into_iter()
            .collect(),
    }
}

fn expand_home(path: &str) -> PathBuf {
    let home = std::env::home_dir;
    match path.strip_prefix("~/") {
        Some(rest) => home().map_or_else(|| PathBuf::from(path), |home| home.join(rest)),
        None if path == "~" => home().unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// The music in `dirs` that librespot can play, as folders of subfolders and songs: what's in
/// the folder for one folder, a folder for each of several. Slow for a big collection (each file
/// is opened), so run off the UI thread.
pub fn scan(dirs: &[PathBuf]) -> Vec<Entry> {
    scan_saving_covers(dirs, &crate::config::cache_path("local-covers"))
}

/// [scan], saving the pictures in the files to `covers` (see [embedded_cover]).
fn scan_saving_covers(dirs: &[PathBuf], covers: &Path) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            warn!("Music folder {} isn't a folder", dir.display());
            continue;
        }
        // librespot gives up on a folder at the first entry it can't read, as here.
        match visit(dir, None, &mut seen, covers) {
            Ok(folder) if !folder.entries.is_empty() => roots.push(folder),
            Ok(_) => {}
            Err(e) => warn!("Couldn't read music folder {}: {e}", dir.display()),
        }
    }
    match <[Folder; 1]>::try_from(roots) {
        Ok([root]) => root.entries,
        Err(roots) => roots
            .into_iter()
            .map(|folder| Entry::Folder(Arc::new(folder)))
            .collect(),
    }
}

/// The folder `dir`, leaving out subfolders without songs. Files with the same tags and length
/// have the same URI, one song to librespot: `seen` keeps them from being listed twice. Without a
/// picture of its own, a folder has `parent_cover`, its parent's.
fn visit(
    dir: &Path,
    parent_cover: Option<&str>,
    seen: &mut HashSet<String>,
    covers: &Path,
) -> io::Result<Folder> {
    let mut folders = Vec::new();
    let mut tracks = Vec::new();
    let cover = folder_image(dir).or_else(|| parent_cover.map(str::to_string));
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            let folder = visit(&path, cover.as_deref(), seen, covers)?;
            if !folder.entries.is_empty() {
                folders.push(folder);
            }
            continue;
        }
        let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !EXTENSIONS.contains(&extension.to_lowercase().as_str()) {
            continue;
        }
        match read(&path, extension, cover.as_deref(), covers) {
            Some(track) if seen.insert(track.uri.clone()) => tracks.push(track),
            Some(_) => debug!("{} is the same song as another file", path.display()),
            None => debug!(
                "Can't play {}: no tags, no length, or not 44.1 kHz stereo",
                path.display()
            ),
        }
    }
    folders.sort_by_cached_key(|folder| folder.name.to_lowercase());
    tracks.sort_by_cached_key(|track| {
        (
            track.album.as_deref().unwrap_or_default().to_lowercase(),
            track.disc_number,
            track.track_number,
            track.title.to_lowercase(),
        )
    });
    for (index, track) in tracks.iter_mut().enumerate() {
        track.list_index = index;
    }
    // The folder's own picture (or its parent's), else the first one in it.
    let cover = cover
        .or_else(|| tracks.iter().find_map(|track| track.cover_url.clone()))
        .or_else(|| folders.iter().find_map(|folder| folder.cover_url.clone()));
    Ok(Folder {
        cover_url: cover,
        name: dir.file_name().map_or_else(
            || dir.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        entries: folders
            .into_iter()
            .map(|folder| Entry::Folder(Arc::new(folder)))
            .chain(tracks.into_iter().map(Entry::Track))
            .collect(),
    })
}

/// The track in the file at `path`, as librespot sees it (see `local_file.rs` in
/// librespot-playback): `None` for files it skips, those without tags or a known length, and
/// for files it can't play. Its cover is the picture in the file (saved to `covers`), else
/// `folder_cover`.
fn read(path: &Path, extension: &str, folder_cover: Option<&str>, covers: &Path) -> Option<Track> {
    let source = MediaSourceStream::new(Box::new(File::open(path).ok()?), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;

    // The container's tags, else those found probing (ID3 in MP3s); the latest revision.
    let mut metadata = probed.format.metadata();
    if metadata.current().is_none()
        && let Some(probe_metadata) = probed.metadata.get()
    {
        metadata = probe_metadata;
    }
    metadata.skip_to_latest();
    let revision = metadata.current()?;
    let tags = revision.tags().to_vec();
    let embedded_cover = embedded_cover(revision.visuals(), covers);

    let (mut artist, mut album, mut title, mut album_artist) = (None, None, None, None);
    let (mut track_number, mut disc_number) = (0, 0);
    for tag in tags {
        let value = tag.value.to_string();
        match tag.std_key {
            Some(StandardTagKey::Artist) => artist = Some(value),
            Some(StandardTagKey::Album) => album = Some(value),
            Some(StandardTagKey::TrackTitle) => title = Some(value),
            Some(StandardTagKey::AlbumArtist) => album_artist = Some(value),
            Some(StandardTagKey::TrackNumber) => track_number = leading_number(&value),
            Some(StandardTagKey::DiscNumber) => disc_number = leading_number(&value),
            _ => {}
        }
    }

    let params = &probed.format.default_track()?.codec_params;
    if params.sample_rate != Some(SAMPLE_RATE)
        || params.channels.map(|channels| channels.count()) != Some(CHANNELS)
    {
        return None;
    }
    let time = params.time_base?.calc_time(params.n_frames?);
    // librespot divides by the length in whole seconds.
    if time.seconds == 0 {
        return None;
    }
    let encode = |part: &Option<String>| -> String {
        url::form_urlencoded::byte_serialize(part.as_deref().unwrap_or_default().as_bytes())
            .collect()
    };
    let uri = SpotifyUri::Local {
        artist: encode(&artist),
        album_title: encode(&album),
        track_title: encode(&title),
        duration: Duration::from_secs(time.seconds),
    }
    .to_uri()
    .ok()?;

    let artists = vec![artist.unwrap_or_else(|| "Unknown artist".into())];
    Some(Track {
        id: None,
        uri,
        title: title.unwrap_or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        track_number,
        disc_number: disc_number as i32,
        duration: (time.seconds * 1000 + (time.frac * 1000.0) as u64) as u32,
        album_artists: album_artist.map_or_else(|| artists.clone(), |a| vec![a]),
        artists,
        artist_ids: Vec::new(),
        album,
        album_id: None,
        cover_url: embedded_cover.or_else(|| folder_cover.map(str::to_string)),
        url: String::new(),
        added_at: None,
        list_index: 0,
        is_local: true,
        is_playable: Some(true),
    })
}

/// Names a folder's picture goes by (any case), in the order they're looked for.
const FOLDER_IMAGES: &[&str] = &["cover", "folder", "front", "album"];
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// The `file://` URL of the picture for the folder `dir`: `cover.jpg`, `folder.png` and the like.
fn folder_image(dir: &Path) -> Option<String> {
    let images: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        })
        .collect();
    let stem = |path: &Path| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
    };
    FOLDER_IMAGES.iter().find_map(|name| {
        let image = images
            .iter()
            .find(|path| stem(path).as_deref() == Some(name))?;
        file_url(image)
    })
}

/// The `file://` URL of the front cover among `visuals` (else the first picture), saved to the
/// folder `covers` as its own file the first time it's seen: covers are drawn from files. Songs
/// of an album have the same picture, saved once.
fn embedded_cover(visuals: &[Visual], covers: &Path) -> Option<String> {
    let visual = visuals
        .iter()
        .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
        .or_else(|| visuals.first())?;
    let extension = match visual.media_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        _ => return None,
    };
    let mut hasher = std::hash::DefaultHasher::new();
    visual.data.hash(&mut hasher);
    let path = covers.join(format!("{:016x}.{extension}", hasher.finish()));
    if !path.exists() {
        // Written whole, then moved into place, so a cover is never read half written.
        let partial = path.with_extension("part");
        let saved = fs::create_dir_all(covers)
            .and_then(|()| fs::write(&partial, &visual.data))
            .and_then(|()| fs::rename(&partial, &path));
        if let Err(e) = saved {
            warn!("Couldn't save a cover to {}: {e}", path.display());
            return None;
        }
    }
    file_url(&path)
}

fn file_url(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// The number a tag like "3" or "3/12" starts with.
fn leading_number(value: &str) -> u32 {
    let digits: String = value
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_track_numbers_with_totals() {
        assert_eq!(leading_number("3/12"), 3);
        assert_eq!(leading_number(" 07 "), 7);
        assert_eq!(leading_number("A1"), 0);
    }

    #[test]
    fn expands_the_home_folder() {
        let home = std::env::home_dir().unwrap();
        assert_eq!(expand_home("~/Music"), home.join("Music"));
        assert_eq!(expand_home("music"), PathBuf::from("music"));
    }

    /// A folder of `files` (path, fixture name), removed when dropped.
    struct MusicDir(PathBuf);

    impl MusicDir {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!("ttunes-{name}-{}", std::process::id()));
            for (path, fixture) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/testdata")
                    .join(fixture);
                fs::copy(fixture, path).unwrap();
            }
            fs::create_dir_all(root.join("empty")).unwrap();
            Self(root)
        }
    }

    impl Drop for MusicDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Folder(folder) => format!("{}/", folder.name),
                Entry::Track(track) => track.title.clone(),
            })
            .collect()
    }

    #[test]
    fn keeps_folders_with_songs_first_then_songs() {
        let dir = MusicDir::new(
            "tree",
            &[
                ("two.mp3", "two.mp3"),
                ("jazz/one.mp3", "one.mp3"),
                ("two copy.mp3", "two.mp3"),
                ("not music.txt", "one.mp3"),
            ],
        );
        let entries = scan(std::slice::from_ref(&dir.0));
        // "empty" has no songs; the copy of two.mp3 is the same song, listed once.
        assert_eq!(names(&entries), ["jazz/", "Song two"]);
        let Entry::Folder(jazz) = &entries[0] else {
            unreachable!()
        };
        assert_eq!(names(&jazz.entries), ["Song one"]);
        assert_eq!(jazz.tracks()[0].uri, "spotify:local:Tester::Song+one:1");
    }

    #[test]
    fn covers_come_from_the_file_else_the_folder() {
        let dir = MusicDir::new(
            "covers",
            &[
                ("album/art.mp3", "art.mp3"),
                ("album/Cover.PNG", "cover.png"),
                ("album/disc 2/one.mp3", "one.mp3"),
                ("bare/two.mp3", "two.mp3"),
            ],
        );
        let covers = dir.0.join("saved covers");
        let entries = scan_saving_covers(std::slice::from_ref(&dir.0), &covers);
        let [Entry::Folder(album), Entry::Folder(bare)] = &entries[..] else {
            panic!("{entries:?}")
        };
        let folder_image = file_url(&dir.0.join("album/Cover.PNG"));
        assert_eq!(album.cover_url, folder_image);
        let tracks = album.tracks();
        let embedded = tracks.iter().find(|t| t.title == "Song art").unwrap();
        let without = tracks.iter().find(|t| t.title == "Song one").unwrap();
        assert_eq!(without.cover_url, folder_image);
        // A subfolder without a picture has its parent's.
        let Entry::Folder(disc) = &album.entries[0] else {
            panic!("{album:?}")
        };
        assert_eq!(disc.cover_url, folder_image);
        // The picture in the file, saved where covers go, the same as the original.
        let saved = crate::utils::local_path(embedded.cover_url.as_deref().unwrap()).unwrap();
        assert!(saved.starts_with(&covers), "{saved:?}");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/cover.png");
        assert_eq!(fs::read(saved).unwrap(), fs::read(fixture).unwrap());
        assert_eq!(bare.cover_url, None);
        assert_eq!(bare.tracks()[0].cover_url, None);
    }

    #[test]
    fn several_music_folders_are_a_folder_each() {
        let first = MusicDir::new("first", &[("one.mp3", "one.mp3")]);
        let second = MusicDir::new("second", &[("two.mp3", "two.mp3")]);
        let entries = scan(&[
            first.0.clone(),
            second.0.clone(),
            PathBuf::from("/nonexistent"),
        ]);
        let folders: Vec<_> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Folder(folder) => folder.tracks().len(),
                Entry::Track(_) => 0,
            })
            .collect();
        assert_eq!(folders, [1, 1]);
    }

    #[test]
    fn local_uris_parse_back_the_same() {
        let uri = SpotifyUri::Local {
            artist: "AC%2FDC".into(),
            album_title: "Back+in+Black".into(),
            track_title: "Hells+Bells%3A+Live".into(),
            duration: Duration::from_secs(312),
        };
        assert_eq!(SpotifyUri::from_uri(&uri.to_uri().unwrap()).unwrap(), uri);
    }
}
