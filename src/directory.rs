use crate::{
    Problem, SyncIntention, SyncedEntry,
    music_file::{FsEntryTrait, InvalidFile, MusicFile},
};
use pathdiff::diff_paths;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
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

// The root of the tree is a Directory with an empty relative path.
#[derive(Debug, Clone)]
pub struct Directory {
    pub base_path: PathBuf,
    pub relative_path: PathBuf,

    // object filename (not path)
    children: BTreeMap<PathBuf, FsEntry>,

    pub expanded: bool, // TODO this is from view, not from model, move it
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

    pub fn empty() -> Directory {
        Directory::new(&PathBuf::new(), &PathBuf::new(), BTreeMap::new())
    }

    // Builds a whole tree rooted at 'root_path' out of relative file paths.
    pub fn from_paths(
        files: HashSet<PathBuf>,
        root_path: PathBuf,
        index: BTreeMap<PathBuf, SyncedEntry>,
    ) -> Directory {
        let mut root = Directory::new(&root_path, &PathBuf::new(), BTreeMap::new());

        for file in files.iter() {
            let sync_info = index.get(file).unwrap_or(&SyncedEntry {
                intention: SyncIntention::Unspecified,
                synced: false,
            });

            root.add_entry_recursive(file, sync_info.clone());
        }

        return root;
    }

