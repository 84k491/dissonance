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
            let d = Directory::new(&root_path, relative_path, BTreeMap::new());
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
    pub entries: BTreeMap<PathBuf, FsEntry>, // TODO don't pub

    root_path: PathBuf,
}

impl FileTree {
    pub fn empty() -> FileTree {
        FileTree {
            entries: BTreeMap::new(),
            root_path: PathBuf::new(),
        }
    }

    pub fn create_index(&self) -> BTreeMap<PathBuf, SyncedEntry> {
        return Self::do_create_index(&self.entries);
    }

    fn do_create_index(entries: &BTreeMap<PathBuf, FsEntry>) -> BTreeMap<PathBuf, SyncedEntry> {
        let mut map = BTreeMap::<PathBuf, SyncedEntry>::new();

        for (_, entry) in entries.iter() {
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

    pub fn from(
        files: HashSet<PathBuf>,
        root_path: PathBuf,
        index: BTreeMap<PathBuf, SyncedEntry>,
    ) -> FileTree {
        let mut ft = FileTree {
            entries: BTreeMap::new(),
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
        let e = match Self::find_entry_mut(&mut self.entries, rel_path) {
            Some(e) => e,
            None => {
                return;
            }
        };

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

    fn detach_root_parent(rel_path: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
        let parent: PathBuf = rel_path.components().take(1).collect();
        let right_part: PathBuf = rel_path.components().skip(1).collect();

        let parent_opt: Option<PathBuf> = if parent == PathBuf::new() {
            None
        } else {
            Some(parent)
        };

        let right_part_opt: Option<PathBuf> = if right_part == PathBuf::new() {
            None
        } else {
            Some(right_part)
        };

        if right_part_opt.is_none() && parent_opt.is_some() {
            return (None, parent_opt); // the only component is not a parent
        }

        return (parent_opt, right_part_opt);
    }

    fn find_entry<'a>(
        entries: &'a BTreeMap<PathBuf, FsEntry>,
        rel_path_right_part: &Path,
    ) -> Option<&'a FsEntry> {
        let (parent, right_part) = Self::detach_root_parent(rel_path_right_part);

        // empty path
        if parent.is_none() && right_part.is_none() {
            return None;
        }

        if let Some(parent) = parent {
            if right_part.is_none() {
                println!(
                    "ERROR: no right path with parent: {}",
                    rel_path_right_part.display()
                );
                return None;
            }
            let right_part = right_part.unwrap();

            let entry = entries.iter().find(|(filename, _)| **filename == parent);

            if entry.is_none() {
                return None;
            }
            let entry = entry.unwrap().1;

            match entry {
                FsEntry::FsDirectory(d) => {
                    if d.relative_path.file_name().expect("Error on file name") != parent {
                        return None;
                    }

                    // recurse in
                    return Self::find_entry(&d.children, right_part.as_path());
                }

                // searching only directories if parent exists
                _ => return None,
            };
        }

        // only filename left here, just searching for match
        let right_part = right_part.unwrap();

        let entry = entries
            .iter()
            .find(|(filename, _)| **filename == right_part);

        if entry.is_none() {
            return None;
        }
        let entry = entry.unwrap().1;

        match entry {
            FsEntry::FsDirectory(d) => {
                if d.relative_path.file_name().expect("Error on file name") == right_part {
                    return Some(entry);
                }
                // no recursion
            }
            FsEntry::FsFile(f) => {
                if f.relative_path.file_name().expect("Error on file name") == right_part {
                    return Some(entry);
                }
            }
            FsEntry::FsMusicFile(mf) => {
                if mf.relative_path.file_name().expect("Error on file name") == right_part {
                    return Some(entry);
                }
            }
        }

        None
    }

    // TODO don't copy-paste the method above
    fn find_entry_mut<'a>(
        entries: &'a mut BTreeMap<PathBuf, FsEntry>,
        rel_path_right_part: &Path,
    ) -> Option<&'a mut FsEntry> {
        let (parent, right_part) = Self::detach_root_parent(rel_path_right_part);

        // empty path
        if parent.is_none() && right_part.is_none() {
            return None;
        }

        if let Some(parent) = parent {
            if right_part.is_none() {
                println!(
                    "ERROR: no right path with parent: {}",
                    rel_path_right_part.display()
                );
                return None;
            }
            let right_part = right_part.unwrap();

            let entry = entries
                .iter_mut()
                .find(|(filename, _)| **filename == parent);

            if entry.is_none() {
                return None;
            }
            let entry = entry.unwrap();

            match entry.1 {
                FsEntry::FsDirectory(d) => {
                    // recurse in
                    return Self::find_entry_mut(&mut d.children, right_part.as_path());
                }

                // searching only directories if parent exists
                _ => return None,
            }
        }

        // only filename left here, just searching for match
        let right_part = right_part.unwrap();

        let entry = entries
            .iter_mut()
            .find(|(filename, _)| **filename == right_part);
        if entry.is_none() {
            return None;
        }
        let entry = entry.unwrap();

        match entry.1 {
            FsEntry::FsDirectory(d) => {
                if d.relative_path.file_name().expect("Error on file name") == entry.0 {
                    return Some(entry.1);
                }
                // no recursion
            }
            FsEntry::FsFile(f) => {
                if f.relative_path.file_name().expect("Error on file name") == right_part {
                    return Some(entry.1);
                }
            }
            FsEntry::FsMusicFile(mf) => {
                if mf.relative_path.file_name().expect("Error on file name") == right_part {
                    return Some(entry.1);
                }
            }
        }

        None
    }

