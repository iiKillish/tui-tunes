use crate::model::episode::Episode;
use crate::model::show::Show;
use crate::tui::header::PageHeader;
use crate::tui::page::Ctx;
use crate::tui::pages::ListPage;
use crate::tui::widgets::list::ItemList;

/// A podcast: its episodes, loaded in the background.
pub type ShowPage = ListPage<Episode>;

impl ListPage<Episode> {
    pub fn new_show(show: Show, ctx: &Ctx) -> Self {
        let list = ItemList::new(Default::default()).with_source(&show.name);
        let spotify = ctx.spotify.clone();
        let id = show.id.clone();
        Self::new(show.name.clone(), list)
            .with_header(PageHeader {
                kind: "Podcast".into(),
                title: show.name.clone(),
                details: Vec::new(),
                cover_url: show.cover_url.clone(),
                playable: false,
                liked: None,
            })
            .load_in_background(move || spotify.api.show_episodes(&id))
    }
}
