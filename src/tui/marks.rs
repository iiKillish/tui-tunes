//! Acting on several songs at once, in lists of songs and Up next: songs marked with `v`,
//! Shift+v, Ctrl+click or Shift+click, which the queue, save, add and delete commands and the
//! menu then act on instead of the selected song. Each mark remembers which song it is, so marks
//! follow their songs when the list changes underneath ([Marks::sync]) and never land on others.

use crate::traits::ListItem;

/// Marked positions in a list, with what's there.
#[derive(Default)]
pub struct Marks {
    /// Positions, sorted, each with its song's identity (see [identity]).
    marks: Vec<(usize, String)>,
    /// The position marked last, which a range mark (Shift+v, Shift+click) starts from.
    anchor: Option<usize>,
}

/// What tells a song in a list apart from the others: its URI. `None` for what can't be marked
/// (albums, playlists, artists, folders: things with a page of their own).
pub fn identity<T: ListItem + ?Sized>(item: &T) -> Option<String> {
    match item.track_ref() {
        Some(track) if !track.uri.is_empty() => Some(track.uri.clone()),
        Some(track) => Some(format!("{}\u{0}{}", track.title, track.artists.join(","))),
        // Episodes.
        None if item.target().is_none() => item.share_url(),
        None => None,
    }
}

/// "3 songs", for messages and the menu of marked songs.
pub fn songs(count: usize) -> String {
    if count == 1 {
        "1 song".into()
    } else {
        format!("{count} songs")
    }
}

impl Marks {
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn clear(&mut self) {
        self.marks.clear();
        self.anchor = None;
    }

    pub fn contains(&self, index: usize) -> bool {
        self.marks.binary_search_by_key(&index, |(i, _)| *i).is_ok()
    }

    /// The marked positions, in list order.
    pub fn indices(&self) -> Vec<usize> {
        self.marks.iter().map(|(i, _)| *i).collect()
    }

    /// Where a range mark starts: the position marked last, if it's still marked.
    pub fn anchor(&self) -> Option<usize> {
        self.anchor.filter(|&a| self.contains(a))
    }

    /// Mark `index` (holding the song `id`), or unmark it if it's marked.
    pub fn toggle(&mut self, index: usize, id: String) {
        match self.marks.binary_search_by_key(&index, |(i, _)| *i) {
            Ok(at) => {
                self.marks.remove(at);
            }
            Err(at) => {
                self.marks.insert(at, (index, id));
                self.anchor = Some(index);
            }
        }
    }

    /// Mark `index` (holding the song `id`), if it isn't already.
    pub fn mark(&mut self, index: usize, id: String) {
        if let Err(at) = self.marks.binary_search_by_key(&index, |(i, _)| *i) {
            self.marks.insert(at, (index, id));
        }
        self.anchor = Some(index);
    }

    /// Unmark the positions `keep` says no to.
    pub fn retain(&mut self, keep: impl Fn(usize) -> bool) {
        self.marks.retain(|(i, _)| keep(*i));
    }

    /// Match the marks up with the list as it is now (`len` items, `id_at` giving each one's
    /// identity): a mark whose song moved follows it to the nearest place it's at now, and one
    /// whose song is gone goes.
    pub fn sync(&mut self, len: usize, id_at: impl Fn(usize) -> Option<String>) {
        if self
            .marks
            .iter()
            .all(|(i, id)| id_at(*i).as_ref() == Some(id))
        {
            return;
        }
        let anchor = self.anchor;
        let mut synced: Vec<(usize, String)> = Vec::with_capacity(self.marks.len());
        let mut new_anchor = None;
        for (index, id) in std::mem::take(&mut self.marks) {
            let free = |i: usize, synced: &[(usize, String)]| {
                i < len && !synced.iter().any(|(s, _)| *s == i) && id_at(i).as_ref() == Some(&id)
            };
            let found = (0..len).find_map(|distance| {
                [index.checked_sub(distance), index.checked_add(distance)]
                    .into_iter()
                    .flatten()
                    .find(|&i| free(i, &synced))
            });
            if let Some(found) = found {
                if anchor == Some(index) {
                    new_anchor = Some(found);
                }
                synced.push((found, id));
            }
        }
        synced.sort_by_key(|(i, _)| *i);
        self.marks = synced;
        self.anchor = new_anchor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> impl Fn(usize) -> Option<String> {
        let list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        move |i| list.get(i).cloned()
    }

    #[test]
    fn marks_follow_their_songs_and_go_with_them() {
        let mut marks = Marks::default();
        marks.toggle(1, "b".into());
        marks.toggle(3, "d".into());
        assert_eq!(marks.anchor(), Some(3));
        // "a" was removed: everything moved up one.
        marks.sync(4, ids(&["b", "c", "d", "e"]));
        assert_eq!(marks.indices(), [0, 2]);
        assert_eq!(marks.anchor(), Some(2));
        // "b" is gone too.
        marks.sync(3, ids(&["c", "d", "e"]));
        assert_eq!(marks.indices(), [1]);
        marks.toggle(1, "d".into());
        assert!(marks.is_empty());
        assert_eq!(marks.anchor(), None);
    }

    #[test]
    fn duplicates_keep_one_mark_each() {
        let mut marks = Marks::default();
        marks.mark(0, "x".into());
        marks.mark(1, "x".into());
        marks.sync(3, ids(&["y", "x", "x"]));
        assert_eq!(marks.indices(), [1, 2]);
    }
}
