use iced::{
    Application, Command, Element, Length, Settings, Theme, alignment, executor,
    widget::{Column, TextInput, button, column, container, row, scrollable, text},
};
use iced::{Background, Color};
use serde::{Deserialize, Serialize};

use crate::music_file::music_file::MusicFile;
use crate::{
    file_tree::file_tree::{FileTree, FsEntry, load_dir},
    music_file::music_file::Directory,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Display},
    fs::{self, File},
    io::BufWriter,
    path::PathBuf,
};

mod file_tree;
mod music_file;
mod tags;

fn main() -> iced::Result {
    DissonanceApp::run(Settings::default())
}

#[derive(Debug, Clone)]
enum Message {
    SourceUpdated(String),
    SourceSubmited,
    DestinationUpdated(String),
    DestinationSubmited,
    RootLoaded(Vec<FsEntry>),
    ToggleDir(PathBuf),
    SelectFile(PathBuf),
    FixTags(PathBuf),
    RemoveTags(PathBuf),
    MoveFile(PathBuf),
    KeepSync(PathBuf),
    DropSync(PathBuf),
    StartSync,
    StartIndexing,
}

#[derive(Clone, Serialize, Deserialize)]
struct AppSavedState {
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
}

static CONFIG_ABS_PATH: &'static str = "/home/bakar/.config/dissonance";
static STATE_FILENAME: &'static str = "saved_state.json";
static INDEX_FILENAME: &'static str = "index.json";

struct DissonanceApp {
    file_tree: FileTree,
    selected: Option<PathBuf>,

    input_source: String,
    input_destination: String,

    source: Option<PathBuf>,
    destination: Option<PathBuf>,

    sync_info: BTreeMap<PathBuf, SyncedEntry>,
}

#[derive(Debug)]
enum Problem {
    MissingTags,
    MismatchedTags,
    MismatchedPath,
    // MissingAlbumArt,
}

