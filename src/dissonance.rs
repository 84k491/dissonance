use serde::{Deserialize, Serialize};

use crate::directory::load_dir_hash_set_files_only;
use crate::{
    directory::{Directory, FsEntry},
    music_file::{FsEntryTrait, InvalidFile, MusicFile},
    remote_tags::{
        RemoteAlbumMappingDatabase, RemoteAlbumMeta, RemoteAlbumTags, RemoteArtistMappingDatabase,
        RemoteArtistMeta,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt::{self, Display},
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSavedState {
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
}

static CONFIG_REL_PATH: &str = ".config/dissonance";
static STATE_FILENAME: &str = "saved_state.json";
static INDEX_FILENAME: &str = "index.json";
static REMOTE_ARTIST_MAPPING_FILENAME: &str = "RemoteArtistMapping.json";
static REMOTE_ALBUM_MAPPING_FILENAME: &str = "RemoteAlbumMapping.json";

// Everything the app persists lives in one directory. Tests point
// 'DISSONANCE_CONFIG_DIR' at a directory of their own, so that they never touch
// the files of a real run.
fn config_file_path(filename: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("DISSONANCE_CONFIG_DIR") {
        return PathBuf::from(dir).join(filename);
    }

    let home = std::env::home_dir().expect("Failed to get home dir");
    home.join(PathBuf::from(CONFIG_REL_PATH)).join(filename)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    InvalidCharacters,
    EmptyDirectory,
    InvalidFile,
    MissingTags,
    MismatchedTags,
    MismatchedPath,
    MissingTrackNumber,
    MissingYear,
    MissingRemoteArtistMapping,
    MissingRemoteAlbumMapping,
}

impl Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Action {
    RemoveTags,
    FixTags,
    FixCharacters,
    MoveFile,
    ForceSync,
    KeepSync,
    DropSync,
    DeleteEntry,
    MapRemoteTagSource,
    ResetRemoteMapping,
    InstallRemoteTags,
    MapRemoteAlbumTagSource,
    ResetRemoteAlbumMapping,
    InstallRemoteAlbumTags,
}

impl Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct CopyFileAction {
    pub from_base_abs: PathBuf,
    pub to_base_abs: PathBuf,
    pub relative: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RemoveFileAction {
    pub base_abs: PathBuf,
    pub relative: PathBuf,
}

#[derive(Debug, Clone)]
pub enum FilesystemAction {
    Copy(CopyFileAction),
    Remove(RemoveFileAction),
}

#[derive(Debug, Clone)]
pub struct FilesystemActionReport {
    pub action: FilesystemAction,
    pub status: bool,
    pub iter: usize,
    pub total_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum SyncIntention {
    Unspecified,
    MixedDir,
    ForceSync,
    KeepSync,
    DropSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedEntry {
    pub intention: SyncIntention,
    pub synced: bool,
}

pub struct Dissonance {
    pub root_dir: Directory,
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub destination_files: Option<HashSet<PathBuf>>,
    pub filesystem_actions: Vec<FilesystemAction>,
}

impl Drop for Dissonance {
    fn drop(&mut self) {
        let index = self.root_dir.create_index_recursive();
        Self::save_index(index);
        self.save_remote_mapping_database();
        self.save_remote_album_mapping_database();
        save_state(self.source.clone(), self.destination.clone());
    }
}

impl Dissonance {
    pub fn new(saved_state: AppSavedState) -> Self {
        Self {
            root_dir: Directory::empty(),
            source: saved_state.source,
            destination: saved_state.destination,
            destination_files: None,
            filesystem_actions: Vec::new(),
        }
    }

    pub fn set_source(&mut self, path: PathBuf) {
        self.source = Some(path);
    }

    pub fn set_destination(&mut self, path: PathBuf) {
        self.destination = Some(path);
    }

    pub fn handle_root_loaded(&mut self, nodes: HashSet<PathBuf>) {
        let index = Self::load_index();
        let remote_mapping_database = Self::load_remote_mapping_database();
        let remote_album_mapping_database = Self::load_remote_album_mapping_database();
        self.root_dir = Directory::from_paths(nodes, self.source.clone().unwrap(), index);
        self.root_dir
            .set_remote_mapping_database(&remote_mapping_database);
        self.root_dir
            .set_remote_album_mapping_database(&remote_album_mapping_database);

        println!("Source dir loaded");

        if self.source.is_none() || self.destination.is_none() {
            return;
        }

        println!("Indexing destination files...");

        let dest_entries =
            load_dir_hash_set_files_only(self.destination.clone().unwrap(), PathBuf::new());
        self.update_index_destination(&dest_entries);
        self.destination_files = Some(dest_entries);
    }

    pub fn sync_with_destination(&mut self) {
        println!("Syncing with destination");

        let index = self.root_dir.create_index_recursive();
        let mut filesystem_actions = Vec::new();

        let unsynced: BTreeMap<PathBuf, SyncedEntry> = index
            .iter()
            .filter(|(_, e)| !e.synced)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let to_remove_from_dest = unsynced
            .iter()
            .filter(|(_, v)| {
                v.intention == SyncIntention::DropSync || v.intention == SyncIntention::ForceSync
            })
            .map(|(k, _)| {
                FilesystemAction::Remove(RemoveFileAction {
                    base_abs: self.destination.clone().unwrap(),
                    relative: k.clone(),
                })
            })
            .collect::<Vec<_>>();

        println!("Removing {} files", to_remove_from_dest.len());
        filesystem_actions.extend(to_remove_from_dest);

        let destination_extra = self
            .destination_files
            .as_ref()
            .unwrap()
            .iter()
            .filter(|path| self.root_dir.find_recursive(path).is_none())
            .map(|k| {
                FilesystemAction::Remove(RemoveFileAction {
                    base_abs: self.destination.clone().unwrap(),
                    relative: k.clone(),
                })
            })
            .collect::<Vec<_>>();

        filesystem_actions.extend(destination_extra);

        let to_copy_to_dest = unsynced
            .iter()
            .filter(|(_, v)| {
                v.intention == SyncIntention::KeepSync || v.intention == SyncIntention::ForceSync
            })
            .map(|(k, _)| {
                FilesystemAction::Copy(CopyFileAction {
                    from_base_abs: self.source.clone().unwrap(),
                    to_base_abs: self.destination.clone().unwrap(),
                    relative: k.clone(),
                })
            })
            .collect::<Vec<_>>();

        filesystem_actions.extend(to_copy_to_dest);
        self.filesystem_actions = filesystem_actions;
    }

    pub fn handle_filesystem_action_done(&mut self, report: &FilesystemActionReport) {
        if !report.status {
            println!("Filesystem action failed: {:?}", report.action);
            return;
        }

        match &report.action {
            FilesystemAction::Copy(copy_action) => {
                let dfiles = self.destination_files.as_mut().unwrap();
                dfiles.insert(copy_action.relative.clone());
                self.root_dir.set_sync_info_recursive(
                    &copy_action.relative,
                    SyncedEntry {
                        intention: SyncIntention::KeepSync,
                        synced: true,
                    },
                );
            }
            FilesystemAction::Remove(remove_action) => {
                let dfiles = self.destination_files.as_mut().unwrap();
                dfiles.remove(&remove_action.relative);
                self.root_dir.set_sync_info_recursive(
                    &remove_action.relative,
                    SyncedEntry {
                        intention: SyncIntention::DropSync,
                        synced: true,
                    },
                );
            }
        }
    }

    pub fn get_suitable_actions(&self, entry: &FsEntry) -> BTreeSet<Action> {
        match entry {
            FsEntry::FsMusicFile(mf) => self.get_suitable_actions_for_music_file(mf),
            FsEntry::FsDirectory(d) => self.get_suitable_actions_for_dir(d),
            FsEntry::FsFile(f) => self.get_suitable_actions_for_invalid_file(f),
        }
    }

    fn load_index() -> BTreeMap<PathBuf, SyncedEntry> {
        let path = config_file_path(INDEX_FILENAME);

        let file = match File::open(path) {
            Err(e) => {
                println!("Failed to open index: {}", e);
                return BTreeMap::new();
            }
            Ok(f) => f,
        };

        let map: BTreeMap<PathBuf, SyncedEntry> = match serde_json::from_reader(file) {
            Err(_) => return BTreeMap::new(),
            Ok(m) => m,
        };

        println!("Index loaded from json: {} files", map.len());
        map
    }

    fn save_index(index: BTreeMap<PathBuf, SyncedEntry>) {
        let path = config_file_path(INDEX_FILENAME);

        match fs::create_dir_all(path.parent().expect("No parent on index save")) {
            Err(e) => println!("Failed to create dir {}: {}", path.display(), e),
            Ok(_) => {}
        }

        let file = match File::create(path) {
            Err(_) => {
                println!("Failed to create index.json");
                return;
            }
            Ok(f) => BufWriter::new(f),
        };

        match serde_json::to_writer_pretty(file, &index) {
            Err(_) => println!("Failed to write to index.json"),
            Ok(_) => {}
        }

        println!("Index saved: {} files", index.len());
    }

    fn load_remote_mapping_database() -> RemoteArtistMappingDatabase {
        let path = config_file_path(REMOTE_ARTIST_MAPPING_FILENAME);

        let file = match File::open(path) {
            Err(e) => {
                println!("Failed to open the remote artist mapping: {}", e);
                return RemoteArtistMappingDatabase::new();
            }
            Ok(f) => f,
        };

        let map: RemoteArtistMappingDatabase = match serde_json::from_reader(file) {
            Err(e) => {
                println!("Failed to parse the remote artist mapping: {}", e);
                return RemoteArtistMappingDatabase::new();
            }
            Ok(m) => m,
        };

        println!(
            "Remote artist mapping loaded from json: {} artists",
            map.len()
        );
        map
    }

    fn save_remote_mapping(map: &RemoteArtistMappingDatabase) {
        let path = config_file_path(REMOTE_ARTIST_MAPPING_FILENAME);
        match fs::create_dir_all(path.parent().expect("No parent on mapping save")) {
            Err(e) => println!("Failed to create dir {}: {}", path.display(), e),
            Ok(_) => {}
        }

        let file = match File::create(&path) {
            Err(e) => {
                println!("Failed to create {}: {}", path.display(), e);
                return;
            }
            Ok(f) => BufWriter::new(f),
        };

        match serde_json::to_writer_pretty(file, map) {
            Err(e) => println!("Failed to write to {}: {}", path.display(), e),
            Ok(_) => println!("Remote artist mapping saved: {} artists", map.len()),
        }
    }

    fn load_remote_album_mapping_database() -> RemoteAlbumMappingDatabase {
        let path = config_file_path(REMOTE_ALBUM_MAPPING_FILENAME);

        let file = match File::open(path) {
            Err(e) => {
                println!("Failed to open the remote album mapping: {}", e);
                return RemoteAlbumMappingDatabase::new();
            }
            Ok(f) => f,
        };

        let map: RemoteAlbumMappingDatabase = match serde_json::from_reader(file) {
            Err(e) => {
                println!("Failed to parse the remote album mapping: {}", e);
                return RemoteAlbumMappingDatabase::new();
            }
            Ok(m) => m,
        };

        println!(
            "Remote album mapping loaded from json: {} albums",
            map.len()
        );
        map
    }

    fn save_remote_album_mapping(map: &RemoteAlbumMappingDatabase) {
        let path = config_file_path(REMOTE_ALBUM_MAPPING_FILENAME);
        match fs::create_dir_all(path.parent().expect("No parent on mapping save")) {
            Err(e) => println!("Failed to create dir {}: {}", path.display(), e),
            Ok(_) => {}
        }

        let file = match File::create(&path) {
            Err(e) => {
                println!("Failed to create {}: {}", path.display(), e);
                return;
            }
            Ok(f) => BufWriter::new(f),
        };

        match serde_json::to_writer_pretty(file, map) {
            Err(e) => println!("Failed to write to {}: {}", path.display(), e),
            Ok(_) => println!("Remote album mapping saved: {} albums", map.len()),
        }
    }

    // TODO why it is used? Avoid?
    pub fn artist_local_name(&self, rel_path: &Path) -> Option<String> {
        match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => d.mapped_artist_name().map(|name| name.to_string()),
            _ => None,
        }
    }

    pub fn remote_artist_meta(&self, rel_path: &Path) -> Option<RemoteArtistMeta> {
        match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => d.remote_artist_meta().clone(),
            _ => None,
        }
    }

    // Adopts a finished mapping request. Nothing is written to the file here:
    // the whole of it is rewritten out of the tree on closing.
    pub fn set_remote_artist_mapping(&mut self, rel_path: &Path, meta: &RemoteArtistMeta) {
        self.root_dir
            .set_remote_meta_recursive(rel_path, Some(meta.clone()));
    }

    pub fn reset_remote_artist_mapping(&mut self, rel_path: &Path) {
        self.root_dir.set_remote_meta_recursive(rel_path, None);
    }

    // Writes the mapping of every artist of the tree, replacing the file as a
    // whole. An empty tree means the source was never loaded, so there is
    // nothing to write it out of.
    fn save_remote_mapping_database(&self) {
        if self.root_dir.children().is_empty() {
            return;
        }

        Self::save_remote_mapping(&self.root_dir.create_remote_mapping_recursive());
    }

    fn save_remote_album_mapping_database(&self) {
        if self.root_dir.children().is_empty() {
            return;
        }

        Self::save_remote_album_mapping(&self.root_dir.create_remote_album_mapping_recursive());
    }

    pub fn local_album_name_and_track_count(&self, rel_path: &Path) -> Option<(String, u32)> {
        match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => {
                let (_, album) = d.mapped_album_key()?;
                Some((album.to_string(), d.track_count()))
            }
            _ => None,
        }
    }

    pub fn artist_remote_id_of_album(&self, album_rel_path: &Path) -> Option<String> {
        let artist_path = album_rel_path.parent()?;

        let artist = match self.root_dir.find_recursive(artist_path) {
            Some(FsEntry::FsDirectory(d)) if d.is_artist() => d,
            _ => return None,
        };

        match artist.remote_artist_meta() {
            Some(RemoteArtistMeta::Mapped { id, .. }) => Some(id.clone()),
            _ => None,
        }
    }

    pub fn remote_album_meta(&self, rel_path: &Path) -> Option<RemoteAlbumMeta> {
        match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => d.remote_album_meta().clone(),
            _ => None,
        }
    }

    pub fn set_remote_album_mapping(&mut self, rel_path: &Path, meta: &RemoteAlbumMeta) {
        self.root_dir
            .set_remote_album_meta_recursive(rel_path, Some(meta.clone()));
    }

    pub fn reset_remote_album_mapping(&mut self, rel_path: &Path) {
        self.root_dir
            .set_remote_album_meta_recursive(rel_path, None);
    }

    pub fn install_remote_tags(&mut self, rel_path: &Path, remote_name: &str) {
        let files = match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => d.children_music_files_recursive(),
            _ => return,
        };

        for file in files {
            let sync_data = match self.root_dir.find_recursive(&file) {
                Some(FsEntry::FsMusicFile(mf)) => mf.sync_data.clone(),
                _ => continue,
            };

            self.write_artist_tag(&file, remote_name);

            // read the file again, so that its problems and the meta of every
            // directory above it are recalculated. Only the node of the tree is
            // dropped here, never the file itself
            if !self.root_dir.remove_entry_recursive(&file) {
                println!("Failed to forget file: {}", file.display());
            }
            self.root_dir.add_entry_recursive(&file, sync_data);
        }

        // the requested name is kept on the artist itself, so that the mapping
        // written on closing holds the remote name as well as the local one
        if let Some(RemoteArtistMeta::Mapped { id, .. }) = self.remote_artist_meta(rel_path) {
            let named = RemoteArtistMeta::Mapped {
                id,
                name: Some(remote_name.to_string()),
            };
            self.root_dir
                .set_remote_meta_recursive(rel_path, Some(named));
        }
    }

    // Writes the remote album name and year into every music file below,
    // leaving the artist, the title and the track number alone. The files are
    // not moved, so a 'MismatchedPath' may appear and is fixed by 'MoveFile':
    // both halves of an album directory name are tag based.
    pub fn install_remote_album_tags(&mut self, rel_path: &Path, remote: &RemoteAlbumTags) {
        let files = match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsDirectory(d)) => d.children_music_files_recursive(),
            _ => return,
        };

        for file in files {
            let sync_data = match self.root_dir.find_recursive(&file) {
                Some(FsEntry::FsMusicFile(mf)) => mf.sync_data.clone(),
                _ => continue,
            };

            self.write_album_tags(&file, remote);

            // read the file again, so that its problems and the meta of every
            // directory above it are recalculated. Only the node of the tree is
            // dropped here, never the file itself
            if !self.root_dir.remove_entry_recursive(&file) {
                println!("Failed to forget file: {}", file.display());
            }
            self.root_dir.add_entry_recursive(&file, sync_data);
        }

        // the requested tags are kept on the album itself, so that the mapping
        // written on closing holds the remote name as well as the local one
        if let Some(RemoteAlbumMeta::Mapped { id, .. }) = self.remote_album_meta(rel_path) {
            let named = RemoteAlbumMeta::Mapped {
                id,
                name: Some(remote.name.clone()),
            };
            self.root_dir
                .set_remote_album_meta_recursive(rel_path, Some(named));
        }
    }

    // Replaces the album and the year of one music file, keeping every other
    // tag of it.
    fn write_album_tags(&self, rel_path: &Path, remote: &RemoteAlbumTags) {
        let mf = match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsMusicFile(mf)) => mf,
            _ => return,
        };

        let mut tags = mf.tags();
        tags.album = remote.name.clone();

        // an empty year means the source told none: emptying the local one
        // would only turn into a 'MissingYear' problem
        if !remote.year.is_empty() {
            tags.year = remote.year.clone();
        }

        mf.set_tags(&tags);
    }

    // Replaces the artist of one music file, keeping every other tag of it.
    fn write_artist_tag(&self, rel_path: &Path, artist: &str) {
        let mf = match self.root_dir.find_recursive(rel_path) {
            Some(FsEntry::FsMusicFile(mf)) => mf,
            _ => return,
        };

        let mut tags = mf.tags();
        tags.artist = artist.to_string();
        mf.set_tags(&tags);
    }

    pub fn set_sync_intention(&mut self, rel_path: PathBuf, intention: SyncIntention) {
        let entry = self.root_dir.find_recursive(&rel_path);
        match entry {
            Some(FsEntry::FsMusicFile(_)) => {}
            Some(FsEntry::FsDirectory(d)) => {
                let children = d.children_music_files_recursive();
                for child in children {
                    self.set_sync_intention(child.clone(), intention.clone());
                }
                return;
            }
            _ => return,
        }

        let dest_available = self.destination.is_some();
        let is_in_dest = self.destination_files.as_ref().unwrap().contains(&rel_path);

        let synced = match intention {
            SyncIntention::KeepSync => dest_available && is_in_dest,
            SyncIntention::DropSync => dest_available && !is_in_dest,
            SyncIntention::ForceSync | SyncIntention::Unspecified | SyncIntention::MixedDir => {
                false
            }
        };

        self.root_dir
            .set_sync_info_recursive(&rel_path, SyncedEntry { intention, synced });
    }

    fn update_index_destination(&mut self, dest_entries: &HashSet<PathBuf>) {
        let sync_info = self.root_dir.create_index_recursive();
        for (rel_path, entry) in sync_info.iter() {
            let is_in_dest = dest_entries.contains(rel_path);

            match entry.intention {
                SyncIntention::KeepSync => {
                    self.root_dir.set_sync_info_recursive(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::KeepSync,
                            synced: is_in_dest,
                        },
                    );
                }
                SyncIntention::DropSync => {
                    self.root_dir.set_sync_info_recursive(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::DropSync,
                            synced: !is_in_dest,
                        },
                    );
                }
                SyncIntention::Unspecified => {
                    self.root_dir.set_sync_info_recursive(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::Unspecified,
                            synced: false,
                        },
                    );
                }
                SyncIntention::ForceSync => {
                    self.root_dir.set_sync_info_recursive(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::ForceSync,
                            synced: false,
                        },
                    );
                }
                SyncIntention::MixedDir => {
                    println!(
                        "WARN: MixedDir intention for a file: {}",
                        rel_path.display()
                    );
                    self.root_dir.set_sync_info_recursive(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::Unspecified,
                            synced: false,
                        },
                    );
                }
            }
        }
    }

    fn get_suitable_actions_for_invalid_file(&self, file: &InvalidFile) -> BTreeSet<Action> {
        let mut actions = BTreeSet::new();

        for problem in file.find_problems() {
            if problem == Problem::InvalidFile {
                actions.insert(Action::DeleteEntry);
            }
        }

        actions
    }

    fn get_suitable_actions_for_music_file(&self, mf: &MusicFile) -> BTreeSet<Action> {
        let problems = mf.find_problems();
        let mut actions = BTreeSet::new();

        if problems.is_empty() {
            actions.insert(Action::RemoveTags);
        } else {
            for problem in problems {
                match problem {
                    Problem::MissingTags => {
                        actions.insert(Action::FixTags);
                    }
                    Problem::MismatchedTags | Problem::MismatchedPath => {
                        actions.insert(Action::FixTags);
                        actions.insert(Action::MoveFile);
                        actions.insert(Action::RemoveTags);
                    }
                    _ => {}
                }
            }
        }

        match mf.sync_data.intention {
            SyncIntention::KeepSync => {
                actions.insert(Action::DropSync);
                actions.insert(Action::ForceSync);
            }
            SyncIntention::ForceSync => {
                actions.insert(Action::DropSync);
            }
            SyncIntention::DropSync => {
                actions.insert(Action::KeepSync);
            }
            SyncIntention::Unspecified => {
                actions.insert(Action::KeepSync);
                actions.insert(Action::DropSync);
            }
            SyncIntention::MixedDir => {
                println!(
                    "WARN: MixedDir intention for a music file: {}",
                    mf.relative_path.display()
                );
            }
        }

        actions
    }

    fn get_suitable_actions_for_dir(&self, d: &Directory) -> BTreeSet<Action> {
        let mut actions = BTreeSet::new();

        match d.intention() {
            SyncIntention::KeepSync => {
                actions.insert(Action::DropSync);
                actions.insert(Action::ForceSync);
            }
            SyncIntention::ForceSync => {
                actions.insert(Action::DropSync);
            }
            SyncIntention::DropSync => {
                actions.insert(Action::KeepSync);
            }
            SyncIntention::Unspecified | SyncIntention::MixedDir => {
                actions.insert(Action::KeepSync);
                actions.insert(Action::DropSync);
            }
        }

        match d.remote_artist_meta() {
            None => {
                if d.is_artist() {
                    actions.insert(Action::MapRemoteTagSource);
                }
            }
            Some(RemoteArtistMeta::NoSuitable) => {
                actions.insert(Action::ResetRemoteMapping);
            }
            Some(RemoteArtistMeta::Mapped { .. }) => {
                actions.insert(Action::InstallRemoteTags);
                actions.insert(Action::ResetRemoteMapping);
            }
        }

        if d.is_album() {
            match d.remote_album_meta() {
                None => {
                    // an album is searched among the albums of a remote artist,
                    // so the artist above has to be mapped first
                    if self.artist_remote_id_of_album(&d.relative_path).is_some() {
                        actions.insert(Action::MapRemoteAlbumTagSource);
                    }
                }
                Some(RemoteAlbumMeta::NoSuitable) => {
                    actions.insert(Action::ResetRemoteAlbumMapping);
                }
                Some(RemoteAlbumMeta::Mapped { .. }) => {
                    actions.insert(Action::InstallRemoteAlbumTags);
                    actions.insert(Action::ResetRemoteAlbumMapping);
                }
            }
        }

        for problem in d.find_problems() {
            match problem {
                Problem::MissingTags => {
                    actions.insert(Action::FixTags);
                }
                Problem::MismatchedTags | Problem::MismatchedPath => {
                    actions.insert(Action::FixTags);
                    actions.insert(Action::MoveFile);
                    actions.insert(Action::RemoveTags);
                }
                Problem::MissingTrackNumber | Problem::MissingYear => {
                    actions.insert(Action::RemoveTags);
                    actions.insert(Action::FixTags);
                }
                Problem::InvalidFile | Problem::EmptyDirectory => {
                    actions.insert(Action::DeleteEntry);
                }
                Problem::InvalidCharacters => {
                    actions.insert(Action::FixCharacters);
                }
                // the mapping actions are picked by the remote meta above
                Problem::MissingRemoteArtistMapping | Problem::MissingRemoteAlbumMapping => {}
            }
        }

        if d.find_problems().is_empty() {
            actions.insert(Action::RemoveTags);
        }

        actions
    }

    pub fn fix_tags(&mut self, path: PathBuf) {
        if self.source.is_none() {
            return;
        }

        let entry = match self.root_dir.find_recursive(&path) {
            Some(entry) => entry,
            None => return,
        };

        match entry {
            FsEntry::FsMusicFile(mf) => {
                let tags = mf.compose_tags_from_path();
                mf.set_tags(&tags);

                let sync_data = mf.sync_data.clone();

                if !self.root_dir.remove_entry_recursive(&path) {
                    println!("Failed to forget file: {}", path.display());
                }
                self.root_dir.add_entry_recursive(&path, sync_data);
            }
            FsEntry::FsDirectory(d) => {
                let children: Vec<PathBuf> = d
                    .children()
                    .iter()
                    .map(|(_, entry)| entry.rel_path().clone())
                    .collect();
                for child in children {
                    self.fix_tags(child);
                }
            }
            _ => {}
        }
    }

    pub fn remove_tags(&mut self, path: PathBuf) {
        if self.source.is_none() {
            return;
        }

        match self.root_dir.find_recursive(&path) {
            Some(FsEntry::FsMusicFile(mf)) => {
                mf.remove_tags();

                let sync_data = mf.sync_data.clone();

                if !self.root_dir.remove_entry_recursive(&path) {
                    println!("Failed to forget file: {}", path.display());
                }
                self.root_dir.add_entry_recursive(&path, sync_data);
            }
            Some(FsEntry::FsDirectory(d)) => {
                let children: Vec<PathBuf> = d
                    .children()
                    .iter()
                    .map(|(_, entry)| entry.rel_path().clone())
                    .collect();
                for child in children {
                    self.remove_tags(child);
                }
            }
            _ => {}
        }
    }

    pub fn delete_entry(&mut self, rel_path: PathBuf) {
        let mut abs_path = self.source.clone().unwrap();
        abs_path.push(&rel_path);

        if abs_path.is_file() {
            match fs::remove_file(abs_path) {
                Ok(()) => {
                    self.root_dir.remove_entry_recursive(&rel_path);
                }
                Err(e) => {
                    println!("Failed to remove file {}: {}", rel_path.display(), e);
                }
            }
        }
    }

    pub fn move_entry_to_tag_based_path(&mut self, path: PathBuf) -> Option<PathBuf> {
        let (rel_path, tag_based_rel_path) = {
            let entry = self.root_dir.find_recursive(&path);
            let file = match entry {
                Some(FsEntry::FsMusicFile(mf)) => mf,
                Some(FsEntry::FsDirectory(d)) => {
                    let children: Vec<PathBuf> = d
                        .children()
                        .iter()
                        .map(|(_, entry)| entry.rel_path().clone())
                        .collect();
                    let dir_path = d.relative_path.clone();

                    for child in children {
                        self.move_entry_to_tag_based_path(child);
                    }

                    let dir = self
                        .root_dir
                        .find_recursive(&dir_path)
                        .expect("No dir on moving");
                    match dir {
                        FsEntry::FsDirectory(d) => {
                            if d.children().is_empty() {
                                self.root_dir.remove_entry_recursive(&dir_path);
                            }
                        }
                        _ => panic!("Expected dir"),
                    }

                    return None;
                }
                _ => return None,
            };

            (
                file.relative_path.clone(),
                file.compose_path_from_tags(&file.tags()),
            )
        };

        self.move_file(rel_path, tag_based_rel_path.clone());
        Some(tag_based_rel_path)
    }

    fn move_file(&mut self, from: PathBuf, to: PathBuf) {
        let mut source_full_path = self.source.clone().unwrap();
        source_full_path.push(&from);

        let mut target_full_path = self.source.clone().unwrap();
        target_full_path.push(&to);

        let _ = fs::create_dir_all(target_full_path.parent().unwrap());
        if let Err(err) = fs::rename(&source_full_path, &target_full_path) {
            println!("Moving to {:?} failed, err: {}", target_full_path, err);
            return;
        }

        self.root_dir.add_entry_recursive(
            &to,
            SyncedEntry {
                intention: SyncIntention::Unspecified,
                synced: false,
            },
        );
        if !self.root_dir.remove_entry_recursive(&from) {
            println!("Failed to forget file: {}", from.to_string_lossy());
        }

        let _ = remove_empty_subdirs::remove_empty_subdirs(&self.source.clone().unwrap());
    }

    pub fn fix_characters(&mut self, old_path: &PathBuf) -> Option<PathBuf> {
        let entry = match self.root_dir.find_recursive(old_path) {
            Some(entry) => entry.clone(),
            None => return None,
        };

        match entry {
            FsEntry::FsMusicFile(mf) => {
                let mut new_path_str = mf.relative_path.to_string_lossy().to_string();
                crate::tags::tags::INVALID_CHARS.iter().for_each(|c| {
                    new_path_str = new_path_str.replace(c, "_");
                });

                let new_path = PathBuf::from(new_path_str);
                self.move_file(old_path.clone(), new_path.clone());
                Some(new_path)
            }
            FsEntry::FsDirectory(d) => {
                let children: Vec<PathBuf> = d
                    .children()
                    .iter()
                    .map(|(_, entry)| entry.rel_path().clone())
                    .collect();

                for child in children {
                    self.fix_characters(&child);
                }

                self.root_dir
                    .remove_entry_recursive(&d.relative_path.clone());
                let _ = remove_empty_subdirs::remove_empty_subdirs(&self.source.clone().unwrap());
                None
            }
            _ => None,
        }
    }
}

