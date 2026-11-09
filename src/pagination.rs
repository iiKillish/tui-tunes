use crate::library::Library;
use crate::traits::ListItem;
use log::{debug, warn};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// How many times [ApiResult::fetch_all] asks for a page before giving up on it.
const FETCH_ATTEMPTS: u32 = 3;

/// How long [ApiResult::fetch_all] waits before asking for a failed page again, doubled after
/// each failure.
const FETCH_RETRY_DELAY: Duration = Duration::from_secs(1);

/// How long [Pagination::call] waits before asking again for a page that didn't load, so a list
/// scrolled to its end doesn't ask Spotify on every redraw while it's failing.
const FAILED_PAGE_COOLDOWN: Duration = Duration::from_secs(5);

pub struct ApiPage<I> {
    pub offset: u32,
    pub total: u32,
    pub items: Vec<I>,
}
pub type FetchPageFn<I> = dyn Fn(u32) -> Option<ApiPage<I>> + Send + Sync;
pub struct ApiResult<I> {
    offset: Arc<RwLock<u32>>,
    limit: u32,
    pub total: u32,
    pub items: Arc<RwLock<Vec<I>>>,
    fetch_page: Arc<FetchPageFn<I>>,
    /// Whether the first page loaded; if not, `total` is 0 for want of knowing better.
    first_loaded: bool,
}

impl<I: ListItem + Clone> ApiResult<I> {
    pub fn new(limit: u32, fetch_page: Arc<FetchPageFn<I>>) -> Self {
        let items = Arc::new(RwLock::new(Vec::new()));
        if let Some(first_page) = fetch_page(0) {
            debug!(
                "fetched first page, items: {}, total: {}",
                first_page.items.len(),
                first_page.total
            );
            items.write().unwrap().extend(first_page.items);
            Self {
                offset: Arc::new(RwLock::new(first_page.offset)),
                limit,
                total: first_page.total,
                items,
                fetch_page: fetch_page.clone(),
                first_loaded: true,
            }
        } else {
            Self {
                offset: Arc::new(RwLock::new(0)),
                limit,
                total: 0,
                items,
                fetch_page: fetch_page.clone(),
                first_loaded: false,
            }
        }
    }

    fn offset(&self) -> u32 {
        *self.offset.read().unwrap()
    }

    pub fn at_end(&self) -> bool {
        (self.offset() + self.limit) >= self.total
    }

    /// Load all the pages left, asking again a few times (waiting longer each time) for any that
    /// fail. Returns whether everything loaded: `false` if a page (perhaps the first) still
    /// failed, and then [Self::items] holds only the pages before it.
    pub fn fetch_all(&mut self) -> bool {
        self.fetch_all_retrying(FETCH_RETRY_DELAY)
    }

    fn fetch_all_retrying(&mut self, delay: Duration) -> bool {
        let fetch_page = self.fetch_page.clone();
        let fetch = |offset: u32| {
            let mut wait = delay;
            for attempt in 1..=FETCH_ATTEMPTS {
                if let Some(page) = fetch_page(offset) {
                    return Some(page);
                }
                if attempt < FETCH_ATTEMPTS {
                    warn!("page at offset {offset} failed to load, asking again in {wait:?}");
                    std::thread::sleep(wait);
                    wait *= 2;
                }
            }
            warn!("page at offset {offset} failed to load, giving up");
            None
        };

        if !self.first_loaded {
            let Some(first_page) = fetch(0) else {
                return false;
            };
            self.total = first_page.total;
            *self.offset.write().unwrap() = first_page.offset;
            self.items.write().unwrap().extend(first_page.items);
            self.first_loaded = true;
        }
        while !self.at_end() {
            let offset = self.offset() + self.limit;
            let Some(page) = fetch(offset) else {
                return false;
            };
            // Always move on, whatever offset the page claims, so this ends.
            *self.offset.write().unwrap() = page.offset.max(offset);
            self.items.write().unwrap().extend(page.items);
        }
        true
    }

    pub fn apply_pagination(self, pagination: &Pagination<I>) {
        let total = self.total as usize;
        let fetched_items = self.items.read().unwrap().len();
        pagination.set(
            fetched_items,
            total,
            Box::new(move |_| {
                self.next();
            }),
        )
    }

    /// Load the next page, once. `None` if there's none left or it failed to load (and then the
    /// same page is asked for next time).
    pub fn next(&self) -> Option<Vec<I>> {
        let offset = self.offset() + self.limit;
        debug!("fetching next page at offset {offset}");
        if !self.at_end() {
            if let Some(next_page) = (self.fetch_page)(offset) {
                *self.offset.write().unwrap() = next_page.offset.max(offset);
                self.items.write().unwrap().extend(next_page.items.clone());
                Some(next_page.items)
            } else {
                None
            }
        } else {
            debug!("paginator is at end");
            None
        }
    }
}