impl Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
enum Action {
    RemoveTags,
    FixTags,
    MoveFile,
    KeepSync,
    DropSync,
    // GetAlbumArt,
    // ApplyCustomTags,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, Ord, PartialOrd)]
enum SyncIntention {
    Unspecified, // top prio
    KeepSync,
    DropSync, // low prio
}

#[derive(Clone, Serialize, Deserialize)]
struct SyncedEntry {
    rel_path: PathBuf,
    intention: SyncIntention,
    synced: bool,
}

impl Action {
    fn to_message(&self, rel_path: PathBuf) -> Message {
        match self {
            Action::FixTags => Message::FixTags(rel_path),
            Action::MoveFile => Message::MoveFile(rel_path),
            Action::RemoveTags => Message::RemoveTags(rel_path),
            Action::KeepSync => Message::KeepSync(rel_path),
            Action::DropSync => Message::DropSync(rel_path),
        }
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Drop for DissonanceApp {
    fn drop(&mut self) {
        Self::save_index(&self.sync_info);
        save_state(self.source.clone(), self.destination.clone());
    }
}

impl Application for DissonanceApp {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let saved_state = load_saved_state();
        let index = DissonanceApp::load_index();
        (
            Self {
                file_tree: FileTree::empty(),
                input_source: saved_state
                    .source
                    .clone()
                    .unwrap_or_default()
                    .as_os_str()
                    .to_string_lossy()
                    .into(),
                input_destination: saved_state
                    .destination
                    .clone()
                    .unwrap_or_default()
                    .as_os_str()
                    .to_string_lossy()
                    .into(),
                selected: None,
                source: saved_state.source.clone(),
                destination: saved_state.destination.clone(),
                sync_info: index,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "Dissonance".into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SourceUpdated(source_abs_path) => {
                self.input_source = source_abs_path.clone();

                Command::none()
            }

            Message::SourceSubmited => {
                let path = PathBuf::from(self.input_source.clone());
                if path.exists() && path.is_dir() {
                    self.source = Some(path);
                }

                Command::none()
            }

            Message::DestinationUpdated(dest_abs_path) => {
                self.input_destination = dest_abs_path.clone();

                Command::none()
            }

            Message::DestinationSubmited => {
                let path = PathBuf::from(self.input_destination.clone());
                if path.exists() && path.is_dir() {
                    self.destination = Some(path);
                }

                Command::none()
            }

            Message::StartIndexing => {
                // TODO check if there is a source
                let source = self.source.as_ref().unwrap();
                Command::perform(
                    load_root_dir(source.clone(), source.clone()),
                    Message::RootLoaded,
                )
            }

            Message::RootLoaded(nodes) => {
                self.file_tree = FileTree::from(nodes);

                if self.source.is_none() || self.destination.is_none() {
                    return Command::none();
                }

                self.update_index_source();
                println!(
                    "Index updated with source files: {} files in total",
                    self.sync_info.len()
                );
                self.update_index_destination(); // TODO async
                println!(
                    "Index updated with destination files: {} files in total",
                    self.sync_info.len()
                );

                Command::none()
            }

            Message::ToggleDir(path) => {
                self.file_tree.toggle_dir(&path);
                self.selected = Some(path.clone());
                Command::none()
            }

            Message::SelectFile(path) => {
                self.selected = Some(path);
                Command::none()
            }
            Message::FixTags(rel_path) => {
                self.fix_tags(rel_path);
                Command::none()
            }
            Message::RemoveTags(rel_path) => {
                self.remove_tags(rel_path);
                Command::none()
            }
            Message::MoveFile(rel_path) => {
                self.move_file(rel_path);
                Command::none()
            }
            Message::KeepSync(rel_path) => {
                self.set_sync_intention(rel_path, SyncIntention::KeepSync, true);
                Command::none()
            }
            Message::DropSync(rel_path) => {
                self.set_sync_intention(rel_path, SyncIntention::DropSync, true);
                Command::none()
            }
            Message::StartSync => {
                self.sync_with_destination();
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let top_panel = self.render_top_panel();
        let main_panel = self.render_main_panel();

        column![
            container(top_panel)
                .height(Length::FillPortion(1))
                .width(Length::Fill)
                .style(iced::theme::Container::Custom(Box::new(PaneStyle::Left)))
                .padding(10),
            container(main_panel)
                .height(Length::FillPortion(6))
                .width(Length::Fill)
                .style(iced::theme::Container::Custom(Box::new(PaneStyle::Right)))
                .padding(10),
        ]
        .height(Length::Fill)
        .into()
    }
}

impl DissonanceApp {
    fn sync_with_destination(&mut self) {
        println!("Syncing with destination");

        let unsynced = self
            .sync_info
            .iter()
            .filter(|(_, e)| !e.synced)
            .map(|(_, v)| v.clone())
            .collect::<Vec<SyncedEntry>>();

        let to_remove_from_dest = unsynced
            .iter()
            .filter(|k| k.intention == SyncIntention::DropSync)
            .map(|k| k.rel_path.clone())
            .collect::<Vec<PathBuf>>();

        to_remove_from_dest.iter().for_each(|k| {
            let abs_path = self.destination.clone().unwrap().join(k);
            std::fs::remove_file(&abs_path).unwrap();
            println!("Removed: {}", abs_path.display());
        });

        let to_copy_to_dest = unsynced
            .iter()
            .filter(|k| k.intention == SyncIntention::KeepSync)
            .map(|k| k.rel_path.clone())
            .collect::<Vec<PathBuf>>();

        let _ = remove_empty_subdirs::remove_empty_subdirs(&self.destination.clone().unwrap());

        to_copy_to_dest.iter().for_each(|k| {
            let source_abs_path = self.source.clone().unwrap().join(k);
            let dest_abs_path = self.destination.clone().unwrap().join(k);

            match fs::create_dir_all(dest_abs_path.parent().expect("No parent on mkdir")) {
                Err(e) => println!(
                    "ERROR Failed to create dir: {} ({})",
                    dest_abs_path.display(),
                    e
                ),
                Ok(_) => {}
            }

            match std::fs::copy(&source_abs_path, &dest_abs_path) {
                Err(e) => println!(
                    "ERROR Failed to copy: {} ({})",
                    source_abs_path.display(),
                    e
                ),
                Ok(_) => println!("Copied: {}", dest_abs_path.display()),
            }
        });

        self.update_index_destination();
    }

    fn load_index() -> BTreeMap<PathBuf, SyncedEntry> {
        let path = PathBuf::from(CONFIG_ABS_PATH).join(INDEX_FILENAME);
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

    fn set_sync_intention(
        &mut self,
        rel_path: PathBuf,
        intention: SyncIntention,
        top_level_recursion: bool,
    ) {
        let fs_entry = match self.file_tree.find(&rel_path) {
            Some(e) => e,
            _ => return,
        };

        match fs_entry.clone() {
            FsEntry::FsDirectory(d) => {
                for child in d.children.iter() {
                    let rel_path = match child {
                        FsEntry::FsFile(f) => f.relative_path.clone(),
                        FsEntry::FsMusicFile(mf) => mf.relative_path.clone(),
                        FsEntry::FsDirectory(d) => d.relative_path.clone(),
                    };
                    self.set_sync_intention(rel_path, intention.clone(), false);
                }
            }
            FsEntry::FsMusicFile(mf) => {
                self.sync_info
                    .entry(mf.relative_path.clone()) // creates new
                    .and_modify(|e| e.intention = intention.clone());
            }
            _ => {}
        }

        if top_level_recursion {
            self.update_index_destination();
        }
    }

    fn save_index(index: &BTreeMap<PathBuf, SyncedEntry>) {
        let path = PathBuf::from(CONFIG_ABS_PATH).join(INDEX_FILENAME);
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

    fn update_index_source(&mut self) {
        let local_entries = self.file_tree.flat();

        // remove from index those entries that are not in local files
        self.sync_info = self
            .sync_info
            .iter()
            .filter(|(k, _)| local_entries.contains(*k))
            .map(|(k, v)| (k.clone(), (*v).clone()))
            .collect();

        // add to index all those local files that are not in index
        let to_add_to_index: BTreeMap<PathBuf, SyncedEntry> = local_entries
            .iter()
            .filter(|k| !self.sync_info.contains_key(*k))
            .map(|k| {
                (
                    k.clone(),
                    SyncedEntry {
                        rel_path: k.clone(),
                        intention: SyncIntention::Unspecified,
                        synced: false,
                    },
                )
            })
            .collect();

        self.sync_info.extend(to_add_to_index);
    }

    fn update_index_destination(&mut self) {
        if self.destination.is_none() {
            return;
        }

        let dest_entries = load_dir(self.destination.clone().unwrap(), PathBuf::new());
        let dest_tree = FileTree::from(dest_entries);
        let dest_entries = dest_tree.flat();

        // add to index (with {drop, unsync}) those entries that are in dest, but not in index // they will be removed from index on next local scan
        let to_add_to_index: BTreeMap<PathBuf, SyncedEntry> = dest_entries
            .iter()
            .filter(|k| !self.sync_info.contains_key(*k))
            .map(|k| {
                (
                    k.clone(),
                    SyncedEntry {
                        rel_path: k.clone(),
                        intention: SyncIntention::DropSync,
                        synced: false,
                    },
                )
            })
            .collect();
        self.sync_info.extend(to_add_to_index);

        // update existing entries
        for (rel_path, e) in self.sync_info.iter_mut() {
            let is_in_dest = dest_entries.contains(rel_path);

            match e.intention {
                SyncIntention::KeepSync => {
                    e.synced = is_in_dest;
                }
                SyncIntention::DropSync => {
                    e.synced = !is_in_dest;
                }
                SyncIntention::Unspecified => {
                    e.synced = false;
                }
            };
        }
    }

    fn render_top_panel(&self) -> iced::widget::Row<'static, Message> {
        let targets = column![
            TextInput::new("Source...", &self.input_source,)
                .on_input(Message::SourceUpdated)
                .on_submit(Message::SourceSubmited)
                .size(12),
            TextInput::new("Destination...", &self.input_destination)
                .on_input(Message::DestinationUpdated)
                .on_submit(Message::DestinationSubmited)
                .size(12),
            row![
                button(text("Sync").size(16)).on_press(Message::StartSync),
                button(text("Index").size(16)).on_press(Message::StartIndexing),
            ]
        ];

        row![targets]
    }

    fn render_main_panel(&self) -> iced::widget::Row<'_, Message> {
        let tree_view = scrollable(self.render_tree(&self.file_tree.entries, 0))
            .height(Length::Fill)
            .width(Length::Fill);

        let info_panel = self.render_info_panel();
        let actions_panel = self.render_actions_panel();

        row![
            container(tree_view)
                .padding(10)
                .height(Length::Fill)
                .width(Length::FillPortion(2))
                .style(iced::theme::Container::Custom(Box::new(TreePanelStyle {}))),
            container(info_panel)
                .padding(10)
                .height(Length::Fill)
                .width(Length::FillPortion(4)),
            container(actions_panel)
                .padding(10)
                .height(Length::Fill)
                .width(Length::FillPortion(1))
                .style(iced::theme::Container::Custom(Box::new(
                    ActionPanelStyle {}
                ))),
        ]
    }

    fn render_info_panel(&self) -> iced::widget::Column<'static, Message> {
        column![
            container(self.render_file_panel())
                .width(Length::Fill)
                .height(Length::FillPortion(3)),
            container(self.render_problems_panel())
                .width(Length::Fill)
                .height(Length::FillPortion(1)),
        ]
    }

    fn render_problems_panel(&self) -> iced::widget::Column<'static, Message> {
        if self.source.is_none() || self.selected.is_none() {
            return column![];
        }
        let entry = FsEntry::from(
            self.source.as_ref().unwrap(),
            self.selected.as_ref().unwrap(),
        );

        let problems = match entry {
            FsEntry::FsFile(_) => {
                vec![]
            }
            FsEntry::FsDirectory(_) => {
                vec![]
            }
            FsEntry::FsMusicFile(mf) => mf.find_problems(),
        };

        if problems.is_empty() {
            return column![];
        }

        let column = problems.iter().fold(Column::new().spacing(4), |col, p| {
            col.push(text(p.to_string()).size(16))
        });

        return column;
    }

    fn get_suitable_actions_for_music_file(&self, mf: &MusicFile) -> BTreeSet<Action> {
        let problems = mf.find_problems();

        // TODO don't let fix tags if file doesn't have both parents

        let mut actions = BTreeSet::<Action>::new();

        if !problems.is_empty() {
            for p in problems {
                match p {
                    Problem::MissingTags => {
                        actions.insert(Action::FixTags);
                    }
                    Problem::MismatchedTags | Problem::MismatchedPath => {
                        actions.insert(Action::FixTags);
                        actions.insert(Action::MoveFile);
                        actions.insert(Action::RemoveTags);
                    }
                }
            }
        } else {
            actions.insert(Action::RemoveTags);
        }

        let intention = match self.sync_info.get(&mf.relative_path) {
            None => {
                println!(
                    "ERROR Missing sync info for: {}",
                    mf.relative_path.display()
                );
                SyncIntention::Unspecified
            }
            Some(i) => i.intention.clone(),
        };

        match intention {
            SyncIntention::KeepSync => {
                actions.insert(Action::DropSync);
            }
            SyncIntention::DropSync => {
                actions.insert(Action::KeepSync);
            }
            SyncIntention::Unspecified => {
                actions.insert(Action::KeepSync);
                actions.insert(Action::DropSync);
            }
        }

        actions
    }

    fn get_suitable_actions_for_dir(&self, d: &Directory) -> BTreeSet<Action> {
        let intention = self.induce_dir_intention(d);

        let mut actions = BTreeSet::<Action>::new();
        match intention {
            SyncIntention::KeepSync => {
                actions.insert(Action::DropSync);
            }
            SyncIntention::DropSync => {
                actions.insert(Action::KeepSync);
            }
            SyncIntention::Unspecified => {
                actions.insert(Action::KeepSync);
                actions.insert(Action::DropSync);
            }
        }

        actions
    }

    fn get_suitable_actions(&self, entry: &FsEntry) -> BTreeSet<Action> {
        let actions = match entry {
            FsEntry::FsMusicFile(mf) => self.get_suitable_actions_for_music_file(mf),
            FsEntry::FsDirectory(d) => self.get_suitable_actions_for_dir(d),
            _ => return BTreeSet::new(),
        };
        return actions;
    }

    fn render_actions_panel(&self) -> iced::widget::Column<'static, Message> {
        if self.source.is_none() || self.selected.is_none() {
            return column![];
        }
        let entry = FsEntry::from(
            self.source.as_ref().unwrap(),
            self.selected.as_ref().unwrap(),
        );

        let actions = self.get_suitable_actions(&entry);

        let column = actions.iter().fold(Column::new().spacing(4), |col, a| {
            col.push(
                button(text(a.to_string()).size(16))
                    .on_press(a.to_message(self.selected.clone().unwrap())),
            )
        });

        column
    }

    fn render_file_panel(&self) -> iced::widget::Container<'static, Message> {
        if self.source.is_none() || self.selected.is_none() {
            return container(text("No file selected"));
        }

        let entry = self.file_tree.find(&self.selected.as_ref().unwrap());
        if entry.is_none() {
            return container(text("File not found"));
        }
        let entry = entry.unwrap();

        match entry {
            FsEntry::FsFile(f) => {
                return container(text(f.relative_path.display()).size(16))
                    .width(Length::Fill)
                    .height(Length::FillPortion(1))
                    .align_x(alignment::Horizontal::Left)
                    .align_y(alignment::Vertical::Top)
                    .style(iced::theme::Container::Custom(Box::new(InfoPanelStyle {})));
            }
            FsEntry::FsDirectory(d) => {
                return container(text(d.relative_path.display()).size(16))
                    .width(Length::Fill)
                    .height(Length::FillPortion(1))
                    .align_x(alignment::Horizontal::Left)
                    .align_y(alignment::Vertical::Top)
                    .style(iced::theme::Container::Custom(Box::new(InfoPanelStyle {})));
            }
            FsEntry::FsMusicFile(mf) => {
                let tags = mf.tags();

                container(column!(
                    text(tags.artist).size(16),
                    text(tags.album).size(16),
                    text(tags.title).size(16)
                ))
                .width(Length::Fill)
                .height(Length::FillPortion(1))
                .align_x(alignment::Horizontal::Left)
                .align_y(alignment::Vertical::Top)
                .style(iced::theme::Container::Custom(Box::new(InfoPanelStyle {})))
            }
        }
    }

