use iced::{
    Application, Command, Element, Length, Settings, Theme, alignment, executor,
    widget::{Column, button, column, container, row, scrollable, text},
};
use iced::{Background, Color};

use crate::music_file::music_file::{Directory, File, MusicFile};
use pathdiff::diff_paths;
use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};

mod music_file;
mod tags;

fn main() -> iced::Result {
    DissonanceApp::run(Settings::default())
}

#[derive(Debug, Clone)]
enum FsEntry {
    FsFile(File),
    FsMusicFile(MusicFile),
    FsDirectory(Directory),
}

impl FsEntry {
    fn from(root_path: &PathBuf, relative_path: &PathBuf) -> FsEntry {
        let abs_path = root_path.clone().join(&relative_path);

        if abs_path.is_dir() {
            let children = load_dir(root_path.clone(), relative_path.clone());
            let d = Directory::new(&root_path, relative_path, children);
            return FsEntry::FsDirectory(d);
        } else {
            let f = File::new(&root_path, &relative_path);
            let mf = MusicFile::from(f.clone());

            match mf {
                None => {
                    return FsEntry::FsFile(f);
                }
                Some(mf) => {
                    return FsEntry::FsMusicFile(mf);
                }
            };
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    SourceSet(PathBuf),
    RootLoaded(Vec<FsEntry>),
    ToggleDir(PathBuf),
    SelectFile(PathBuf),
    FixTags(PathBuf),
    MoveFile(PathBuf),
}

struct DissonanceApp {
    tree: Vec<FsEntry>,
    selected: Option<PathBuf>,
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
}

#[derive(Debug)]
enum Problem {
    MissingTags,
    MismatchedTags,
    MismatchedPath,
    // MissingAlbumArt,
}

fn action_to_message(action: Action, rel_path: PathBuf) -> Message {
    match action {
        Action::FixTags => Message::FixTags(rel_path),
        Action::MoveFile => Message::MoveFile(rel_path),
    }
}

impl Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
enum Action {
    FixTags,
    MoveFile,
    // GetAlbumArt,
    // ApplyCustomTags,
}

impl Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
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

        (
            Self {
                tree: Vec::new(),
                selected: None,
                source: Some(s),
                destination: Some(d),
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
                self.tree = nodes;
                Command::none()
            }

            Message::ToggleDir(path) => {
                toggle_dir(&mut self.tree, &path);
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
            Message::MoveFile(rel_path) => {
                println!("MoveFile: {}", rel_path.to_string_lossy());
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
        let tree_view = scrollable(render_tree(&self.tree, 0))
            .height(Length::Fill)
            .width(Length::Fill);

        let info_panel = self.render_info_panel();
        let actions_panel = self.render_actions_panel();

        row![
            container(tree_view)
                .padding(10)
                .height(Length::Fill)
                .width(Length::FillPortion(1))
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
            FsEntry::FsMusicFile(mf) => find_problems(&mf),
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

        let problems = match entry {
            FsEntry::FsFile(_) => {
                vec![]
            }
            FsEntry::FsDirectory(_) => {
                vec![]
            }
            FsEntry::FsMusicFile(mf) => find_problems(&mf),
        };

        let actions = get_suitable_actions(problems);

        let column = actions.iter().fold(Column::new().spacing(4), |col, a| {
            col.push(
                button(text(a.to_string()).size(16)).on_press(action_to_message(
                    (*a).clone(),
                    self.selected.clone().unwrap(),
                )),
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

    fn fix_tags(&self, path: PathBuf) {
        if self.source.is_none() || self.selected.is_none() {
            return;
        }

        let mf_ref = find_file(&self.tree, path);
        if mf_ref.is_none() {
            return;
        }
        let mf = mf_ref.unwrap();

        let tags = mf.compose_tags_from_path();
        mf.set_tags(&tags);

        println!("Tags set for: {}", mf.relative_path.display());
    }
}

fn find_file(tree: &Vec<FsEntry>, path: PathBuf) -> Option<&MusicFile> {
    for entry in tree {
        if let FsEntry::FsMusicFile(mf) = entry {
            if mf.relative_path == path {
                return Some(&mf);
            }
        }
        if let FsEntry::FsDirectory(d) = entry {
            if let Some(mf) = find_file(&d.children, path.clone()) {
                return Some(mf);
            }
        }
    }
    return None;
}

fn find_problems(mf: &MusicFile) -> Vec<Problem> {
    let mut ret = Vec::<Problem>::new();

    if !mf.tag_available() {
        ret.push(Problem::MissingTags);
    }

    let installed_tags = mf.tags();
    let mut path_tags = mf.compose_tags_from_path();
    path_tags.track_number = installed_tags.track_number.clone();

    if path_tags != installed_tags {
        ret.push(Problem::MismatchedTags);
    }

    let tags_path = mf.compose_path_from_tags(&installed_tags);

    if mf.relative_path != tags_path {
        ret.push(Problem::MismatchedPath);
    }

    return ret;
}

async fn get_source() -> PathBuf {
    let s = PathBuf::from("/home/bakar/tmp/mu/source");
    s
}

async fn load_root_dir(root_path: PathBuf, target_rel_path: PathBuf) -> Vec<FsEntry> {
    return load_dir(root_path, target_rel_path);
}

fn load_dir(root_path: PathBuf, target_rel_path: PathBuf) -> Vec<FsEntry> {
    let mut nodes = Vec::<FsEntry>::new();
    let target_abs_path = root_path.join(&target_rel_path);

    let read_dir = std::fs::read_dir(&target_abs_path);
    if let Err(_) = read_dir {
        return vec![];
    }

    let read_dir = read_dir.unwrap();

    for entry in read_dir {
        if let Err(_) = entry {
            continue;
        }

        let absolute_path = entry.unwrap().path();
        let relative_path = diff_paths(&absolute_path, &root_path)
            .expect("Can't create relative path")
            .to_path_buf();

        let e = FsEntry::from(&root_path, &relative_path);
        nodes.push(e);
    }

    nodes
}

fn toggle_dir(nodes: &mut Vec<FsEntry>, target: &Path) {
    for node in nodes {
        match node {
            FsEntry::FsDirectory(d) => {
                if d.relative_path == target {
                    d.expanded = !d.expanded;
                } else {
                    toggle_dir(&mut d.children, target);
                }
            }
            _ => continue,
        };
    }
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
                button(text(label)).on_press(Message::SelectFile(f.relative_path.clone()))
            }
            FsEntry::FsDirectory(d) => {
                button(text(label)).on_press(Message::ToggleDir(d.relative_path.clone()))
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

fn get_suitable_actions(problems: Vec<Problem>) -> Vec<Action> {
    let mut actions = Vec::new();
    for p in problems {
        match p {
            Problem::MissingTags => {
                actions.push(Action::FixTags);
            }
            Problem::MismatchedTags | Problem::MismatchedPath => {
                actions.push(Action::FixTags);
                actions.push(Action::MoveFile);
            }
        }
    }

    actions
}