    pub fn children(&self) -> &BTreeMap<PathBuf, FsEntry> {
        &self.children
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

    // Sync info of every music file below this directory, keyed by relative path.
    pub fn create_index_recursive(&self) -> BTreeMap<PathBuf, SyncedEntry> {
        let mut map = BTreeMap::<PathBuf, SyncedEntry>::new();

        for (_, entry) in self.children.iter() {
            match entry {
                FsEntry::FsMusicFile(mf) => {
                    map.insert(mf.relative_path.clone(), mf.sync_data.clone());
                }
                FsEntry::FsDirectory(d) => {
                    map.extend(d.create_index_recursive());
                }
                _ => {}
            }
        }

        return map;
    }

    pub fn find_recursive(&self, rel_path: &Path) -> Option<&FsEntry> {
        let (parent, right_part) = detach_root_parent(rel_path);

        if let Some(parent) = parent {
            let right_part = match right_part {
                Some(r) => r,
                None => {
                    println!("ERROR: no right path with parent: {}", rel_path.display());
                    return None;
                }
            };

            return match self.children.get(&parent) {
                // recurse in
                Some(FsEntry::FsDirectory(d)) => d.find_recursive(&right_part),

                // searching only directories if parent exists
                _ => None,
            };
        }

        // only filename left here, the children are keyed by filename
        self.children.get(&right_part?)
    }

    fn find_recursive_mut(&mut self, rel_path: &Path) -> Option<&mut FsEntry> {
        let (parent, right_part) = detach_root_parent(rel_path);

        if let Some(parent) = parent {
            let right_part = match right_part {
                Some(r) => r,
                None => {
                    println!("ERROR: no right path with parent: {}", rel_path.display());
                    return None;
                }
            };

            return match self.children.get_mut(&parent) {
                // recurse in
                Some(FsEntry::FsDirectory(d)) => d.find_recursive_mut(&right_part),

                // searching only directories if parent exists
                _ => None,
            };
        }

        // only filename left here, the children are keyed by filename
        self.children.get_mut(&right_part?)
    }

    // Sets sync info on the entry at 'rel_path'; directories propagate it to
    // their own children.
    pub fn set_sync_info_recursive(&mut self, rel_path: &Path, sync_entry: SyncedEntry) {
        match self.find_recursive_mut(rel_path) {
            Some(FsEntry::FsMusicFile(mf)) => mf.set_sync_info(sync_entry),
            Some(FsEntry::FsDirectory(d)) => d.set_sync_info(sync_entry),
            _ => {}
        }
    }

    // Adds an entry at 'entry_rel_path', creating the missing directories on the
    // way down.
    pub fn add_entry_recursive(&mut self, entry_rel_path: &PathBuf, sync_info: SyncedEntry) {
        let filename = match entry_rel_path.file_name() {
            Some(name) => PathBuf::from(name),
            None => return,
        };

        let parent_rel_path = entry_rel_path.parent().unwrap_or(Path::new(""));

        let parent: &mut Directory = if parent_rel_path == Path::new("") {
            self
        } else {
            if self.find_recursive(parent_rel_path).is_none() {
                self.add_entry_recursive(&parent_rel_path.to_path_buf(), sync_info.clone());
            }

            match self.find_recursive_mut(parent_rel_path) {
                Some(FsEntry::FsDirectory(d)) => d,
                _ => {
                    println!("ERROR: no parent dir for: {}", entry_rel_path.display());
                    return;
                }
            }
        };

        let mut entry = FsEntry::from(&parent.base_path.clone(), entry_rel_path);
        match entry {
            FsEntry::FsMusicFile(ref mut mf) => {
                mf.sync_data = sync_info;
            }
            _ => {}
        }

        parent.children.insert(filename, entry);
    }

    pub fn remove_entry_recursive(&mut self, rel_path: &Path) -> bool {
        let (parent, right_part) = detach_root_parent(rel_path);

        if let Some(parent) = parent {
            let right_part = match right_part {
                Some(r) => r,
                None => {
                    println!("ERROR: no right path with parent: {}", rel_path.display());
                    return false;
                }
            };

            return match self.children.get_mut(&parent) {
                // recurse in
                Some(FsEntry::FsDirectory(d)) => d.remove_entry_recursive(&right_part),

                // searching only directories if parent exists
                _ => false,
            };
        }

        // only filename left here, the children are keyed by filename
        match right_part {
            Some(filename) => self.children.remove(&filename).is_some(),
            None => false,
        }
    }

    pub fn toggle_dir_recursive(&mut self, target: &Path) {
        match self.find_recursive_mut(target) {
            Some(FsEntry::FsDirectory(d)) => {
                d.expanded = !d.expanded;
            }
            _ => {}
        };
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

// All those impementations are recursive
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

// Splits a relative path into its first component and the rest of the path.
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

#[cfg(test)]
mod tests {
    use super::{Directory, FsEntry, load_dir_hash_set_files_only};
    use crate::{SyncIntention, SyncedEntry};
    use std::{
        collections::{BTreeMap, HashSet},
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    enum TestEntryKind {
        MusicFile,
        File,
        EmptyDir,
    }

    struct TestEntry<'a> {
        path: &'a str,
        kind: TestEntryKind,
    }

    struct TestFilesystem {
        root: PathBuf,
    }

    impl TestFilesystem {
        fn new(entries: &[TestEntry<'_>]) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("dissonance-file-tree-test-{unique}"));

            for entry in entries {
                let path = root.join(entry.path);
                match entry.kind {
                    TestEntryKind::MusicFile | TestEntryKind::File => {
                        fs::create_dir_all(path.parent().expect("test file without parent"))
                            .expect("create parent directories for test file");
                        fs::write(&path, b"test").expect("create test file");
                    }
                    TestEntryKind::EmptyDir => {
                        fs::create_dir_all(&path).expect("create test directory");
                    }
                }
            }

            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for TestFilesystem {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root).expect("remove test directory");
            }
        }
    }

    #[test]
    fn load_dir_basic_scenario() {
        let fs = TestFilesystem::new(&[
            TestEntry {
                path: "root.txt",
                kind: TestEntryKind::File,
            },
            TestEntry {
                path: "artist/album/song.mp3",
                kind: TestEntryKind::MusicFile,
            },
            TestEntry {
                path: "empty",
                kind: TestEntryKind::EmptyDir,
            },
        ]);

        let files = load_dir_hash_set_files_only(fs.root().to_path_buf(), PathBuf::new());

        let expected = HashSet::from([
            PathBuf::from("root.txt"),
            PathBuf::from("artist/album/song.mp3"),
        ]);

        assert_eq!(files, expected);
    }

    #[test]
    fn scan_every_type_of_entry_and_create_default_index() {
        let fs = TestFilesystem::new(&[
            TestEntry {
                path: "artist/album/song.mp3",
                kind: TestEntryKind::MusicFile,
            },
            TestEntry {
                path: "artist/readme.txt",
                kind: TestEntryKind::File,
            },
        ]);

        let empty_index = BTreeMap::<PathBuf, SyncedEntry>::new();
        let files = load_dir_hash_set_files_only(fs.root().to_path_buf(), PathBuf::new());
        let root = Directory::from_paths(files, fs.root().to_path_buf(), empty_index);

        assert!(matches!(
            root.find_recursive(PathBuf::from("artist").as_path()),
            Some(FsEntry::FsDirectory(_))
        ));
        assert!(matches!(
            root.find_recursive(PathBuf::from("artist/album").as_path()),
            Some(FsEntry::FsDirectory(_))
        ));
        assert!(matches!(
            root.find_recursive(PathBuf::from("artist/readme.txt").as_path()),
            Some(FsEntry::FsFile(_))
        ));

        match root.find_recursive(PathBuf::from("artist/album/song.mp3").as_path()) {
            Some(FsEntry::FsMusicFile(mf)) => {
                assert_eq!(mf.sync_data.intention, SyncIntention::Unspecified);
                assert!(!mf.sync_data.synced);
            }
            _ => panic!("expected music file entry"),
        }

        let created_index = root.create_index_recursive();
        assert_eq!(created_index.len(), 1);

        let indexed_song = created_index
            .get(&PathBuf::from("artist/album/song.mp3"))
            .expect("expected song in created index");

        assert_eq!(indexed_song.intention, SyncIntention::Unspecified);
        assert!(!indexed_song.synced);
    }
}
