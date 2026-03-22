use crate::tags::tags::Tags;
use crate::{FsEntry, Problem, SyncIntention, SyncedEntry};
use audiotags::{AudioTagEdit, AudioTagWrite, Id3v2Tag, Tag};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;

pub trait FsEntryTrait {
    fn find_problems(&self) -> BTreeSet<Problem>;
    fn has_problems(&self) -> bool;
    fn set_sync_info(&mut self, intention: SyncedEntry);
    fn sync_info(&self) -> SyncedEntry;
}

#[derive(Debug, Clone)]
pub struct InvalidFile {
    // pub base_path: PathBuf,
    pub relative_path: PathBuf,
}
impl InvalidFile {
    pub fn from(_base: &PathBuf, relative: &PathBuf) -> InvalidFile {
        let ret = InvalidFile {
            // base_path: base.clone(),
            relative_path: relative.clone(),
        };
        ret
    }
}

impl FsEntryTrait for InvalidFile {
    fn find_problems(&self) -> BTreeSet<Problem> {
        return [Problem::InvalidFile].into_iter().collect();
    }
    fn has_problems(&self) -> bool {
        return true;
    }
    fn sync_info(&self) -> SyncedEntry {
        return SyncedEntry {
            intention: SyncIntention::DropSync,
            synced: false,
        };
    }
    fn set_sync_info(&mut self, _: SyncedEntry) {
        // do nothing
    }
}

fn is_music_file(relative: &PathBuf) -> bool {
    let music_file_extensions = HashSet::from(["mp3", "wav", "flac"]);

    let ext = match relative.extension() {
        None => return false,
        Some(e) => e,
    }
    .to_string_lossy()
    .into_owned()
    .to_lowercase();

    music_file_extensions.contains(ext.as_str())
}

#[derive(Debug, Clone)]
pub struct MusicFile {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,

    pub sync_data: SyncedEntry,

    has_problems: bool,
}
impl MusicFile {
    pub fn from(base: &PathBuf, relative: &PathBuf, intention: SyncIntention) -> Option<MusicFile> {
        if !is_music_file(relative) {
            return None;
        }

        let mut ret = MusicFile {
            base_path: base.clone(),
            relative_path: relative.clone(),
            sync_data: SyncedEntry {
                intention: intention,
                synced: false,
            },
            has_problems: true,
        };

        let problems = ret.find_problems();
        ret.has_problems = !problems.is_empty();

        Some(ret)
    }

    pub fn tag_available(&self) -> bool {
        let mut full_path = self.base_path.clone();
        full_path.push(&self.relative_path);
        let tag = Tag::new().read_from_path(full_path);
        return tag.is_ok();
    }

    pub fn empty_tags() -> Tags {
        Tags {
            title: String::new(),
            album: String::new(),
            album_artist: String::new(),
            artist: String::new(),
            track_number: String::new(),
        }
    }

    // [01] Abcedgh.mp3
    // [10] SongName.mp3
    fn title_and_number_from_file_stem(rel_path: &PathBuf) -> Option<(String, Option<u32>)> {
        let stem = rel_path.file_stem();

        let stem = if stem.is_some() {
            String::from(stem.unwrap().to_string_lossy())
        } else {
            return None;
        };

        let rest = match stem.strip_prefix('[') {
            Some(r) => r,
            None => return Some((stem.to_string(), None)),
        };

        let end_bracket_pos = match rest.find(']') {
            Some(pos) => pos,
            None => return Some((stem.to_string(), None)),
        };

        let (num_str, remaining) = rest.split_at(end_bracket_pos);

        let number = match num_str.parse::<u32>() {
            Ok(n) => n,
            Err(_) => return Some((stem.to_string(), None)),
        };

        // Safe: remaining starts with ']'
        let title = remaining[1..].trim_start();

        return Some((title.to_string(), Some(number)));
    }

    fn title_and_number_to_file_stem(title: &str, number: Option<u32>) -> String {
        if let Some(num) = number {
            return format!("[{:02}] {}", num, title);
        }

        title.to_string()
    }

    pub fn compose_tags_from_path(&self) -> Tags {
        let mut ret = Self::empty_tags();
        let track_and_number_opt = Self::title_and_number_from_file_stem(&self.relative_path);

        if track_and_number_opt.is_some() {
            let (title, num) = track_and_number_opt.unwrap();
            ret.title = title;
            if let Some(num) = num {
                ret.track_number = num.to_string();
            }
        }

        if let Some(album_path) = self.relative_path.parent() {
            if let Some(album_dir) = album_path.file_name() {
                ret.album = album_dir.to_str().unwrap().to_string();
            }

            if let Some(artist_path) = album_path.parent() {
                if let Some(artist_dir) = artist_path.file_name() {
                    ret.artist = artist_dir.to_str().unwrap().to_string();
                }
            }
        }

        ret.remove_slashes();
        ret.remove_invalid_symbols();

        return ret;
    }