    fn fix_tags(&mut self, path: PathBuf) {
        if self.source.is_none() || self.selected.is_none() {
            return;
        }

        let mf_ref = self.file_tree.find(&path);
        let mf = match mf_ref {
            Some(FsEntry::FsMusicFile(mf)) => mf,
            _ => return,
        };

        let tags = mf.compose_tags_from_path();
        mf.set_tags(&tags);

        if !self.file_tree.remove_entry(&path) {
            println!("Failed to forget file: {}", path.display());
        }
        self.file_tree.add_entry(&path);
    }

    fn remove_tags(&mut self, path: PathBuf) {
        if self.source.is_none() || self.selected.is_none() {
            return;
        }

        let mf_ref = self.file_tree.find(&path);
        let mf = match mf_ref {
            Some(FsEntry::FsMusicFile(mf)) => mf,
            _ => return,
        };

        let tags = MusicFile::empty_tags();
        mf.set_tags(&tags);

        if !self.file_tree.remove_entry(&path) {
            println!("Failed to forget file: {}", path.display());
        }
        self.file_tree.add_entry(&path);
    }

    fn move_file(&mut self, path: PathBuf) {
        let (rel_path, tag_based_rel_path) = {
            let mf_opt = self.file_tree.find(&path);
            let file = match mf_opt {
                Some(FsEntry::FsMusicFile(mf)) => mf,
                _ => return,
            };

            (
                file.relative_path.clone(),
                file.compose_path_from_tags(&file.tags()),
            )
        };

        let mut source_full_path = self.source.clone().unwrap().to_path_buf();
        source_full_path.push(&rel_path);

        let mut target_full_path = self.source.clone().unwrap().to_path_buf();
        target_full_path.push(tag_based_rel_path.clone());

        let _ = fs::create_dir_all(target_full_path.parent().unwrap());
        let move_res = fs::rename(&source_full_path, &target_full_path);
        if let Err(err) = move_res {
            println!("Moving to {:?} failed, err: {}", target_full_path, err);
            return;
        }

        self.file_tree.add_entry(&tag_based_rel_path);
        if !self.file_tree.remove_entry(&rel_path) {
            println!("Failed to forget file: {}", rel_path.to_string_lossy());
        }

        self.selected = Some(tag_based_rel_path);

        let _ = remove_empty_subdirs::remove_empty_subdirs(&self.source.clone().unwrap());
    }

