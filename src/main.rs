use futures::SinkExt;
use iced::{
    Alignment, Application, Command, Element, Length, Settings, Subscription, Theme, alignment,
    executor, futures, subscription,
    widget::{Column, TextInput, button, column, container, progress_bar, row, scrollable, text},
};
use serde::{Deserialize, Serialize};

use crate::{file_tree::load_dir_hash_set_files_only, music_file::MusicFile};
use crate::{
    file_tree::{FileTree, FsEntry},
    music_file::Directory,
    music_file::FsEntryTrait,
};
use crate::{
    music_file::InvalidFile,
    style::{ActionPanelStyle, ButtonStyle, InfoPanelStyle, PaneStyle, TreePanelStyle},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    convert::Infallible,
    fmt::{self, Display},
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::PathBuf,
};

mod file_tree;
mod music_file;
mod style;
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

    RootLoaded(HashSet<PathBuf>),

    ToggleDir(PathBuf),
    SelectFile(PathBuf),

    FixTags(PathBuf),
    RemoveTags(PathBuf),
    MoveFile(PathBuf),
    DeleteFile(PathBuf),
    FixCharacters(PathBuf),

    KeepSync(PathBuf),
    DropSync(PathBuf),

    StartSync,
    StartIndexing,

    FilesystemActionDone(FilesystemActionReport),
    // ProgressWindowClose,
}

#[derive(Clone, Serialize, Deserialize)]
struct AppSavedState {
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
}

static CONFIG_ABS_PATH: &'static str = "/home/bakar/.config/dissonance"; // TODO
static STATE_FILENAME: &'static str = "saved_state.json";
static INDEX_FILENAME: &'static str = "index.json";

struct DissonanceApp {
    file_tree: FileTree,
    selected: Option<PathBuf>,

    input_source: String,
    input_destination: String,

    source: Option<PathBuf>,
    destination: Option<PathBuf>,

    destination_files: Option<HashSet<PathBuf>>,

    sub_iter: usize,
    filesystem_actions: Vec<FilesystemAction>,

    show_progress_window: bool,
    progress: f32, // 0.0 ..= 1.0
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Problem {
    InvalidCharacters,
    EmptyDirectory,
    InvalidFile,
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
    FixCharacters,
    MoveFile,
    KeepSync,
    DropSync,
    DeleteEntry,
    // GetAlbumArt,
    // ApplyCustomTags,
    // ReinstallTags
    // ConvertToMp3
}

#[derive(Debug, Clone)]
struct CopyFileAction {
    from_base_abs: PathBuf,
    to_base_abs: PathBuf,
    relative: PathBuf,
}

#[derive(Debug, Clone)]
struct RemoveFileAction {
    base_abs: PathBuf,
    relative: PathBuf,
}

#[derive(Debug, Clone)]
enum FilesystemAction {
    Copy(CopyFileAction),
    Remove(RemoveFileAction),
}

#[derive(Debug, Clone)]
struct FilesystemActionReport {
    action: FilesystemAction,
    status: bool,
    iter: usize,
    total_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq, Ord, PartialOrd)]
enum SyncIntention {
    Unspecified, // top prio
    MixedDir,
    KeepSync,
    DropSync, // low prio
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncedEntry {
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
            Action::DeleteEntry => Message::DeleteFile(rel_path),
            Action::FixCharacters => Message::FixCharacters(rel_path),
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
        let index = self.file_tree.create_index();
        Self::save_index(index);
        save_state(self.source.clone(), self.destination.clone());
    }
}

fn process_filesystem_action(
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

            let res = mtp_copy(&from_abs, &to_abs);

            let ok = match res {
                Err(_) => false,
                Ok(_) => true,
            };

            FilesystemActionReport {
                action: FilesystemAction::Copy(copy_action.clone()),
                status: ok,
                iter: iter,
                total_size: total_size,
            }
        }
        FilesystemAction::Remove(remove_action) => {
            let from_abs = remove_action.base_abs.join(&remove_action.relative);
            println!("Removing: {}", from_abs.display());
            std::fs::remove_file(&from_abs).unwrap();

            FilesystemActionReport {
                action: FilesystemAction::Remove(remove_action.clone()),
                status: true,
                iter: iter,
                total_size: total_size,
            }
        }
    }
}

impl Application for DissonanceApp {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn subscription(&self) -> Subscription<Message> {
        let files: Vec<FilesystemAction> = self.filesystem_actions.clone();

        if files.is_empty() {
            return Subscription::none();
        }

