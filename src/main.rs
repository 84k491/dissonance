use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
};

use slint::{Color, ModelRc, VecModel, Weak};

pub(crate) use crate::dissonance::{Problem, SyncIntention, SyncedEntry};
pub(crate) use crate::directory::FsEntry;

use crate::{
    dissonance::{
        Dissonance, FilesystemAction, FilesystemActionReport, load_saved_state,
        process_filesystem_action,
    },
    directory::load_dir_hash_set_files_only,
    music_file::FsEntryTrait,
};

mod dissonance;
mod directory;
mod music_file;
mod tags;

slint::include_modules!();

type Backend = Arc<Mutex<Dissonance>>;
type Selected = Arc<Mutex<Option<PathBuf>>>;

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
    // Keeps the current progress window alive; lives on the UI thread only.
    let progress_window: Rc<RefCell<Option<ProgressWindow>>> = Rc::new(RefCell::new(None));

    ui.set_source(source_str.into());
    ui.set_destination(dest_str.into());

    // Source / destination submitted (Enter pressed)
    ui.on_source_accepted(on_source_accepted(ui.as_weak(), backend.clone()));
    ui.on_destination_accepted(on_destination_accepted(ui.as_weak(), backend.clone()));

    ui.on_start_index(on_start_index(
        ui.as_weak(),
        backend.clone(),
        selected.clone(),
    ));

    ui.on_start_sync(on_start_sync(
        ui.as_weak(),
        backend.clone(),
        selected.clone(),
        progress_window.clone(),
    ));

    ui.on_row_clicked(on_row_clicked(
        ui.as_weak(),
        backend.clone(),
        selected.clone(),
    ));

    ui.on_action_clicked(on_action_clicked(
        ui.as_weak(),
        backend.clone(),
        selected.clone(),
    ));

    // Initial paint.
    {
        let b = backend.lock().unwrap();
        let s = selected.lock().unwrap();
        refresh(&ui, &b, &s);
    }

    ui.run()
}

/// Store the source directory typed into the main window, if it exists.
fn on_source_accepted(weak: Weak<AppWindow>, backend: Backend) -> impl FnMut() + 'static {
    move || {
        let ui = weak.upgrade().unwrap();
        let path = PathBuf::from(ui.get_source().to_string());
        if path.exists() && path.is_dir() {
            backend.lock().unwrap().set_source(path);
        }
    }
}

/// Store the destination directory typed into the main window, if it exists.
fn on_destination_accepted(weak: Weak<AppWindow>, backend: Backend) -> impl FnMut() + 'static {
    move || {
        let ui = weak.upgrade().unwrap();
        let path = PathBuf::from(ui.get_destination().to_string());
        if path.exists() && path.is_dir() {
            backend.lock().unwrap().set_destination(path);
        }
    }
}

/// Index: scan the source directory on a background thread, then rebuild the tree.
fn on_start_index(
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
) -> impl FnMut() + 'static {
    move || {
        let source = { backend.lock().unwrap().source.clone() };
        let Some(source) = source else {
            return;
        };
        let weak = weak.clone();
        let backend = backend.clone();
        let selected = selected.clone();
        thread::spawn(move || scan_source(source, weak, backend, selected));
    }
}

/// Background half of the indexing: walk the source tree, then hand the result
/// back to the UI thread.
fn scan_source(source: PathBuf, weak: Weak<AppWindow>, backend: Backend, selected: Selected) {
    let nodes = load_dir_hash_set_files_only(source.clone(), source);
    weak.upgrade_in_event_loop(move |ui| apply_scanned_source(ui, nodes, backend, selected))
        .ok();
}

/// UI-thread half of the indexing: adopt the scanned nodes and repaint.
fn apply_scanned_source(
    ui: AppWindow,
    nodes: HashSet<PathBuf>,
    backend: Backend,
    selected: Selected,
) {
    let mut b = backend.lock().unwrap();
    b.handle_root_loaded(nodes);
    let s = selected.lock().unwrap();
    refresh(&ui, &b, &s);
}

/// Sync: build the filesystem actions, open a separate progress window, then
/// apply the actions on a background thread, reporting progress to that window
/// and refreshing the main window as each file completes. The progress window
/// stays open until the user closes it.
fn on_start_sync(
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
    progress_window: Rc<RefCell<Option<ProgressWindow>>>,
) -> impl FnMut() + 'static {
    move || {
        let actions = {
            let mut b = backend.lock().unwrap();
            b.sync_with_destination();
            b.filesystem_actions.clone()
        };
        if actions.is_empty() {
            return;
        }

        // Open a fresh progress window, replacing (and dropping) any previous one.
        let pw = match ProgressWindow::new() {
            Ok(pw) => pw,
            Err(_) => return,
        };
        pw.set_progress(0.0);
        pw.set_done(false);
        pw.on_close_window(on_close_progress_window(pw.as_weak()));
        let _ = pw.show();
        let pw_weak = pw.as_weak();
        *progress_window.borrow_mut() = Some(pw);

        let app_weak = weak.clone();
        let backend = backend.clone();
        let selected = selected.clone();
        thread::spawn(move || apply_actions(actions, app_weak, pw_weak, backend, selected));
    }
}