    fn is_dir_synced(&self, dir: &Directory) -> bool {
        dir.children
            .iter()
            .map(|c| match c {
                FsEntry::FsFile(_) => false,
                FsEntry::FsMusicFile(mf) => match self.sync_info.get(&mf.relative_path) {
                    None => false,
                    Some(e) => e.synced,
                },
                FsEntry::FsDirectory(d) => self.is_dir_synced(d),
            })
            .reduce(|acc, v| acc && v)
            .unwrap_or(false)
    }

    fn induce_dir_intention(&self, dir: &Directory) -> SyncIntention {
        let a = dir
            .children
            .iter()
            .map(|c| match c {
                FsEntry::FsFile(_) => SyncIntention::DropSync,
                FsEntry::FsMusicFile(mf) => match self.sync_info.get(&mf.relative_path) {
                    None => SyncIntention::Unspecified,
                    Some(e) => e.intention.clone(),
                },
                FsEntry::FsDirectory(d) => self.induce_dir_intention(d),
            })
            .reduce(|acc, v| if acc < v { acc } else { v })
            .unwrap_or(SyncIntention::Unspecified);

        return a;
    }

    fn render_tree(
        &self,
        nodes: &Vec<FsEntry>,
        indent: usize,
    ) -> iced::widget::Column<'_, Message> {
        let mut col = column!().spacing(4);

