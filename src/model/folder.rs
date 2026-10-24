use std::sync::Arc;

use crate::library::Library;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::traits::{ListItem, OpenTarget};

/// A folder of music files on this computer (see [crate::local_files]): its subfolders, then its
/// songs. Only folders with songs somewhere in them are kept.
#[derive(Clone, Debug, Default)]
pub struct Folder {
    pub name: String,
    /// Its picture (`cover.jpg` and the like, else the first song's cover), a `file://` URL.
    pub cover_url: Option<String>,
    pub entries: Vec<Entry>,
}

/// What's in a [Folder].
#[derive(Clone, Debug)]
// Most entries are songs: boxing them would cost more than the few folders' padding.
#[allow(clippy::large_enum_variant)]
pub enum Entry {
    Folder(Arc<Folder>),
    Track(Track),
}

impl Folder {
    /// Its songs and those of its subfolders, in the order they're shown.
    pub fn tracks(&self) -> Vec<Track> {
        let mut tracks = Vec::new();
        self.collect_tracks(&mut tracks);
        tracks
    }

    fn collect_tracks(&self, tracks: &mut Vec<Track>) {
        for entry in &self.entries {
            match entry {
                Entry::Folder(folder) => folder.collect_tracks(tracks),
                Entry::Track(track) => tracks.push(track.clone()),
            }
        }
    }

    fn playables(&self) -> Vec<Playable> {
        self.tracks().into_iter().map(Playable::Track).collect()
    }
}

impl ListItem for Entry {
    fn target(&self) -> Option<OpenTarget> {
        match self {
            Self::Folder(folder) => Some(OpenTarget::Folder(folder.clone())),
            Self::Track(_) => None,
        }
    }

    fn opens_on_play(&self) -> bool {
        matches!(self, Self::Folder(_))
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        match self {
            Self::Folder(_) => false,
            Self::Track(track) => track.is_playing(queue),
        }
    }

    fn display_left(&self, library: &Library) -> String {
        match self {
            Self::Folder(folder) if library.cfg.values().use_nerdfont.unwrap_or_default() => {
                format!("\u{f024b} {}", folder.name)
            }
            Self::Folder(folder) => format!("{}/", folder.name),
            Self::Track(track) => track.display_left(library),
        }
    }

    fn display_center(&self, library: &Library) -> String {
        match self {
            Self::Folder(_) => String::new(),
            Self::Track(track) => track.display_center(library),
        }
    }

    fn display_right(&self, library: &Library) -> String {
        match self {
            Self::Folder(folder) => {
                let count = folder.tracks().len();
                format!("{count} {}", if count == 1 { "song" } else { "songs" })
            }
            Self::Track(track) => track.display_right(library),
        }
    }

    fn play(&mut self, queue: &Queue) {
        match self {
            Self::Folder(folder) => {
                let index = queue.append_next_from(&folder.playables(), Some(&folder.name));
                queue.play(index, true, true);
            }
            Self::Track(track) => track.play(queue),
        }
    }

    fn play_next(&mut self, queue: &Queue) {
        match self {
            Self::Folder(folder) => {
                for track in folder.playables().into_iter().rev() {
                    queue.insert_after_current(track);
                }
            }
            Self::Track(track) => track.play_next(queue),
        }
    }

    fn queue(&mut self, queue: &Queue) {
        match self {
            Self::Folder(folder) => {
                for track in folder.playables() {
                    queue.append(track);
                }
            }
            Self::Track(track) => track.queue(queue),
        }
    }

    fn save(&mut self, _library: &Library) {}

    fn share_url(&self) -> Option<String> {
        None
    }

    fn thumbnail_url(&self) -> Option<String> {
        match self {
            Self::Folder(folder) => folder.cover_url.clone(),
            Self::Track(track) => track.cover_url.clone(),
        }
    }

    fn track(&self) -> Option<Track> {
        self.track_ref().cloned()
    }

    fn track_ref(&self) -> Option<&Track> {
        match self {
            Self::Folder(_) => None,
            Self::Track(track) => Some(track),
        }
    }

    fn is_playable(&self) -> bool {
        matches!(self, Self::Track(_))
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}
