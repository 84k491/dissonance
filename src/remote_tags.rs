// Mapping of local artists and albums onto those of a remote tag source.
//
// Nothing here knows which remote source is used; production wires an
// implementation of 'RemoteInfoProvider' in, unit tests wire a mock in.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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

// One track of a remote album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTrack {
    pub id: String,
    pub name: String,
    pub track_number: u32,

    // Zero means the source told nothing about the set.
    pub disc_number: u32,
    pub total_discs: u32,
}

// One local file taking part in the matching. The name is what the file is
// called on disk, free of the track number, because the tags are exactly what
// is not trusted here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub name: String,
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

    // The album install writes the artist as well, so that one click on an
    // album is enough even when the artist tags were never installed.
    pub artist_name: String,

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

// One option of the track list, ranked against the name of one local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackCandidate {
    pub track: RemoteTrack,
    pub likeness_percent: u32,
}

// What the user picked for one local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackSelection {
    Chosen(RemoteTrack),

    // This file is left as it is; the rest of the album goes on.
    Skip,

    // The whole install is dropped, the files already matched included.
    Cancelled,
}

// What a track install writes into one music file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTrackTags {
    pub title: String,
    pub track_number: String,

    // Empty when the source told nothing about the set.
    pub disc_number: String,
    pub total_discs: String,
}

impl RemoteTrackTags {
    fn of(track: &RemoteTrack) -> RemoteTrackTags {
        let number_or_empty = |n: u32| match n {
            0 => String::new(),
            n => n.to_string(),
        };

        RemoteTrackTags {
            title: track.name.clone(),
            track_number: track.track_number.to_string(),
            disc_number: number_or_empty(track.disc_number),
            total_discs: number_or_empty(track.total_discs),
        }
    }
}

// How alike two names have to be to be matched without asking the user.
pub const AUTO_MATCH_LIKENESS: u32 = 99;

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

    // (albumId) -> trackList. Asked once per install, never per file.
    fn album_tracks(&self, album_id: &str) -> Result<Vec<RemoteTrack>, String>;

    // (albumId) -> cover bytes, a jpeg.
    fn album_cover(&self, album_id: &str) -> Result<Vec<u8>, String>;
}

pub trait ArtistSelector {
    // (artistList) -> finalArtist
    fn select(&self, local_name: &str, candidates: &[ArtistCandidate]) -> Selection;
}

pub trait AlbumSelector {
    // (albumList) -> finalAlbum
    fn select(&self, local_name: &str, candidates: &[AlbumCandidate]) -> AlbumSelection;
}

pub trait TrackSelector {
    // (localFile, trackList) -> finalTrack
    fn select(&self, local: &LocalTrack, candidates: &[TrackCandidate]) -> TrackSelection;
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
        artist_name: album.artist_name,
        year: album.year,
    })
}

// Maps every local file of one album onto a track of the remote album.
//
// The remote list is asked for once and kept as a pool: a track that is taken
// leaves it, so two local files can never end up on one remote track.
//
// The pool is emptied in two passes. The first one matches whatever is alike
// enough to need no user at all, best pair first, so that an early file cannot
// take a track that fits a later one better. Only what is left over is put to
// the user, one file at a time, against what is left of the pool.
//
// 'Ok(None)' means the user cancelled: nothing at all is to be installed.
// A skipped file is simply absent from the result.
pub fn resolve_album_track_mapping(
    album_id: &str,
    locals: &[LocalTrack],
    provider: &dyn RemoteInfoProvider,
    selector: &dyn TrackSelector,
) -> Result<Option<Vec<(PathBuf, RemoteTrackTags)>>, String> {
    let mut pool = provider.album_tracks(album_id)?;
    let mut matched: Vec<Option<RemoteTrack>> = vec![None; locals.len()];

    auto_match(locals, &mut pool, &mut matched);

    for (index, local) in locals.iter().enumerate() {
        if matched[index].is_some() {
            continue;
        }

        let candidates = rank_track_candidates(&local.name, &pool);

        match selector.select(local, &candidates) {
            TrackSelection::Chosen(track) => {
                take_from_pool(&mut pool, &track.id);
                matched[index] = Some(track);
            }
            TrackSelection::Skip => {}
            TrackSelection::Cancelled => return Ok(None),
        }
    }

    let ret = locals
        .iter()
        .zip(matched)
        .filter_map(|(local, track)| {
            Some((local.path.clone(), RemoteTrackTags::of(&track?)))
        })
        .collect();

    Ok(Some(ret))
}

