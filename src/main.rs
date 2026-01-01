use iced::{
    Application, Command, Element, Length, Settings, Theme, alignment, executor,
    widget::{Column, button, column, container, row, scrollable, text},
};
use iced::{Background, Color};
use serde::{Deserialize, Serialize};

use crate::file_tree::file_tree::{FileTree, FsEntry, load_dir};
use crate::music_file::music_file::MusicFile;
use std::{
    collections::{BTreeMap, HashSet},
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
    SourceSet(PathBuf),
    RootLoaded(Vec<FsEntry>),
    ToggleDir(PathBuf),
    SelectFile(PathBuf),
    FixTags(PathBuf),
    RemoveTags(PathBuf),
    MoveFile(PathBuf),
}

struct DissonanceApp {
    file_tree: FileTree,
    selected: Option<PathBuf>,

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

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum Action {
    RemoveTags,
    FixTags,
    MoveFile,
    // GetAlbumArt,
    // ApplyCustomTags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SyncIntention {
    KeepSync,
    DropSync,
    Unspecified,
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
    }
}

impl Application for DissonanceApp {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let s = PathBuf::from("/home/bakar/tmp/mu/source");
        let d = PathBuf::from("/home/bakar/tmp/mu/dest");

        let index = DissonanceApp::load_index();
        (
            Self {
                file_tree: FileTree::empty(),
                selected: None,
                source: Some(s),
                destination: Some(d),
                sync_info: index,
            },
            Command::perform(get_source(), Message::SourceSet),
        )
    }

    fn title(&self) -> String {
        "Dissonance".into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SourceSet(root_path) => Command::perform(
                load_root_dir(root_path.clone(), root_path),
                Message::RootLoaded,
            ),

            Message::RootLoaded(nodes) => {
                self.file_tree = FileTree::from(nodes);
                self.update_index_source();
                println!(
                    "Index updated with source files: {} files in total",
                    self.sync_info.len()
                );
                self.update_index_destination();
                println!(
                    "Index updated with destination files: {} files in total",
                    self.sync_info.len()
                );
                Command::none()
            }

            Message::ToggleDir(path) => {
                self.file_tree.toggle_dir(&path);
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
    fn load_index() -> BTreeMap<PathBuf, SyncedEntry> {
        let file = match File::open("index.json") {
            Err(_) => return BTreeMap::new(),
            Ok(f) => f,
        };

        let map: BTreeMap<PathBuf, SyncedEntry> = match serde_json::from_reader(file) {
            Err(_) => return BTreeMap::new(),
            Ok(m) => m,
        };

        println!("Index loaded from json: {} files", map.len());
        map
    }

    fn save_index(index: &BTreeMap<PathBuf, SyncedEntry>) {
        let file = match File::create("index.json") {
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

        println!("Index saved to json: {} files", index.len());
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
        let source_str: Option<String> = self
            .source
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        let dest_str: Option<String> = self
            .destination
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        let targets = column![
            text(format!(
                "Source: {}",
                source_str.unwrap_or_else(|| String::from("<unset>"))
            )),
            text(format!(
                "Destination: {}",
                dest_str.unwrap_or_else(|| String::from("<unset>"))
            )),
        ];

        row![targets]
    }

    fn render_main_panel(&self) -> iced::widget::Row<'_, Message> {
        let tree_view = scrollable(render_tree(&self.file_tree.entries, 0))
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

    fn render_actions_panel(&self) -> iced::widget::Column<'static, Message> {
        if self.source.is_none() || self.selected.is_none() {
            return column![];
        }
        let entry = FsEntry::from(
            self.source.as_ref().unwrap(),
            self.selected.as_ref().unwrap(),
        );

        let actions = get_suitable_actions(&entry);

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

        let mf = MusicFile::new(
            &self.source.as_ref().unwrap(),
            &self.selected.as_ref().unwrap(),
        );
        if mf.is_none() {
            return container(text("Not a music file"));
        }
        let mf = mf.unwrap();

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
    }
}

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

async fn get_source() -> PathBuf {
    let s = PathBuf::from("/home/bakar/tmp/mu/source");
    s
}

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

#[derive(Debug, Clone, Copy)]
enum ButtonStyle {
    Correct,
    Problematic,
}

impl button::StyleSheet for ButtonStyle {
    type Style = Theme;

    fn active(&self, _style: &Theme) -> button::Appearance {
        match self {
            ButtonStyle::Correct => button::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.80, 0.80, 0.80))),
                ..Default::default()
            },
            ButtonStyle::Problematic => button::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.80, 0.80, 0.60))),
                ..Default::default()
            },
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

fn render_tree(nodes: &Vec<FsEntry>, indent: usize) -> iced::widget::Column<'_, Message> {
    let mut col = column!().spacing(4);

    for node in nodes {
        let rel_path = match node {
            FsEntry::FsFile(f) => &f.relative_path,
            FsEntry::FsMusicFile(mf) => &mf.relative_path,
            FsEntry::FsDirectory(d) => &d.relative_path,
        };
        let name = rel_path.file_name().unwrap().to_string_lossy().to_string();

        let label = match node {
            FsEntry::FsFile(_) => format!("  {}", name),
            FsEntry::FsMusicFile(_) => format!("  {}", name),
            FsEntry::FsDirectory(d) => {
                if d.expanded {
                    format!("V {}", name)
                } else {
                    format!("> {}", name)
                }
            }
        };

        let button = match node {
            FsEntry::FsFile(f) => {
                button(text(label)).on_press(Message::SelectFile(f.relative_path.clone()))
            }
            FsEntry::FsMusicFile(f) => {
                let style_enum = if f.has_problems() {
                    ButtonStyle::Problematic
                } else {
                    ButtonStyle::Correct
                };
                button(text(label))
                    .on_press(Message::SelectFile(f.relative_path.clone()))
                    .style(iced::theme::Button::Custom(Box::new(style_enum)))
            }
            FsEntry::FsDirectory(d) => {
                let style_enum = if d.has_problems() {
                    ButtonStyle::Problematic
                } else {
                    ButtonStyle::Correct
                };
                button(text(label))
                    .on_press(Message::ToggleDir(d.relative_path.clone()))
                    .style(iced::theme::Button::Custom(Box::new(style_enum)))
            }
        };

        col = col.push(container(button).padding([0, 0, 0, (indent as u16) * 16]));

        match node {
            FsEntry::FsDirectory(d) => {
                if d.expanded {
                    col = col.push(render_tree(&d.children, indent + 1));
                }
            }
            _ => {}
        };
    }

    col
}

fn get_suitable_actions(entry: &FsEntry) -> HashSet<Action> {
    let problems = match entry {
        FsEntry::FsFile(_) => {
            // TODO add "remove" action
            vec![]
        }
        FsEntry::FsDirectory(_) => {
            vec![]
        }
        FsEntry::FsMusicFile(mf) => mf.find_problems(),
    };

    // TODO don't let fix tags if file doesn't have both parents

    let mut actions = HashSet::<Action>::new();

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

    actions
}
