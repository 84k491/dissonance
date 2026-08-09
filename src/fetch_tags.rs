// FetchTags: take an album directory, find the matching album in a remote tag
// source and overwrite the tags of every file in it.
//
// The whole flow runs linearly on one background thread. Whenever it needs a
// decision it can not make on its own it asks the user through a Prompt, which
// blocks until the answer arrives, so the steps read top to bottom:
//
//   local album -> remote artist -> remote album -> track mapping -> new tags
//
// An artist or a track is taken without asking when exactly one candidate is
// character equal to the local name; otherwise the user picks it from the list.
// The album is always confirmed by the user: an artist has plenty of albums
// sharing a name - editions, reissues, singles - and picking the wrong one
// mistags the whole directory.

use std::{path::PathBuf, thread};

use slint::Weak;

use crate::{
    AppWindow, Backend, Selected,
    directory::{DirectoryMetadata, FsEntry},
    music_file::MusicFile,
    picker::{Prompt, Reply, WindowPrompt},
    remote_tag_source::{HttpTagSource, RemoteAlbum, RemoteArtist, RemoteTagSource, RemoteTrack},
    tags::tags::Tags,
};

struct LocalTrack {
    rel_path: PathBuf,
    title: String,
}

struct LocalAlbum {
    artist: String,
    album: String,
    tracks: Vec<LocalTrack>,
}

/// Start the flow for the album directory at 'rel_path'. Returns immediately;
/// everything else happens on a background thread.
pub fn start(rel_path: PathBuf, ui: Weak<AppWindow>, backend: Backend, selected: Selected) {
    thread::spawn(move || run(rel_path, ui, backend, selected));
}

fn run(rel_path: PathBuf, ui: Weak<AppWindow>, backend: Backend, selected: Selected) {
    let source = HttpTagSource::new();
    let prompt = WindowPrompt::new();

    let local = match collect_local_album(&rel_path, &backend) {
        Some(local) => local,
        None => {
            prompt.notify(
                "FetchTags",
                format!("{} is not an album directory", rel_path.display()),
            );
            return;
        }
    };

    let updates = match plan(&local, &source, &prompt) {
        Some(updates) => updates,
        None => return,
    };

    let count = updates.len();
    apply(updates, ui, backend, selected);
    prompt.notify("FetchTags", format!("Retagged {} files", count));
}

/// Everything between the local album and the tags to write: the remote lookups
/// and every question the user has to answer. Gives up on 'None'.
fn plan(
    local: &LocalAlbum,
    source: &dyn RemoteTagSource,
    prompt: &dyn Prompt,
) -> Option<Vec<(PathBuf, Tags)>> {
    let artist = fetch_artist(source, prompt, &local.artist)?;
    let album = fetch_album(source, prompt, &artist, &local.album)?;

    let tracks = request(prompt, source.fetch_tracks(&album.id))?;
    if tracks.is_empty() {
        prompt.notify(
            "FetchTags",
            format!("The remote source has no tracks for '{}'", album.name),
        );
        return None;
    }

    let mapping = map_tracks(prompt, &local.tracks, tracks)?;

    let updates = mapping
        .into_iter()
        .map(|(rel_path, track)| (rel_path, compose_tags(&album, &track)))
        .collect();

    Some(updates)
}

/// Read everything the flow needs about the local album, then let go of the
/// backend: the rest of the flow talks to the network and to the user.
fn collect_local_album(rel_path: &PathBuf, backend: &Backend) -> Option<LocalAlbum> {
    let backend = backend.lock().unwrap();

    let dir = match backend.root_dir.find_recursive(rel_path) {
        Some(FsEntry::FsDirectory(d)) => d,
        _ => return None,
    };

    let meta = match dir.meta() {
        DirectoryMetadata::AlbumMeta(meta) => meta,
        _ => return None,
    };

    let tracks = dir
        .children()
        .values()
        .filter_map(|child| match child {
            FsEntry::FsMusicFile(mf) => Some(LocalTrack {
                rel_path: mf.relative_path.clone(),
                title: local_title(mf),
            }),
            _ => None,
        })
        .collect();

    Some(LocalAlbum {
        artist: meta.artist.artist.clone(),
        album: meta.album_name.clone(),
        tracks,
    })
}