        for node in nodes {
            let rel_path = match node {
                FsEntry::FsFile(f) => &f.relative_path,
                FsEntry::FsMusicFile(mf) => &mf.relative_path,
                FsEntry::FsDirectory(d) => &d.relative_path,
            };
            let label = rel_path.file_name().unwrap().to_string_lossy().to_string();

            let button = match node {
                FsEntry::FsFile(f) => {
                    button(text(label)).on_press(Message::SelectFile(f.relative_path.clone()))
                }
                FsEntry::FsMusicFile(f) => {
                    let sync_info = self
                        .sync_info
                        .get(&f.relative_path)
                        .expect("Can't get sync info on tree render");

                    let style = ButtonStyle {
                        selected: self.selected == Some(f.relative_path.clone()),
                        has_problem: f.has_problems(),
                        intention: sync_info.intention.clone(),
                    };
                    let prefix = if sync_info.synced { "" } else { "* " };
                    let label = format!("{}{}", prefix, label);

                    button(text(label))
                        .on_press(Message::SelectFile(f.relative_path.clone()))
                        .style(iced::theme::Button::Custom(Box::new(style)))
                }
                FsEntry::FsDirectory(d) => {
                    let synced = self.is_dir_synced(d);
                    let style = ButtonStyle {
                        selected: self.selected == Some(d.relative_path.clone()),
                        has_problem: d.has_problems(),
                        intention: self.induce_dir_intention(d),
                    };

                    let prefix = if synced { "" } else { "* " };
                    let label = format!("{}{}", prefix, label);

                    button(text(label))
                        .on_press(Message::ToggleDir(d.relative_path.clone()))
                        .style(iced::theme::Button::Custom(Box::new(style)))
                }
            };

            col = col.push(container(button).padding([0, 0, 0, (indent as u16) * 16]));

            match node {
                FsEntry::FsDirectory(d) => {
                    if d.expanded {
                        col = col.push(self.render_tree(&d.children, indent + 1));
                    }
                }
                _ => {}
            };
        }

