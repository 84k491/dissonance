// The only place that knows which remote tag source is used, and only in urls.

use crate::remote_tags::{RemoteAlbum, RemoteArtist, RemoteInfoProvider};
use serde::Deserialize;
use std::time::Duration;

static SEARCH_URL: &str = "https://itunes.apple.com/search";
static LOOKUP_URL: &str = "https://itunes.apple.com/lookup";

static SEARCH_LIMIT: &str = "25";

// The listing of one artist's albums, capped at what the source allows.
static ALBUM_LIMIT: &str = "200";
static REQUEST_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Deserialize)]
struct RemoteResponse {
    results: Vec<RemoteResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteResult {
    artist_id: Option<i64>,
    artist_name: Option<String>,

    // a collection is an album; a response can hold both kinds at once
    collection_id: Option<i64>,
    collection_name: Option<String>,
    track_count: Option<u32>,
    release_date: Option<String>,
}

impl RemoteResult {
    fn into_artist(self) -> Option<RemoteArtist> {
        Some(RemoteArtist {
            id: self.artist_id?.to_string(),
            name: self.artist_name?,
        })
    }

    // 'None' for anything that is not a collection: the album listing of an
    // artist leads with the artist record itself, which has no collection id.
    fn into_album(self) -> Option<RemoteAlbum> {
        Some(RemoteAlbum {
            id: self.collection_id?.to_string(),
            name: self.collection_name?,
            artist_name: self.artist_name.unwrap_or_default(),
            year: Self::year_of(self.release_date),

            // zero means the source told no count
            track_count: self.track_count.unwrap_or_default(),
        })
    }

    // "1969-09-26T07:00:00Z" -> "1969"
    fn year_of(release_date: Option<String>) -> String {
        let date = release_date.unwrap_or_default();

        match date.char_indices().nth(4) {
            Some((end, _)) => date[..end].to_string(),
            None => String::new(),
        }
    }
}

pub struct HttpRemoteInfoProvider {
    client: reqwest::blocking::Client,
}

impl HttpRemoteInfoProvider {
    pub fn new() -> HttpRemoteInfoProvider {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("Failed to build a http client");

        HttpRemoteInfoProvider { client }
    }

    fn request(&self, url: &str, query: &[(&str, &str)]) -> Result<Vec<RemoteResult>, String> {
        let response = self
            .client
            .get(url)
            .query(query)
            .send()
            .map_err(|e| format!("Remote request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Remote request failed: {}", response.status()));
        }

        let parsed: RemoteResponse = response
            .json()
            .map_err(|e| format!("Failed to parse the remote response: {}", e))?;

        Ok(parsed.results)
    }

    fn request_artists(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<RemoteArtist>, String> {
        Ok(self
            .request(url, query)?
            .into_iter()
            .filter_map(RemoteResult::into_artist)
            .collect())
    }

    fn request_albums(&self, url: &str, query: &[(&str, &str)]) -> Result<Vec<RemoteAlbum>, String> {
        Ok(self
            .request(url, query)?
            .into_iter()
            .filter_map(RemoteResult::into_album)
            .collect())
    }
}

impl RemoteInfoProvider for HttpRemoteInfoProvider {
    fn search_artists(&self, local_name: &str) -> Result<Vec<RemoteArtist>, String> {
        self.request_artists(
            SEARCH_URL,
            &[
                ("term", local_name),
                ("entity", "musicArtist"),
                ("limit", SEARCH_LIMIT),
            ],
        )
    }

    fn artist_by_id(&self, id: &str) -> Result<RemoteArtist, String> {
        self.request_artists(LOOKUP_URL, &[("id", id)])?
            .into_iter()
            .next()
            .ok_or(format!("No remote artist with id {}", id))
    }

    // The albums of one artist, asked for by the artist's own id.
    fn search_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String> {
        self.request_albums(
            LOOKUP_URL,
            &[
                ("id", artist_id),
                ("entity", "album"),
                ("limit", ALBUM_LIMIT),
            ],
        )
    }

    fn album_by_id(&self, id: &str) -> Result<RemoteAlbum, String> {
        self.request_albums(LOOKUP_URL, &[("id", id)])?
            .into_iter()
            .next()
            .ok_or(format!("No remote album with id {}", id))
    }
}

#[cfg(test)]
mod tests {
    use super::{HttpRemoteInfoProvider, RemoteResult};
    use crate::remote_tags::RemoteInfoProvider;

    #[test]
    fn a_year_is_the_head_of_a_release_date() {
        assert_eq!(
            RemoteResult::year_of(Some("1969-09-26T07:00:00Z".to_string())),
            "1969"
        );
        assert_eq!(RemoteResult::year_of(None), "");
        assert_eq!(RemoteResult::year_of(Some("196".to_string())), "");
    }

    // Goes over the net, so it stays out of a normal run:
    //   cargo test -- --ignored
    #[test]
    #[ignore]
    fn the_remote_source_answers_a_search_and_a_lookup() {
        let provider = HttpRemoteInfoProvider::new();

        let found = provider
            .search_artists("Jack Johnson")
            .expect("search failed");
        assert!(!found.is_empty());

        let first = found.first().expect("no artists found").clone();
        let by_id = provider.artist_by_id(&first.id).expect("lookup failed");

        assert_eq!(by_id, first);
    }

    // Goes over the net as well:
    //   cargo test -- --ignored
    #[test]
    #[ignore]
    fn the_remote_source_answers_an_album_listing_and_a_lookup() {
        let provider = HttpRemoteInfoProvider::new();

        // the album listing of an artist leads with the artist record, which
        // must not turn up among the albums
        let albums = provider.search_albums("136975").expect("listing failed");
        assert!(!albums.is_empty());
        assert!(albums.iter().all(|album| !album.id.is_empty()));
        assert!(albums.iter().any(|album| album.track_count > 0));
        assert!(albums.iter().any(|album| album.year.len() == 4));

        let first = albums.first().expect("no albums found").clone();
        let by_id = provider.album_by_id(&first.id).expect("lookup failed");

        assert_eq!(by_id.id, first.id);
        assert_eq!(by_id.name, first.name);
    }
}
