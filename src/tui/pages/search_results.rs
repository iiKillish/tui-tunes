//! Search results: an "All" tab like Spotify's (the top result, then rows of songs, artists,
//! albums and playlists), then a list per kind of item, loading more as they're scrolled.
//! Each kind is searched for on its own thread, so tabs fill in as their results arrive.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use rspotify::model::SearchType;
use rspotify::model::search::SearchResult;

use crate::application::ASYNC_RUNTIME;
use crate::events::EventManager;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::episode::Episode;
use crate::model::playlist::Playlist;
use crate::model::show::Show;
use crate::model::track::Track;
use crate::pagination::Pagination;
use crate::spotify_api::WebApi;
use crate::traits::ListItem;
use crate::tui::page::Ctx;
use crate::tui::pages::loading::LoadingList;
use crate::tui::widgets::cards::{Card, CardGrid, CardLayout, Section};
use crate::tui::widgets::list::ItemList;
use crate::tui::widgets::tabs::Tabbed;

/// Results asked for at a time.
/// Results per request: Spotify allows at most 10 per search (since its 2026 API changes).
const PAGE_SIZE: u32 = 10;
/// Cards in each row of the All tab.
const PER_ROW: usize = 10;

/// Pulls one kind of item and the total number found out of a search result.
type Extract<I> = fn(SearchResult) -> Option<(Vec<I>, u32)>;

/// One kind of result: the items found so far, and whether the first page has arrived.
struct Results<I> {
    items: Arc<RwLock<Vec<I>>>,
    done: Arc<AtomicBool>,
}

impl<I> Clone for Results<I> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            done: self.done.clone(),
        }
    }
}

impl<I: ListItem + Clone> Results<I> {
    fn new() -> Self {
        Self {
            items: Arc::default(),
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    /// A list of these results, saying "Searching…" until they arrive.
    fn tab(&self, title: &str, list: ItemList<I>) -> LoadingList<I> {
        let done = self.done.clone();
        LoadingList::new(title, list, move || !done.load(Ordering::Relaxed))
            .loading_message("Searching…")
            .empty_message("No results")
    }

    /// Search for `query` on a thread, filling the results and setting `pagination` up to load
    /// the rest as the list is scrolled.
    fn search(
        &self,
        api: WebApi,
        query: String,
        kind: SearchType,
        extract: Extract<I>,
        pagination: Pagination<I>,
        events: EventManager,
    ) {
        let results = self.clone();
        thread::spawn(move || {
            let fetch = move |offset: usize| {
                api.search(kind, &query, PAGE_SIZE, offset as u32)
                    .ok()
                    .and_then(extract)
            };
            if let Some((items, total)) = fetch(0) {
                let loaded = items.len();
                *results.items.write().unwrap() = items;
                if total as usize > loaded {
                    pagination.set(
                        loaded,
                        total as usize,
                        Box::new(move |content| {
                            let offset = content.read().unwrap().len();
                            if let Some((items, _)) = fetch(offset) {
                                content.write().unwrap().extend(items);
                            }
                        }),
                    );
                }
            }
            results.done.store(true, Ordering::Relaxed);
            events.trigger();
        });
    }
}

/// What the All tab's top result is, by index into the results of its kind.
#[derive(Debug, PartialEq, Eq)]
enum TopResult {
    Artist(usize),
    Album(usize),
    Playlist(usize),
    Track(usize),
}

/// The top result: an exact name match (an artist, else an album, else a playlist), else the
/// first song, else the first artist.
fn top_result<'a>(
    query: &str,
    artists: impl IntoIterator<Item = &'a str>,
    albums: impl IntoIterator<Item = &'a str>,
    playlists: impl IntoIterator<Item = &'a str>,
    tracks: usize,
    any_artists: bool,
) -> Option<TopResult> {
    let query = query.trim().to_lowercase();
    let exact = |names: &mut dyn Iterator<Item = &'a str>| {
        names
            .enumerate()
            .find(|(_, name)| name.trim().to_lowercase() == query)
            .map(|(i, _)| i)
    };
    exact(&mut artists.into_iter())
        .map(TopResult::Artist)
        .or_else(|| exact(&mut albums.into_iter()).map(TopResult::Album))
        .or_else(|| exact(&mut playlists.into_iter()).map(TopResult::Playlist))
        .or_else(|| (tracks > 0).then_some(TopResult::Track(0)))
        .or_else(|| any_artists.then_some(TopResult::Artist(0)))
}

