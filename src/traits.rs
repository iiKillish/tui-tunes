use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::category::Category;
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
    fn toggle_saved(&mut self, library: &Library);
    fn save(&mut self, library: &Library);
    fn unsave(&mut self, library: &Library);
    /// What opening the item shows: a page for albums, playlists, artists, shows and categories.
    /// `None` for tracks and episodes (they get a context menu instead).
    fn target(&self) -> Option<OpenTarget> {
        None
    }
    fn share_url(&self) -> Option<String>;

    /// Get the album that contains this [ListItem].
    fn album(&self, _queue: &Queue) -> Option<Album> {
        None
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        None
    }

    fn track(&self) -> Option<Track> {
        None
    }

    /// Cover to show next to the item in lists, if it has one.
    fn thumbnail_url(&self) -> Option<String> {
        None
    }

    #[allow(unused_variables)]
    #[inline]
    fn is_saved(&self, library: &Library) -> Option<bool> {
        None
    }

    #[inline]
    fn is_playable(&self) -> bool {
        false
    }

    fn as_listitem(&self) -> Box<dyn ListItem>;
}