pub type Paginator<I> = Box<dyn Fn(Arc<RwLock<Vec<I>>>) + Send + Sync>;

/// Manages the loading of ListItems, to increase performance and decrease
/// memory usage.
///
/// `loaded_content`: The amount of currently loaded items
/// `max_content`: The maximum amount of items
/// `callback`: Loads the next page into the list it's given, on a thread of its own
/// `busy`: Whether `callback` is running, so the next page isn't requested twice
/// `failed_at`: When `callback` last loaded nothing, so it isn't called again straight away
#[derive(Clone)]
pub struct Pagination<I: ListItem> {
    loaded_content: Arc<RwLock<usize>>,
    max_content: Arc<RwLock<Option<usize>>>,
    callback: Arc<RwLock<Option<Paginator<I>>>>,
    busy: Arc<RwLock<bool>>,
    failed_at: Arc<RwLock<Option<Instant>>>,
}

/// Clears [Pagination]'s `busy` when dropped, so it's cleared even if the callback panics.
struct NotBusy(Arc<RwLock<bool>>);

impl Drop for NotBusy {
    fn drop(&mut self) {
        // Not unwrap: this may run while unwinding from a panic that poisoned it.
        match self.0.write() {
            Ok(mut busy) => *busy = false,
            Err(poisoned) => *poisoned.into_inner() = false,
        }
    }
}

impl<I: ListItem> Default for Pagination<I> {
    fn default() -> Self {
        Self {
            loaded_content: Arc::new(RwLock::new(0)),
            max_content: Arc::new(RwLock::new(None)),
            callback: Arc::new(RwLock::new(None)),
            busy: Arc::new(RwLock::new(false)),
            failed_at: Arc::new(RwLock::new(None)),
        }
    }
}

impl<I: ListItem + Clone> Pagination<I> {
    pub fn clear(&mut self) {
        *self.max_content.write().unwrap() = None;
        *self.callback.write().unwrap() = None;
    }
    pub fn set(&self, loaded_content: usize, max_content: usize, callback: Paginator<I>) {
        *self.loaded_content.write().unwrap() = loaded_content;
        *self.max_content.write().unwrap() = Some(max_content);
        *self.callback.write().unwrap() = Some(callback);
        *self.failed_at.write().unwrap() = None;
    }

    pub fn loaded_content(&self) -> usize {
        *self.loaded_content.read().unwrap()
    }

    pub fn max_content(&self) -> Option<usize> {
        *self.max_content.read().unwrap()
    }

    /// Whether the last page asked for didn't load, recently enough not to ask again yet.
    fn cooling_down(&self) -> bool {
        self.failed_at
            .read()
            .unwrap()
            .is_some_and(|at| at.elapsed() < FAILED_PAGE_COOLDOWN)
    }

    pub fn call(&self, content: &Arc<RwLock<Vec<I>>>, library: Arc<Library>) {
        self.call_then(content, move || library.trigger_redraw());
    }