/// The results page for `query`.
pub fn new(query: &str, ctx: &Ctx) -> Tabbed {
    let tracks = Results::<Track>::new();
    let artists = Results::<Artist>::new();
    let albums = Results::<Album>::new();
    let playlists = Results::<Playlist>::new();
    let shows = Results::<Show>::new();
    let episodes = Results::<Episode>::new();

    let source = format!("Search “{query}”");
    let track_list = ItemList::new(tracks.items.clone()).with_source(&source);
    let artist_list = ItemList::new(artists.items.clone());
    let album_list = ItemList::new(albums.items.clone());
    let playlist_list = ItemList::new(playlists.items.clone());
    let show_list = ItemList::new(shows.items.clone());
    let episode_list = ItemList::new(episodes.items.clone());

    // Searches run on a thread of their own: the token is refreshed first, so the parallel
    // requests don't all try to refresh it at once.
    {
        let api = ctx.spotify.api.clone();
        let events = ctx.events.clone();
        let query = query.to_string();
        let (tracks, artists, albums, playlists, shows, episodes) = (
            tracks.clone(),
            artists.clone(),
            albums.clone(),
            playlists.clone(),
            shows.clone(),
            episodes.clone(),
        );
        let paginations = (
            track_list.pagination().clone(),
            artist_list.pagination().clone(),
            album_list.pagination().clone(),
            playlist_list.pagination().clone(),
            show_list.pagination().clone(),
            episode_list.pagination().clone(),
        );
        thread::spawn(move || {
            if let Some(refresh) = api.update_token()
                && let Some(runtime) = ASYNC_RUNTIME.get()
            {
                let _ = runtime.block_on(refresh);
            }
            let q = || query.clone();
            let e = || events.clone();
            tracks.search(
                api.clone(),
                q(),
                SearchType::Track,
                extract_tracks,
                paginations.0,
                e(),
            );
            artists.search(
                api.clone(),
                q(),
                SearchType::Artist,
                extract_artists,
                paginations.1,
                e(),
            );
            albums.search(
                api.clone(),
                q(),
                SearchType::Album,
                extract_albums,
                paginations.2,
                e(),
            );
            playlists.search(
                api.clone(),
                q(),
                SearchType::Playlist,
                extract_playlists,
                paginations.3,
                e(),
            );
            shows.search(
                api.clone(),
                q(),
                SearchType::Show,
                extract_shows,
                paginations.4,
                e(),
            );
            episodes.search(
                api,
                q(),
                SearchType::Episode,
                extract_episodes,
                paginations.5,
                e(),
            );
        });
    }

    Tabbed::new(format!("“{query}”"))
        .tab(
            "All",
            overview(query, &tracks, &artists, &albums, &playlists),
        )
        .tab("Songs", tracks.tab("Songs", track_list))
        .tab("Artists", artists.tab("Artists", artist_list))
        .tab("Albums", albums.tab("Albums", album_list))
        .tab("Playlists", playlists.tab("Playlists", playlist_list))
        .tab("Podcasts", shows.tab("Podcasts", show_list))
        .tab("Episodes", episodes.tab("Episodes", episode_list))
}

