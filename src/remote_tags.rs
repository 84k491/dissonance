// Mapping of local artists and albums onto those of a remote tag source.
//
// Nothing here knows which remote source is used; production wires an
// implementation of 'RemoteInfoProvider' in, unit tests wire a mock in.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteArtist {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAlbum {
    pub id: String,
    pub name: String,
    pub artist_name: String,

    // The year is shown to the user, it is never part of the likeness.
    pub year: String,

    // Zero means the remote source did not tell the count.
    pub track_count: u32,
}

// Mapping state of a single local artist. An absent mapping is 'Option::None',
// it has no variant of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteArtistMeta {
    // The mapping was requested and the user found no suitable option.
    NoSuitable,

    // The storage keeps the id alone, so a mapping read from the file starts
    // with 'None' here; a request fills the name in.
    Mapped { id: String, name: Option<String> },
}

// Mapping state of a single local album, the twin of 'RemoteArtistMeta'.
//
// The name is kept for the mapping key alone: an album has to be found again
// under its remote name once an install renames it. It is never persisted, so a
// mapping read from the file starts with 'None' here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAlbumMeta {
    NoSuitable,
    Mapped { id: String, name: Option<String> },
}

// What an album install writes into every music file below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAlbumTags {
    pub name: String,

    // Empty when the remote source told no year; the local one is then left
    // alone rather than emptied.
    pub year: String,
}

// One option of the list shown to the user, with its likeness to the local name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistCandidate {
    pub artist: RemoteArtist,
    pub likeness_percent: u32,
}

// One option of the album list. The percentage is the likeness of the names
// alone; the track count orders the list instead of scoring it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumCandidate {
    pub album: RemoteAlbum,
    pub likeness_percent: u32,

    // The remote track count sits within the tolerance of the local one. Such
    // candidates head the list, whatever their names say.
    pub track_count_matches: bool,
}

// What the user picked in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Chosen(RemoteArtist),
    NoSuitable,
    Cancelled,
}

// What the user picked in the album list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlbumSelection {
    Chosen(RemoteAlbum),
    NoSuitable,
    Cancelled,
}

pub trait RemoteInfoProvider: Send + Sync {
    // (localArtist) -> artistList
    fn search_artists(&self, local_name: &str) -> Result<Vec<RemoteArtist>, String>;

    // (id) -> artist. Needed because the storage keeps the id alone.
    fn artist_by_id(&self, id: &str) -> Result<RemoteArtist, String>;

    // (artistId) -> albumList. An album is only ever mapped under an artist
    // that is mapped already, so the artist id is what narrows the search.
    fn search_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String>;

    // (id) -> album. Needed because the storage keeps the id alone.
    fn album_by_id(&self, id: &str) -> Result<RemoteAlbum, String>;
}

pub trait ArtistSelector {
    // (artistList) -> finalArtist
    fn select(&self, local_name: &str, candidates: &[ArtistCandidate]) -> Selection;
}

pub trait AlbumSelector {
    // (albumList) -> finalAlbum
    fn select(&self, local_name: &str, candidates: &[AlbumCandidate]) -> AlbumSelection;
}

// The whole request flow, free of locks and threads: the caller decides on
// which thread it runs and how the result is applied.
//
// 'Ok(None)' means the user cancelled and the state must stay as it was.
pub fn resolve_artist_mapping(
    local_name: &str,
    provider: &dyn RemoteInfoProvider,
    selector: &dyn ArtistSelector,
) -> Result<Option<RemoteArtistMeta>, String> {
    let artists = provider.search_artists(local_name)?;
    let candidates = rank_candidates(local_name, artists);

    match selector.select(local_name, &candidates) {
        Selection::Chosen(artist) => Ok(Some(RemoteArtistMeta::Mapped {
            id: artist.id,
            name: Some(artist.name),
        })),
        Selection::NoSuitable => Ok(Some(RemoteArtistMeta::NoSuitable)),
        Selection::Cancelled => Ok(None),
    }
}