pub fn process_filesystem_action(
    file: &FilesystemAction,
    iter: usize,
    total_size: usize,
) -> FilesystemActionReport {
    match file {
        FilesystemAction::Copy(copy_action) => {
            let from_abs = copy_action.from_base_abs.join(&copy_action.relative);
            let to_abs = copy_action.to_base_abs.join(&copy_action.relative);
            match fs::create_dir_all(to_abs.parent().expect("No parent on mkdir")) {
                Err(e) => println!("ERROR Failed to create dir: {} ({})", to_abs.display(), e),
                Ok(_) => {}
            }
            println!("Copying: {} to {}", from_abs.display(), to_abs.display());

            let ok = mtp_copy(&from_abs, &to_abs).is_ok();

            FilesystemActionReport {
                action: FilesystemAction::Copy(copy_action.clone()),
                status: ok,
                iter,
                total_size,
            }
        }
        FilesystemAction::Remove(remove_action) => {
            let from_abs = remove_action.base_abs.join(&remove_action.relative);
            println!("Removing: {}", from_abs.display());
            std::fs::remove_file(&from_abs).unwrap();

            FilesystemActionReport {
                action: FilesystemAction::Remove(remove_action.clone()),
                status: true,
                iter,
                total_size,
            }
        }
    }
}

