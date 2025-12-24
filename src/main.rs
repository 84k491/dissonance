use iced::{
    Application, Command, Element, Length, Settings, Theme, alignment, executor,
    widget::{button, column, container, row, scrollable, text},
};
use std::path::{Path, PathBuf};

fn main() -> iced::Result {
    DissonanceApp::run(Settings::default())
}

#[derive(Debug, Clone)]
struct FsNode {
    path: PathBuf,
    is_dir: bool,
    expanded: bool,
    children: Vec<FsNode>,
}

#[derive(Debug, Clone)]
enum Message {
    RootLoaded(Vec<FsNode>),
    ToggleDir(PathBuf),
    SelectFile(PathBuf),
}

struct DissonanceApp {
    tree: Vec<FsNode>,
    selected: Option<PathBuf>,
}

impl Application for DissonanceApp {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        (
            Self {
                tree: Vec::new(),
                selected: None,
            },
            Command::perform(
                load_dir(std::env::current_dir().unwrap()),
                Message::RootLoaded,
            ),
        )
    }

    fn title(&self) -> String {
        "Iced File Tree".into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
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
        let top_panel = render_top_panel();
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
    fn render_main_panel(&self) -> iced::widget::Row<'_, Message> {
        let tree_view = scrollable(render_tree(&self.tree, 0))
            .height(Length::Fill)
            .width(Length::Fill);

        let line = self
            .selected
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No file selected".into());

        let info_panel = render_info_panel(line);
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
}

async fn load_dir(path: PathBuf) -> Vec<FsNode> {
    let mut nodes = Vec::new();

    if let Ok(read_dir) = std::fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_dir = path.is_dir();

            nodes.push(FsNode {
                path,
                is_dir,
                expanded: false,
                children: Vec::new(),
            });
        }
    }

    nodes.sort_by_key(|n| (!n.is_dir, n.path.clone()));
    nodes
}

/* =========================
Tree Helpers
========================= */

fn toggle_dir(nodes: &mut [FsNode], target: &Path) {
    for node in nodes {
        if node.path == target && node.is_dir {
            if !node.expanded {
                if let Ok(read_dir) = std::fs::read_dir(&node.path) {
                    node.children = read_dir
                        .flatten()
                        .map(|e| {
                            let path = e.path();
                            FsNode {
                                is_dir: path.is_dir(),
                                path,
                                expanded: false,
                                children: Vec::new(),
                            }
                        })
                        .collect();
                }
            }
            node.expanded = !node.expanded;
            return;
        }

        if node.expanded {
            toggle_dir(&mut node.children, target);
        }
    }
}

use iced::{Background, Color};

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

fn render_tree(nodes: &[FsNode], indent: usize) -> iced::widget::Column<'_, Message> {
    let mut col = column!().spacing(4);

    for node in nodes {
        let name = node
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| node.path.display().to_string());

        let label = if node.is_dir {
            if node.expanded {
                format!("V {}", name)
            } else {
                format!("> {}", name)
            }
        } else {
            format!("  {}", name)
        };

        let button = if node.is_dir {
            button(text(label)).on_press(Message::ToggleDir(node.path.clone()))
        } else {
            button(text(label)).on_press(Message::SelectFile(node.path.clone()))
        };

        col = col.push(container(button).padding([0, 0, 0, (indent as u16) * 16]));

        if node.is_dir && node.expanded {
            col = col.push(render_tree(&node.children, indent + 1));
        }
    }

    col
}

fn render_top_panel() -> iced::widget::Row<'static, Message> {
    row![]
}

fn render_info_panel(line: String) -> iced::widget::Container<'static, Message> {
    container(text(line.clone()).size(16))
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .style(iced::theme::Container::Custom(Box::new(InfoPanelStyle {})))
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