// The album request flow, the twin of 'resolve_artist_mapping'. The artist id
// comes from the mapping of the artist directory above.
//
// 'Ok(None)' means the user cancelled and the state must stay as it was.
pub fn resolve_album_mapping(
    artist_id: &str,
    local_name: &str,
    local_track_count: u32,
    provider: &dyn RemoteInfoProvider,
    selector: &dyn AlbumSelector,
) -> Result<Option<RemoteAlbumMeta>, String> {
    let albums = provider.search_albums(artist_id)?;
    let candidates = rank_album_candidates(local_name, local_track_count, albums);

    match selector.select(local_name, &candidates) {
        AlbumSelection::Chosen(album) => Ok(Some(RemoteAlbumMeta::Mapped {
            id: album.id,
            name: Some(album.name),
        })),
        AlbumSelection::NoSuitable => Ok(Some(RemoteAlbumMeta::NoSuitable)),
        AlbumSelection::Cancelled => Ok(None),
    }
}

// The remote name of a mapping, taken from the in-session cache or requested by id.
pub fn resolve_remote_name(
    meta: &RemoteArtistMeta,
    provider: &dyn RemoteInfoProvider,
) -> Result<String, String> {
    match meta {
        RemoteArtistMeta::NoSuitable => Err("No suitable remote artist".to_string()),
        RemoteArtistMeta::Mapped {
            name: Some(name), ..
        } => Ok(name.clone()),
        RemoteArtistMeta::Mapped { id, name: None } => {
            Ok(provider.artist_by_id(id)?.name)
        }
    }
}

// The tags an album install writes, always asked of the source by id. The
// mapping keeps no year, and the name it does keep is the key material of an
// earlier request, so the id is the only thing worth trusting here.
pub fn resolve_remote_album_tags(
    meta: &RemoteAlbumMeta,
    provider: &dyn RemoteInfoProvider,
) -> Result<RemoteAlbumTags, String> {
    let id = match meta {
        RemoteAlbumMeta::NoSuitable => return Err("No suitable remote album".to_string()),
        RemoteAlbumMeta::Mapped { id, .. } => id,
    };

    let album = provider.album_by_id(id)?;

    Ok(RemoteAlbumTags {
        name: album.name,
        year: album.year,
    })
}

// Sorts the found artists by likeness to the local name, the closest first.
pub fn rank_candidates(local_name: &str, artists: Vec<RemoteArtist>) -> Vec<ArtistCandidate> {
    let mut ret: Vec<ArtistCandidate> = artists
        .into_iter()
        .map(|artist| ArtistCandidate {
            likeness_percent: likeness_percent(local_name, &artist.name),
            artist,
        })
        .collect();

    ret.sort_by(|a, b| b.likeness_percent.cmp(&a.likeness_percent));

    ret
}

// How far a remote track count may sit from the local one and still be called
// the same album. The remote source counts bonus items a local album does not
// hold, so an exact comparison would push the right album down the list.
pub const TRACK_COUNT_TOLERANCE: u32 = 1;

// Whether two track counts are close enough to mean the same album. A zero
// count is unknown: the local directory is empty or the source told nothing.
pub fn track_counts_match(local: u32, remote: u32) -> bool {
    if local == 0 || remote == 0 {
        return false;
    }

    local.abs_diff(remote) <= TRACK_COUNT_TOLERANCE
}

// Sorts the found albums by track count first and by likeness of the names
// second. The percentage stays the likeness of the names alone, so the count
// orders the list without touching what is shown.
pub fn rank_album_candidates(
    local_name: &str,
    local_track_count: u32,
    albums: Vec<RemoteAlbum>,
) -> Vec<AlbumCandidate> {
    let mut ret: Vec<AlbumCandidate> = albums
        .into_iter()
        .map(|album| AlbumCandidate {
            likeness_percent: likeness_percent(local_name, &album.name),
            track_count_matches: track_counts_match(local_track_count, album.track_count),
            album,
        })
        .collect();

    ret.sort_by(|a, b| {
        b.track_count_matches
            .cmp(&a.track_count_matches)
            .then(b.likeness_percent.cmp(&a.likeness_percent))
    });

    ret
}

// 100 * (1 - distance / max_len), case insensitive, over chars.
pub fn likeness_percent(left: &str, right: &str) -> u32 {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();

    let max_len = left.len().max(right.len());
    if max_len == 0 {
        return 100;
    }

    let distance = levenshtein(&left, &right);
    let ratio = 1.0 - (distance as f64 / max_len as f64);

    (ratio * 100.0).round() as u32
}

