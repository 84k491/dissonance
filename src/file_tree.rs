use crate::{
    SyncIntention, SyncedEntry,
    music_file::{Directory, FsEntryTrait, InvalidFile, MusicFile},
};
use pathdiff::diff_paths;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub enum FsEntry {
    FsFile(InvalidFile),
    FsMusicFile(MusicFile),
    FsDirectory(Directory),
}

impl FsEntry {
    pub fn from(root_path: &PathBuf, relative_path: &PathBuf) -> FsEntry {
        let abs_path = root_path.clone().join(&relative_path);

        if !abs_path.exists() {
            panic!("File does not exist: {}", abs_path.display());
        }

        if abs_path.is_dir() {
            let d = Directory::new(&root_path, relative_path, Vec::<FsEntry>::new());
            return FsEntry::FsDirectory(d);
        } else {
            let sync_info = SyncedEntry {
                intention: SyncIntention::Unspecified,
                synced: false,
            };

            let mf = MusicFile::from(&root_path, &relative_path, sync_info.intention.clone());

            match mf {
                Some(mf) => FsEntry::FsMusicFile(mf),
                None => {
                    let f = InvalidFile::from(&root_path, &relative_path);
                    return FsEntry::FsFile(f);
                }
            }
        }
    }

    pub fn rel_path(&self) -> &PathBuf {
        match &self {
            FsEntry::FsFile(f) => return &f.relative_path,
            FsEntry::FsMusicFile(mf) => return &mf.relative_path,
            FsEntry::FsDirectory(d) => return &d.relative_path,
        }
    }
}

pub struct FileTree {
    pub entries: Vec<FsEntry>, // TODO don't pub

    root_path: PathBuf,
}

impl FileTree {
    pub fn empty() -> FileTree {
        FileTree {
            entries: Vec::<FsEntry>::new(),
            root_path: PathBuf::new(),
        }
    }

    pub fn create_index(&self) -> BTreeMap<PathBuf, SyncedEntry> {
        return Self::do_create_index(&self.entries);
    }

    fn do_create_index(entries: &Vec<FsEntry>) -> BTreeMap<PathBuf, SyncedEntry> {
        let mut map = BTreeMap::<PathBuf, SyncedEntry>::new();

        for entry in entries.iter() {
            match entry {
                FsEntry::FsMusicFile(mf) => {
                    map.insert(mf.relative_path.clone(), mf.sync_data.clone());
                }
                FsEntry::FsDirectory(d) => {
                    let sub_index = Self::do_create_index(&d.children);
                    map.extend(sub_index);
                }
                _ => {}
            }
        }

        return map;
    }

    // TODO unused
    // fn flat_entries(entries: &Vec<FsEntry>) -> HashSet<PathBuf> {
    //     let mut res = HashSet::<PathBuf>::new();
    //
    //     for entry in entries.iter() {
    //         match entry {
    //             FsEntry::FsDirectory(d) => {
    //                 let d_res = Self::flat_entries(&d.children);
    //                 res.extend(d_res);
    //             }
    //             FsEntry::FsFile(_) => { /*don't index those files*/ }
    //             FsEntry::FsMusicFile(mf) => {
    //                 res.insert(mf.relative_path.clone());
    //             }
    //         }
    //     }
    //
    //     return res;
    // }
    //
    // pub fn flat(&self) -> HashSet<PathBuf> {
    //     Self::flat_entries(&self.entries)
    // }

    pub fn from(
        files: HashSet<PathBuf>,
        root_path: PathBuf,
        index: BTreeMap<PathBuf, SyncedEntry>,
    ) -> FileTree {
        let mut ft = FileTree {
            entries: Vec::<FsEntry>::new(),
            root_path: root_path,
        };

        for file in files.iter() {
            let sync_info = index.get(file).unwrap_or(&SyncedEntry {
                intention: SyncIntention::Unspecified,
                synced: false,
            });

            ft.add_entry(file, sync_info.clone());
        }

        return ft;
    }

    pub fn set_sync_info(&mut self, rel_path: &PathBuf, sync_entry: SyncedEntry) {
        let e: &mut FsEntry = Self::find_entry_mut(&mut self.entries, rel_path).unwrap();
        match e {
            FsEntry::FsMusicFile(mf) => {
                mf.set_sync_info(sync_entry);
            }
            FsEntry::FsDirectory(d) => {
                d.set_sync_info(sync_entry);
            }
            _ => {}
        }
    }

    pub fn find(&self, rel_path: &Path) -> Option<&FsEntry> {
        Self::find_entry(&self.entries, rel_path)
    }

