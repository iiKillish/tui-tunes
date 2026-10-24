use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use std::sync::Arc;

use crate::model::category::Category;
use crate::model::folder::Folder;
use crate::model::playlist::Playlist;
use crate::model::show::Show;
use crate::model::track::Track;
use crate::queue::Queue;

/// A page an item opens; see [ListItem::target].
#[derive(Clone)]
pub enum OpenTarget {
    Album(Album),
    Playlist(Playlist),
    Artist(Artist),
    Show(Show),
    Category(Category),
    Folder(Arc<Folder>),
}

pub trait ListItem: Sync + Send + 'static {
    fn is_playing(&self, queue: &Queue) -> bool;
    fn display_left(&self, library: &Library) -> String;
    fn display_center(&self, _library: &Library) -> String {
        "".to_string()
    }
    fn display_right(&self, library: &Library) -> String;
    fn play(&mut self, queue: &Queue);
    fn play_next(&mut self, queue: &Queue);
    fn queue(&mut self, queue: &Queue);
    fn save(&mut self, library: &Library);
    /// Whether playing the item (Enter) opens it instead, as for folders: you go into a folder.
    fn opens_on_play(&self) -> bool {
        false
    }
    /// What opening the item shows: a page for albums, playlists, artists, shows, categories and
    /// folders.
    /// `None` for tracks and episodes (they get a context menu instead).
    fn target(&self) -> Option<OpenTarget> {
        None
    }
    fn share_url(&self) -> Option<String>;

    /// The album that contains this [ListItem], from what the item knows of it: without its
    /// tracks or release year, which its page loads.
    fn album(&self) -> Option<Album> {
        None
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        None
    }

    fn track(&self) -> Option<Track> {
        None
    }

    /// [Self::track] without the copy, e.g. for drawing a row.
    fn track_ref(&self) -> Option<&Track> {
        None
    }

    /// Cover to show next to the item in lists, if it has one.
    fn thumbnail_url(&self) -> Option<String> {
        None
    }

    #[inline]
    fn is_playable(&self) -> bool {
        false
    }

    fn as_listitem(&self) -> Box<dyn ListItem>;
}