fn levenshtein(left: &[char], right: &[char]) -> usize {
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }

    // only the previous and the current row of the matrix are kept
    let mut prev: Vec<usize> = (0..=right.len()).collect();
    let mut cur: Vec<usize> = vec![0; right.len() + 1];

    for (i, l) in left.iter().enumerate() {
        cur[0] = i + 1;

        for (j, r) in right.iter().enumerate() {
            let substitution = prev[j] + if l == r { 0 } else { 1 };
            let insertion = cur[j] + 1;
            let deletion = prev[j + 1] + 1;

            cur[j + 1] = substitution.min(insertion).min(deletion);
        }

        std::mem::swap(&mut prev, &mut cur);
    }

    prev[right.len()]
}

// The id written for a mapping that was requested and found nothing suitable.
const NO_SUITABLE_ID: &str = "NoSuitable";

// A single record of the storage. The id "NoSuitable" means "no suitable", a
// null one means the mapping is absent, just like a missing key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteArtistEntry {
    pub id: Option<String>,
}

// The persistent artist name -> remote id map.
//
// Several names can point to a single id: the local name and the remote one are
// both stored when they differ.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteArtistMappingDatabase {
    entries: BTreeMap<String, RemoteArtistEntry>,
}

impl RemoteArtistMappingDatabase {
    pub fn new() -> RemoteArtistMappingDatabase {
        RemoteArtistMappingDatabase::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // The mapping of a local artist, 'None' when it was never requested.
    pub fn meta_for(&self, artist_name: &str) -> Option<RemoteArtistMeta> {
        let entry = self.entries.get(artist_name)?;

        let id = match &entry.id {
            // a null id is "absent" as well
            None => return None,
            Some(id) if id == NO_SUITABLE_ID => return Some(RemoteArtistMeta::NoSuitable),
            Some(id) => id,
        };

        // the file holds no names, only a request can tell one
        Some(RemoteArtistMeta::Mapped {
            id: id.clone(),
            name: None,
        })
    }

    // Stores a finished mapping. A remote name differing from the local one
    // gets an entry of its own, pointing to the same id.
    pub fn set(&mut self, local_name: &str, meta: &RemoteArtistMeta) {
        match meta {
            RemoteArtistMeta::NoSuitable => {
                self.entries.insert(
                    local_name.to_string(),
                    RemoteArtistEntry {
                        id: Some(NO_SUITABLE_ID.to_string()),
                    },
                );
            }
            RemoteArtistMeta::Mapped { id, name } => {
                self.entries.insert(
                    local_name.to_string(),
                    RemoteArtistEntry {
                        id: Some(id.clone()),
                    },
                );

                if let Some(name) = name {
                    if name != local_name {
                        self.entries.insert(
                            name.clone(),
                            RemoteArtistEntry {
                                id: Some(id.clone()),
                            },
                        );
                    }
                }
            }
        }
    }

}

// A single record of the album storage, the same shape as the artist one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAlbumEntry {
    pub id: Option<String>,
}

// The persistent album mapping, nested: an album name means nothing on its own,
// so it is kept under the artist it belongs to.
//
//   { "Beatles": { "Revolver": { "id": "1642995371" } } }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteAlbumMappingDatabase {
    // artist name -> album name -> record
    entries: BTreeMap<String, BTreeMap<String, RemoteAlbumEntry>>,
}

impl RemoteAlbumMappingDatabase {
    pub fn new() -> RemoteAlbumMappingDatabase {
        RemoteAlbumMappingDatabase::default()
    }

    // Every album of every artist.
    pub fn len(&self) -> usize {
        self.entries.values().map(|albums| albums.len()).sum()
    }

    // The mapping of a local album, 'None' when it was never requested.
    pub fn meta_for(&self, artist_name: &str, album_name: &str) -> Option<RemoteAlbumMeta> {
        let entry = self.entries.get(artist_name)?.get(album_name)?;

        let id = match &entry.id {
            // a null id is "absent" as well
            None => return None,
            Some(id) if id == NO_SUITABLE_ID => return Some(RemoteAlbumMeta::NoSuitable),
            Some(id) => id,
        };

        // the file holds no names, only a request can tell one
        Some(RemoteAlbumMeta::Mapped {
            id: id.clone(),
            name: None,
        })
    }