    pub fn compose_path_from_tags(&self, input_tags: &Tags) -> PathBuf {
        let tags = {
            let mut t = input_tags.clone();
            t.remove_null_bytes();
            t.remove_invalid_symbols();
            t
        };

        let ext = self.relative_path.extension().unwrap().to_str().unwrap();
        let mut ret = PathBuf::new();

        ret.push(&tags.artist);
        // it's a path, no need to push album artist
        ret.push(&tags.album);

        let file_stem =
            Self::title_and_number_to_file_stem(&tags.title, tags.track_number.parse::<u32>().ok());
        ret.push(file_stem + "." + ext);

        return ret;
    }

    pub fn tags(&self) -> Tags {
        let mut full_path = self.base_path.clone();
        full_path.push(&self.relative_path);
        if full_path.exists() == false {
            panic!("Tags: File does not exist: {}", full_path.display());
        }
        let tag = Tag::new().read_from_path(&full_path);
        let tag = match tag {
            Ok(t) => t,
            Err(err) => {
                println!(
                    "Tags: Error reading tags of a file: {}: {}",
                    full_path.display(),
                    err
                );
                return Self::empty_tags();
            }
        };
        let mut ret = Tags {
            title: String::new(),
            album: String::new(),
            album_artist: String::new(),
            artist: String::new(),
            track_number: String::new(),
        };

        if let Some(title) = tag.title() {
            ret.title = title.to_string();
        }

        if let Some(album) = tag.album_title() {
            ret.album = album.to_string();
        }

        if let Some(artist) = tag.artist() {
            ret.artist = artist.to_string();
        }

        if let Some(album_artist) = tag.album_artist() {
            ret.album_artist = album_artist.to_string();
        }

        if let Some(track_number) = tag.track_number() {
            ret.track_number = track_number.to_string();
        }

        ret.remove_slashes();
        ret.remove_invalid_symbols();
        return ret;
    }

    pub fn set_tags(&self, tags: &Tags) {
        let mut full_path = self.base_path.clone();
        full_path.push(&self.relative_path);

        let mut tag = Box::new(Id3v2Tag::new());

        tag.remove_album_artist();
        tag.set_title(&tags.title.as_str());
        tag.set_album_title(&tags.album.as_str());
        tag.set_artist(&tags.artist.as_str());
        if let Ok(tn) = tags.track_number.parse::<u16>() {
            tag.set_track_number(tn);
        } else {
            println!(
                "WARN Failed to parse track number: {}, value: '{}'",
                self.relative_path.display(),
                tags.track_number
            );
        }
        tag.write_to_path(full_path.to_str().unwrap())
            .expect(format!("ERR Fail to save tags to {:?}", full_path).as_str());
    }

    pub fn remove_tags(&self) {
        let et = Self::empty_tags();
        self.set_tags(&et);
    }
}

impl FsEntryTrait for MusicFile {
    fn set_sync_info(&mut self, s_data: SyncedEntry) {
        self.sync_data = s_data;
    }

    fn has_problems(&self) -> bool {
        return self.has_problems;
    }

    fn find_problems(&self) -> BTreeSet<Problem> {
        let mut ret = BTreeSet::<Problem>::new();

        if !self.tag_available() {
            ret.insert(Problem::MissingTags);
        }

        let installed_tags = self.tags();
        let mut path_tags = self.compose_tags_from_path();
        path_tags.track_number = installed_tags.track_number.clone();

        if installed_tags.track_number.is_empty() {
            ret.insert(Problem::MissingTrackNumber);
        }

        if path_tags != installed_tags {
            ret.insert(Problem::MismatchedTags);
        }

        let tags_path = self.compose_path_from_tags(&installed_tags);

        if self.relative_path != tags_path {
            ret.insert(Problem::MismatchedPath);
        }

        if has_invalid_chars_in_path(&self.relative_path) {
            ret.insert(Problem::InvalidCharacters);
        }

        return ret;
    }

    fn sync_info(&self) -> SyncedEntry {
        return self.sync_data.clone();
    }
}

#[derive(Debug, Clone)]
pub struct Directory {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,
    pub children: BTreeMap<PathBuf, FsEntry>, // filename(not path) to object
    pub expanded: bool,                       // TODO this is from view, not from model, move it
}
impl Directory {
    pub fn new(
        base: &PathBuf,
        relative_path: &PathBuf,
        children: BTreeMap<PathBuf, FsEntry>,
    ) -> Directory {
        let ret = Directory {
            base_path: base.clone(),
            relative_path: relative_path.clone(),
            children: children,
            expanded: false,
        };
        ret
    }