        return subscription::channel(
            self.sub_iter,
            100,
            move |mut output| async move {
                println!("Processing {} files", files.len());

                let total_size = files.len();
                for (iter, file) in files.iter().enumerate() {
                    let res = process_filesystem_action(&file, iter, total_size);

                    let _ = output.send(Message::FilesystemActionDone(res)).await;
                }

                futures::future::pending::<Infallible>().await
            },
        );
    }

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let saved_state = load_saved_state();
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
                destination_files: None,
                sub_iter: 0,
                filesystem_actions: Vec::new(),
                show_progress_window: false,
                progress: 0.0,
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
                let index = DissonanceApp::load_index();
                self.file_tree = FileTree::from(nodes, self.source.clone().unwrap(), index);

                println!("Source dir loaded");

                if self.source.is_none() || self.destination.is_none() {
                    return Command::none();
                }

                // TODO do i need it?
                // println!("Indexing source files...");
                // self.update_index_source();

                if self.destination.is_none() {
                    println!("No destination. Skipping");
                    return Command::none();
                }

                println!("Indexing destination files...");

                let dest_entries =
                    load_dir_hash_set_files_only(self.destination.clone().unwrap(), PathBuf::new());
                self.update_index_destination(&dest_entries); // TODO async
                self.destination_files = Some(dest_entries);

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
                self.move_file_to_tag_based_path(rel_path);
                Command::none()
            }
            Message::DeleteFile(rel_path) => {
                self.delete_entry(rel_path);
                Command::none()
            }
            Message::KeepSync(rel_path) => {
                self.set_sync_intention(rel_path, SyncIntention::KeepSync);
                Command::none()
            }
            Message::DropSync(rel_path) => {
                self.set_sync_intention(rel_path, SyncIntention::DropSync);
                Command::none()
            }
            Message::StartSync => {
                self.sync_with_destination();
                self.show_progress_window = true;
                self.sub_iter = self.sub_iter + 1;
                Command::none()
            }
            Message::FixCharacters(rel_path) => {
                self.fix_characters(&rel_path);
                Command::none()
            }

            Message::FilesystemActionDone(report) => {
                self.progress = report.iter as f32 / report.total_size as f32;
                if report.iter + 1 == report.total_size {
                    self.show_progress_window = false;
                    self.progress = 0.0;
                }

                if !report.status {
                    println!("Filesystem action failed: {:?}", report.action);
                    return Command::none();
                }

                match report.action {
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
                        let p = remove_action.relative.clone();
                        dfiles.remove(&p);

                        // does nothing if absent
                        self.file_tree.set_sync_info(
                            &remove_action.relative,
                            SyncedEntry {
                                intention: SyncIntention::DropSync,
                                synced: true,
                            },
                        );
                    }
                }

                Command::none()
            } // Message::ProgressWindowClose => {
              //     self.show_progress_window = false;
              //     self.progress = 0.0;
              //     Command::none()
              // }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let top_panel = self.render_top_panel();
        let main_panel = self.render_main_panel();

        let main = column![
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
        .into();

        if self.show_progress_window {
            return Self::progress_window(self.progress);
        } else {
            return main;
        }
    }
}