    fn find_entry<'a>(entries: &'a Vec<FsEntry>, rel_path: &Path) -> Option<&'a FsEntry> {
        for entry in entries.iter() {
            match entry {
                FsEntry::FsDirectory(d) => {
                    if d.relative_path == rel_path {
                        return Some(entry);
                    } else {
                        if let Some(entry) = Self::find_entry(&d.children, rel_path) {
                            return Some(entry);
                        }
                    }
                }
                FsEntry::FsFile(f) => {
                    if f.relative_path == rel_path {
                        return Some(entry);
                    }
                }
                FsEntry::FsMusicFile(mf) => {
                    if mf.relative_path == rel_path {
                        return Some(entry);
                    }
                }
            }
        }

        None
    }

    fn find_entry_mut<'a>(
        entries: &'a mut Vec<FsEntry>,
        rel_path: &Path,
    ) -> Option<&'a mut FsEntry> {
        for entry in entries.iter_mut() {
            let matches = match entry {
                FsEntry::FsDirectory(d) => d.relative_path == rel_path,
                FsEntry::FsFile(f) => f.relative_path == rel_path,
                FsEntry::FsMusicFile(mf) => mf.relative_path == rel_path,
            };

            if matches {
                return Some(entry);
            }

            if let FsEntry::FsDirectory(d) = entry {
                if let Some(e) = Self::find_entry_mut(&mut d.children, rel_path) {
                    return Some(e);
                }
            }
        }
        None
    }

    pub fn remove_entry(&mut self, rel_path: &PathBuf) -> bool {
        Self::do_remove_entry(&mut self.entries, rel_path)
    }

    fn do_remove_entry(entries: &mut Vec<FsEntry>, rel_path: &PathBuf) -> bool {
        let mut iter_to_remove: Option<usize> = None;

        for (i, entry) in entries.iter_mut().enumerate() {
            match entry {
                FsEntry::FsDirectory(d) => {
                    if d.relative_path == *rel_path {
                        iter_to_remove = Some(i);
                    } else {
                        if Self::do_remove_entry(&mut d.children, rel_path) {
                            return true;
                        }
                    }
                }
                FsEntry::FsFile(f) => {
                    if f.relative_path == *rel_path {
                        iter_to_remove = Some(i);
                    }
                }
                FsEntry::FsMusicFile(mf) => {
                    if mf.relative_path == *rel_path {
                        iter_to_remove = Some(i);
                    }
                }
            }
        }

        if let Some(index) = iter_to_remove {
            entries.remove(index);
            return true;
        } else {
            return false;
        }
    }

    pub fn add_entry(&mut self, entry_rel_path: &PathBuf, sync_info: SyncedEntry) {
        let parent_rel_path = entry_rel_path.parent().unwrap();

        if parent_rel_path == Path::new("") {
            self.entries
                .push(FsEntry::from(&self.root_path.clone(), &entry_rel_path));
            return;
        }

        let parent = {
            let parent_opt = Self::find_entry_mut(&mut self.entries, parent_rel_path);
            if let None = parent_opt {
                let parent = entry_rel_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_path_buf();

                self.add_entry(&parent, sync_info.clone());
                Self::find_entry_mut(&mut self.entries, parent_rel_path)
                    .expect("Error on adding entry")
            } else {
                parent_opt.unwrap()
            }
        };

        match parent {
            FsEntry::FsDirectory(d) => {
                let mut entry = FsEntry::from(&d.base_path, entry_rel_path);
                match entry {
                    FsEntry::FsMusicFile(ref mut mf) => {
                        mf.sync_data = sync_info;
                    }
                    _ => {}
                }
                d.children.push(entry);
            }
            _ => {}
        };
    }

    pub fn toggle_dir(&mut self, target: &Path) {
        Self::toggle_dir_entries(&mut self.entries, target);
    }

    fn toggle_dir_entries(entries: &mut Vec<FsEntry>, target: &Path) {
        for entry in entries.iter_mut() {
            match entry {
                FsEntry::FsDirectory(d) => {
                    if d.relative_path == target {
                        d.expanded = !d.expanded;
                    } else {
                        Self::toggle_dir_entries(&mut d.children, target);
                    }
                }
                _ => continue,
            };
        }
    }
}

// pub fn load_dir(root_path: PathBuf, target_rel_path: PathBuf) -> Vec<FsEntry> {
//     let mut nodes = Vec::<FsEntry>::new();
//     let target_abs_path = root_path.join(&target_rel_path);
//
//     let read_dir = std::fs::read_dir(&target_abs_path);
//     if let Err(_) = read_dir {
//         return vec![];
//     }
//
//     let read_dir = read_dir.unwrap();
//
//     for entry in read_dir {
//         if let Err(_) = entry {
//             continue;
//         }
//
//         let absolute_path = entry.unwrap().path();
//         let relative_path = diff_paths(&absolute_path, &root_path)
//             .expect("Can't create relative path")
//             .to_path_buf();
//
//         let e = FsEntry::from(&root_path, &relative_path);
//         nodes.push(e);
//     }
//
//     nodes
// }

pub fn load_dir_hash_set_files_only(
    root_path: PathBuf,
    target_rel_path: PathBuf,
) -> HashSet<PathBuf> {
    let mut nodes = HashSet::new();
    let target_abs_path = root_path.join(&target_rel_path);

    let read_dir = std::fs::read_dir(&target_abs_path);
    if let Err(_) = read_dir {
        return HashSet::new();
    }

    let read_dir = read_dir.unwrap();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                continue;
            }
        };

        let absolute_path = entry.path().clone();
        let relative_path = diff_paths(&absolute_path, &root_path)
            .expect("Can't create relative path")
            .to_path_buf();

        if absolute_path.is_file() {
            nodes.insert(relative_path);
            continue;
        }

        let children = load_dir_hash_set_files_only(root_path.clone(), relative_path);
        nodes.extend(children);
    }

    nodes
}
