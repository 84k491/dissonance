// Mapping of local artists onto artists of a remote tag source.
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

// One option of the list shown to the user, with its likeness to the local name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistCandidate {
    pub artist: RemoteArtist,
    pub likeness_percent: u32,
}

// What the user picked in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Chosen(RemoteArtist),
    NoSuitable,
    Cancelled,
}

pub trait RemoteInfoProvider: Send + Sync {
    // (localArtist) -> artistList
    fn search_artists(&self, local_name: &str) -> Result<Vec<RemoteArtist>, String>;

    // (id) -> artist. Needed because the storage keeps the id alone.
    fn artist_by_id(&self, id: &str) -> Result<RemoteArtist, String>;
}

pub trait ArtistSelector {
    // (artistList) -> finalArtist
    fn select(&self, local_name: &str, candidates: &[ArtistCandidate]) -> Selection;
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

#[cfg(test)]
pub mod mocks {
    use super::{ArtistCandidate, RemoteArtist, RemoteInfoProvider, Selection};

    // A provider seeded with every artist it knows about. A real call filters
    // that universe, like a remote search would.
    pub struct MockRemoteInfoProvider {
        universe: Vec<RemoteArtist>,
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
                failure: None,
            }
        }

        // A provider that fails every request, to test the network error path.
        pub fn failing(message: &str) -> MockRemoteInfoProvider {
            MockRemoteInfoProvider {
                universe: Vec::new(),
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
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteArtist, RemoteArtistMappingDatabase, RemoteArtistMeta, RemoteInfoProvider, likeness_percent,
        mocks::{MockArtistSelector, MockChoice, MockRemoteInfoProvider},
        rank_candidates, resolve_artist_mapping, resolve_remote_name,
    };

    fn universe() -> MockRemoteInfoProvider {
        MockRemoteInfoProvider::new(&[
            ("123456", "The Beatles"),
            ("890123468", "Beatles Tribute Band"),
            ("555", "Jack Johnson"),
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
}
