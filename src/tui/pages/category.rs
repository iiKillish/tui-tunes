use crate::model::category::Category;
use crate::model::playlist::Playlist;
use crate::tui::page::Ctx;
use crate::tui::pages::ListPage;
use crate::tui::widgets::list::ItemList;

/// A browse category: its playlists.
pub type CategoryPage = ListPage<Playlist>;

impl ListPage<Playlist> {
    pub fn new_category(category: Category, ctx: &Ctx) -> Self {
        let playlists = ctx.spotify.api.category_playlists(&category.id);
        let list = ItemList::new(playlists.items.clone());
        playlists.apply_pagination(list.pagination());
        Self::new(category.name.clone(), list)
    }
}