    pub fn remove_entry(&mut self, rel_path: &PathBuf) -> bool {
        Self::do_remove_entry(&mut self.entries, rel_path)
    }

    fn do_remove_entry(
        entries: &mut BTreeMap<PathBuf, FsEntry>,
        rel_path_right_part: &PathBuf,
    ) -> bool {
        let (parent, right_part) = Self::detach_root_parent(rel_path_right_part);

        // empty path
        if parent.is_none() && right_part.is_none() {
            return false;
        }

        if let Some(parent) = parent {
            if right_part.is_none() {
                println!(
                    "ERROR: no right path with parent: {}",
                    rel_path_right_part.display()
                );
                return false;
            }
            let right_part = right_part.unwrap();

            let entry = entries
                .iter_mut()
                .find(|(filename, _)| **filename == parent);

            if entry.is_none() {
                return false;
            }
            let entry = entry.unwrap();

            match entry.1 {
                FsEntry::FsDirectory(d) => {
                    // recurse in
                    return Self::do_remove_entry(&mut d.children, &right_part);
                }

                // searching only directories if parent exists
                _ => return false,
            }
        }

        // only filename left here, just searching for match
        let right_part = right_part.unwrap();

        let removed = entries.remove(&right_part);

        return removed.is_some();
    }

    pub fn add_entry(&mut self, entry_rel_path: &PathBuf, sync_info: SyncedEntry) {
        let parent_rel_path = entry_rel_path.parent().unwrap();

        if parent_rel_path == Path::new("") {
            let filename = PathBuf::from(entry_rel_path.file_name().unwrap());

            self.entries.insert(
                filename,
                FsEntry::from(&self.root_path.clone(), &entry_rel_path),
            );
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
                let filename = PathBuf::from(entry_rel_path.file_name().unwrap());
                d.children.insert(filename, entry);
            }
            _ => {}
        };
    }

    pub fn toggle_dir(&mut self, target: &Path) {
        let e = Self::find_entry_mut(&mut self.entries, target);
        match e {
            Some(FsEntry::FsDirectory(d)) => {
                d.expanded = !d.expanded;
            }
            _ => {}
        };
    }
}

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
