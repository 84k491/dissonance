// A remote catalogue of artists, albums and tracks the app can fetch tags from.
//
// The flow in 'fetch_tags' only ever talks to the types below, so switching to
// another catalogue is a matter of rewriting the three requests. Ids are opaque
// strings: every catalogue names its entries the way it wants.
//
// The catalogue behind these endpoints needs no key and is rate limited to
// roughly 20 requests per minute.

use serde::Deserialize;

static SEARCH_URL: &str = "https://itunes.apple.com/search";
static LOOKUP_URL: &str = "https://itunes.apple.com/lookup";
static LIMIT: &str = "200";

#[derive(Debug, Clone)]
pub struct RemoteArtist {
    pub id: String,
    pub name: String,

    // Anything worth showing next to the name while picking, e.g. a genre.
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct RemoteAlbum {
    pub id: String,
    pub name: String,
    pub artist_name: String,
    pub year: String,
    pub track_count: u32,
}

#[derive(Debug, Clone)]
pub struct RemoteTrack {
    pub name: String,
    pub artist_name: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub year: String,
}

// What 'fetch_tags' knows about a catalogue. Implemented over HTTP below, and
// over canned data in tests.
pub trait RemoteTagSource {
    fn search_artists(&self, name: &str) -> Result<Vec<RemoteArtist>, String>;
    fn fetch_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String>;
    fn fetch_tracks(&self, album_id: &str) -> Result<Vec<RemoteTrack>, String>;
}

pub struct HttpTagSource {}

impl HttpTagSource {
    pub fn new() -> HttpTagSource {
        HttpTagSource {}
    }
}

impl RemoteTagSource for HttpTagSource {
    fn search_artists(&self, name: &str) -> Result<Vec<RemoteArtist>, String> {
        let results = get(
            SEARCH_URL,
            &[
                ("term", name),
                ("entity", "musicArtist"),
                ("attribute", "artistTerm"),
                ("limit", LIMIT),
            ],
        )?;

        let artists = results
            .into_iter()
            .filter(|e| e.wrapper_type == "artist")
            .filter_map(|e| {
                Some(RemoteArtist {
                    id: e.artist_id?.to_string(),
                    name: e.artist_name?,
                    description: e.primary_genre_name.unwrap_or_default(),
                })
            })
            .collect();

        Ok(artists)
    }

    fn fetch_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String> {
        let results = get(
            LOOKUP_URL,
            &[("id", artist_id), ("entity", "album"), ("limit", LIMIT)],
        )?;

        let albums = results
            .into_iter()
            .filter(|e| e.wrapper_type == "collection")
            .filter_map(|e| {
                Some(RemoteAlbum {
                    id: e.collection_id?.to_string(),
                    name: e.collection_name?,
                    artist_name: e.artist_name.unwrap_or_default(),
                    year: year_from_release_date(&e.release_date),
                    track_count: e.track_count.unwrap_or(0),
                })
            })
            .collect();

        Ok(albums)
    }

    fn fetch_tracks(&self, album_id: &str) -> Result<Vec<RemoteTrack>, String> {
        let results = get(
            LOOKUP_URL,
            &[("id", album_id), ("entity", "song"), ("limit", LIMIT)],
        )?;

        let mut tracks: Vec<RemoteTrack> = results
            .into_iter()
            .filter(|e| e.wrapper_type == "track")
            .filter_map(|e| {
                Some(RemoteTrack {
                    name: e.track_name?,
                    artist_name: e.artist_name.unwrap_or_default(),
                    track_number: e.track_number.unwrap_or(0),
                    disc_number: e.disc_number.unwrap_or(1),
                    year: year_from_release_date(&e.release_date),
                })
            })
            .collect();

        tracks.sort_by_key(|t| (t.disc_number, t.track_number));

        Ok(tracks)
    }
}

// Every response is a flat list of loosely typed entries; which fields are
// present depends on 'wrapperType'.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawEntry {
    wrapper_type: String,
    artist_id: Option<u64>,
    artist_name: Option<String>,
    primary_genre_name: Option<String>,
    collection_id: Option<u64>,
    collection_name: Option<String>,
    track_count: Option<u32>,
    track_name: Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    release_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    results: Vec<RawEntry>,
}

// "1997-05-21T07:00:00Z" -> "1997"
fn year_from_release_date(release_date: &Option<String>) -> String {
    let date = match release_date {
        Some(d) => d,
        None => return String::new(),
    };

    match date.get(..4) {
        Some(year) if year.chars().all(|c| c.is_ascii_digit()) => year.to_string(),
        _ => String::new(),
    }
}

fn get(url: &str, query: &[(&str, &str)]) -> Result<Vec<RawEntry>, String> {
    let mut request = ureq::get(url);
    for (key, value) in query {
        request = request.query(*key, *value);
    }

    let mut response = request
        .call()
        .map_err(|e| format!("Remote request failed: {}", e))?;

    let body: RawResponse = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Failed to parse the remote response: {}", e))?;

    Ok(body.results)
}

#[cfg(test)]
mod tests {
    use super::year_from_release_date;

    #[test]
    fn year_is_taken_from_the_head_of_the_release_date() {
        assert_eq!(
            year_from_release_date(&Some("1997-05-21T07:00:00Z".to_string())),
            "1997"
        );
        assert_eq!(year_from_release_date(&Some("199".to_string())), "");
        assert_eq!(year_from_release_date(&Some("abcd-01".to_string())), "");
        assert_eq!(year_from_release_date(&None), "");
    }
}
