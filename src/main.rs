use iced::{
    Application, Command, Element, Length, Settings, Theme, alignment, executor,
    widget::{button, column, container, row, scrollable, text},
};
use pathdiff::diff_paths;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
enum Message {
    SourceSet(PathBuf),
    RootLoaded(Vec<FsEntry>),
    ToggleDir(PathBuf),
    SelectFile(PathBuf),
}

struct DissonanceApp {
    tree: Vec<FsEntry>,
    selected: Option<PathBuf>,
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
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
            Message::SourceSet(root_path) => {
                Command::perform(load_root_dir(root_path.clone(), root_path), Message::RootLoaded)
            }

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
        let actions_panel = render_actions_panel();

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
        ]
    }

    fn render_info_panel(&self) -> iced::widget::Container<'static, Message> {
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
        let is_dir = absolute_path.is_dir();
        let relative_path = diff_paths(&absolute_path, &root_path)
            .expect("Can't create relative path")
            .to_path_buf();

        if is_dir {
            let children = load_dir(root_path.clone(), relative_path.clone());
            let d = Directory::new(&root_path, relative_path, children);
            nodes.push(FsEntry::FsDirectory(d));
        } else {
            let f = File::new(&root_path, &relative_path);
            let mf = MusicFile::from(f.clone());

            match mf {
                None => {
                    nodes.push(FsEntry::FsFile(f));
                }
                Some(mf) => {
                    nodes.push(FsEntry::FsMusicFile(mf));
                }
            };
        }
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

use iced::{Background, Color};

use crate::music_file::music_file::{Directory, File, MusicFile};

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

fn render_actions_panel() -> iced::widget::Container<'static, Message> {
    container(text("Actions").size(16))
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .style(iced::theme::Container::Custom(Box::new(
            ActionPanelStyle {},
        )))
}