pub fn load_saved_state() -> AppSavedState {
    let path = config_file_path(STATE_FILENAME);
    let file = match File::open(&path) {
        Err(e) => {
            println!("Failed to open saved state {}: {}", path.display(), e);
            return AppSavedState {
                source: None,
                destination: None,
            };
        }
        Ok(f) => f,
    };

    let state = match serde_json::from_reader(file) {
        Err(_) => {
            return AppSavedState {
                source: None,
                destination: None,
            };
        }
        Ok(m) => m,
    };

    println!("Saved state loaded from json");
    state
}

fn save_state(source: Option<PathBuf>, destination: Option<PathBuf>) {
    let path = config_file_path(STATE_FILENAME);
    match fs::create_dir_all(path.parent().expect("No parent on state save")) {
        Err(e) => println!("Failed to create dir {}: {}", path.display(), e),
        Ok(_) => {}
    }

    let file = match File::create(path) {
        Err(_) => {
            println!("Failed to create index.json");
            return;
        }
        Ok(f) => BufWriter::new(f),
    };

    let saved_state = AppSavedState {
        source,
        destination,
    };

    match serde_json::to_writer_pretty(file, &saved_state) {
        Err(_) => println!("Failed to write to index.json"),
        Ok(_) => {}
    }

    println!("State saved");
}