/// Hide the progress window when its close button is pressed.
fn on_close_progress_window(pw_weak: Weak<ProgressWindow>) -> impl FnMut() + 'static {
    move || {
        if let Some(pw) = pw_weak.upgrade() {
            let _ = pw.hide();
        }
    }
}

/// Background half of the sync: run every filesystem action, reporting each one
/// back to the UI thread as it completes.
fn apply_actions(
    actions: Vec<FilesystemAction>,
    app_weak: Weak<AppWindow>,
    pw_weak: Weak<ProgressWindow>,
    backend: Backend,
    selected: Selected,
) {
    let total = actions.len();
    for (i, action) in actions.iter().enumerate() {
        let report = process_filesystem_action(action, i, total);
        let backend = backend.clone();
        let selected = selected.clone();
        let app_weak = app_weak.clone();
        let pw_weak = pw_weak.clone();
        slint::invoke_from_event_loop(move || {
            report_action_done(report, app_weak, pw_weak, backend, selected)
        })
        .ok();
    }
}

/// UI-thread half of the sync: adopt one action's result, repaint the main
/// window and advance the progress bar.
fn report_action_done(
    report: FilesystemActionReport,
    app_weak: Weak<AppWindow>,
    pw_weak: Weak<ProgressWindow>,
    backend: Backend,
    selected: Selected,
) {
    let mut b = backend.lock().unwrap();
    b.handle_filesystem_action_done(&report);
    {
        let s = selected.lock().unwrap();
        if let Some(app) = app_weak.upgrade() {
            refresh(&app, &b, &s);
        }
    }
    drop(b);

    if let Some(pw) = pw_weak.upgrade() {
        let processed = report.iter + 1;
        pw.set_progress(processed as f32 / report.total_size as f32);
        pw.set_done(processed == report.total_size);
    }
}

/// Tree row clicked: toggle a directory, or select any entry.
fn on_row_clicked(
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
) -> impl FnMut(slint::SharedString) + 'static {
    move |path| {
        let ui = weak.upgrade().unwrap();
        let path = PathBuf::from(path.to_string());
        let mut b = backend.lock().unwrap();
        let is_dir = matches!(b.root_dir.find_recursive(&path), Some(FsEntry::FsDirectory(_)));
        if is_dir {
            b.root_dir.toggle_dir_recursive(&path);
        }
        let mut s = selected.lock().unwrap();
        *s = Some(path);
        refresh(&ui, &b, &s);
    }
}

/// Action button clicked: apply it to the currently selected entry.
fn on_action_clicked(
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
) -> impl FnMut(slint::SharedString) + 'static {
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
}

/// Recompute every UI-exposed model from the current backend state.
fn refresh(ui: &AppWindow, backend: &Dissonance, selected: &Option<PathBuf>) {
    let mut tree_rows = Vec::new();
    flatten_tree(
        backend,
        backend.root_dir.children(),
        0,
        selected,
        &mut tree_rows,
    );
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
    selected: &Option<PathBuf>,
    out: &mut Vec<TreeRow>,
) {
    for node in nodes.values() {
        let rel_path = node.rel_path();
        let name = rel_path.file_name().unwrap().to_string_lossy().into_owned();
        let is_selected = selected.as_ref() == Some(rel_path);

        let (label, bg, text_color) = match node {
            FsEntry::FsFile(_) => (name, Color::from_argb_u8(0, 0, 0, 0), problem_color(false)),
            FsEntry::FsMusicFile(f) => {
                let prefix = if f.sync_data.synced { "" } else { "* " };
                (
                    format!("{prefix}{name}"),
                    intention_color(f.sync_data.intention.clone(), is_selected),
                    problem_color(f.has_problems()),
                )
            }
            FsEntry::FsDirectory(d) => {
                let prefix = if d.synced() { "" } else { "* " };
                (
                    format!("{prefix}{name}"),
                    intention_color(d.intention(), is_selected),
                    problem_color(d.has_problems()),
                )
            }
        };

        out.push(TreeRow {
            label: label.into(),
            indent,
            path: rel_path.to_string_lossy().into_owned().into(),
            bg,
            text_color,
        });

        if let FsEntry::FsDirectory(d) = node {
            if d.expanded {
                flatten_tree(backend, d.children(), indent + 1, selected, out);
            }
        }
    }
}

/// Background tint for a tree row, derived from its sync intention. Selected
/// rows are darkened. Mirrors the old iced `ButtonStyle`.
fn intention_color(intention: SyncIntention, selected: bool) -> Color {
    let coef = if selected { 0.7 } else { 1.0 };
    let (r, g, b) = match intention {
        SyncIntention::KeepSync => (0.83, 0.89, 1.0),
        SyncIntention::ForceSync => (0.53, 0.49, 1.0),
        SyncIntention::DropSync => (0.97, 0.84, 0.85),
        SyncIntention::Unspecified => (1.0, 0.9, 0.7),
        SyncIntention::MixedDir => (0.9, 0.85, 0.94),
    };
    let to_u8 = |v: f32| (v * coef * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::from_rgb_u8(to_u8(r), to_u8(g), to_u8(b))
}

/// Text color for a tree row: red when the entry has problems, else black.
fn problem_color(has_problem: bool) -> Color {
    if has_problem {
        Color::from_rgb_u8(230, 26, 26)
    } else {
        Color::from_rgb_u8(0, 0, 0)
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
    let entry = match backend.root_dir.find_recursive(selected) {
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