    pub fn children_recursive(&self) -> Vec<PathBuf> {
        let mut ret = Vec::<PathBuf>::new();

        for (_, child) in &self.children {
            match child {
                FsEntry::FsDirectory(d) => {
                    ret.extend(d.children_recursive());
                }
                FsEntry::FsMusicFile(mf) => {
                    ret.push(mf.relative_path.clone());
                }
                _ => {}
            }
        }

        return ret;
    }

    pub fn intention(&self) -> SyncIntention {
        self.children
            .iter()
            .map(|(_, c)| match c {
                FsEntry::FsFile(_) => SyncIntention::DropSync,
                FsEntry::FsMusicFile(mf) => mf.sync_data.intention.clone(),
                FsEntry::FsDirectory(d) => d.sync_info().intention,
            })
            .reduce(|acc, v| match (acc, v) {
                (SyncIntention::Unspecified, _) => SyncIntention::Unspecified,
                (_, SyncIntention::Unspecified) => SyncIntention::Unspecified,

                (SyncIntention::ForceSync, SyncIntention::DropSync) => SyncIntention::MixedDir,
                (SyncIntention::ForceSync, SyncIntention::KeepSync) => SyncIntention::ForceSync,
                (SyncIntention::ForceSync, SyncIntention::ForceSync) => SyncIntention::ForceSync,

                (SyncIntention::KeepSync, SyncIntention::KeepSync) => SyncIntention::KeepSync,
                (SyncIntention::KeepSync, SyncIntention::DropSync) => SyncIntention::MixedDir,
                (SyncIntention::KeepSync, SyncIntention::ForceSync) => SyncIntention::ForceSync,

                (SyncIntention::DropSync, SyncIntention::KeepSync) => SyncIntention::MixedDir,
                (SyncIntention::DropSync, SyncIntention::DropSync) => SyncIntention::DropSync,
                (SyncIntention::DropSync, SyncIntention::ForceSync) => SyncIntention::MixedDir,

                (_, SyncIntention::MixedDir) => SyncIntention::MixedDir,
                (SyncIntention::MixedDir, _) => SyncIntention::MixedDir,
            })
            .unwrap_or(SyncIntention::Unspecified)
    }

    pub fn synced(&self) -> bool {
        self.children
            .iter()
            .map(|(_, c)| match c {
                FsEntry::FsFile(_) => false,
                FsEntry::FsMusicFile(mf) => mf.sync_data.synced,
                FsEntry::FsDirectory(d) => d.sync_info().synced,
            })
            .all(|v| v)
    }
}

impl FsEntryTrait for Directory {
    fn set_sync_info(&mut self, s_data: SyncedEntry) {
        for (_, child) in &mut self.children {
            match child {
                FsEntry::FsFile(f) => f.set_sync_info(s_data.clone()),
                FsEntry::FsDirectory(d) => d.set_sync_info(s_data.clone()),
                FsEntry::FsMusicFile(mf) => mf.set_sync_info(s_data.clone()),
            }
        }
    }

    fn has_problems(&self) -> bool {
        for (_, child) in &self.children {
            match child {
                FsEntry::FsFile(_) => {
                    return true;
                }
                FsEntry::FsDirectory(d) => {
                    // TODO merge those criterias
                    if d.has_problems() {
                        return true;
                    }
                }
                FsEntry::FsMusicFile(mf) => {
                    if mf.has_problems() {
                        return true;
                    }
                }
            }
        }

        return false;
    }

    fn find_problems(&self) -> BTreeSet<Problem> {
        let mut ret = BTreeSet::<Problem>::new();

        if self.children.is_empty() {
            ret.insert(Problem::EmptyDirectory);
            return ret;
        }

        let mut first = true;

        for (_, e) in &self.children {
            let problems = match e {
                FsEntry::FsDirectory(d) => d.find_problems(),
                FsEntry::FsFile(f) => f.find_problems(),
                FsEntry::FsMusicFile(mf) => mf.find_problems(),
            };

            if first {
                first = false;
                ret.extend(problems);
                continue;
            }

            ret = ret.intersection(&problems).cloned().collect();
        }

        return ret;
    }

    fn sync_info(&self) -> SyncedEntry {
        let intention = self.intention();
        let synced = self.synced();

        return SyncedEntry { intention, synced };
    }
}
fn has_invalid_chars_in_path(rel_path: &PathBuf) -> bool {
    let str = rel_path.to_string_lossy().into_owned();

    crate::tags::tags::INVALID_CHARS
        .iter()
        .any(|c| str.contains(*c))
}