    // Stores a finished mapping under every name the album can be found by.
    //
    // Either install can rename either level of the key: installing the artist
    // tags renames the artist above, installing the album tags renames the
    // album itself. The cross product of the known names is written, so that
    // the mapping is found again whatever was installed and in which order.
    pub fn set(&mut self, artist_names: &[&str], album_names: &[&str], meta: &RemoteAlbumMeta) {
        let id = match meta {
            RemoteAlbumMeta::NoSuitable => NO_SUITABLE_ID.to_string(),
            RemoteAlbumMeta::Mapped { id, .. } => id.clone(),
        };

        for artist in artist_names {
            for album in album_names {
                if artist.is_empty() || album.is_empty() {
                    continue;
                }

                self.entries
                    .entry(artist.to_string())
                    .or_default()
                    .insert(
                        album.to_string(),
                        RemoteAlbumEntry {
                            id: Some(id.clone()),
                        },
                    );
            }
        }
    }
}

#[cfg(test)]
pub mod mocks {
    use super::{
        AlbumCandidate, AlbumSelection, ArtistCandidate, RemoteAlbum, RemoteArtist,
        RemoteInfoProvider, Selection,
    };

    // A provider seeded with every artist and album it knows about. A real call
    // filters that universe, like a remote search would.
    pub struct MockRemoteInfoProvider {
        universe: Vec<RemoteArtist>,

        // the artist id an album belongs to, beside the album itself
        albums: Vec<(String, RemoteAlbum)>,

        failure: Option<String>,
    }

    impl MockRemoteInfoProvider {
        pub fn new(universe: &[(&str, &str)]) -> MockRemoteInfoProvider {
            MockRemoteInfoProvider {
                universe: universe
                    .iter()
                    .map(|(id, name)| RemoteArtist {
                        id: id.to_string(),
                        name: name.to_string(),
                    })
                    .collect(),
                albums: Vec::new(),
                failure: None,
            }
        }

        // Seeds the albums, as (artist id, album id, album name, year, track
        // count). The artist name of an album is taken from the artist universe.
        pub fn with_albums(
            mut self,
            albums: &[(&str, &str, &str, &str, u32)],
        ) -> MockRemoteInfoProvider {
            for (artist_id, album_id, name, year, track_count) in albums {
                let artist_name = self
                    .universe
                    .iter()
                    .find(|artist| artist.id == *artist_id)
                    .map(|artist| artist.name.clone())
                    .unwrap_or_default();

                self.albums.push((
                    artist_id.to_string(),
                    RemoteAlbum {
                        id: album_id.to_string(),
                        name: name.to_string(),
                        artist_name,
                        year: year.to_string(),
                        track_count: *track_count,
                    },
                ));
            }

            self
        }

        // A provider that fails every request, to test the network error path.
        pub fn failing(message: &str) -> MockRemoteInfoProvider {
            MockRemoteInfoProvider {
                universe: Vec::new(),
                albums: Vec::new(),
                failure: Some(message.to_string()),
            }
        }

        // A name matches when the two share a word, in any direction.
        fn matches(local_name: &str, remote_name: &str) -> bool {
            let local = local_name.to_lowercase();
            let remote = remote_name.to_lowercase();

            if local == remote || remote.contains(&local) || local.contains(&remote) {
                return true;
            }

            local
                .split_whitespace()
                .any(|word| remote.split_whitespace().any(|other| other == word))
        }
    }

    impl RemoteInfoProvider for MockRemoteInfoProvider {
        fn search_artists(&self, local_name: &str) -> Result<Vec<RemoteArtist>, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            Ok(self
                .universe
                .iter()
                .filter(|artist| Self::matches(local_name, &artist.name))
                .cloned()
                .collect())
        }

