use crate::model::category::Category;
use crate::model::playlist::Playlist;
use crate::tui::page::Ctx;
use crate::tui::pages::ListPage;
use crate::tui::widgets::list::ItemList;

/// A browse category: its playlists, loaded in the background.
pub type CategoryPage = ListPage<Playlist>;

impl ListPage<Playlist> {
    pub fn new_category(category: Category, ctx: &Ctx) -> Self {
        let spotify = ctx.spotify.clone();
        let id = category.id.clone();
        Self::new(category.name.clone(), ItemList::new(Default::default()))
            .load_in_background(move || spotify.api.category_playlists(&id))
    }
}
