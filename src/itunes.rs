// The only place that knows which remote tag source is used, and only in urls.

use crate::remote_tags::{RemoteArtist, RemoteInfoProvider};
use serde::Deserialize;
use std::time::Duration;

static SEARCH_URL: &str = "https://itunes.apple.com/search";
static LOOKUP_URL: &str = "https://itunes.apple.com/lookup";

static SEARCH_LIMIT: &str = "25";
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
}

impl RemoteResult {
    fn into_artist(self) -> Option<RemoteArtist> {
        Some(RemoteArtist {
            id: self.artist_id?.to_string(),
            name: self.artist_name?,
        })
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

    fn request(&self, url: &str, query: &[(&str, &str)]) -> Result<Vec<RemoteArtist>, String> {
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

        Ok(parsed
            .results
            .into_iter()
            .filter_map(RemoteResult::into_artist)
            .collect())
    }
}

impl RemoteInfoProvider for HttpRemoteInfoProvider {
    fn search_artists(&self, local_name: &str) -> Result<Vec<RemoteArtist>, String> {
        self.request(
            SEARCH_URL,
            &[
                ("term", local_name),
                ("entity", "musicArtist"),
                ("limit", SEARCH_LIMIT),
            ],
        )
    }

    fn artist_by_id(&self, id: &str) -> Result<RemoteArtist, String> {
        self.request(LOOKUP_URL, &[("id", id)])?
            .into_iter()
            .next()
            .ok_or(format!("No remote artist with id {}", id))
    }
}

#[cfg(test)]
mod tests {
    use super::HttpRemoteInfoProvider;
    use crate::remote_tags::RemoteInfoProvider;

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
}