// Pairs up everything alike enough to need no user, the closest pair first.
// The matched tracks leave the pool.
fn auto_match(
    locals: &[LocalTrack],
    pool: &mut Vec<RemoteTrack>,
    matched: &mut [Option<RemoteTrack>],
) {
    // every pair worth taking, as (likeness, local index, pool index)
    let mut pairs: Vec<(u32, usize, usize)> = Vec::new();

    for (local_index, local) in locals.iter().enumerate() {
        for (pool_index, track) in pool.iter().enumerate() {
            let likeness = likeness_percent(&local.name, &track.name);

            if likeness >= AUTO_MATCH_LIKENESS {
                pairs.push((likeness, local_index, pool_index));
            }
        }
    }

    // the closest first; the indices only keep the order of equals steady
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut taken_tracks = vec![false; pool.len()];

    for (_, local_index, pool_index) in pairs {
        if matched[local_index].is_some() || taken_tracks[pool_index] {
            continue;
        }

        matched[local_index] = Some(pool[pool_index].clone());
        taken_tracks[pool_index] = true;
    }

    let mut index = 0;
    pool.retain(|_| {
        let keep = !taken_tracks[index];
        index += 1;
        keep
    });
}

// Drops the first track of that id, so that a pool seeded with duplicates
// still gives every local file one track of its own.
fn take_from_pool(pool: &mut Vec<RemoteTrack>, id: &str) {
    if let Some(index) = pool.iter().position(|track| track.id == id) {
        pool.remove(index);
    }
}