fn mtp_copy(src: &PathBuf, dst: &PathBuf) -> io::Result<u64> {
    let mut input = File::open(src)?;
    let mut output = match File::create(dst) {
        Err(e) => {
            println!("Failed to create output file: {}", e);
            return Err(e);
        }
        Ok(f) => f,
    };

    let mut buf = [0u8; 64 * 1024];
    let mut total = 0;

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        total += n as u64;
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::{
        Action, AppSavedState, Dissonance, FilesystemAction, Problem, SyncIntention,
        process_filesystem_action,
    };
    use crate::{
        directory::{Directory, FsEntry},
        music_file::{FsEntryTrait, MusicFile},
        remote_tags::{
            RemoteAlbumMappingDatabase, RemoteAlbumMeta, RemoteAlbumTags,
            RemoteArtistMappingDatabase, RemoteArtistMeta,
            mocks::{
                MockAlbumChoice, MockAlbumSelector, MockArtistSelector, MockChoice,
                MockRemoteInfoProvider,
            },
            resolve_album_mapping, resolve_artist_mapping, resolve_remote_album_tags,
            resolve_remote_name,
        },
    };
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
        sync::Once,
        time::{SystemTime, UNIX_EPOCH},
    };

    // Keeps every test off the config files of a real run.
    fn redirect_config_dir() {
        static ONCE: Once = Once::new();

        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join("dissonance-test-config");
            fs::create_dir_all(&dir).expect("create test config dir");

            // safety: done once, before any test reads the variable
            unsafe { std::env::set_var("DISSONANCE_CONFIG_DIR", &dir) };
        });
    }

    enum TestEntryKind {
        MusicFile,
        InvalidFile,
        EmptyDir,
    }

    struct TestEntry<'a> {
        path: &'a str,
        kind: TestEntryKind,
    }

    struct TestDirs {
        root: PathBuf,
        source: PathBuf,
        destination: PathBuf,
    }

    impl TestDirs {
        fn new(source_entries: &[TestEntry<'_>], destination_entries: &[TestEntry<'_>]) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("dissonance-backend-test-{unique}"));
            let source = root.join("source");
            let destination = root.join("destination");

            fs::create_dir_all(&source).expect("create source root");
            fs::create_dir_all(&destination).expect("create destination root");
            Self::populate(&source, source_entries);
            Self::populate(&destination, destination_entries);

            Self {
                root,
                source,
                destination,
            }
        }

        fn populate(base: &Path, entries: &[TestEntry<'_>]) {
            for entry in entries {
                let path = base.join(entry.path);
                match entry.kind {
                    TestEntryKind::MusicFile | TestEntryKind::InvalidFile => {
                        fs::create_dir_all(path.parent().expect("test file without parent"))
                            .expect("create parent directories for test file");
                        fs::write(&path, b"test").expect("create test file");
                    }
                    TestEntryKind::EmptyDir => {
                        fs::create_dir_all(&path).expect("create test directory");
                    }
                }
            }
        }

        fn source(&self) -> &Path {
            &self.source
        }

        fn destination(&self) -> &Path {
            &self.destination
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root).expect("remove test directory");
            }
        }
    }

    fn apply_filesystem_actions(backend: &mut Dissonance) {
        let actions = backend.filesystem_actions.clone();
        let total_size = actions.len();

        for (iter, action) in actions.iter().enumerate() {
            let report = process_filesystem_action(action, iter, total_size);
            backend.handle_filesystem_action_done(&report);
        }
    }

    #[test]
    fn basic_sync_scenario() {
        redirect_config_dir();

        let dirs = TestDirs::new(
            &[
                TestEntry {
                    path: "artist/album/song.mp3",
                    kind: TestEntryKind::MusicFile,
                },
                TestEntry {
                    path: "artist/readme.txt",
                    kind: TestEntryKind::InvalidFile,
                },
                TestEntry {
                    path: "empty",
                    kind: TestEntryKind::EmptyDir,
                },
            ],
            &[],
        );
        let saved_state = AppSavedState {
            source: Some(dirs.source().to_path_buf()),
            destination: Some(dirs.destination().to_path_buf()),
        };
        let mut backend = Dissonance::new(saved_state);

        let source_files = crate::directory::load_dir_hash_set_files_only(
            dirs.source().to_path_buf(),
            PathBuf::new(),
        );
        backend.handle_root_loaded(source_files);
        backend.set_sync_intention(
            PathBuf::from("artist/album/song.mp3"),
            SyncIntention::KeepSync,
        );

        backend.sync_with_destination();

        assert_eq!(backend.filesystem_actions.len(), 1);
        match &backend.filesystem_actions[0] {
            FilesystemAction::Copy(copy) => {
                assert_eq!(copy.from_base_abs, dirs.source().to_path_buf());
                assert_eq!(copy.to_base_abs, dirs.destination().to_path_buf());
                assert_eq!(copy.relative, PathBuf::from("artist/album/song.mp3"));
            }
            other => panic!("expected copy action, got {other:?}"),
        }

        apply_filesystem_actions(&mut backend);

        assert!(dirs.destination().join("artist/album/song.mp3").is_file());

        assert!(!!!dirs.destination().join("empty").exists());
        assert!(!!!dirs.destination().join("artist/readme.txt").exists());
    }

    static ARTIST_DIR: &str = "Beatles";
    static ALBUM_DIR: &str = "Beatles/[1966] Revolver";
    static FIRST_SONG: &str = "Beatles/[1966] Revolver/[01] Taxman.mp3";
    static SECOND_SONG: &str = "Beatles/[1966] Revolver/[02] Eleanor Rigby.mp3";

    fn remote_universe() -> MockRemoteInfoProvider {
        MockRemoteInfoProvider::new(&[
            ("123456", "The Beatles"),
            ("890123468", "Beatles Tribute Band"),
            ("555", "Jack Johnson"),
        ])
    }

    fn remote_album_universe() -> MockRemoteInfoProvider {
        remote_universe().with_albums(&[
            ("123456", "1642995371", "Revolver (2022 Mix)", "2022", 14),
            (
                "123456",
                "1441164426",
                "Abbey Road (Remastered)",
                "1969",
                19,
            ),
        ])
    }

    fn write_tags(base: &Path, rel: &str, artist: &str, title: &str, track_number: &str) {
        let file = MusicFile::from(
            &base.to_path_buf(),
            &PathBuf::from(rel),
            SyncIntention::Unspecified,
        )
        .expect("not a music file");

        let mut tags = MusicFile::empty_tags();
        tags.artist = artist.to_string();
        tags.album = "Revolver".to_string();
        tags.year = "1966".to_string();
        tags.title = title.to_string();
        tags.track_number = track_number.to_string();

        file.set_tags(&tags);
    }

    fn artist_tag_of(base: &Path, rel: &str) -> String {
        MusicFile::from(
            &base.to_path_buf(),
            &PathBuf::from(rel),
            SyncIntention::Unspecified,
        )
        .expect("not a music file")
        .tags()
        .artist
    }

    // One artist directory holding one album of two songs, with no mapping yet.
    fn backend_with_an_artist() -> (TestDirs, Dissonance) {
        redirect_config_dir();

        let dirs = TestDirs::new(
            &[
                TestEntry {
                    path: FIRST_SONG,
                    kind: TestEntryKind::MusicFile,
                },
                TestEntry {
                    path: SECOND_SONG,
                    kind: TestEntryKind::MusicFile,
                },
            ],
            &[],
        );

        write_tags(dirs.source(), FIRST_SONG, "Beatles", "Taxman", "1");
        write_tags(dirs.source(), SECOND_SONG, "Beatles", "Eleanor Rigby", "2");

        let saved_state = AppSavedState {
            source: Some(dirs.source().to_path_buf()),
            destination: Some(dirs.destination().to_path_buf()),
        };
        let mut backend = Dissonance::new(saved_state);

        let source_files = crate::directory::load_dir_hash_set_files_only(
            dirs.source().to_path_buf(),
            PathBuf::new(),
        );
        backend.handle_root_loaded(source_files);

        // start from an empty mapping, whatever a previous run left behind
        backend
            .root_dir
            .set_remote_mapping_database(&RemoteArtistMappingDatabase::new());
        backend
            .root_dir
            .set_remote_album_mapping_database(&RemoteAlbumMappingDatabase::new());

        (dirs, backend)
    }

    // The same tree, with the artist mapped: an album is only ever searched
    // among the albums of a mapped artist.
    fn backend_with_a_mapped_artist() -> (TestDirs, Dissonance) {
        let (dirs, mut backend) = backend_with_an_artist();

        let meta = resolve_artist_mapping(
            "Beatles",
            &remote_album_universe(),
            &MockArtistSelector::new(MockChoice::ByName("The Beatles".to_string())),
        )
        .expect("request failed")
        .expect("cancelled");

        backend.set_remote_artist_mapping(Path::new(ARTIST_DIR), &meta);

        (dirs, backend)
    }

    // Maps the album onto "Revolver (2022 Mix)" and answers with its meta.
    fn map_the_album(backend: &mut Dissonance) -> RemoteAlbumMeta {
        let artist_id = backend
            .artist_remote_id_of_album(Path::new(ALBUM_DIR))
            .expect("the artist is not mapped");
        let (local_name, track_count) = backend
            .local_album_name_and_track_count(Path::new(ALBUM_DIR))
            .expect("no album name");

        let meta = resolve_album_mapping(
            &artist_id,
            &local_name,
            track_count,
            &remote_album_universe(),
            &MockAlbumSelector::new(MockAlbumChoice::ByName("Revolver (2022 Mix)".to_string())),
        )
        .expect("request failed")
        .expect("cancelled");

        backend.set_remote_album_mapping(Path::new(ALBUM_DIR), &meta);

        meta
    }

    fn year_tag_of(base: &Path, rel: &str) -> String {
        MusicFile::from(
            &base.to_path_buf(),
            &PathBuf::from(rel),
            SyncIntention::Unspecified,
        )
        .expect("not a music file")
        .tags()
        .year
    }

    fn album_tag_of(base: &Path, rel: &str) -> String {
        MusicFile::from(
            &base.to_path_buf(),
            &PathBuf::from(rel),
            SyncIntention::Unspecified,
        )
        .expect("not a music file")
        .tags()
        .album
    }

    fn dir_at<'a>(backend: &'a Dissonance, rel_path: &str) -> &'a Directory {
        match backend.root_dir.find_recursive(Path::new(rel_path)) {
            Some(FsEntry::FsDirectory(d)) => d,
            _ => panic!("expected a directory at {}", rel_path),
        }
    }

    fn actions_of(backend: &Dissonance, rel_path: &str) -> BTreeSet<Action> {
        let entry = backend
            .root_dir
            .find_recursive(Path::new(rel_path))
            .expect("expected an entry");

        backend.get_suitable_actions(entry)
    }

    #[test]
    fn an_unmapped_artist_is_problematic_and_offers_the_request() {
        let (_dirs, backend) = backend_with_an_artist();

        let artist = dir_at(&backend, ARTIST_DIR);
        assert_eq!(artist.artist_name(), Some("Beatles"));
        assert_eq!(artist.remote_artist_meta(), &None);
        assert!(artist.has_problems());
        assert!(
            artist
                .find_problems()
                .contains(&Problem::MissingRemoteArtistMapping)
        );

        let actions = actions_of(&backend, ARTIST_DIR);
        assert!(actions.contains(&Action::MapRemoteTagSource));
        assert!(!actions.contains(&Action::ResetRemoteMapping));
        assert!(!actions.contains(&Action::InstallRemoteTags));

        // the album below belongs to the same artist, but it is not an artist
        // directory, so it is neither mapped nor problematic
        let album = dir_at(&backend, "Beatles/[1966] Revolver");
        assert!(!album.is_artist());
        assert_eq!(album.artist_name(), Some("Beatles"));
        assert_eq!(album.mapped_artist_name(), None);
        assert!(
            !album
                .find_problems()
                .contains(&Problem::MissingRemoteArtistMapping)
        );

        let album_actions = actions_of(&backend, "Beatles/[1966] Revolver");
        assert!(!album_actions.contains(&Action::MapRemoteTagSource));
    }

    #[test]
    fn mapping_an_artist_stores_both_names_and_clears_the_problem() {
        let (_dirs, mut backend) = backend_with_an_artist();

        let local_name = backend
            .artist_local_name(Path::new(ARTIST_DIR))
            .expect("no artist name");

        let meta = resolve_artist_mapping(
            &local_name,
            &remote_universe(),
            &MockArtistSelector::new(MockChoice::ByName("The Beatles".to_string())),
        )
        .expect("request failed")
        .expect("cancelled");

        backend.set_remote_artist_mapping(Path::new(ARTIST_DIR), &meta);

        assert_eq!(
            backend.remote_artist_meta(Path::new(ARTIST_DIR)),
            Some(RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            })
        );

        // the local name and the differing remote one
        let saved = backend.root_dir.create_remote_mapping_recursive();
        assert_eq!(saved.len(), 2);
        assert!(saved.meta_for("Beatles").is_some());
        assert!(saved.meta_for("The Beatles").is_some());

        let artist = dir_at(&backend, ARTIST_DIR);
        assert!(
            !artist
                .find_problems()
                .contains(&Problem::MissingRemoteArtistMapping)
        );

        let actions = actions_of(&backend, ARTIST_DIR);
        assert!(actions.contains(&Action::InstallRemoteTags));
        assert!(actions.contains(&Action::ResetRemoteMapping));
        assert!(!actions.contains(&Action::MapRemoteTagSource));
    }

    #[test]
    fn no_suitable_fires_no_problem_and_offers_the_reset_only() {
        let (_dirs, mut backend) = backend_with_an_artist();

        let local_name = backend
            .artist_local_name(Path::new(ARTIST_DIR))
            .expect("no artist name");

        let meta = resolve_artist_mapping(
            &local_name,
            &remote_universe(),
            &MockArtistSelector::new(MockChoice::NoSuitable),
        )
        .expect("request failed")
        .expect("cancelled");

        backend.set_remote_artist_mapping(Path::new(ARTIST_DIR), &meta);

        let artist = dir_at(&backend, ARTIST_DIR);
        assert_eq!(
            artist.remote_artist_meta(),
            &Some(RemoteArtistMeta::NoSuitable)
        );
        assert!(
            !artist
                .find_problems()
                .contains(&Problem::MissingRemoteArtistMapping)
        );

        let actions = actions_of(&backend, ARTIST_DIR);
        assert!(actions.contains(&Action::ResetRemoteMapping));
        assert!(!actions.contains(&Action::MapRemoteTagSource));
        assert!(!actions.contains(&Action::InstallRemoteTags));
    }

    #[test]
    fn a_reset_brings_the_artist_back_to_the_starting_state() {
        let (_dirs, mut backend) = backend_with_an_artist();

        backend.set_remote_artist_mapping(Path::new(ARTIST_DIR), &RemoteArtistMeta::NoSuitable);
        backend.reset_remote_artist_mapping(Path::new(ARTIST_DIR));

        let artist = dir_at(&backend, ARTIST_DIR);
        assert_eq!(artist.remote_artist_meta(), &None);
        assert!(
            artist
                .find_problems()
                .contains(&Problem::MissingRemoteArtistMapping)
        );

        let actions = actions_of(&backend, ARTIST_DIR);
        assert!(actions.contains(&Action::MapRemoteTagSource));
        assert!(!actions.contains(&Action::ResetRemoteMapping));

        // the mapping written on closing is built out of the tree, so a reset
        // artist has no entry in it at all
        assert_eq!(backend.root_dir.create_remote_mapping_recursive().len(), 0);
    }

    #[test]
    fn installing_remote_tags_writes_the_artist_into_every_file() {
        let (dirs, mut backend) = backend_with_an_artist();

        let local_name = backend
            .artist_local_name(Path::new(ARTIST_DIR))
            .expect("no artist name");

        let meta = resolve_artist_mapping(
            &local_name,
            &remote_universe(),
            &MockArtistSelector::new(MockChoice::ByName("The Beatles".to_string())),
        )
        .expect("request failed")
        .expect("cancelled");

        backend.set_remote_artist_mapping(Path::new(ARTIST_DIR), &meta);

        let stored = backend
            .remote_artist_meta(Path::new(ARTIST_DIR))
            .expect("no mapping");
        let remote_name = resolve_remote_name(&stored, &remote_universe()).expect("no remote name");

        backend.install_remote_tags(Path::new(ARTIST_DIR), &remote_name);

        assert_eq!(artist_tag_of(dirs.source(), FIRST_SONG), "The Beatles");
        assert_eq!(artist_tag_of(dirs.source(), SECOND_SONG), "The Beatles");

        // the meta was recalculated, and the renamed artist is still mapped
        let artist = dir_at(&backend, ARTIST_DIR);
        assert_eq!(artist.artist_name(), Some("The Beatles"));
        assert_eq!(
            artist.remote_artist_meta(),
            &Some(RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            })
        );

        // the artist is known under its remote name now, and that is the only
        // name the tree can write on closing
        let saved = backend.root_dir.create_remote_mapping_recursive();
        assert_eq!(saved.len(), 1);
        assert!(saved.meta_for("The Beatles").is_some());
    }

    #[test]
    fn a_name_of_a_mapping_read_from_the_file_is_requested_by_id() {
        let (_dirs, mut backend) = backend_with_an_artist();

        backend.set_remote_artist_mapping(
            Path::new(ARTIST_DIR),
            &RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: Some("The Beatles".to_string()),
            },
        );

        // a fresh session reads what was written, and knows the id alone
        let written = backend.root_dir.create_remote_mapping_recursive();
        let restored: RemoteArtistMappingDatabase =
            serde_json::from_str(&serde_json::to_string(&written).unwrap()).unwrap();
        backend.root_dir.set_remote_mapping_database(&restored);

        let stored = backend
            .remote_artist_meta(Path::new(ARTIST_DIR))
            .expect("no mapping");
        assert_eq!(
            stored,
            RemoteArtistMeta::Mapped {
                id: "123456".to_string(),
                name: None,
            }
        );

        assert_eq!(
            resolve_remote_name(&stored, &remote_universe()),
            Ok("The Beatles".to_string())
        );
    }

    #[test]
    fn an_unmapped_album_offers_no_request_until_the_artist_is_mapped() {
        let (_dirs, backend) = backend_with_an_artist();

        let album = dir_at(&backend, ALBUM_DIR);
        assert!(album.is_album());
        assert_eq!(album.mapped_album_key(), Some(("Beatles", "Revolver")));
        assert_eq!(album.track_count(), 2);

        // an unmapped album is problematic whether or not its artist is mapped
        assert!(album.has_problems());
        assert!(
            album
                .find_problems()
                .contains(&Problem::MissingRemoteAlbumMapping)
        );

        // but there is nothing to search among until the artist is mapped
        assert_eq!(
            backend.artist_remote_id_of_album(Path::new(ALBUM_DIR)),
            None
        );
        let actions = actions_of(&backend, ALBUM_DIR);
        assert!(!actions.contains(&Action::MapRemoteAlbumTagSource));
        assert!(!actions.contains(&Action::ResetRemoteAlbumMapping));
        assert!(!actions.contains(&Action::InstallRemoteAlbumTags));
    }

    #[test]
    fn an_unmapped_album_under_a_mapped_artist_offers_the_request() {
        let (_dirs, backend) = backend_with_a_mapped_artist();

        assert_eq!(
            backend.artist_remote_id_of_album(Path::new(ALBUM_DIR)),
            Some("123456".to_string())
        );

        let actions = actions_of(&backend, ALBUM_DIR);
        assert!(actions.contains(&Action::MapRemoteAlbumTagSource));
        assert!(!actions.contains(&Action::ResetRemoteAlbumMapping));
        assert!(!actions.contains(&Action::InstallRemoteAlbumTags));

        // the artist directory above is not an album and is never asked. It
        // does report the album's problem, because a directory reports the
        // problems common to its children, but it claims no mapping of its own
        let artist = dir_at(&backend, ARTIST_DIR);
        assert_eq!(artist.mapped_album_key(), None);
        assert_eq!(artist.remote_album_meta(), &None);
        assert!(!actions_of(&backend, ARTIST_DIR).contains(&Action::MapRemoteAlbumTagSource));
    }

    #[test]
    fn mapping_an_album_stores_it_under_every_known_name() {
        let (_dirs, mut backend) = backend_with_a_mapped_artist();
        map_the_album(&mut backend);

        assert_eq!(
            backend.remote_album_meta(Path::new(ALBUM_DIR)),
            Some(RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: Some("Revolver (2022 Mix)".to_string()),
            })
        );

        // both names of the artist and both names of the album
        let saved = backend.root_dir.create_remote_album_mapping_recursive();
        assert_eq!(saved.len(), 4);
        assert!(saved.meta_for("Beatles", "Revolver").is_some());
        assert!(saved.meta_for("Beatles", "Revolver (2022 Mix)").is_some());
        assert!(saved.meta_for("The Beatles", "Revolver").is_some());
        assert!(
            saved
                .meta_for("The Beatles", "Revolver (2022 Mix)")
                .is_some()
        );

        let album = dir_at(&backend, ALBUM_DIR);
        assert!(
            !album
                .find_problems()
                .contains(&Problem::MissingRemoteAlbumMapping)
        );

        let actions = actions_of(&backend, ALBUM_DIR);
        assert!(actions.contains(&Action::InstallRemoteAlbumTags));
        assert!(actions.contains(&Action::ResetRemoteAlbumMapping));
        assert!(!actions.contains(&Action::MapRemoteAlbumTagSource));
    }

    #[test]
    fn album_no_suitable_fires_no_problem_and_offers_the_reset_only() {
        let (_dirs, mut backend) = backend_with_a_mapped_artist();

        backend.set_remote_album_mapping(Path::new(ALBUM_DIR), &RemoteAlbumMeta::NoSuitable);

        let album = dir_at(&backend, ALBUM_DIR);
        assert_eq!(
            album.remote_album_meta(),
            &Some(RemoteAlbumMeta::NoSuitable)
        );
        assert!(
            !album
                .find_problems()
                .contains(&Problem::MissingRemoteAlbumMapping)
        );

        let actions = actions_of(&backend, ALBUM_DIR);
        assert!(actions.contains(&Action::ResetRemoteAlbumMapping));
        assert!(!actions.contains(&Action::MapRemoteAlbumTagSource));
        assert!(!actions.contains(&Action::InstallRemoteAlbumTags));

        // a "no suitable" album is written, so that it is not asked again. The
        // artist above is mapped and its remote name is known, so the album is
        // written under both names the artist can be found by
        let saved = backend.root_dir.create_remote_album_mapping_recursive();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            saved.meta_for("Beatles", "Revolver"),
            Some(RemoteAlbumMeta::NoSuitable)
        );
        assert_eq!(
            saved.meta_for("The Beatles", "Revolver"),
            Some(RemoteAlbumMeta::NoSuitable)
        );
    }

    #[test]
    fn an_album_reset_brings_it_back_to_the_starting_state() {
        let (_dirs, mut backend) = backend_with_a_mapped_artist();

        backend.set_remote_album_mapping(Path::new(ALBUM_DIR), &RemoteAlbumMeta::NoSuitable);
        backend.reset_remote_album_mapping(Path::new(ALBUM_DIR));

        let album = dir_at(&backend, ALBUM_DIR);
        assert_eq!(album.remote_album_meta(), &None);
        assert!(
            album
                .find_problems()
                .contains(&Problem::MissingRemoteAlbumMapping)
        );

        let actions = actions_of(&backend, ALBUM_DIR);
        assert!(actions.contains(&Action::MapRemoteAlbumTagSource));
        assert!(!actions.contains(&Action::ResetRemoteAlbumMapping));

        // the mapping written on closing is built out of the tree, so a reset
        // album has no entry in it at all
        assert_eq!(
            backend
                .root_dir
                .create_remote_album_mapping_recursive()
                .len(),
            0
        );
    }

    #[test]
    fn installing_remote_album_tags_writes_the_album_into_every_file() {
        let (dirs, mut backend) = backend_with_a_mapped_artist();
        map_the_album(&mut backend);

        let stored = backend
            .remote_album_meta(Path::new(ALBUM_DIR))
            .expect("no mapping");
        let remote =
            resolve_remote_album_tags(&stored, &remote_album_universe()).expect("no remote tags");

        backend.install_remote_album_tags(Path::new(ALBUM_DIR), &remote);

        assert_eq!(
            album_tag_of(dirs.source(), FIRST_SONG),
            "Revolver (2022 Mix)"
        );
        assert_eq!(
            album_tag_of(dirs.source(), SECOND_SONG),
            "Revolver (2022 Mix)"
        );

        // the remote year went into every file, the local 1966 is gone
        assert_eq!(year_tag_of(dirs.source(), FIRST_SONG), "2022");
        assert_eq!(year_tag_of(dirs.source(), SECOND_SONG), "2022");

        // the artist, the title and the track number are left alone
        assert_eq!(artist_tag_of(dirs.source(), FIRST_SONG), "Beatles");
        let first = match backend.root_dir.find_recursive(Path::new(FIRST_SONG)) {
            Some(FsEntry::FsMusicFile(mf)) => mf.tags(),
            _ => panic!("expected a music file"),
        };
        assert_eq!(first.title, "Taxman");
        assert_eq!(first.track_number, "1");

        // the meta was recalculated, and the renamed album is still mapped
        let album = dir_at(&backend, ALBUM_DIR);
        assert_eq!(album.album_name(), Some("Revolver (2022 Mix)"));
        assert_eq!(
            album.remote_album_meta(),
            &Some(RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: Some("Revolver (2022 Mix)".to_string()),
            })
        );
    }

    #[test]
    fn an_empty_remote_year_leaves_the_local_year_alone() {
        let (dirs, mut backend) = backend_with_a_mapped_artist();
        map_the_album(&mut backend);

        // a source that told no year for this album
        backend.install_remote_album_tags(
            Path::new(ALBUM_DIR),
            &RemoteAlbumTags {
                name: "Revolver (2022 Mix)".to_string(),
                year: String::new(),
            },
        );

        assert_eq!(
            album_tag_of(dirs.source(), FIRST_SONG),
            "Revolver (2022 Mix)"
        );

        // emptying it would only turn into a 'MissingYear' problem
        assert_eq!(year_tag_of(dirs.source(), FIRST_SONG), "1966");
        assert!(
            !dir_at(&backend, ALBUM_DIR)
                .find_problems()
                .contains(&Problem::MissingYear)
        );
    }

    #[test]
    fn an_installed_album_name_leaves_a_path_problem_that_move_file_fixes() {
        let (dirs, mut backend) = backend_with_a_mapped_artist();
        map_the_album(&mut backend);

        backend.install_remote_album_tags(
            Path::new(ALBUM_DIR),
            &RemoteAlbumTags {
                name: "Revolver (2022 Mix)".to_string(),
                year: "2022".to_string(),
            },
        );

        // the files were not moved, so the path no longer matches the tags
        assert!(
            dir_at(&backend, ALBUM_DIR)
                .find_problems()
                .contains(&Problem::MismatchedPath)
        );

        backend.move_entry_to_tag_based_path(PathBuf::from(ALBUM_DIR));

        // the installed year is part of the directory name as well
        let moved = "Beatles/[2022] Revolver (2022 Mix)/[01] Taxman.mp3";
        assert!(dirs.source().join(moved).exists());
        assert!(!dirs.source().join(FIRST_SONG).exists());

        let album = dir_at(&backend, "Beatles/[2022] Revolver (2022 Mix)");
        assert!(!album.find_problems().contains(&Problem::MismatchedPath));
    }

    #[test]
    fn an_album_name_read_from_the_file_is_requested_by_id() {
        let (_dirs, mut backend) = backend_with_a_mapped_artist();
        map_the_album(&mut backend);

        // a fresh session reads what was written, and knows the id alone
        let written = backend.root_dir.create_remote_album_mapping_recursive();
        let restored: RemoteAlbumMappingDatabase =
            serde_json::from_str(&serde_json::to_string(&written).unwrap()).unwrap();
        backend
            .root_dir
            .set_remote_album_mapping_database(&restored);

        let stored = backend
            .remote_album_meta(Path::new(ALBUM_DIR))
            .expect("no mapping");
        assert_eq!(
            stored,
            RemoteAlbumMeta::Mapped {
                id: "1642995371".to_string(),
                name: None,
            }
        );

        // one request tells the name and the year together
        let remote =
            resolve_remote_album_tags(&stored, &remote_album_universe()).expect("no remote tags");
        assert_eq!(remote.name, "Revolver (2022 Mix)");
        assert_eq!(remote.year, "2022");
    }
}
