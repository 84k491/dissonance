use futures::SinkExt;
use iced::{
    Alignment, Application, Command, Element, Length, Settings, Subscription, Theme, alignment,
    executor, futures, subscription,
    widget::{Column, TextInput, button, column, container, progress_bar, row, scrollable, text},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    convert::Infallible,
    path::PathBuf,
};

pub(crate) use crate::dissonance::{Problem, SyncIntention, SyncedEntry};
pub(crate) use crate::file_tree::FsEntry;

use crate::{
    dissonance::{
        Action, Dissonance, FilesystemAction, FilesystemActionReport, load_saved_state,
        process_filesystem_action,
    },
    music_file::FsEntryTrait,
    style::{ActionPanelStyle, ButtonStyle, InfoPanelStyle, PaneStyle, TreePanelStyle},
};

mod dissonance;
mod file_tree;
mod music_file;
mod style;
mod tags;

fn main() -> iced::Result {
    DissonanceFrontend::run(Settings::default())
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

    ForceSync(PathBuf),
    KeepSync(PathBuf),
    DropSync(PathBuf),

    StartSync,
    StartIndexing,

    FilesystemActionDone(FilesystemActionReport),
}
struct DissonanceFrontend {
    backend: Dissonance,
    selected: Option<PathBuf>,
    input_source: String,
    input_destination: String,
    sub_iter: usize,
    show_progress_window: bool,
    progress: f32,
}

fn action_to_message(action: &Action, rel_path: PathBuf) -> Message {
    match action {
        Action::FixTags => Message::FixTags(rel_path),
        Action::MoveFile => Message::MoveFile(rel_path),
        Action::RemoveTags => Message::RemoveTags(rel_path),
        Action::KeepSync => Message::KeepSync(rel_path),
        Action::ForceSync => Message::ForceSync(rel_path),
        Action::DropSync => Message::DropSync(rel_path),
        Action::DeleteEntry => Message::DeleteFile(rel_path),
        Action::FixCharacters => Message::FixCharacters(rel_path),
    }
}

impl Application for DissonanceFrontend {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn subscription(&self) -> Subscription<Message> {
        let files: Vec<FilesystemAction> = self.backend.filesystem_actions.clone();

        // TODO hangs if nothing to do
        if files.is_empty() {
            return Subscription::none();
        }

        return subscription::channel(self.sub_iter, 100, move |mut output| async move {
            println!("Processing {} files", files.len());

            let total_size = files.len();
            for (iter, file) in files.iter().enumerate() {
                let res = process_filesystem_action(&file, iter, total_size);

                let _ = output.send(Message::FilesystemActionDone(res)).await;
            }

            // let _ = remove_empty_subdirs::remove_empty_subdirs(&self.destination.clone().unwrap());
            futures::future::pending::<Infallible>().await
        });
    }

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let saved_state = load_saved_state();
        (
            Self {
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
                backend: Dissonance::new(saved_state),
                selected: None,
                sub_iter: 0,
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
                self.input_source = source_abs_path;
                Command::none()
            }
            Message::SourceSubmited => {
                let path = PathBuf::from(self.input_source.clone());
                if path.exists() && path.is_dir() {
                    self.backend.set_source(path);
                }
                Command::none()
            }
            Message::DestinationUpdated(dest_abs_path) => {
                self.input_destination = dest_abs_path;
                Command::none()
            }
            Message::DestinationSubmited => {
                let path = PathBuf::from(self.input_destination.clone());
                if path.exists() && path.is_dir() {
                    self.backend.set_destination(path);
                }
                Command::none()
            }
            Message::StartIndexing => {
                let source = self.backend.source.as_ref().unwrap();
                Command::perform(
                    load_root_dir(source.clone(), source.clone()),
                    Message::RootLoaded,
                )
            }
            Message::RootLoaded(nodes) => {
                self.backend.handle_root_loaded(nodes);
                Command::none()
            }
            Message::ToggleDir(path) => {
                self.backend.file_tree.toggle_dir(&path);
                self.selected = Some(path);
                Command::none()
            }
            Message::SelectFile(path) => {
                self.selected = Some(path);
                Command::none()
            }
            Message::FixTags(rel_path) => {
                self.backend.fix_tags(rel_path);
                Command::none()
            }
            Message::RemoveTags(rel_path) => {
                self.backend.remove_tags(rel_path);
                Command::none()
            }
            Message::MoveFile(rel_path) => {
                self.selected = self.backend.move_entry_to_tag_based_path(rel_path);
                Command::none()
            }
            Message::DeleteFile(rel_path) => {
                self.backend.delete_entry(rel_path);
                Command::none()
            }
            Message::ForceSync(rel_path) => {
                self.backend
                    .set_sync_intention(rel_path, SyncIntention::ForceSync);
                Command::none()
            }
            Message::KeepSync(rel_path) => {
                self.backend
                    .set_sync_intention(rel_path, SyncIntention::KeepSync);
                Command::none()
            }
            Message::DropSync(rel_path) => {
                self.backend
                    .set_sync_intention(rel_path, SyncIntention::DropSync);
                Command::none()
            }
            Message::StartSync => {
                self.backend.sync_with_destination();
                self.show_progress_window = true;
                self.sub_iter += 1;
                Command::none()
            }
            Message::FixCharacters(rel_path) => {
                self.selected = self.backend.fix_characters(&rel_path);
                Command::none()
            }
            Message::FilesystemActionDone(report) => {
                self.progress = report.iter as f32 / report.total_size as f32;
                if report.iter + 1 == report.total_size {
                    self.show_progress_window = false;
                    self.progress = 0.0;
                }

                self.backend.handle_filesystem_action_done(&report);
                Command::none()
            }
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
            Self::progress_window(self.progress)
        } else {
            main
        }
    }
}

impl DissonanceFrontend {
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
        let tree_view = scrollable(self.render_tree(&self.backend.file_tree.entries, 0))
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
        if self.backend.source.is_none() || self.selected.is_none() {
            return column![];
        }
        let selected_path = self.selected.as_ref().unwrap();
        let entry = self.backend.file_tree.find(selected_path);
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

    fn render_actions_panel(&self) -> iced::widget::Column<'static, Message> {
        if self.backend.source.is_none() || self.selected.is_none() {
            return column![];
        }

        let selected_path = self.selected.as_ref().unwrap();
        let entry = self.backend.file_tree.find(selected_path);
        if entry.is_none() {
            return column![];
        }
        let entry = entry.unwrap();

        let actions = self.backend.get_suitable_actions(entry);

        let column = actions.iter().fold(Column::new().spacing(4), |col, a| {
            col.push(
                button(text(a.to_string()).size(16))
                    .on_press(action_to_message(a, self.selected.clone().unwrap())),
            )
        });

        column
    }

    fn render_file_panel(&self) -> iced::widget::Container<'static, Message> {
        if self.backend.source.is_none() || self.selected.is_none() {
            return container(text("No file selected"));
        }

        let entry = self
            .backend
            .file_tree
            .find(&self.selected.as_ref().unwrap());
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

    fn render_tree(
        &self,
        nodes: &BTreeMap<PathBuf, FsEntry>,
        indent: usize,
    ) -> iced::widget::Column<'_, Message> {
        let mut col = column!().spacing(4);

        for (_, node) in nodes {
            let rel_path = node.rel_path();
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
                    let synced = self.backend.is_dir_synced(d);
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
    crate::file_tree::load_dir_hash_set_files_only(root_path, target_rel_path)
}