        col
    }
}

// TODO remove
// fn print_tree(tree: &Vec<FsEntry>) {
//     for entry in tree {
//         match entry {
//             FsEntry::FsFile(f) => {
//                 println!("File: {}", f.relative_path.display());
//             }
//             FsEntry::FsDirectory(d) => {
//                 println!("Directory: {}", d.relative_path.display());
//                 print_tree(&d.children);
//             }
//             FsEntry::FsMusicFile(mf) => {
//                 println!("MusicFile: {}", mf.relative_path.display());
//             }
//         }
//     }
// }

async fn load_root_dir(root_path: PathBuf, target_rel_path: PathBuf) -> Vec<FsEntry> {
    return load_dir(root_path, target_rel_path);
}

#[derive(Debug, Clone, Copy)]
enum PaneStyle {
    Left,
    Right,
}

impl container::StyleSheet for PaneStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Theme) -> container::Appearance {
        match self {
            PaneStyle::Left => container::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.50, 0.50, 0.70))),
                border: iced::Border {
                    color: Color::BLACK,
                    width: 2.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            PaneStyle::Right => container::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.80, 0.80, 0.70))),
                border: iced::Border {
                    color: Color::BLACK,
                    width: 2.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }
}

struct ButtonStyle {
    selected: bool,
    has_problem: bool,
    intention: SyncIntention,
}