        fn artist_by_id(&self, id: &str) -> Result<RemoteArtist, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            self.universe
                .iter()
                .find(|artist| artist.id == id)
                .cloned()
                .ok_or(format!("Unknown artist id: {}", id))
        }

        fn search_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            // every album of that artist, like the remote listing gives
            Ok(self
                .albums
                .iter()
                .filter(|(owner, _)| owner == artist_id)
                .map(|(_, album)| album.clone())
                .collect())
        }

        fn album_by_id(&self, id: &str) -> Result<RemoteAlbum, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            self.album_universe()
                .find(|album| album.id == id)
                .cloned()
                .ok_or(format!("Unknown album id: {}", id))
        }
    }

    impl MockRemoteInfoProvider {
        fn album_universe(&self) -> impl Iterator<Item = &RemoteAlbum> {
            self.albums.iter().map(|(_, album)| album)
        }
    }

    // The rule a mock selector applies to the list it is given.
    pub enum MockChoice {
        // Picks the option with this exact remote name.
        ByName(String),

        // Picks the closest option.
        Best,

        NoSuitable,
        Cancel,
    }

    pub struct MockArtistSelector {
        choice: MockChoice,
    }

    impl MockArtistSelector {
        pub fn new(choice: MockChoice) -> MockArtistSelector {
            MockArtistSelector { choice }
        }
    }

    impl super::ArtistSelector for MockArtistSelector {
        fn select(&self, _local_name: &str, candidates: &[ArtistCandidate]) -> Selection {
            let picked: Option<&ArtistCandidate> = match &self.choice {
                MockChoice::NoSuitable => return Selection::NoSuitable,
                MockChoice::Cancel => return Selection::Cancelled,
                MockChoice::Best => candidates.first(),
                MockChoice::ByName(name) => candidates.iter().find(|c| c.artist.name == *name),
            };

            match picked {
                Some(candidate) => Selection::Chosen(candidate.artist.clone()),
                None => Selection::Cancelled,
            }
        }
    }

    // The rule a mock album selector applies to the list it is given.
    pub enum MockAlbumChoice {
        // Picks the option with this exact remote name.
        ByName(String),

        // Picks the head of the ranked list.
        Best,

        NoSuitable,
        Cancel,
    }

    pub struct MockAlbumSelector {
        choice: MockAlbumChoice,
    }

    impl MockAlbumSelector {
        pub fn new(choice: MockAlbumChoice) -> MockAlbumSelector {
            MockAlbumSelector { choice }
        }
    }

    impl super::AlbumSelector for MockAlbumSelector {
        fn select(&self, _local_name: &str, candidates: &[AlbumCandidate]) -> AlbumSelection {
            let picked: Option<&AlbumCandidate> = match &self.choice {
                MockAlbumChoice::NoSuitable => return AlbumSelection::NoSuitable,
                MockAlbumChoice::Cancel => return AlbumSelection::Cancelled,
                MockAlbumChoice::Best => candidates.first(),
                MockAlbumChoice::ByName(name) => {
                    candidates.iter().find(|c| c.album.name == *name)
                }
            };

            match picked {
                Some(candidate) => AlbumSelection::Chosen(candidate.album.clone()),
                None => AlbumSelection::Cancelled,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteAlbumMappingDatabase, RemoteAlbumMeta, RemoteArtist, RemoteArtistMappingDatabase,
        RemoteArtistMeta, RemoteInfoProvider, likeness_percent,
        mocks::{
            MockAlbumChoice, MockAlbumSelector, MockArtistSelector, MockChoice,
            MockRemoteInfoProvider,
        },
        rank_album_candidates, rank_candidates, resolve_album_mapping, resolve_artist_mapping,
        resolve_remote_album_tags, resolve_remote_name, track_counts_match,
    };

    fn universe() -> MockRemoteInfoProvider {
        MockRemoteInfoProvider::new(&[
            ("123456", "The Beatles"),
            ("890123468", "Beatles Tribute Band"),
            ("555", "Jack Johnson"),
        ])
    }

    // The same artists, with the albums the source holds for them. The track
    // counts are the ones the real source reports.
    fn album_universe() -> MockRemoteInfoProvider {
        universe().with_albums(&[
            ("123456", "1441164426", "Abbey Road (Remastered)", "1969", 19),
            ("123456", "1441133100", "Abbey Road (2019 Mix)", "1969", 17),
            ("123456", "1642995371", "Revolver (2022 Mix)", "2022", 14),
            ("555", "999", "Brushfire Fairytales", "2001", 14),
        ])
    }

    #[test]
    fn likeness_of_equal_names_is_full() {
        assert_eq!(likeness_percent("The Beatles", "the beatles"), 100);
        assert_eq!(likeness_percent("", ""), 100);
    }

    #[test]
    fn likeness_drops_with_distance() {
        // "beatles" -> "the beatles": 4 inserts over 11 chars
        assert_eq!(likeness_percent("Beatles", "The Beatles"), 64);
        assert!(likeness_percent("Beatles", "Jack Johnson") < 30);
    }

    #[test]
    fn candidates_are_sorted_by_likeness() {
        let found = universe().search_artists("Beatles").expect("search failed");
        let candidates = rank_candidates("Beatles", found);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].artist.name, "The Beatles");
        assert!(candidates[0].likeness_percent > candidates[1].likeness_percent);
    }

    #[test]
    fn selecting_an_artist_maps_it() {
        let meta = resolve_artist_mapping(
            "Beatles",
            &universe(),
            &MockArtistSelector::new(MockChoice::ByName("The Beatles".to_string())),
        )
        .expect("resolve failed");

        assert_eq!(
            meta,
            Some(RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            })
        );
    }

    #[test]
    fn no_suitable_and_cancel_are_distinct() {
        let no_suitable = resolve_artist_mapping(
            "Beatles",
            &universe(),
            &MockArtistSelector::new(MockChoice::NoSuitable),
        )
        .expect("resolve failed");
        assert_eq!(no_suitable, Some(RemoteArtistMeta::NoSuitable));

        let cancelled = resolve_artist_mapping(
            "Beatles",
            &universe(),
            &MockArtistSelector::new(MockChoice::Cancel),
        )
        .expect("resolve failed");
        assert_eq!(cancelled, None);
    }

    #[test]
    fn a_failing_provider_reports_an_error() {
        let result = resolve_artist_mapping(
            "Beatles",
            &MockRemoteInfoProvider::failing("no network"),
            &MockArtistSelector::new(MockChoice::Best),
        );

        assert_eq!(result, Err("no network".to_string()));
    }

    #[test]
    fn a_differing_remote_name_gets_an_entry_of_its_own() {
        let mut map = RemoteArtistMappingDatabase::new();
        map.set(
            "Beatles",
            &RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            },
        );

        assert_eq!(map.len(), 2);
        assert!(matches!(
            map.meta_for("Beatles"),
            Some(RemoteArtistMeta::Mapped { .. })
        ));
        assert!(matches!(
            map.meta_for("The Beatles"),
            Some(RemoteArtistMeta::Mapped { .. })
        ));
    }

    #[test]
    fn a_matching_remote_name_gets_a_single_entry() {
        let mut map = RemoteArtistMappingDatabase::new();
        map.set(
            "Jack Johnson",
            &RemoteArtistMeta::Mapped {
                id: "555".to_string(),
                name: Some("Jack Johnson".to_string()),
            },
        );

        assert_eq!(map.len(), 1);
    }

    #[test]
    fn only_the_id_is_persisted() {
        let mut map = RemoteArtistMappingDatabase::new();
        map.set(
            "Beatles",
            &RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            },
        );
        map.set("Unknown Band", &RemoteArtistMeta::NoSuitable);

        let json = serde_json::to_string(&map).expect("serialize failed");
        assert_eq!(
            json,
            r#"{"Beatles":{"id":"123456"},"The Beatles":{"id":"123456"},"Unknown Band":{"id":"NoSuitable"}}"#
        );

        let restored: RemoteArtistMappingDatabase = serde_json::from_str(&json).expect("parse failed");

        // the name is gone after a round trip, the id is not
        assert_eq!(
            restored.meta_for("Beatles"),
            Some(RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: None,
            })
        );
        assert_eq!(
            restored.meta_for("Unknown Band"),
            Some(RemoteArtistMeta::NoSuitable)
        );
        assert_eq!(restored.meta_for("Never Requested"), None);
    }

    #[test]
    fn a_null_id_reads_as_absent() {
        let map: RemoteArtistMappingDatabase =
            serde_json::from_str(r#"{"Unknown Band":{"id":null}}"#).expect("parse failed");

        assert_eq!(map.meta_for("Unknown Band"), None);
    }

    #[test]
    fn a_name_the_mapping_does_not_hold_is_requested_by_id() {
        let known = resolve_remote_name(
            &RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            },
            &MockRemoteInfoProvider::failing("must not be called"),
        );
        assert_eq!(known, Ok("The Beatles".to_string()));

        let requested = resolve_remote_name(
            &RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: None,
            },
            &universe(),
        );
        assert_eq!(requested, Ok("The Beatles".to_string()));
    }

    #[test]
    fn a_mock_provider_finds_an_artist_by_id() {
        assert_eq!(
            universe().artist_by_id("555"),
            Ok(RemoteArtist {
                id: "555".to_string(),
                name: "Jack Johnson".to_string(),
            })
        );
        assert!(universe().artist_by_id("nope").is_err());
    }

    #[test]
    fn a_track_count_within_the_tolerance_matches() {
        assert!(track_counts_match(19, 19));

        // the source counts bonus items a local album does not hold
        assert!(track_counts_match(19, 20));
        assert!(track_counts_match(19, 18));

        assert!(!track_counts_match(19, 17));
        assert!(!track_counts_match(19, 40));

        // a zero count is unknown, never a match
        assert!(!track_counts_match(0, 19));
        assert!(!track_counts_match(19, 0));
    }

    #[test]
    fn album_candidates_are_ordered_by_track_count_before_the_name() {
        let found = album_universe()
            .search_albums("123456")
            .expect("listing failed");

        // a rip of the remastered edition: 19 tracks
        let candidates = rank_album_candidates("Abbey Road", 19, found);

        // the name alone would put "(2019 Mix)" first, because it is a shorter
        // suffix; the track count is what tells the right album
        assert_eq!(candidates[0].album.name, "Abbey Road (Remastered)");
        assert!(candidates[0].track_count_matches);
        assert_eq!(candidates[1].album.name, "Abbey Road (2019 Mix)");
        assert!(!candidates[1].track_count_matches);

        // the percentage stays the likeness of the names alone, so the head of
        // the list is allowed to show a lower one than the option below it
        assert_eq!(candidates[0].likeness_percent, 43);
        assert_eq!(candidates[1].likeness_percent, 48);
    }

    #[test]
    fn a_differing_track_count_moves_the_head_of_the_album_list() {
        let found = album_universe()
            .search_albums("123456")
            .expect("listing failed");

        // the very same local name, ripped from the 2019 mix instead
        let candidates = rank_album_candidates("Abbey Road", 17, found);

        assert_eq!(candidates[0].album.name, "Abbey Road (2019 Mix)");
        assert!(candidates[0].track_count_matches);
    }

    #[test]
    fn an_album_listing_holds_one_artist_only() {
        let albums = album_universe()
            .search_albums("555")
            .expect("listing failed");

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Brushfire Fairytales");
        assert_eq!(albums[0].artist_name, "Jack Johnson");
        assert_eq!(albums[0].year, "2001");
        assert_eq!(albums[0].track_count, 14);
    }

    #[test]
    fn selecting_an_album_maps_it() {
        let meta = resolve_album_mapping(
            "123456",
            "Abbey Road",
            19,
            &album_universe(),
            &MockAlbumSelector::new(MockAlbumChoice::Best),
        )
        .expect("resolve failed");

        assert_eq!(
            meta,
            Some(RemoteAlbumMeta::Mapped {
                id: "1441164426".to_string(),
                name: Some("Abbey Road (Remastered)".to_string()),
            })
        );
    }

    #[test]
    fn album_no_suitable_and_cancel_are_distinct() {
        let no_suitable = resolve_album_mapping(
            "123456",
            "Abbey Road",
            19,
            &album_universe(),
            &MockAlbumSelector::new(MockAlbumChoice::NoSuitable),
        )
        .expect("resolve failed");
        assert_eq!(no_suitable, Some(RemoteAlbumMeta::NoSuitable));

        let cancelled = resolve_album_mapping(
            "123456",
            "Abbey Road",
            19,
            &album_universe(),
            &MockAlbumSelector::new(MockAlbumChoice::Cancel),
        )
        .expect("resolve failed");
        assert_eq!(cancelled, None);
    }

    #[test]
    fn a_failing_provider_reports_an_album_error() {
        let result = resolve_album_mapping(
            "123456",
            "Abbey Road",
            19,
            &MockRemoteInfoProvider::failing("no network"),
            &MockAlbumSelector::new(MockAlbumChoice::Best),
        );

        assert_eq!(result, Err("no network".to_string()));
    }

    #[test]
    fn an_album_is_stored_under_every_known_name() {
        let mut map = RemoteAlbumMappingDatabase::new();
        map.set(
            &["Beatles", "The Beatles"],
            &["Revolver", "Revolver (2022 Mix)"],
            &RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: Some("Revolver (2022 Mix)".to_string()),
            },
        );

        // either install can rename either half of the key, so every pairing is
        // written and the mapping is found again whatever was installed
        assert_eq!(map.len(), 4);
        assert!(map.meta_for("Beatles", "Revolver").is_some());
        assert!(map.meta_for("Beatles", "Revolver (2022 Mix)").is_some());
        assert!(map.meta_for("The Beatles", "Revolver").is_some());
        assert!(
            map.meta_for("The Beatles", "Revolver (2022 Mix)")
                .is_some()
        );

        // the album name means nothing under the wrong artist
        assert_eq!(map.meta_for("Jack Johnson", "Revolver"), None);
    }

    #[test]
    fn only_the_album_id_is_persisted() {
        let mut map = RemoteAlbumMappingDatabase::new();
        map.set(
            &["Beatles"],
            &["Revolver"],
            &RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: Some("Revolver (2022 Mix)".to_string()),
            },
        );
        map.set(&["Beatles"], &["Bootleg"], &RemoteAlbumMeta::NoSuitable);

        let json = serde_json::to_string(&map).expect("serialize failed");
        assert_eq!(
            json,
            r#"{"Beatles":{"Bootleg":{"id":"NoSuitable"},"Revolver":{"id":"1642995371"}}}"#
        );

        let restored: RemoteAlbumMappingDatabase =
            serde_json::from_str(&json).expect("parse failed");

        // the name is gone after a round trip, the id is not
        assert_eq!(
            restored.meta_for("Beatles", "Revolver"),
            Some(RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: None,
            })
        );
        assert_eq!(
            restored.meta_for("Beatles", "Bootleg"),
            Some(RemoteAlbumMeta::NoSuitable)
        );
        assert_eq!(restored.meta_for("Beatles", "Never Requested"), None);
        assert_eq!(restored.meta_for("Never Requested", "Revolver"), None);
    }

    #[test]
    fn a_null_album_id_reads_as_absent() {
        let map: RemoteAlbumMappingDatabase =
            serde_json::from_str(r#"{"Beatles":{"Bootleg":{"id":null}}}"#).expect("parse failed");

        assert_eq!(map.meta_for("Beatles", "Bootleg"), None);
    }

    #[test]
    fn album_tags_are_always_requested_by_id() {
        // the mapping keeps no year, so the source is asked even when the name
        // is known already
        for name in [None, Some("Abbey Road (Remastered)".to_string())] {
            let meta = RemoteAlbumMeta::Mapped {
                id: "1441164426".to_string(),
                name,
            };

            let requested =
                resolve_remote_album_tags(&meta, &album_universe()).expect("no tags");
            assert_eq!(requested.name, "Abbey Road (Remastered)");
            assert_eq!(requested.year, "1969");

            // and an install cannot go through while the source is unreachable
            assert_eq!(
                resolve_remote_album_tags(&meta, &MockRemoteInfoProvider::failing("no network")),
                Err("no network".to_string())
            );
        }

        assert!(
            resolve_remote_album_tags(&RemoteAlbumMeta::NoSuitable, &album_universe()).is_err()
        );
    }

    #[test]
    fn a_mock_provider_finds_an_album_by_id() {
        assert_eq!(
            album_universe()
                .album_by_id("1642995371")
                .map(|album| album.name),
            Ok("Revolver (2022 Mix)".to_string())
        );
        assert!(album_universe().album_by_id("nope").is_err());
    }
}