    /// Load the next page into `content` on a thread of its own, then call `done`. Returns the
    /// thread, if one was started.
    fn call_then(
        &self,
        content: &Arc<RwLock<Vec<I>>>,
        done: impl FnOnce() + Send + 'static,
    ) -> Option<std::thread::JoinHandle<()>> {
        if self.cooling_down() {
            return None;
        }
        {
            let mut busy = self.busy.write().unwrap();
            if *busy {
                return None;
            }
            *busy = true;
        }
        let pagination = self.clone();
        let content = content.clone();
        Some(std::thread::spawn(move || {
            let _not_busy = NotBusy(pagination.busy.clone());
            let called = {
                let cb = pagination.callback.read().unwrap();
                if let Some(ref cb) = *cb {
                    debug!("calling paginator!");
                    let before = content.read().unwrap().len();
                    cb(content.clone());
                    let after = content.read().unwrap().len();
                    *pagination.loaded_content.write().unwrap() = after;
                    *pagination.failed_at.write().unwrap() = (after == before).then(Instant::now);
                    true
                } else {
                    false
                }
            };
            if called {
                done();
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::playlist::Playlist;
    use std::sync::Mutex;

    fn item(n: u32) -> Playlist {
        Playlist {
            id: n.to_string(),
            name: String::new(),
            owner_id: String::new(),
            owner_name: None,
            snapshot_id: String::new(),
            num_tracks: 0,
            tracks: None,
            collaborative: false,
            cover_url: None,
        }
    }

    /// Pages of `limit` out of `total` items, failing whenever `fails` says so for the offset
    /// and the number of times it was asked for before. Counts the requests.
    fn pages(
        limit: u32,
        total: u32,
        fails: impl Fn(u32, usize) -> bool + Send + Sync + 'static,
    ) -> (Arc<FetchPageFn<Playlist>>, Arc<Mutex<Vec<u32>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let fetch = move |offset: u32| {
            let mut asked = log.lock().unwrap();
            let before = asked.iter().filter(|&&o| o == offset).count();
            asked.push(offset);
            if fails(offset, before) {
                return None;
            }
            Some(ApiPage {
                offset,
                total,
                items: (offset..(offset + limit).min(total)).map(item).collect(),
            })
        };
        (Arc::new(fetch), asked)
    }

    fn ids(result: &ApiResult<Playlist>) -> Vec<String> {
        result
            .items
            .read()
            .unwrap()
            .iter()
            .map(|p| p.id.clone())
            .collect()
    }

    #[test]
    fn fetch_all_loads_every_page() {
        let (fetch, asked) = pages(2, 5, |_, _| false);
        let mut result = ApiResult::new(2, fetch);
        assert!(result.fetch_all_retrying(Duration::ZERO));
        assert_eq!(ids(&result), ["0", "1", "2", "3", "4"]);
        assert_eq!(*asked.lock().unwrap(), [0, 2, 4]);
    }

    #[test]
    fn fetch_all_gives_up_on_a_page_that_keeps_failing() {
        let (fetch, asked) = pages(2, 6, |offset, _| offset == 2);
        let mut result = ApiResult::new(2, fetch);
        assert!(!result.fetch_all_retrying(Duration::ZERO));
        assert_eq!(ids(&result), ["0", "1"]);
        assert_eq!(*asked.lock().unwrap(), [0, 2, 2, 2]);
    }

    #[test]
    fn fetch_all_asks_again_for_a_page_that_failed_once() {
        let (fetch, _) = pages(2, 4, |offset, before| offset == 2 && before == 0);
        let mut result = ApiResult::new(2, fetch);
        assert!(result.fetch_all_retrying(Duration::ZERO));
        assert_eq!(ids(&result), ["0", "1", "2", "3"]);
    }

    #[test]
    fn fetch_all_asks_again_for_a_failed_first_page() {
        let (fetch, _) = pages(2, 3, |offset, before| offset == 0 && before == 0);
        let mut result = ApiResult::new(2, fetch);
        assert_eq!(result.total, 0);
        assert!(result.fetch_all_retrying(Duration::ZERO));
        assert_eq!(result.total, 3);
        assert_eq!(ids(&result), ["0", "1", "2"]);
    }

    #[test]
    fn fetch_all_reports_a_first_page_that_never_loads() {
        let (fetch, _) = pages(2, 3, |_, _| true);
        let mut result = ApiResult::new(2, fetch);
        assert!(!result.fetch_all_retrying(Duration::ZERO));
        assert!(ids(&result).is_empty());
    }

    #[test]
    fn fetch_all_ends_when_pages_claim_not_to_move_on() {
        let fetch = |_offset: u32| {
            Some(ApiPage {
                offset: 0,
                total: 6,
                items: vec![item(0)],
            })
        };
        let mut result = ApiResult::new(2, Arc::new(fetch));
        assert!(result.fetch_all_retrying(Duration::ZERO));
        assert_eq!(ids(&result).len(), 3);
    }

    #[test]
    fn pagination_isnt_left_busy_by_a_panicking_callback() {
        let pagination: Pagination<Playlist> = Pagination::default();
        pagination.set(0, 10, Box::new(|_| panic!("page failed")));
        let content = Arc::new(RwLock::new(Vec::new()));
        let thread = pagination.call_then(&content, || {}).unwrap();
        assert!(thread.join().is_err());
        assert!(!*pagination.busy.read().unwrap());
    }

    #[test]
    fn pagination_isnt_left_busy_without_a_callback() {
        let pagination: Pagination<Playlist> = Pagination::default();
        let content = Arc::new(RwLock::new(Vec::new()));
        pagination
            .call_then(&content, || {})
            .unwrap()
            .join()
            .unwrap();
        assert!(!*pagination.busy.read().unwrap());
    }

    #[test]
    fn pagination_waits_before_asking_again_for_a_failed_page() {
        let pagination: Pagination<Playlist> = Pagination::default();
        let calls = Arc::new(Mutex::new(0));
        let counted = calls.clone();
        pagination.set(0, 10, Box::new(move |_| *counted.lock().unwrap() += 1));
        let content = Arc::new(RwLock::new(Vec::new()));
        pagination
            .call_then(&content, || {})
            .unwrap()
            .join()
            .unwrap();
        assert!(pagination.call_then(&content, || {}).is_none());
        assert_eq!(*calls.lock().unwrap(), 1);
    }
}
