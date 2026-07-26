use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use slint::{ModelRc, VecModel};

pub(crate) use crate::dissonance::{Problem, SyncIntention, SyncedEntry};
pub(crate) use crate::file_tree::FsEntry;

use crate::{
    dissonance::{Dissonance, load_saved_state, process_filesystem_action},
    file_tree::load_dir_hash_set_files_only,
    music_file::FsEntryTrait,
};

mod dissonance;
mod file_tree;
mod music_file;
mod tags;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let saved_state = load_saved_state();

    let source_str = saved_state
        .source
        .clone()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest_str = saved_state
        .destination
        .clone()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let ui = AppWindow::new()?;
    let backend = Arc::new(Mutex::new(Dissonance::new(saved_state)));
    let selected: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

    ui.set_source(source_str.into());
    ui.set_destination(dest_str.into());

    // Source / destination submitted (Enter pressed)
    ui.on_source_accepted({
        let weak = ui.as_weak();
        let backend = backend.clone();
        move || {
            let ui = weak.upgrade().unwrap();
            let path = PathBuf::from(ui.get_source().to_string());
            if path.exists() && path.is_dir() {
                backend.lock().unwrap().set_source(path);
            }
        }
    });

    ui.on_destination_accepted({
        let weak = ui.as_weak();
        let backend = backend.clone();
        move || {
            let ui = weak.upgrade().unwrap();
            let path = PathBuf::from(ui.get_destination().to_string());
            if path.exists() && path.is_dir() {
                backend.lock().unwrap().set_destination(path);
            }
        }
    });

    // Index: scan the source directory on a background thread, then rebuild the tree.
    ui.on_start_index({
        let weak = ui.as_weak();
        let backend = backend.clone();
        let selected = selected.clone();
        move || {
            let source = { backend.lock().unwrap().source.clone() };
            let Some(source) = source else {
                return;
            };
            let weak = weak.clone();
            let backend = backend.clone();
            let selected = selected.clone();
            thread::spawn(move || {
                let nodes = load_dir_hash_set_files_only(source.clone(), source.clone());
                weak.upgrade_in_event_loop(move |ui| {
                    let mut b = backend.lock().unwrap();
                    b.handle_root_loaded(nodes);
                    let s = selected.lock().unwrap();
                    refresh(&ui, &b, &s);
                })
                .ok();
            });
        }
    });

    // Sync: build the filesystem actions, then apply them on a background thread,
    // reporting progress back to the UI for each processed file.
    ui.on_start_sync({
        let weak = ui.as_weak();
        let backend = backend.clone();
        let selected = selected.clone();
        move || {
            let ui = weak.upgrade().unwrap();
            let actions = {
                let mut b = backend.lock().unwrap();
                b.sync_with_destination();
                b.filesystem_actions.clone()
            };
            ui.set_progress(0.0);
            ui.set_show_progress(true);
            if actions.is_empty() {
                ui.set_show_progress(false);
                return;
            }

            let weak = weak.clone();
            let backend = backend.clone();
            let selected = selected.clone();
            thread::spawn(move || {
                let total = actions.len();
                for (i, action) in actions.iter().enumerate() {
                    let report = process_filesystem_action(action, i, total);
                    let backend = backend.clone();
                    let selected = selected.clone();
                    weak.upgrade_in_event_loop(move |ui| {
                        let mut b = backend.lock().unwrap();
                        ui.set_progress(report.iter as f32 / report.total_size as f32);
                        if report.iter + 1 == report.total_size {
                            ui.set_show_progress(false);
                            ui.set_progress(0.0);
                        }
                        b.handle_filesystem_action_done(&report);
                        let s = selected.lock().unwrap();
                        refresh(&ui, &b, &s);
                    })
                    .ok();
                }
            });
        }
    });

    // Tree row clicked: toggle a directory, or select any entry.
    ui.on_row_clicked({
        let weak = ui.as_weak();
        let backend = backend.clone();
        let selected = selected.clone();
        move |path| {
            let ui = weak.upgrade().unwrap();
            let path = PathBuf::from(path.to_string());
            let mut b = backend.lock().unwrap();
            let is_dir = matches!(b.file_tree.find(&path), Some(FsEntry::FsDirectory(_)));
            if is_dir {
                b.file_tree.toggle_dir(&path);
            }
            let mut s = selected.lock().unwrap();
            *s = Some(path);
            refresh(&ui, &b, &s);
        }
    });

    // Action button clicked: apply it to the currently selected entry.
    ui.on_action_clicked({
        let weak = ui.as_weak();
        let backend = backend.clone();
        let selected = selected.clone();
        move |action| {
            let ui = weak.upgrade().unwrap();
            let path = { selected.lock().unwrap().clone() };
            let Some(path) = path else {
                return;
            };
            let mut b = backend.lock().unwrap();
            match action.as_str() {
                "FixTags" => b.fix_tags(path),
                "RemoveTags" => b.remove_tags(path),
                "MoveFile" => {
                    let new_path = b.move_entry_to_tag_based_path(path);
                    *selected.lock().unwrap() = new_path;
                }
                "DeleteEntry" => b.delete_entry(path),
                "FixCharacters" => {
                    let new_path = b.fix_characters(&path);
                    *selected.lock().unwrap() = new_path;
                }
                "ForceSync" => b.set_sync_intention(path, SyncIntention::ForceSync),
                "KeepSync" => b.set_sync_intention(path, SyncIntention::KeepSync),
                "DropSync" => b.set_sync_intention(path, SyncIntention::DropSync),
                _ => {}
            }
            let s = selected.lock().unwrap();
            refresh(&ui, &b, &s);
        }
    });

    // Initial paint.
    {
        let b = backend.lock().unwrap();
        let s = selected.lock().unwrap();
        refresh(&ui, &b, &s);
    }

    ui.run()
}

