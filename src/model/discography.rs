//! An artist's releases, grouped as Spotify's apps group them: their albums, singles and EPs,
//! compilations, and the releases they appear on. Each group is loaded a page at a time (see
//! [crate::spotify_api::WebApi::artist_releases]) and kept newest first, without the releases
//! Spotify lists twice.

use std::collections::HashSet;

use crate::model::album::Album;

/// A group of an artist's releases, in the order they're shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReleaseGroup {
    Albums,
    Singles,
    Compilations,
    AppearsOn,
}

impl ReleaseGroup {
    pub const ALL: [Self; 4] = [
        Self::Albums,
        Self::Singles,
        Self::Compilations,
        Self::AppearsOn,
    ];

    /// The heading the group is shown under.
    pub fn title(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Singles => "Singles and EPs",
            Self::Compilations => "Compilations",
            Self::AppearsOn => "Appears On",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One release: the album, its full release date (`2024-05-17`, or less precise: `2024`) and
/// Spotify's kind of album (`album`, `single` or `compilation`).
#[derive(Clone)]
pub struct Release {
    pub album: Album,
    pub release_date: String,
    pub album_type: String,
}

impl Release {
    /// What two listings of the same release share: Spotify lists some releases twice (an
    /// explicit and a clean version, or the same album released again in another market) with
    /// the same name, artists and release date.
    fn key(&self) -> (String, String, String) {
        (
            self.album.title.trim().to_lowercase(),
            self.album.artists.join(", ").to_lowercase(),
            self.release_date.clone(),
        )
    }

    /// What's shown under the title of its card: the year and the kind of release, or for
    /// releases the artist appears on, whose they are.
    pub fn subtitle(&self, group: ReleaseGroup) -> String {
        let what = match group {
            ReleaseGroup::AppearsOn => self.album.artists.join(", "),
            _ => match self.album_type.as_str() {
                "single" => "Single".into(),
                "compilation" => "Compilation".into(),
                _ => "Album".into(),
            },
        };
        if self.album.year.is_empty() {
            what
        } else {
            format!("{} • {what}", self.album.year)
        }
    }
}

/// One page of releases as Spotify sent it.
pub struct ReleasePage {
    pub releases: Vec<Release>,
    /// Releases in the group in all, duplicates included.
    pub total: usize,
}

/// The releases of one group loaded so far.
#[derive(Default)]
pub struct Releases {
    pub releases: Vec<Release>,
    /// Releases Spotify listed so far (duplicates included): where the next page starts.
    pub listed: usize,
    /// Releases Spotify has in the group, once its first page is in.
    pub total: Option<usize>,
    /// A page is being loaded.
    pub loading: bool,
    /// The last page asked for couldn't be loaded, so no more are asked for.
    pub failed: bool,
    seen: HashSet<(String, String, String)>,
}

impl Releases {
    /// Whether Spotify has more releases in the group than have been loaded (and asking for
    /// them hasn't failed).
    pub fn has_more(&self) -> bool {
        !self.failed && self.total.is_none_or(|total| self.listed < total)
    }

    /// Whether the group's first page is in, or couldn't be loaded.
    pub fn is_done(&self) -> bool {
        self.total.is_some() || self.failed
    }

    /// Add a page of releases, leaving out the ones already there, and keep them newest first.
    pub fn add(&mut self, page: ReleasePage) {
        self.listed += page.releases.len();
        self.total = Some(page.total);
        if page.releases.is_empty() {
            // Nothing more will come, whatever the total said.
            self.total = Some(self.listed);
        }
        for release in page.releases {
            if self.seen.insert(release.key()) {
                self.releases.push(release);
            }
        }
        // Stable, so releases of the same day keep Spotify's order. Dates compare as text: they
        // are year first, and a date with just a year sorts before the days of that year.
        self.releases
            .sort_by(|a, b| b.release_date.cmp(&a.release_date));
    }
}

/// All the groups of an artist's releases.
#[derive(Default)]
pub struct Discography {
    groups: [Releases; 4],
}

impl Discography {
    pub fn group(&self, group: ReleaseGroup) -> &Releases {
        &self.groups[group.index()]
    }

    pub fn group_mut(&mut self, group: ReleaseGroup) -> &mut Releases {
        &mut self.groups[group.index()]
    }

    /// The groups with releases, in order, with their releases: the ones without are left out.
    pub fn shown(&self) -> Vec<(ReleaseGroup, &[Release])> {
        ReleaseGroup::ALL
            .iter()
            .map(|&g| (g, self.group(g).releases.as_slice()))
            .filter(|(_, releases)| !releases.is_empty())
            .collect()
    }

    /// Whether every group's first page is in (or couldn't be loaded).
    pub fn is_done(&self) -> bool {
        self.groups.iter().all(Releases::is_done)
    }

    /// Whether any group couldn't be loaded.
    pub fn failed(&self) -> bool {
        self.groups.iter().any(|g| g.failed)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A release without an ID (so opening it loads nothing), out on `date`.
    pub fn release(title: &str, date: &str, album_type: &str) -> Release {
        let mut album = Album::stub(
            String::new(),
            title.into(),
            vec!["Some Artist".into()],
            None,
        );
        album.id = None;
        album.url = None;
        album.year = date.split('-').next().unwrap_or_default().into();
        Release {
            album,
            release_date: date.into(),
            album_type: album_type.into(),
        }
    }

    fn titles(releases: &Releases) -> Vec<&str> {
        releases
            .releases
            .iter()
            .map(|r| r.album.title.as_str())
            .collect()
    }

    #[test]
    fn releases_listed_twice_are_shown_once() {
        let mut releases = Releases::default();
        releases.add(ReleasePage {
            releases: vec![
                release("Pang", "2019-10-18", "album"),
                // Its clean version.
                release("Pang", "2019-10-18", "album"),
                // Released again later: a release of its own.
                release("Pang", "2020-02-14", "album"),
            ],
            total: 4,
        });
        // And on the next page, under another case.
        releases.add(ReleasePage {
            releases: vec![release("PANG", "2019-10-18", "album")],
            total: 4,
        });
        assert_eq!(titles(&releases), ["Pang", "Pang"]);
        assert_eq!(releases.listed, 4);
        assert!(!releases.has_more());
    }

    #[test]
    fn releases_are_newest_first_across_pages() {
        let mut releases = Releases::default();
        assert!(releases.has_more() && !releases.is_done());
        releases.add(ReleasePage {
            releases: vec![release("B", "2020-01-01", "album")],
            total: 3,
        });
        assert!(releases.has_more() && releases.is_done());
        releases.add(ReleasePage {
            releases: vec![
                release("C", "2018", "album"),
                release("A", "2021-06-01", "album"),
            ],
            total: 3,
        });
        assert_eq!(titles(&releases), ["A", "B", "C"]);
        assert!(!releases.has_more());
    }

    #[test]
    fn an_empty_page_ends_the_group() {
        let mut releases = Releases::default();
        releases.add(ReleasePage {
            releases: Vec::new(),
            total: 10,
        });
        assert!(!releases.has_more());
    }

    #[test]
    fn groups_without_releases_are_left_out() {
        let mut discography = Discography::default();
        discography
            .group_mut(ReleaseGroup::Singles)
            .add(ReleasePage {
                releases: vec![release("One", "2020", "single")],
                total: 1,
            });
        let shown: Vec<_> = discography.shown().iter().map(|(g, _)| *g).collect();
        assert_eq!(shown, [ReleaseGroup::Singles]);
        assert!(!discography.is_done());
    }

    #[test]
    fn subtitles_say_the_year_and_kind() {
        let single = release("One", "2020-03-01", "single");
        assert_eq!(single.subtitle(ReleaseGroup::Singles), "2020 • Single");
        assert_eq!(
            single.subtitle(ReleaseGroup::AppearsOn),
            "2020 • Some Artist"
        );
        let undated = release("Two", "", "album");
        assert_eq!(undated.subtitle(ReleaseGroup::Albums), "Album");
    }
}