// The installed title, falling back to the one the file name implies.
fn local_title(mf: &MusicFile) -> String {
    let title = mf.tags().title;
    if !title.is_empty() {
        return title;
    }

    mf.compose_tags_from_path().title
}

fn fetch_artist(
    source: &dyn RemoteTagSource,
    prompt: &dyn Prompt,
    local_artist: &str,
) -> Option<RemoteArtist> {
    let artists = request(prompt, source.search_artists(local_artist))?;

    choose(
        prompt,
        artists,
        local_artist,
        |a| a.name.clone(),
        |a| {
            if a.description.is_empty() {
                a.name.clone()
            } else {
                format!("{} ({})", a.name, a.description)
            }
        },
        "Which remote artist is it?".to_string(),
        format!("Local artist: '{}'", local_artist),
    )
}

fn fetch_album(
    source: &dyn RemoteTagSource,
    prompt: &dyn Prompt,
    artist: &RemoteArtist,
    local_album: &str,
) -> Option<RemoteAlbum> {
    let albums = request(prompt, source.fetch_albums(&artist.id))?;

    pick(
        prompt,
        albums,
        local_album,
        |a| {
            let year = if a.year.is_empty() {
                String::new()
            } else {
                format!("[{}] ", a.year)
            };
            format!("{}{} ({} tracks)", year, a.name, a.track_count)
        },
        "Which remote album is it?".to_string(),
        format!("Local album: '{}' by '{}'", local_album, artist.name),
    )
}

/// Take the single candidate that is character equal to 'query', or fall back
/// to asking the user.
fn choose<T>(
    prompt: &dyn Prompt,
    candidates: Vec<T>,
    query: &str,
    key: impl Fn(&T) -> String,
    label: impl Fn(&T) -> String,
    heading: String,
    message: String,
) -> Option<T> {
    let exact: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| key(c) == query)
        .map(|(i, _)| i)
        .collect();

    if let [only] = exact.as_slice() {
        return candidates.into_iter().nth(*only);
    }

    pick(prompt, candidates, query, label, heading, message)
}

/// Let the user pick a candidate. Skipping or cancelling gives up on the whole
/// flow: there is nothing to tag files with without an answer.
fn pick<T>(
    prompt: &dyn Prompt,
    candidates: Vec<T>,
    query: &str,
    label: impl Fn(&T) -> String,
    heading: String,
    message: String,
) -> Option<T> {
    if candidates.is_empty() {
        prompt.notify("FetchTags", format!("Nothing found for '{}'", query));
        return None;
    }

    let labels = candidates.iter().map(label).collect();

    match prompt.ask(heading, message, labels) {
        Reply::Picked(i) => candidates.into_iter().nth(i),
        Reply::Skip | Reply::Cancel => None,
    }
}

/// Pair every local file with a remote track. Titles that match character for
/// character are paired first, so the user is only asked about the leftovers,
/// and only ever sees tracks that are still free.
fn map_tracks(
    prompt: &dyn Prompt,
    local: &[LocalTrack],
    tracks: Vec<RemoteTrack>,
) -> Option<Vec<(PathBuf, RemoteTrack)>> {
    // Taken tracks are replaced with None to keep them out of the next round.
    let mut pool: Vec<Option<RemoteTrack>> = tracks.into_iter().map(Some).collect();
    let mut mapping: Vec<(PathBuf, RemoteTrack)> = Vec::new();
    let mut unmatched: Vec<&LocalTrack> = Vec::new();

    for file in local {
        let exact = pool
            .iter()
            .position(|t| matches!(t, Some(t) if t.name == file.title));

        match exact {
            Some(i) => mapping.push((file.rel_path.clone(), pool[i].take().unwrap())),
            None => unmatched.push(file),
        }
    }

    for file in unmatched {
        // Positions in the pool, so the picked label maps back to a track.
        let free: Vec<usize> = pool
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_some())
            .map(|(i, _)| i)
            .collect();

        if free.is_empty() {
            prompt.notify(
                "FetchTags",
                format!(
                    "No track left for '{}', it keeps its tags",
                    file.rel_path.display()
                ),
            );
            continue;
        }

        let labels = free
            .iter()
            .map(|i| {
                let track = pool[*i].as_ref().unwrap();
                format!("[{:02}] {}", track.track_number, track.name)
            })
            .collect();

        let reply = prompt.ask(
            "Which remote track is this file?".to_string(),
            format!("Local file: '{}'", file.rel_path.display()),
            labels,
        );

        match reply {
            Reply::Picked(i) => {
                let taken = pool[free[i]].take().unwrap();
                mapping.push((file.rel_path.clone(), taken));
            }
            // Skip leaves this one file alone, cancel drops the whole album.
            Reply::Skip => continue,
            Reply::Cancel => return None,
        }
    }

    Some(mapping)
}

