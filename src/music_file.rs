use crate::tags::tags::Tags;
use crate::{Problem, SyncIntention, SyncedEntry};
use audiotags::{AudioTagEdit, AudioTagWrite, Id3v2Tag, Tag};
use std::collections::{BTreeSet, HashSet};
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

// What the brackets of a file stem hold, beside the title.
//
//   [01] Abcedgh.mp3    -> track 1, no disc
//   [2-03] SongName.mp3 -> track 3 of disc 2
//
// The disc only appears on an album that has more than one, so a stem without
// it says nothing about the disc rather than saying "disc one".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StemNumbers {
    pub title: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
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
            year: String::new(),
            has_art: false,
            disc_number: String::new(),
            total_discs: String::new(),
        }
    }

    fn title_and_number_from_file_stem(rel_path: &PathBuf) -> Option<StemNumbers> {
        let stem = rel_path.file_stem()?;

        Some(Self::parse_file_stem(&stem.to_string_lossy()))
    }

    // The title the file name tells, free of the numbers, empty when the name
    // has no stem at all.
    pub fn title_from_file_name(&self) -> String {
        Self::title_and_number_from_file_stem(&self.relative_path)
            .map(|numbers| numbers.title)
            .unwrap_or_default()
    }

    // The whole stem is the title whenever the brackets are missing or hold
    // anything but numbers.
    pub fn parse_file_stem(stem: &str) -> StemNumbers {
        let untitled = |title: &str| StemNumbers {
            title: title.to_string(),
            track_number: None,
            disc_number: None,
        };

        let rest = match stem.strip_prefix('[') {
            Some(r) => r,
            None => return untitled(stem),
        };

        let end_bracket_pos = match rest.find(']') {
            Some(pos) => pos,
            None => return untitled(stem),
        };

        let (numbers, remaining) = rest.split_at(end_bracket_pos);

        // "2-03" is a disc and a track, "03" is a track alone
        let (disc_str, num_str) = match numbers.split_once('-') {
            Some((disc, num)) => (Some(disc), num),
            None => (None, numbers),
        };

        let number = match num_str.parse::<u32>() {
            Ok(n) => n,
            Err(_) => return untitled(stem),
        };

        let disc = match disc_str {
            None => None,
            Some(disc) => match disc.parse::<u32>() {
                Ok(d) => Some(d),
                Err(_) => return untitled(stem),
            },
        };

        // Safe: remaining starts with ']'
        let title = remaining[1..].trim_start();

        StemNumbers {
            title: title.to_string(),
            track_number: Some(number),
            disc_number: disc,
        }
    }

    // [10] SongName.mp3, and [2-10] SongName.mp3 on a set of several discs.
    //
    // A single disc keeps the plain form: the disc is only worth a rename
    // where it tells two files of one directory apart.
    pub fn title_and_number_to_file_stem(
        title: &str,
        number: Option<u32>,
        disc: Option<u32>,
        total_discs: Option<u32>,
    ) -> String {
        let Some(num) = number else {
            return title.to_string();
        };

        match (disc, total_discs) {
            (Some(disc), Some(total)) if total > 1 => format!("[{}-{:02}] {}", disc, num, title),
            _ => format!("[{:02}] {}", num, title),
        }
    }

    // [1999] AlbumName
    //
    // The whole name of the directory, never its "stem": a directory has no
    // extension, and taking one would cut an album like "Vol. 3 The Subliminal
    // Verses" down to "Vol" at its last dot.
    pub fn album_and_year_from_dir_name(dir: &PathBuf) -> Option<(String, Option<u32>)> {
        let stem = String::from(dir.file_name()?.to_string_lossy());

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
        let album = remaining[1..].trim_start();

        return Some((album.to_string(), Some(number)));
    }

    fn dir_name_from_album_and_year(album: &str, year: Option<u32>) -> String {
        if let Some(year) = year {
            if year != 0 {
                return format!("[{:04}] {}", year, album);
            }
        }
        album.to_string()
    }

    pub fn compose_tags_from_path(&self) -> Tags {
        let mut ret = Self::empty_tags();
        let numbers_opt = Self::title_and_number_from_file_stem(&self.relative_path);

        if let Some(numbers) = numbers_opt {
            ret.title = numbers.title;
            if let Some(num) = numbers.track_number {
                ret.track_number = num.to_string();
            }
            if let Some(disc) = numbers.disc_number {
                ret.disc_number = disc.to_string();
            }
        }

        if let Some(album_path) = self.relative_path.parent() {
            let album_and_year_opt = Self::album_and_year_from_dir_name(&album_path.to_path_buf());

            if let Some(album_and_year) = album_and_year_opt {
                ret.album = album_and_year.0;
                if let Some(year) = album_and_year.1 {
                    ret.year = year.to_string();
                }
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
        let album_dir =
            Self::dir_name_from_album_and_year(&tags.album, tags.year.parse::<u32>().ok());
        ret.push(album_dir);

        let file_stem = Self::title_and_number_to_file_stem(
            &tags.title,
            tags.track_number.parse::<u32>().ok(),
            tags.disc_number.parse::<u32>().ok(),
            tags.total_discs.parse::<u32>().ok(),
        );
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
            year: String::new(),
            has_art: false,
            disc_number: String::new(),
            total_discs: String::new(),
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

        if let Some(disc_number) = tag.disc_number() {
            ret.disc_number = disc_number.to_string();
        }

        if let Some(total_discs) = tag.total_discs() {
            ret.total_discs = total_discs.to_string();
        }

        if let Some(year) = tag.year() {
            ret.year = year.to_string();
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

        // a file that says nothing about the set stays that way: a written
        // "1 of 1" would only be noise
        if let Ok(dn) = tags.disc_number.parse::<u16>() {
            tag.set_disc_number(dn);
        }

        if let Ok(td) = tags.total_discs.parse::<u16>() {
            tag.set_total_discs(td);
        }

        if let Ok(y) = tags.year.parse::<i32>() {
            tag.set_year(y);
        } else {
            println!(
                "WARN Failed to parse year: {}, value: '{}'",
                self.relative_path.display(),
                tags.year
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
        let name_tags = self.compose_tags_from_path();
        let mut path_tags = name_tags.clone();
        path_tags.track_number = installed_tags.track_number.clone();

        // the stem carries the disc only on a set of several discs, so it can
        // never disagree with the tags on its own; a stem that does disagree
        // shows up as a 'MismatchedPath' below
        path_tags.disc_number = installed_tags.disc_number.clone();
        path_tags.total_discs = installed_tags.total_discs.clone();

        // Both of these are about the tags alone. A path that holds no number
        // of its own is not asked to: what the tags then lack contradicts
        // nothing, and a path that does disagree is a 'MismatchedTags' or a
        // 'MismatchedPath', which is what tells the user of it.
        if installed_tags.track_number.is_empty() && !name_tags.track_number.is_empty() {
            ret.insert(Problem::MissingTrackNumberInTags);
        }

        // the year is the album directory's to carry
        if installed_tags.year.is_empty() && !name_tags.year.is_empty() {
            ret.insert(Problem::MissingYearInTags);
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

fn has_invalid_chars_in_path(rel_path: &PathBuf) -> bool {
    let str = rel_path.to_string_lossy().into_owned();

    crate::tags::tags::INVALID_CHARS
        .iter()
        .any(|c| str.contains(*c))
}

#[cfg(test)]
mod tests {
    use super::{MusicFile, StemNumbers};
    use std::path::PathBuf;

    fn numbers(title: &str, track: Option<u32>, disc: Option<u32>) -> StemNumbers {
        StemNumbers {
            title: title.to_string(),
            track_number: track,
            disc_number: disc,
        }
    }

    #[test]
    fn a_plain_stem_holds_a_track_number_alone() {
        assert_eq!(
            MusicFile::parse_file_stem("[01] Abcedgh"),
            numbers("Abcedgh", Some(1), None)
        );
        assert_eq!(
            MusicFile::parse_file_stem("[10] SongName"),
            numbers("SongName", Some(10), None)
        );
        assert_eq!(
            MusicFile::parse_file_stem("[100] SongName"),
            numbers("SongName", Some(100), None)
        );
    }

    #[test]
    fn a_dashed_stem_holds_a_disc_and_a_track() {
        assert_eq!(
            MusicFile::parse_file_stem("[2-03] SongName"),
            numbers("SongName", Some(3), Some(2))
        );
        assert_eq!(
            MusicFile::parse_file_stem("[1-01] SongName"),
            numbers("SongName", Some(1), Some(1))
        );
    }

    #[test]
    fn anything_but_numbers_in_the_brackets_is_a_title() {
        // no brackets at all
        assert_eq!(
            MusicFile::parse_file_stem("SongName"),
            numbers("SongName", None, None)
        );

        // an unclosed bracket
        assert_eq!(
            MusicFile::parse_file_stem("[01 SongName"),
            numbers("[01 SongName", None, None)
        );

        // words where the numbers belong
        assert_eq!(
            MusicFile::parse_file_stem("[live] SongName"),
            numbers("[live] SongName", None, None)
        );
        assert_eq!(
            MusicFile::parse_file_stem("[a-03] SongName"),
            numbers("[a-03] SongName", None, None)
        );
        assert_eq!(
            MusicFile::parse_file_stem("[2-b] SongName"),
            numbers("[2-b] SongName", None, None)
        );

        // a title of its own that happens to start with a bracket
        assert_eq!(
            MusicFile::parse_file_stem("[Untitled]"),
            numbers("[Untitled]", None, None)
        );
    }

    #[test]
    fn a_title_of_a_stem_is_trimmed_and_may_be_empty() {
        assert_eq!(
            MusicFile::parse_file_stem("[03]    SongName"),
            numbers("SongName", Some(3), None)
        );
        assert_eq!(MusicFile::parse_file_stem("[03]"), numbers("", Some(3), None));
    }

    #[test]
    fn a_single_disc_is_left_out_of_a_stem() {
        // nothing known about the set
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(3), None, None),
            "[03] SongName"
        );

        // one disc of one
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(3), Some(1), Some(1)),
            "[03] SongName"
        );

        // a disc without a count of the set
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(3), Some(2), None),
            "[03] SongName"
        );
    }

    #[test]
    fn a_set_of_several_discs_puts_the_disc_into_a_stem() {
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(3), Some(2), Some(2)),
            "[2-03] SongName"
        );
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(3), Some(1), Some(2)),
            "[1-03] SongName"
        );
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", Some(12), Some(10), Some(11)),
            "[10-12] SongName"
        );
    }

    #[test]
    fn a_stem_without_a_track_number_is_the_title_alone() {
        assert_eq!(
            MusicFile::title_and_number_to_file_stem("SongName", None, Some(2), Some(2)),
            "SongName"
        );
    }

    #[test]
    fn a_stem_survives_a_round_trip() {
        for (title, track, disc, total) in [
            ("SongName", Some(3), None, None),
            ("SongName", Some(3), Some(2), Some(2)),
            ("SongName", Some(100), Some(11), Some(12)),
        ] {
            let stem = MusicFile::title_and_number_to_file_stem(title, track, disc, total);
            let parsed = MusicFile::parse_file_stem(&stem);

            assert_eq!(parsed.title, title, "stem: {}", stem);
            assert_eq!(parsed.track_number, track, "stem: {}", stem);

            // the disc only makes it into the stem on a set of several
            let expected_disc = match total {
                Some(total) if total > 1 => disc,
                _ => None,
            };
            assert_eq!(parsed.disc_number, expected_disc, "stem: {}", stem);
        }
    }

    #[test]
    fn an_album_directory_holds_a_year_and_a_name() {
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from("[1999] AlbumName")),
            Some(("AlbumName".to_string(), Some(1999)))
        );
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from("AlbumName")),
            Some(("AlbumName".to_string(), None))
        );
    }

    #[test]
    fn a_dot_in_an_album_name_is_not_an_extension() {
        // a directory has no extension: cutting one off at the last dot used
        // to leave "Vol" alone, and no tag could ever agree with that
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from(
                "[2004] Vol. 3 The Subliminal Verses"
            )),
            Some(("Vol. 3 The Subliminal Verses".to_string(), Some(2004)))
        );
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from("Vol. 3")),
            Some(("Vol. 3".to_string(), None))
        );
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from("[1999] ...And Justice for All")),
            Some(("...And Justice for All".to_string(), Some(1999)))
        );
    }

    #[test]
    fn an_album_directory_of_a_whole_path_is_the_last_one() {
        assert_eq!(
            MusicFile::album_and_year_from_dir_name(&PathBuf::from(
                "Slipknot/[2004] Vol. 3 The Subliminal Verses"
            )),
            Some(("Vol. 3 The Subliminal Verses".to_string(), Some(2004)))
        );
    }

    #[test]
    fn an_album_name_survives_a_round_trip_through_a_directory_name() {
        for (album, year) in [
            ("Vol. 3 The Subliminal Verses", Some(2004)),
            ("Revolver", Some(1966)),
            ("...And Justice for All", Some(1988)),
            ("Untitled", None),
        ] {
            let dir = MusicFile::dir_name_from_album_and_year(album, year);
            let parsed = MusicFile::album_and_year_from_dir_name(&PathBuf::from(&dir));

            assert_eq!(parsed, Some((album.to_string(), year)), "dir: {}", dir);
        }
    }
}