impl ButtonStyle {
    fn bg_color(&self) -> Color {
        let coef = if self.selected { 0.7 } else { 1.0 };

        match self.intention {
            SyncIntention::KeepSync => Color::from_rgb(0.90 * coef, 0.90 * coef, 0.90 * coef),
            SyncIntention::DropSync => Color::from_rgb(0.80 * coef, 0.60 * coef, 0.60 * coef),
            SyncIntention::Unspecified => Color::from_rgb(0.20 * coef, 0.90 * coef, 0.90 * coef),
        }
    }

    fn text_color(&self) -> Color {
        if self.has_problem {
            return Color::from_rgb(0.90, 0.10, 0.10);
        } else {
            return Color::BLACK;
        }
    }
}

impl button::StyleSheet for ButtonStyle {
    type Style = Theme;

    fn active(&self, _style: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(self.bg_color())),
            text_color: self.text_color(),
            ..Default::default()
        }
    }
}

struct ActionPanelStyle {}
impl container::StyleSheet for ActionPanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.80, 0.40, 0.30))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

struct InfoPanelStyle {}
impl container::StyleSheet for InfoPanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.60, 0.70, 0.10))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

struct TreePanelStyle {}
impl container::StyleSheet for TreePanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.70, 0.70, 0.70))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

fn load_saved_state() -> AppSavedState {
    let path = PathBuf::from(CONFIG_ABS_PATH).join(STATE_FILENAME);
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

    let state: AppSavedState = match serde_json::from_reader(file) {
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
    let path = PathBuf::from(CONFIG_ABS_PATH).join(STATE_FILENAME);
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

    let ss = AppSavedState {
        source,
        destination,
    };

    match serde_json::to_writer_pretty(file, &ss) {
        Err(_) => println!("Failed to write to index.json"),
        Ok(_) => {}
    }

    println!("State saved");
}