/// Recompute every UI-exposed model from the current backend state.
fn refresh(ui: &AppWindow, backend: &Dissonance, selected: &Option<PathBuf>) {
    let mut tree_rows = Vec::new();
    flatten_tree(backend, &backend.file_tree.entries, 0, &mut tree_rows);
    ui.set_tree(ModelRc::new(VecModel::from(tree_rows)));

    let (info, problems, actions) = build_panels(backend, selected);
    ui.set_info_lines(ModelRc::new(VecModel::from(info)));
    ui.set_problems(ModelRc::new(VecModel::from(problems)));
    ui.set_actions(ModelRc::new(VecModel::from(actions)));
}

/// Flatten the (partially expanded) tree into a list of rows, mirroring the
/// recursive rendering the iced UI used.
fn flatten_tree(
    backend: &Dissonance,
    nodes: &BTreeMap<PathBuf, FsEntry>,
    indent: i32,
    out: &mut Vec<TreeRow>,
) {
    for node in nodes.values() {
        let rel_path = node.rel_path();
        let name = rel_path.file_name().unwrap().to_string_lossy().into_owned();

        let label = match node {
            FsEntry::FsFile(_) => name,
            FsEntry::FsMusicFile(f) => {
                let prefix = if f.sync_data.synced { "" } else { "* " };
                format!("{prefix}{name}")
            }
            FsEntry::FsDirectory(d) => {
                let prefix = if backend.is_dir_synced(d) { "" } else { "* " };
                format!("{prefix}{name}")
            }
        };

        out.push(TreeRow {
            label: label.into(),
            indent,
            path: rel_path.to_string_lossy().into_owned().into(),
        });

        if let FsEntry::FsDirectory(d) = node {
            if d.expanded {
                flatten_tree(backend, &d.children, indent + 1, out);
            }
        }
    }
}

/// Build the info lines, problem list and action buttons for the selected entry.
fn build_panels(
    backend: &Dissonance,
    selected: &Option<PathBuf>,
) -> (Vec<TextRow>, Vec<TextRow>, Vec<ActionRow>) {
    let mut info = Vec::new();
    let mut problems = Vec::new();
    let mut actions = Vec::new();

    if backend.source.is_none() || selected.is_none() {
        info.push(TextRow {
            text: "No file selected".into(),
        });
        return (info, problems, actions);
    }

    let selected = selected.as_ref().unwrap();
    let entry = match backend.file_tree.find(selected) {
        Some(entry) => entry,
        None => {
            info.push(TextRow {
                text: "File not found".into(),
            });
            return (info, problems, actions);
        }
    };

    match entry {
        FsEntry::FsFile(f) => info.push(TextRow {
            text: f.relative_path.display().to_string().into(),
        }),
        FsEntry::FsDirectory(d) => info.push(TextRow {
            text: d.relative_path.display().to_string().into(),
        }),
        FsEntry::FsMusicFile(mf) => {
            let tags = mf.tags();
            info.push(TextRow {
                text: tags.artist.into(),
            });
            info.push(TextRow {
                text: tags.album.into(),
            });
            info.push(TextRow {
                text: tags.title.into(),
            });
        }
    }

    if let FsEntry::FsMusicFile(mf) = entry {
        for problem in mf.find_problems() {
            problems.push(TextRow {
                text: problem.to_string().into(),
            });
        }
    }

    for action in backend.get_suitable_actions(entry) {
        actions.push(ActionRow {
            label: action.to_string().into(),
            action: action.to_string().into(),
        });
    }

    (info, problems, actions)
}