// Sorts the remaining tracks by likeness to one local file name, the closest
// first. Equals keep the order of the album.
pub fn rank_track_candidates(local_name: &str, pool: &[RemoteTrack]) -> Vec<TrackCandidate> {
    let mut ret: Vec<TrackCandidate> = pool
        .iter()
        .map(|track| TrackCandidate {
            likeness_percent: likeness_percent(local_name, &track.name),
            track: track.clone(),
        })
        .collect();

    ret.sort_by(|a, b| {
        b.likeness_percent
            .cmp(&a.likeness_percent)
            .then(a.track.disc_number.cmp(&b.track.disc_number))
            .then(a.track.track_number.cmp(&b.track.track_number))
    });

    ret
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
    use std::cell::RefCell;

    use super::{
        AlbumCandidate, AlbumSelection, ArtistCandidate, LocalTrack, RemoteAlbum, RemoteArtist,
        RemoteInfoProvider, RemoteTrack, Selection, TrackCandidate, TrackSelection,
    };

    // A provider seeded with every artist and album it knows about. A real call
    // filters that universe, like a remote search would.
    pub struct MockRemoteInfoProvider {
        universe: Vec<RemoteArtist>,

        // the artist id an album belongs to, beside the album itself
        albums: Vec<(String, RemoteAlbum)>,

        // the album id a track belongs to, beside the track itself
        tracks: Vec<(String, RemoteTrack)>,

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
                tracks: Vec::new(),
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

        // Seeds the tracks, as (album id, track id, name, disc, track number).
        // The count of the set is taken from the discs actually seeded.
        pub fn with_tracks(
            mut self,
            tracks: &[(&str, &str, &str, u32, u32)],
        ) -> MockRemoteInfoProvider {
            for (album_id, track_id, name, disc_number, track_number) in tracks {
                self.tracks.push((
                    album_id.to_string(),
                    RemoteTrack {
                        id: track_id.to_string(),
                        name: name.to_string(),
                        track_number: *track_number,
                        disc_number: *disc_number,
                        total_discs: 0,
                    },
                ));
            }

            self.recount_discs();
            self
        }

        // Every track of one album is told the same count of the set, the way
        // the remote source answers.
        fn recount_discs(&mut self) {
            for index in 0..self.tracks.len() {
                let (album_id, _) = &self.tracks[index];

                let total = self
                    .tracks
                    .iter()
                    .filter(|(owner, _)| owner == album_id)
                    .map(|(_, track)| track.disc_number)
                    .max()
                    .unwrap_or_default();

                self.tracks[index].1.total_discs = total;
            }
        }

        // A provider that fails every request, to test the network error path.
        pub fn failing(message: &str) -> MockRemoteInfoProvider {
            MockRemoteInfoProvider {
                universe: Vec::new(),
                albums: Vec::new(),
                tracks: Vec::new(),
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

        fn album_tracks(&self, album_id: &str) -> Result<Vec<RemoteTrack>, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            Ok(self
                .tracks
                .iter()
                .filter(|(owner, _)| owner == album_id)
                .map(|(_, track)| track.clone())
                .collect())
        }

        fn album_cover(&self, album_id: &str) -> Result<Vec<u8>, String> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }

            Err(format!("No cover for album id: {}", album_id))
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

    // The rule a mock track selector applies to every list it is given.
    pub enum MockTrackChoice {
        // Picks the head of the ranked list.
        Best,

        // Skips every file it is asked about.
        Skip,

        Cancel,
    }

    pub struct MockTrackSelector {
        choice: MockTrackChoice,

        // every local file the selector was asked about, in order
        asked: RefCell<Vec<String>>,
    }

    impl MockTrackSelector {
        pub fn new(choice: MockTrackChoice) -> MockTrackSelector {
            MockTrackSelector {
                choice,
                asked: RefCell::new(Vec::new()),
            }
        }

        // The files that needed a user, to tell an auto match from a manual one.
        pub fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    impl super::TrackSelector for MockTrackSelector {
        fn select(&self, local: &LocalTrack, candidates: &[TrackCandidate]) -> TrackSelection {
            self.asked.borrow_mut().push(local.name.clone());

            match &self.choice {
                MockTrackChoice::Skip => TrackSelection::Skip,
                MockTrackChoice::Cancel => TrackSelection::Cancelled,
                MockTrackChoice::Best => match candidates.first() {
                    Some(candidate) => TrackSelection::Chosen(candidate.track.clone()),
                    None => TrackSelection::Skip,
                },
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
            MockRemoteInfoProvider, MockTrackChoice, MockTrackSelector,
        },
        rank_album_candidates, rank_candidates, resolve_album_mapping,
        resolve_album_track_mapping, resolve_artist_mapping, resolve_remote_album_tags,
        resolve_remote_name, track_counts_match,
    };
    use super::{LocalTrack, RemoteTrackTags};
    use std::path::PathBuf;

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

    // The tracks of one album, as the mock is seeded with them.
    const ABBEY_ROAD: &[(&str, &str, &str, u32, u32)] = &[
        ("2001", "t1", "Come Together", 1, 1),
        ("2001", "t2", "Something", 1, 2),
        ("2001", "t3", "Oh! Darling", 1, 3),
    ];

    fn tracks_provider() -> MockRemoteInfoProvider {
        MockRemoteInfoProvider::new(&[("1", "Beatles")])
            .with_albums(&[("1", "2001", "Abbey Road", "1969", 3)])
            .with_tracks(ABBEY_ROAD)
    }

    fn locals(names: &[&str]) -> Vec<LocalTrack> {
        names
            .iter()
            .map(|name| LocalTrack {
                path: PathBuf::from(format!("Beatles/Abbey Road/{}.mp3", name)),
                name: name.to_string(),
            })
            .collect()
    }

    fn tags_of<'a>(
        mapping: &'a [(PathBuf, RemoteTrackTags)],
        name: &str,
    ) -> Option<&'a RemoteTrackTags> {
        let path = PathBuf::from(format!("Beatles/Abbey Road/{}.mp3", name));

        mapping
            .iter()
            .find(|(local, _)| *local == path)
            .map(|(_, tags)| tags)
    }

    #[test]
    fn names_that_agree_are_matched_without_a_user() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Together", "Something", "Oh! Darling"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        // a selector that cancels everything was never reached
        assert!(selector.asked().is_empty());
        assert_eq!(mapping.len(), 3);

        let come_together = tags_of(&mapping, "Come Together").expect("no mapping");
        assert_eq!(come_together.title, "Come Together");
        assert_eq!(come_together.track_number, "1");

        assert_eq!(tags_of(&mapping, "Something").unwrap().track_number, "2");
        assert_eq!(tags_of(&mapping, "Oh! Darling").unwrap().track_number, "3");
    }

    #[test]
    fn a_name_off_by_one_letter_still_needs_a_user() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Best);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Togethr", "Something", "Oh! Darling"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        // "Come Togethr" is 12 of 13 letters, that is 92%, under the bar
        assert_eq!(selector.asked(), vec!["Come Togethr".to_string()]);
        assert_eq!(tags_of(&mapping, "Come Togethr").unwrap().title, "Come Together");
    }

    #[test]
    fn the_case_of_a_name_is_not_what_tells_two_tracks_apart() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["come together"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        assert!(selector.asked().is_empty());
        assert_eq!(tags_of(&mapping, "come together").unwrap().track_number, "1");
    }

    #[test]
    fn one_remote_track_is_taken_by_one_local_file_alone() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Best);

        // the same file twice: only one of them can be "Come Together"
        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Together", "Come Together (copy)"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        assert_eq!(selector.asked(), vec!["Come Together (copy)".to_string()]);

        let first = tags_of(&mapping, "Come Together").unwrap();
        let second = tags_of(&mapping, "Come Together (copy)").unwrap();

        assert_eq!(first.track_number, "1");
        assert_ne!(second.track_number, "1");
    }

    #[test]
    fn the_closest_pair_is_matched_first_whatever_the_file_order() {
        let provider = MockRemoteInfoProvider::new(&[("1", "Beatles")])
            .with_albums(&[("1", "2001", "Abbey Road", "1969", 2)])
            .with_tracks(&[
                ("2001", "t1", "Sun", 1, 1),
                ("2001", "t2", "Sun King", 1, 2),
            ]);
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        // "Sun King" leads, and "Sun" is close enough to it to be taken first
        // by a greedy pass; only the closest pair first gets both right
        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Sun King", "Sun"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        assert!(selector.asked().is_empty());
        assert_eq!(tags_of(&mapping, "Sun King").unwrap().track_number, "2");
        assert_eq!(tags_of(&mapping, "Sun").unwrap().track_number, "1");
    }

    #[test]
    fn a_skipped_file_is_left_out_of_the_mapping() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Skip);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Together", "Nothing Like It"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        assert_eq!(selector.asked(), vec!["Nothing Like It".to_string()]);
        assert_eq!(mapping.len(), 1);
        assert!(tags_of(&mapping, "Come Together").is_some());
        assert!(tags_of(&mapping, "Nothing Like It").is_none());
    }

    #[test]
    fn a_cancel_drops_the_whole_install() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Together", "Nothing Like It"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed");

        // the file matched on its own is dropped as well
        assert_eq!(mapping, None);
    }

    #[test]
    fn more_local_files_than_remote_tracks_empty_the_pool() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Best);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Come Together", "Something", "Oh! Darling", "A Hidden One"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        // nothing is left to offer, so the extra file can only be skipped
        assert_eq!(selector.asked(), vec!["A Hidden One".to_string()]);
        assert_eq!(mapping.len(), 3);
        assert!(tags_of(&mapping, "A Hidden One").is_none());
    }

    #[test]
    fn a_set_of_several_discs_carries_its_numbers() {
        let provider = MockRemoteInfoProvider::new(&[("1", "Beatles")])
            .with_albums(&[("1", "2001", "The White Album", "1968", 4)])
            .with_tracks(&[
                ("2001", "t1", "Back in the U.S.S.R.", 1, 1),
                ("2001", "t2", "Birthday", 2, 1),
            ]);
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        let mapping = resolve_album_track_mapping(
            "2001",
            &locals(&["Back in the U.S.S.R.", "Birthday"]),
            &provider,
            &selector,
        )
        .expect("the mapping failed")
        .expect("the mapping was cancelled");

        let first = tags_of(&mapping, "Back in the U.S.S.R.").unwrap();
        let second = tags_of(&mapping, "Birthday").unwrap();

        // both are track one, and only the disc tells them apart
        assert_eq!(first.track_number, "1");
        assert_eq!(first.disc_number, "1");
        assert_eq!(first.total_discs, "2");

        assert_eq!(second.track_number, "1");
        assert_eq!(second.disc_number, "2");
        assert_eq!(second.total_discs, "2");
    }

    #[test]
    fn a_single_disc_leaves_the_set_empty() {
        let provider = MockRemoteInfoProvider::new(&[("1", "Beatles")])
            .with_albums(&[("1", "2001", "Abbey Road", "1969", 1)])
            .with_tracks(&[("2001", "t1", "Come Together", 0, 1)]);
        let selector = MockTrackSelector::new(MockTrackChoice::Cancel);

        let mapping =
            resolve_album_track_mapping("2001", &locals(&["Come Together"]), &provider, &selector)
                .expect("the mapping failed")
                .expect("the mapping was cancelled");

        let tags = tags_of(&mapping, "Come Together").unwrap();
        assert_eq!(tags.disc_number, "");
        assert_eq!(tags.total_discs, "");
    }

    #[test]
    fn a_failing_source_maps_nothing() {
        let provider = MockRemoteInfoProvider::failing("no network");
        let selector = MockTrackSelector::new(MockTrackChoice::Best);

        let result =
            resolve_album_track_mapping("2001", &locals(&["Come Together"]), &provider, &selector);

        assert_eq!(result, Err("no network".to_string()));
        assert!(selector.asked().is_empty());
    }

    #[test]
    fn an_album_of_no_local_files_maps_nothing() {
        let provider = tracks_provider();
        let selector = MockTrackSelector::new(MockTrackChoice::Best);

        let mapping = resolve_album_track_mapping("2001", &[], &provider, &selector)
            .expect("the mapping failed")
            .expect("the mapping was cancelled");

        assert!(mapping.is_empty());
        assert!(selector.asked().is_empty());
    }

    #[test]
    fn the_album_tags_carry_the_remote_artist() {
        let provider = tracks_provider();
        let meta = super::RemoteAlbumMeta::Mapped {
            id: "2001".to_string(),
            name: None,
        };

        let tags = resolve_remote_album_tags(&meta, &provider).expect("lookup failed");

        assert_eq!(tags.name, "Abbey Road");
        assert_eq!(tags.artist_name, "Beatles");
        assert_eq!(tags.year, "1969");
    }
}
