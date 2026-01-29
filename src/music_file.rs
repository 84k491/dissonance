use crate::tags::tags::Tags;
use crate::{FsEntry, Problem};
use audiotags::{Id3v2Tag, Tag};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

pub trait FsEntryTrait {
    fn find_problems(&self) -> BTreeSet<Problem>;
    fn has_problems(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct InvalidFile {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,
}
impl InvalidFile {
    pub fn new(base: &PathBuf, relative: &PathBuf) -> InvalidFile {
        let ret = InvalidFile {
            base_path: base.clone(),
            relative_path: relative.clone(),
        };
        ret
    }

    pub fn is_music_file(&self) -> bool {
        let music_file_extensions = HashSet::from(["mp3", "wav", "flac"]);

        let ext = match self.relative_path.extension() {
            None => return false,
            Some(e) => e,
        }
        .to_string_lossy()
        .into_owned()
        .to_lowercase();

        return music_file_extensions.contains(ext.as_str());
    }
}

impl FsEntryTrait for InvalidFile {
    fn find_problems(&self) -> BTreeSet<Problem> {
        return [Problem::InvalidFile].into_iter().collect();
    }
    fn has_problems(&self) -> bool {
        return true;
    }
}

#[derive(Debug, Clone)]
pub struct MusicFile {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,

    has_problems: bool,
}
impl MusicFile {
    pub fn from(simple_file: InvalidFile) -> Option<MusicFile> {
        // TODO use path as arg
        if !simple_file.is_music_file() {
            return None;
        }

        let mut ret = MusicFile {
            base_path: simple_file.base_path.clone(),
            relative_path: simple_file.relative_path.clone(),
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

    pub fn compose_tags_from_path(&self) -> Tags {
        let mut ret = Self::empty_tags();
        let stem = self.relative_path.file_stem();
        if let Some(title) = stem {
            ret.title = title.to_str().unwrap().to_string();
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
        ret.push(tags.title.clone() + "." + ext);
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

        let tag = Tag::new().read_from_path(&full_path);
        let mut tag = match tag {
            Ok(t) => t,
            Err(_) => Box::new(Id3v2Tag::new()),
        };

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
            .expect(format!("ERR Fail to save to {:?}", full_path).as_str());
    }

    // pub fn remove_tags(&self) {
    //     let mut full_path = self.base_path.clone();
    //     full_path.push(&self.relative_path);
    //     let mut tag = Tag::new().read_from_path(full_path).unwrap();
    //
    //     tag.remove_title();
    //     tag.remove_album_title();
    //     tag.remove_artist();
    //     tag.remove_album_artist();
    //     tag.remove_track_number();
    // }
}

impl FsEntryTrait for MusicFile {
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
}

#[derive(Debug, Clone)]
pub struct Directory {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,
    pub children: Vec<FsEntry>,
    pub expanded: bool, // TODO this is from view, not from model, move it
}
impl Directory {
    pub fn new(base: &PathBuf, relative_path: &PathBuf, children: Vec<FsEntry>) -> Directory {
        let ret = Directory {
            base_path: base.clone(),
            relative_path: relative_path.clone(),
            children: children,
            expanded: false,
        };
        ret
    }
}

impl FsEntryTrait for Directory {
    fn has_problems(&self) -> bool {
        for child in &self.children {
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

        for e in &self.children {
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
}
fn has_invalid_chars_in_path(rel_path: &PathBuf) -> bool {
    let str = rel_path.to_string_lossy().into_owned();

    crate::tags::tags::INVALID_CHARS
        .iter()
        .any(|c| str.contains(*c))
}