fn compose_tags(album: &RemoteAlbum, track: &RemoteTrack) -> Tags {
    let artist = if track.artist_name.is_empty() {
        album.artist_name.clone()
    } else {
        track.artist_name.clone()
    };

    let year = if track.year.is_empty() {
        album.year.clone()
    } else {
        track.year.clone()
    };

    let mut tags = Tags {
        artist,
        album_artist: String::new(),
        album: album.name.clone(),
        year,
        has_art: false,
        title: track.name.clone(),
        track_number: track.track_number.to_string(),
    };

    tags.remove_slashes();
    tags.remove_invalid_symbols();
    tags
}

/// Write the fetched tags on the UI thread and repaint.
fn apply(updates: Vec<(PathBuf, Tags)>, ui: Weak<AppWindow>, backend: Backend, selected: Selected) {
    slint::invoke_from_event_loop(move || {
        let mut b = backend.lock().unwrap();
        b.apply_fetched_tags(updates);

        let s = selected.lock().unwrap();
        if let Some(app) = ui.upgrade() {
            crate::refresh(&app, &b, &s);
        }
    })
    .ok();
}

// Report a failed request and give up on the flow.
fn request<T>(prompt: &dyn Prompt, result: Result<T, String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            println!("FetchTags: {}", e);
            prompt.notify("FetchTags failed", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalAlbum, LocalTrack, plan};
    use crate::{
        picker::{Prompt, Reply},
        remote_tag_source::{RemoteAlbum, RemoteArtist, RemoteTagSource, RemoteTrack},
        tags::tags::Tags,
    };
    use std::{cell::RefCell, collections::VecDeque, path::PathBuf};

    // A catalogue that answers out of canned data instead of the network, and
    // remembers what it was asked for.
    struct FakeTagSource {
        artists: Vec<RemoteArtist>,
        albums: Vec<(String, Vec<RemoteAlbum>)>,
        tracks: Vec<(String, Vec<RemoteTrack>)>,
        requests: RefCell<Vec<String>>,
    }

    impl FakeTagSource {
        fn requests(&self) -> Vec<String> {
            self.requests.borrow().clone()
        }
    }

    impl RemoteTagSource for FakeTagSource {
        fn search_artists(&self, name: &str) -> Result<Vec<RemoteArtist>, String> {
            self.requests
                .borrow_mut()
                .push(format!("search_artists({})", name));
            Ok(self.artists.clone())
        }

        fn fetch_albums(&self, artist_id: &str) -> Result<Vec<RemoteAlbum>, String> {
            self.requests
                .borrow_mut()
                .push(format!("fetch_albums({})", artist_id));
            let found = self.albums.iter().find(|(id, _)| id == artist_id);
            Ok(found.map(|(_, a)| a.clone()).unwrap_or_default())
        }

        fn fetch_tracks(&self, album_id: &str) -> Result<Vec<RemoteTrack>, String> {
            self.requests
                .borrow_mut()
                .push(format!("fetch_tracks({})", album_id));
            let found = self.tracks.iter().find(|(id, _)| id == album_id);
            Ok(found.map(|(_, t)| t.clone()).unwrap_or_default())
        }
    }

    // A user who answers from a script instead of from a window.
    struct ScriptedPrompt {
        answers: RefCell<VecDeque<Reply>>,
        questions: RefCell<Vec<String>>,
        notes: RefCell<Vec<String>>,
    }

    impl ScriptedPrompt {
        fn new(answers: &[Reply]) -> ScriptedPrompt {
            ScriptedPrompt {
                answers: RefCell::new(answers.iter().cloned().collect()),
                questions: RefCell::new(Vec::new()),
                notes: RefCell::new(Vec::new()),
            }
        }

        fn questions(&self) -> Vec<String> {
            self.questions.borrow().clone()
        }

        fn notes(&self) -> Vec<String> {
            self.notes.borrow().clone()
        }

        fn unused_answers(&self) -> usize {
            self.answers.borrow().len()
        }
    }

    impl Prompt for ScriptedPrompt {
        fn ask(&self, heading: String, message: String, items: Vec<String>) -> Reply {
            self.questions.borrow_mut().push(format!(
                "{} {} [{}]",
                heading,
                message,
                items.join(" | ")
            ));

            self.answers
                .borrow_mut()
                .pop_front()
                .expect("the flow asked more questions than the test has answers")
        }

        fn notify(&self, heading: &str, message: String) {
            self.notes
                .borrow_mut()
                .push(format!("{}: {}", heading, message));
        }
    }

    fn artist(id: &str, name: &str, description: &str) -> RemoteArtist {
        RemoteArtist {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }

    fn album(id: &str, name: &str, year: &str, track_count: u32) -> RemoteAlbum {
        RemoteAlbum {
            id: id.to_string(),
            name: name.to_string(),
            artist_name: "Portishead".to_string(),
            year: year.to_string(),
            track_count,
        }
    }

    fn track(name: &str, track_number: u32) -> RemoteTrack {
        RemoteTrack {
            name: name.to_string(),
            artist_name: "Portishead".to_string(),
            track_number,
            disc_number: 1,
            year: "1994".to_string(),
        }
    }

    fn rel_path(file: &str) -> PathBuf {
        PathBuf::from("Portishead/[1994] Dummy").join(file)
    }

    fn local_track(file: &str, title: &str) -> LocalTrack {
        LocalTrack {
            rel_path: rel_path(file),
            title: title.to_string(),
        }
    }

    // One artist matches character for character, the album exists under two
    // editions, and of the four files two titles match a remote track, one is
    // spelled differently and one is not on the album at all.
    fn test_source() -> FakeTagSource {
        FakeTagSource {
            artists: vec![
                artist("77", "Portishead", "Electronic"),
                artist("78", "Portishead Tribute Band", "Pop"),
            ],
            albums: vec![(
                "77".to_string(),
                vec![
                    album("100", "Dummy", "1994", 11),
                    album("101", "Dummy (Remastered)", "2014", 11),
                ],
            )],
            tracks: vec![(
                "100".to_string(),
                vec![
                    track("Mysterons", 1),
                    track("Sour Times", 2),
                    track("Strangers", 3),
                    track("Glory Box", 4),
                ],
            )],
            requests: RefCell::new(Vec::new()),
        }
    }

    fn test_album() -> LocalAlbum {
        LocalAlbum {
            artist: "Portishead".to_string(),
            album: "Dummy".to_string(),
            tracks: vec![
                local_track("[01] Mysterons.mp3", "Mysterons"),
                local_track("[02] Sour Times.mp3", "Sour Times"),
                local_track("[03] Strangers.mp3", "strangers"),
                local_track("[04] Bonus.mp3", "Bonus"),
            ],
        }
    }

    fn tags_of<'a>(updates: &'a [(PathBuf, Tags)], file: &str) -> Option<&'a Tags> {
        updates
            .iter()
            .find(|(path, _)| *path == rel_path(file))
            .map(|(_, tags)| tags)
    }

    #[test]
    fn the_only_exact_artist_is_taken_and_the_album_is_always_asked_about() {
        let source = test_source();
        let prompt = ScriptedPrompt::new(&[
            // Which album: the first edition.
            Reply::Picked(0),
            // Which track is '[03] Strangers.mp3': the only one left.
            Reply::Picked(0),
            // '[04] Bonus.mp3' has no counterpart, leave it alone.
            Reply::Skip,
        ]);

        let updates = plan(&test_album(), &source, &prompt).expect("the flow gave up");

        // The artist was resolved without a question, the album never is.
        assert_eq!(prompt.questions().len(), 3);
        assert!(prompt.questions()[0].starts_with("Which remote album is it?"));
        assert!(prompt.questions()[1].starts_with("Which remote track is this file?"));
        assert!(prompt.questions()[2].starts_with("Which remote track is this file?"));
        assert_eq!(prompt.unused_answers(), 0);

        // The album picked is the one whose tracks were then asked for.
        assert_eq!(
            source.requests(),
            vec![
                "search_artists(Portishead)",
                "fetch_albums(77)",
                "fetch_tracks(100)",
            ]
        );

        // Only tracks still free are offered: the two exact matches are gone by
        // the first track question, and the one just picked by the second.
        assert!(prompt.questions()[1].ends_with("[[03] Strangers | [04] Glory Box]"));
        assert!(prompt.questions()[2].ends_with("[[04] Glory Box]"));

        assert_eq!(updates.len(), 3);

        let strangers =
            tags_of(&updates, "[03] Strangers.mp3").expect("no tags for the third file");
        assert_eq!(strangers.title, "Strangers");
        assert_eq!(strangers.track_number, "3");
        assert_eq!(strangers.album, "Dummy");
        assert_eq!(strangers.artist, "Portishead");
        assert_eq!(strangers.year, "1994");

        let sour = tags_of(&updates, "[02] Sour Times.mp3").expect("no tags for the second file");
        assert_eq!(sour.title, "Sour Times");
        assert_eq!(sour.track_number, "2");

        // The skipped file keeps whatever it had.
        assert!(tags_of(&updates, "[04] Bonus.mp3").is_none());
    }

    #[test]
    fn several_exact_artists_are_asked_about() {
        let mut source = test_source();
        source.artists = vec![
            artist("77", "Portishead", "Electronic"),
            artist("79", "Portishead", "Trip Hop"),
        ];

        let prompt = ScriptedPrompt::new(&[
            // Which artist: the second one, so its albums are fetched.
            Reply::Picked(1),
            // Nothing is found for it, so no album question follows.
        ]);

        let updates = plan(&test_album(), &source, &prompt);

        assert!(updates.is_none());
        assert_eq!(prompt.questions().len(), 1);
        assert!(prompt.questions()[0].starts_with("Which remote artist is it?"));
        assert_eq!(
            source.requests(),
            vec!["search_artists(Portishead)", "fetch_albums(79)"]
        );
        assert_eq!(prompt.notes(), vec!["FetchTags: Nothing found for 'Dummy'"]);
    }

    #[test]
    fn cancelling_the_album_leaves_the_files_alone() {
        let source = test_source();
        let prompt = ScriptedPrompt::new(&[Reply::Cancel]);

        assert!(plan(&test_album(), &source, &prompt).is_none());
        assert_eq!(prompt.questions().len(), 1);

        // Cancelled before the tracks were ever fetched.
        assert_eq!(
            source.requests(),
            vec!["search_artists(Portishead)", "fetch_albums(77)"]
        );
    }

    #[test]
    fn cancelling_a_track_drops_the_whole_album() {
        let source = test_source();
        let prompt = ScriptedPrompt::new(&[Reply::Picked(0), Reply::Cancel]);

        assert!(plan(&test_album(), &source, &prompt).is_none());
        assert_eq!(prompt.questions().len(), 2);
    }

    #[test]
    fn an_album_without_tracks_gives_up() {
        let mut source = test_source();
        source.tracks = vec![("100".to_string(), Vec::new())];

        let prompt = ScriptedPrompt::new(&[Reply::Picked(0)]);

        assert!(plan(&test_album(), &source, &prompt).is_none());
        assert_eq!(
            prompt.notes(),
            vec!["FetchTags: The remote source has no tracks for 'Dummy'"]
        );
    }

    #[test]
    fn files_left_without_a_free_track_keep_their_tags() {
        let mut source = test_source();
        source.tracks = vec![("100".to_string(), vec![track("Mysterons", 1)])];

        // Only the album question: the single track matches the first file, and
        // the other three have nothing left to be mapped to.
        let prompt = ScriptedPrompt::new(&[Reply::Picked(0)]);

        let updates = plan(&test_album(), &source, &prompt).expect("the flow gave up");

        assert_eq!(updates.len(), 1);
        assert!(tags_of(&updates, "[01] Mysterons.mp3").is_some());
        assert_eq!(prompt.notes().len(), 3);
        assert!(prompt.notes()[0].contains("No track left for"));
    }
}