/// The All tab: the top result, then rows of songs, artists, albums and playlists, filled in as
/// the results arrive.
fn overview(
    query: &str,
    tracks: &Results<Track>,
    artists: &Results<Artist>,
    albums: &Results<Album>,
    playlists: &Results<Playlist>,
) -> CardGrid {
    let (tracks, artists, albums, playlists) = (
        tracks.clone(),
        artists.clone(),
        albums.clone(),
        playlists.clone(),
    );
    let query = query.to_string();
    let source = Box::new(move || {
        let done = tracks.is_done() && artists.is_done() && albums.is_done() && playlists.is_done();
        let track_items = tracks.items.read().unwrap();
        let artist_items = artists.items.read().unwrap();
        let album_items = albums.items.read().unwrap();
        let playlist_items = playlists.items.read().unwrap();

        let top = done
            .then(|| {
                top_result(
                    &query,
                    artist_items.iter().map(|a| a.name.as_str()),
                    album_items.iter().map(|a| a.title.as_str()),
                    playlist_items.iter().map(|p| p.name.as_str()),
                    track_items.len(),
                    !artist_items.is_empty(),
                )
                .and_then(|top| match top {
                    TopResult::Artist(i) => artist_items.get(i).cloned().map(Card::artist),
                    TopResult::Album(i) => album_items.get(i).cloned().map(Card::album),
                    TopResult::Playlist(i) => playlist_items.get(i).cloned().map(Card::playlist),
                    TopResult::Track(i) => track_items.get(i).cloned().map(Card::track),
                })
            })
            .flatten();

        let row = |cards: Vec<Card>| cards.into_iter().take(PER_ROW).collect::<Vec<_>>();
        let sections: Vec<Section> = [
            Section::new("Top result", top.into_iter().collect()),
            Section::new(
                "Songs",
                row(track_items.iter().cloned().map(Card::track).collect()),
            ),
            Section::new(
                "Artists",
                row(artist_items.iter().cloned().map(Card::artist).collect()),
            ),
            Section::new(
                "Albums",
                row(album_items.iter().cloned().map(Card::album).collect()),
            ),
            Section::new(
                "Playlists",
                row(playlist_items.iter().cloned().map(Card::playlist).collect()),
            ),
        ]
        .into_iter()
        .filter(|section| !section.cards.is_empty())
        .collect();
        // Still searching: the rows show up as their results arrive, but the top result waits
        // until every kind is in, so it doesn't change under the cursor.
        if !done && sections.is_empty() {
            return None;
        }
        Some(sections)
    });
    CardGrid::new("All", CardLayout::Rows, source)
        .loading_message("Searching…")
        .empty_message("No results")
}

fn extract_tracks(result: SearchResult) -> Option<(Vec<Track>, u32)> {
    match result {
        SearchResult::Tracks(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

fn extract_artists(result: SearchResult) -> Option<(Vec<Artist>, u32)> {
    match result {
        SearchResult::Artists(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

fn extract_albums(result: SearchResult) -> Option<(Vec<Album>, u32)> {
    match result {
        SearchResult::Albums(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

fn extract_playlists(result: SearchResult) -> Option<(Vec<Playlist>, u32)> {
    match result {
        SearchResult::Playlists(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

fn extract_shows(result: SearchResult) -> Option<(Vec<Show>, u32)> {
    match result {
        SearchResult::Shows(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

fn extract_episodes(result: SearchResult) -> Option<(Vec<Episode>, u32)> {
    match result {
        SearchResult::Episodes(page) => {
            Some((page.items.iter().map(Into::into).collect(), page.total))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_result_prefers_exact_artist_then_album_then_playlist() {
        let artists = ["Daft Punk Tribute", "Daft Punk"];
        let albums = ["Discovery", "daft punk"];
        let playlists = ["Daft Punk"];
        assert_eq!(
            top_result("daft punk", artists, albums, playlists, 5, true),
            Some(TopResult::Artist(1))
        );
        assert_eq!(
            top_result(" Daft Punk ", ["Other"], albums, playlists, 5, true),
            Some(TopResult::Album(1))
        );
        assert_eq!(
            top_result("daft punk", ["Other"], ["Discovery"], playlists, 5, true),
            Some(TopResult::Playlist(0))
        );
    }

    #[test]
    fn test_top_result_falls_back_to_first_song_then_artist() {
        assert_eq!(
            top_result("x", ["a"], ["b"], ["c"], 3, true),
            Some(TopResult::Track(0))
        );
        assert_eq!(
            top_result("x", ["a"], [], [], 0, true),
            Some(TopResult::Artist(0))
        );
        assert_eq!(top_result("x", [], [], [], 0, false), None);
    }
}
