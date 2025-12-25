pub mod music_file {
    use crate::FsEntry;
    use crate::tags::tags::Tags;
    use audiotags::Tag;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[derive(Debug, Clone)]
    pub struct File {
        pub base_path: PathBuf,
        pub relative_path: PathBuf,
    }
    impl File {
        pub fn new(base: &PathBuf, relative: &PathBuf) -> File {
            let ret = File {
                base_path: base.clone(),
                relative_path: relative.clone(),
            };
            ret
        }

        pub fn is_music_file(&self) -> bool {
            let music_file_extencions = HashSet::from(["mp3", "wav", "flac"]);

            let ext = match self.relative_path.extension() {
                None => return false,
                Some(e) => e,
            }
            .to_string_lossy()
            .into_owned();

            return music_file_extencions.contains(ext.as_str());
        }
    }

    #[derive(Debug, Clone)]
    pub struct MusicFile {
        pub base_path: PathBuf,
        pub relative_path: PathBuf,
    }
    impl MusicFile {
        pub fn from(simple_file: File) -> Option<MusicFile> {
            if !simple_file.is_music_file() {
                return None;
            }

            let ret = MusicFile {
                base_path: simple_file.base_path.clone(),
                relative_path: simple_file.relative_path.clone(),
            };
            Some(ret)
        }
        pub fn new(base: &PathBuf, relative: &PathBuf) -> Option<MusicFile> {
            let ret = MusicFile {
                base_path: base.clone(),
                relative_path: relative.clone(),
            };
            if ret.tag_available() {
                return Some(ret);
            }
            return None;
        }

        pub fn tag_available(&self) -> bool {
            let mut full_path = self.base_path.clone();
            full_path.push(&self.relative_path);
            let tag = Tag::new().read_from_path(full_path);
            return tag.is_ok();
        }

        pub fn compose_tags_from_path(&self) -> Tags {
            let mut ret = Tags {
                title: String::new(),
                album: String::new(),
                album_artist: String::new(),
                artist: String::new(),
                track_number: String::new(),
            };
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
            let tag = Tag::new().read_from_path(&full_path).unwrap();
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

        pub fn paths_match(&self) -> bool {
            let real_path = self.relative_path.clone();
            let real_path_str = real_path.to_str();
            if real_path_str.is_none() {
                return false;
            }
            let path_from_tags = self.compose_path_from_tags(&self.tags());
            let path_from_tags_str = path_from_tags.to_str();
            if path_from_tags_str.is_none() {
                return false;
            }
            return Some(real_path_str) == Some(path_from_tags_str);
        }

        pub fn set_tags(&self, tags: &Tags) {
            let mut full_path = self.base_path.clone();
            full_path.push(&self.relative_path);
            let mut tag = Tag::new().read_from_path(&full_path).unwrap();
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

        pub fn remove_tags(&self) {
            let mut full_path = self.base_path.clone();
            full_path.push(&self.relative_path);
            let mut tag = Tag::new().read_from_path(full_path).unwrap();

            tag.remove_title();
            tag.remove_album_title();
            tag.remove_artist();
            tag.remove_album_artist();
            tag.remove_track_number();
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
        pub fn new(base: &PathBuf, relative_path: PathBuf, children: Vec<FsEntry>) -> Directory {
            let ret = Directory {
                base_path: base.clone(),
                relative_path: relative_path.clone(),
                children: children,
                expanded: false,
            };
            ret
        }
    }
}