impl DissonanceApp {
    fn sync_with_destination(&mut self) {
        println!("Syncing with destination");

        let index = self.file_tree.create_index();

        let mut filesystem_actions: Vec<FilesystemAction> = Vec::<FilesystemAction>::new();

        let unsynced: BTreeMap<PathBuf, SyncedEntry> = index
            .iter()
            .filter(|(_, e)| !e.synced)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let to_remove_from_dest = unsynced
            .iter()
            .filter(|(_, v)| v.intention == SyncIntention::DropSync)
            .map(|(k, _)| {
                FilesystemAction::Remove(RemoveFileAction {
                    base_abs: self.destination.clone().unwrap(),
                    relative: k.clone(),
                })
            })
            .collect::<Vec<FilesystemAction>>();

        println!("Drop sync for {} files", to_remove_from_dest.len());
        filesystem_actions.extend(to_remove_from_dest);

        // remove those entries that are in dest, but not in index
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
            .collect::<Vec<FilesystemAction>>();

        filesystem_actions.extend(destination_extra);

        let to_copy_to_dest = unsynced
            .iter()
            .filter(|(_, v)| v.intention == SyncIntention::KeepSync)
            .map(|(k, _)| {
                FilesystemAction::Copy(CopyFileAction {
                    from_base_abs: self.source.clone().unwrap(),
                    to_base_abs: self.destination.clone().unwrap(),
                    relative: k.clone(),
                })
            })
            .collect::<Vec<FilesystemAction>>();

        filesystem_actions.extend(to_copy_to_dest);

        self.filesystem_actions = filesystem_actions;

        // TODO

        // println!("Removing empty subdirs");
        // let _ = remove_empty_subdirs::remove_empty_subdirs(&self.destination.clone().unwrap());
        //
        // println!("Finished sync");
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

    fn set_sync_intention(&mut self, rel_path: PathBuf, intention: SyncIntention) {
        let e = self.file_tree.find(&rel_path);
        match e {
            Some(FsEntry::FsMusicFile(_)) => {}
            Some(FsEntry::FsDirectory(d)) => {
                let ch = d.children_recursive();
                for c in ch {
                    self.set_sync_intention(c.clone(), intention.clone());
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
            SyncIntention::Unspecified => false,
            SyncIntention::MixedDir => false,
        };

        self.file_tree
            .set_sync_info(&rel_path, SyncedEntry { intention, synced });
    }

    fn save_index(index: BTreeMap<PathBuf, SyncedEntry>) {
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

    fn update_index_destination(&mut self, dest_entries: &HashSet<PathBuf>) {
        // update existing entries
        // TODO don't create index?
        let sync_info = self.file_tree.create_index();
        for (rel_path, e) in sync_info.iter() {
            let is_in_dest = dest_entries.contains(rel_path);

            match e.intention {
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
        let selected_path = self.selected.as_ref().unwrap();
        let entry = self.file_tree.find(selected_path);
        if entry.is_none() {
            return column![];
        }
        let entry = entry.unwrap();

        let problems = match entry {
            FsEntry::FsFile(_) => BTreeSet::<Problem>::new(),
            FsEntry::FsDirectory(_) => BTreeSet::<Problem>::new(),
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

    fn get_suitable_actions_for_invalid_file(&self, file: &InvalidFile) -> BTreeSet<Action> {
        let mut actions = BTreeSet::<Action>::new();

        let problems = file.find_problems();

        for p in problems {
            match p {
                Problem::InvalidFile => {
                    actions.insert(Action::DeleteEntry);
                }
                _ => {}
            }
        }

        return actions;
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
                    _ => {}
                }
            }
        } else {
            actions.insert(Action::RemoveTags);
        }

        match mf.sync_data.intention {
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
        let mut actions = BTreeSet::<Action>::new();

        match d.intention() {
            SyncIntention::KeepSync => {
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

        let problems = d.find_problems();
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
                Problem::InvalidFile | Problem::EmptyDirectory => {
                    actions.insert(Action::DeleteEntry);
                }
                Problem::InvalidCharacters => {
                    actions.insert(Action::FixCharacters);
                }
            }
        }

        actions
    }

    fn get_suitable_actions(&self, entry: &FsEntry) -> BTreeSet<Action> {
        let actions = match entry {
            FsEntry::FsMusicFile(mf) => self.get_suitable_actions_for_music_file(mf),
            FsEntry::FsDirectory(d) => self.get_suitable_actions_for_dir(d),
            FsEntry::FsFile(f) => self.get_suitable_actions_for_invalid_file(f),
        };
        return actions;
    }

    fn render_actions_panel(&self) -> iced::widget::Column<'static, Message> {
        if self.source.is_none() || self.selected.is_none() {
            return column![];
        }

        let selected_path = self.selected.as_ref().unwrap();
        let entry = self.file_tree.find(selected_path);
        if entry.is_none() {
            return column![];
        }
        let entry = entry.unwrap();

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

        let entry_opt = self.file_tree.find(&path);
        let entry = match entry_opt {
            Some(e) => e,
            _ => return,
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
                let children: Vec<PathBuf> =
                    d.children.iter().map(|e| e.rel_path().clone()).collect();
                for child in children {
                    self.fix_tags(child);
                }
            }
            _ => {}
        };
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

        let sync_data = mf.sync_data.clone();

        if !self.file_tree.remove_entry(&path) {
            println!("Failed to forget file: {}", path.display());
        }
        self.file_tree.add_entry(&path, sync_data);
    }

    fn delete_entry(&mut self, rel_path: PathBuf) {
        let mut abs_path = self.source.clone().unwrap().to_path_buf();
        abs_path.push(&rel_path);

        // rm -r dir
        if abs_path.is_dir() {
            // let files = load_dir(self.source.as_ref().unwrap().clone(), rel_path.clone());
            //
            // for f in files.iter() {
            //     println!("Removing file: {}", f.rel_path().display());
            // }
            //
            // files.into_iter().for_each(|f| {
            //     let rp = f.rel_path().clone();
            //
            //     self.file_tree.remove_entry(&rp);
            //     self.sync_info.remove_entry(&rp);
            //     self.delete_entry(rp);
            // });
            //
            // match fs::remove_dir_all(abs_path) {
            //     Ok(()) => {}
            //     Err(e) => {
            //         println!("Failed to remove directory {}: {}", rel_path.display(), e);
            //     }
            // }
            // self.file_tree.remove_entry(&rel_path);
        } else {
            // rm file
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

    fn move_file_to_tag_based_path(&mut self, path: PathBuf) {
        let (rel_path, tag_based_rel_path) = {
            let mf_opt = self.file_tree.find(&path);
            let file = match mf_opt {
                Some(FsEntry::FsMusicFile(mf)) => mf,
                Some(FsEntry::FsDirectory(d)) => {
                    let children: Vec<PathBuf> =
                        d.children.iter().map(|e| e.rel_path().clone()).collect();

                    let dir_path = d.relative_path.clone();
                    for child in children {
                        self.move_file_to_tag_based_path(child);
                    }

                    self.file_tree.remove_entry(&dir_path);

                    return;
                }
                _ => return,
            };

            (
                file.relative_path.clone(),
                file.compose_path_from_tags(&file.tags()),
            )
        };

        self.move_file(rel_path, tag_based_rel_path.clone());

        self.selected = Some(tag_based_rel_path);
    }

    fn move_file(&mut self, from: PathBuf, to: PathBuf) {
        let mut source_full_path = self.source.clone().unwrap().to_path_buf();
        source_full_path.push(&from);

        let mut target_full_path = self.source.clone().unwrap().to_path_buf();
        target_full_path.push(&to);

        let _ = fs::create_dir_all(target_full_path.parent().unwrap());
        let move_res = fs::rename(&source_full_path, &target_full_path);
        if let Err(err) = move_res {
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

    fn is_dir_synced(&self, dir: &Directory) -> bool {
        dir.children
            .iter()
            .map(|c| match c {
                FsEntry::FsFile(_) => false,
                FsEntry::FsMusicFile(mf) => mf.sync_data.synced,
                FsEntry::FsDirectory(d) => self.is_dir_synced(d),
            })
            .reduce(|acc, v| acc && v)
            .unwrap_or(false)
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
                    let style = ButtonStyle {
                        selected: self.selected == Some(f.relative_path.clone()),
                        has_problem: f.has_problems(),
                        intention: f.sync_data.intention.clone(),
                    };
                    let prefix = if f.sync_data.synced { "" } else { "* " };
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
                        intention: d.intention(),
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

    fn fix_characters(&mut self, old_path: &PathBuf) {
        let entry_opt = self.file_tree.find(&old_path);
        let entry = match entry_opt {
            Some(e) => e.clone(), // only need to read entry and remove it
            _ => return,
        };

        match entry {
            FsEntry::FsMusicFile(mf) => {
                let mut new_path_str = mf.relative_path.to_string_lossy().to_string();
                crate::tags::tags::INVALID_CHARS.iter().for_each(|c| {
                    new_path_str = new_path_str.replace(c, "_");
                });

                let new_path = PathBuf::from(new_path_str);

                self.move_file(old_path.clone(), new_path.clone());
            }
            FsEntry::FsDirectory(d) => {
                {
                    let children: Vec<PathBuf> =
                        d.children.iter().map(|e| e.rel_path().clone()).collect();

                    for child in children {
                        self.fix_characters(&child);
                    }
                }
                self.file_tree.remove_entry(&d.relative_path.clone());

                let _ = remove_empty_subdirs::remove_empty_subdirs(&self.source.clone().unwrap());
            }
            _ => {}
        }
    }

    fn progress_window(progress: f32) -> Element<'static, Message> {
        container(
            column![
                text("Processing…"),
                progress_bar(0.0..=1.0, progress).width(Length::Fill),
            ]
            .spacing(16)
            .align_items(Alignment::Center),
        )
        .padding(20)
        // .width(300)
        // .style(|_| iced::theme::Container::Box)
        .into()
    }
}

async fn load_root_dir(root_path: PathBuf, target_rel_path: PathBuf) -> HashSet<PathBuf> {
    println!("Scanning source dir");
    return load_dir_hash_set_files_only(root_path, target_rel_path);
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
