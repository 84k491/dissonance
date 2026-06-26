use serde::{Deserialize, Serialize};

use crate::file_tree::load_dir_hash_set_files_only;
use crate::{
    file_tree::{FileTree, FsEntry},
    music_file::{Directory, FsEntryTrait, InvalidFile, MusicFile},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt::{self, Display},
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::PathBuf,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSavedState {
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
}

static CONFIG_REL_PATH: &str = ".config/dissonance";
static STATE_FILENAME: &str = "saved_state.json";
static INDEX_FILENAME: &str = "index.json";

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
    pub file_tree: FileTree,
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub destination_files: Option<HashSet<PathBuf>>,
    pub filesystem_actions: Vec<FilesystemAction>,
}

impl Drop for Dissonance {
    fn drop(&mut self) {
        let index = self.file_tree.create_index();
        Self::save_index(index);
        save_state(self.source.clone(), self.destination.clone());
    }
}

impl Dissonance {
    pub fn new(saved_state: AppSavedState) -> Self {
        Self {
            file_tree: FileTree::empty(),
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
        self.file_tree = FileTree::from(nodes, self.source.clone().unwrap(), index);

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

        let index = self.file_tree.create_index();
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
            .filter(|path| self.file_tree.find(path).is_none())
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
                self.file_tree.set_sync_info(
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
                self.file_tree.set_sync_info(
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

    pub fn is_dir_synced(&self, dir: &Directory) -> bool {
        dir.children
            .iter()
            .map(|(_, c)| match c {
                FsEntry::FsFile(_) => false,
                FsEntry::FsMusicFile(mf) => mf.sync_data.synced,
                FsEntry::FsDirectory(d) => self.is_dir_synced(d),
            })
            .reduce(|acc, v| acc && v)
            .unwrap_or(false)
    }

    fn load_index() -> BTreeMap<PathBuf, SyncedEntry> {
        let pwd = std::env::home_dir().expect("Failed to get home dir");
        let path = pwd
            .join(PathBuf::from(CONFIG_REL_PATH))
            .join(INDEX_FILENAME);
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
        let pwd = std::env::home_dir().expect("Failed to get home dir");
        let path = pwd
            .join(PathBuf::from(CONFIG_REL_PATH))
            .join(INDEX_FILENAME);
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

    pub fn set_sync_intention(&mut self, rel_path: PathBuf, intention: SyncIntention) {
        let entry = self.file_tree.find(&rel_path);
        match entry {
            Some(FsEntry::FsMusicFile(_)) => {}
            Some(FsEntry::FsDirectory(d)) => {
                let children = d.children_recursive();
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

        self.file_tree
            .set_sync_info(&rel_path, SyncedEntry { intention, synced });
    }

    fn update_index_destination(&mut self, dest_entries: &HashSet<PathBuf>) {
        let sync_info = self.file_tree.create_index();
        for (rel_path, entry) in sync_info.iter() {
            let is_in_dest = dest_entries.contains(rel_path);

            match entry.intention {
                SyncIntention::KeepSync => {
                    self.file_tree.set_sync_info(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::KeepSync,
                            synced: is_in_dest,
                        },
                    );
                }
                SyncIntention::DropSync => {
                    self.file_tree.set_sync_info(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::DropSync,
                            synced: !is_in_dest,
                        },
                    );
                }
                SyncIntention::Unspecified => {
                    self.file_tree.set_sync_info(
                        rel_path,
                        SyncedEntry {
                            intention: SyncIntention::Unspecified,
                            synced: false,
                        },
                    );
                }
                SyncIntention::ForceSync => {
                    self.file_tree.set_sync_info(
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
                    self.file_tree.set_sync_info(
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

        let entry = match self.file_tree.find(&path) {
            Some(entry) => entry,
            None => return,
        };

        match entry {
            FsEntry::FsMusicFile(mf) => {
                let tags = mf.compose_tags_from_path();
                mf.set_tags(&tags);

                let sync_data = mf.sync_data.clone();

                if !self.file_tree.remove_entry(&path) {
                    println!("Failed to forget file: {}", path.display());
                }
                self.file_tree.add_entry(&path, sync_data);
            }
            FsEntry::FsDirectory(d) => {
                let children: Vec<PathBuf> = d
                    .children
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

        match self.file_tree.find(&path) {
            Some(FsEntry::FsMusicFile(mf)) => {
                mf.remove_tags();

                let sync_data = mf.sync_data.clone();

                if !self.file_tree.remove_entry(&path) {
                    println!("Failed to forget file: {}", path.display());
                }
                self.file_tree.add_entry(&path, sync_data);
            }
            Some(FsEntry::FsDirectory(d)) => {
                let children: Vec<PathBuf> = d
                    .children
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
                    self.file_tree.remove_entry(&rel_path);
                }
                Err(e) => {
                    println!("Failed to remove file {}: {}", rel_path.display(), e);
                }
            }
        }
    }

    pub fn move_entry_to_tag_based_path(&mut self, path: PathBuf) -> Option<PathBuf> {
        let (rel_path, tag_based_rel_path) = {
            let entry = self.file_tree.find(&path);
            let file = match entry {
                Some(FsEntry::FsMusicFile(mf)) => mf,
                Some(FsEntry::FsDirectory(d)) => {
                    let children: Vec<PathBuf> = d
                        .children
                        .iter()
                        .map(|(_, entry)| entry.rel_path().clone())
                        .collect();
                    let dir_path = d.relative_path.clone();

                    for child in children {
                        self.move_entry_to_tag_based_path(child);
                    }

                    let dir = self.file_tree.find(&dir_path).expect("No dir on moving");
                    match dir {
                        FsEntry::FsDirectory(d) => {
                            if d.children.is_empty() {
                                self.file_tree.remove_entry(&dir_path);
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

        self.file_tree.add_entry(
            &to,
            SyncedEntry {
                intention: SyncIntention::Unspecified,
                synced: false,
            },
        );
        if !self.file_tree.remove_entry(&from) {
            println!("Failed to forget file: {}", from.to_string_lossy());
        }

        let _ = remove_empty_subdirs::remove_empty_subdirs(&self.source.clone().unwrap());
    }

    pub fn fix_characters(&mut self, old_path: &PathBuf) -> Option<PathBuf> {
        let entry = match self.file_tree.find(old_path) {
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
                    .children
                    .iter()
                    .map(|(_, entry)| entry.rel_path().clone())
                    .collect();

                for child in children {
                    self.fix_characters(&child);
                }

                self.file_tree.remove_entry(&d.relative_path.clone());
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
    let pwd = std::env::home_dir().expect("Failed to get home dir");
    let path = pwd
        .join(PathBuf::from(CONFIG_REL_PATH))
        .join(STATE_FILENAME);
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
    let pwd = std::env::home_dir().expect("Failed to get home dir");
    let path = pwd
        .join(PathBuf::from(CONFIG_REL_PATH))
        .join(STATE_FILENAME);
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
