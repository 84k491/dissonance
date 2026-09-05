use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
};

use slint::{Color, ModelRc, VecModel, Weak};

pub(crate) use crate::directory::FsEntry;
pub(crate) use crate::dissonance::{Problem, SyncIntention, SyncedEntry};

use crate::{
    album_selector::ModalAlbumSelector,
    artist_selector::ModalArtistSelector,
    directory::load_dir_hash_set_files_only,
    dissonance::{
        Dissonance, FilesystemAction, FilesystemActionReport, load_saved_state,
        process_filesystem_action,
    },
    itunes::HttpRemoteInfoProvider,
    music_file::FsEntryTrait,
    remote_tags::{
        RemoteAlbumMeta, RemoteArtistMeta, resolve_album_mapping, resolve_album_track_mapping,
        resolve_artist_mapping, resolve_remote_album_tags, resolve_remote_name,
    },
    track_selector::ModalTrackSelector,
};

mod album_selector;
mod artist_selector;
mod directory;
mod dissonance;
mod itunes;
mod music_file;
mod remote_tags;
mod tags;
mod track_selector;

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
        let is_dir = matches!(
            b.root_dir.find_recursive(&path),
            Some(FsEntry::FsDirectory(_))
        );
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

        match action.as_str() {
            "MapRemoteTagSource" => {
                start_artist_mapping(path, weak.clone(), backend.clone(), selected.clone());
                return;
            }
            "InstallRemoteTags" => {
                start_remote_tags_install(path, weak.clone(), backend.clone(), selected.clone());
                return;
            }
            "MapRemoteAlbumTagSource" => {
                start_album_mapping(path, weak.clone(), backend.clone(), selected.clone());
                return;
            }
            "InstallRemoteAlbumTags" => {
                start_remote_album_tags_install(
                    path,
                    weak.clone(),
                    backend.clone(),
                    selected.clone(),
                );
                return;
            }
            _ => {}
        }

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
            "ResetRemoteMapping" => b.reset_remote_artist_mapping(&path),
            "ResetRemoteAlbumMapping" => b.reset_remote_album_mapping(&path),
            "ForceSync" => b.set_sync_intention(path, SyncIntention::ForceSync),
            "KeepSync" => b.set_sync_intention(path, SyncIntention::KeepSync),
            "DropSync" => b.set_sync_intention(path, SyncIntention::DropSync),
            _ => {}
        }
        let s = selected.lock().unwrap();
        refresh(&ui, &b, &s);
    }
}

fn start_artist_mapping(path: PathBuf, weak: Weak<AppWindow>, backend: Backend, selected: Selected) {
    let local_name = { backend.lock().unwrap().artist_local_name(&path) };
    let Some(local_name) = local_name else {
        return;
    };

    thread::spawn(move || {
        let provider = HttpRemoteInfoProvider::new();
        let selector = ModalArtistSelector {};

        let meta = match resolve_artist_mapping(&local_name, &provider, &selector) {
            Err(e) => {
                println!("Failed to map artist '{}': {}", local_name, e);
                return;
            }
            // cancelled, the state stays as it was
            Ok(None) => return,
            Ok(Some(meta)) => meta,
        };

        apply_on_ui_thread(weak, backend, selected, move |b| {
            b.set_remote_artist_mapping(&path, &meta)
        });
    });
}

fn start_remote_tags_install(
    path: PathBuf,
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
) {
    let meta = { backend.lock().unwrap().remote_artist_meta(&path) };
    let Some(meta) = meta else {
        return;
    };

    thread::spawn(move || {
        let provider = HttpRemoteInfoProvider::new();

        let remote_name = match resolve_remote_name(&meta, &provider) {
            Err(e) => {
                println!("Failed to get the remote artist name: {}", e);
                return;
            }
            Ok(name) => name,
        };

        apply_on_ui_thread(weak, backend, selected, move |b| {
            b.install_remote_tags(&path, &remote_name)
        });
    });
}

fn start_album_mapping(path: PathBuf, weak: Weak<AppWindow>, backend: Backend, selected: Selected) {
    // everything the request needs is read under the lock, once
    let request = {
        let b = backend.lock().unwrap();
        b.artist_remote_id_of_album(&path)
            .zip(b.local_album_name_and_track_count(&path))
    };
    let Some((artist_id, (local_name, track_count))) = request else {
        return;
    };

    thread::spawn(move || {
        let provider = HttpRemoteInfoProvider::new();
        let selector = ModalAlbumSelector {};

        let meta = match resolve_album_mapping(
            &artist_id,
            &local_name,
            track_count,
            &provider,
            &selector,
        ) {
            Err(e) => {
                println!("Failed to map album '{}': {}", local_name, e);
                return;
            }
            // cancelled, the state stays as it was
            Ok(None) => return,
            Ok(Some(meta)) => meta,
        };

        apply_on_ui_thread(weak, backend, selected, move |b| {
            b.set_remote_album_mapping(&path, &meta)
        });
    });
}

fn start_remote_album_tags_install(
    path: PathBuf,
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
) {
    // everything the install needs is read under the lock, once
    let request = {
        let b = backend.lock().unwrap();
        b.remote_album_meta(&path)
            .map(|meta| (meta, b.local_album_tracks(&path)))
    };
    let Some((meta, locals)) = request else {
        return;
    };

    thread::spawn(move || {
        let provider = HttpRemoteInfoProvider::new();

        let remote = match resolve_remote_album_tags(&meta, &provider) {
            Err(e) => {
                println!("Failed to get the remote album tags: {}", e);
                return;
            }
            Ok(tags) => tags,
        };

        let album_id = match &meta {
            RemoteAlbumMeta::Mapped { id, .. } => id.clone(),
            RemoteAlbumMeta::NoSuitable => return,
        };

        let selector = ModalTrackSelector {
            album_name: remote.name.clone(),
        };

        let tracks = match resolve_album_track_mapping(&album_id, &locals, &provider, &selector) {
            Err(e) => {
                println!("Failed to match the tracks of the album: {}", e);
                return;
            }
            // cancelled, not one file is touched
            Ok(None) => return,
            Ok(Some(tracks)) => tracks.into_iter().collect::<BTreeMap<_, _>>(),
        };

        apply_on_ui_thread(weak, backend, selected, move |b| {
            b.install_remote_album_tags(&path, &remote, &tracks)
        });
    });
}

fn apply_on_ui_thread(
    weak: Weak<AppWindow>,
    backend: Backend,
    selected: Selected,
    change: impl FnOnce(&mut Dissonance) + Send + 'static,
) {
    slint::invoke_from_event_loop(move || {
        let mut b = backend.lock().unwrap();
        change(&mut b);

        let s = selected.lock().unwrap();
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &b, &s);
        }
    })
    .ok();
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

/// How the mapping of an artist directory is shown in the info panel.
fn remote_meta_line(meta: &Option<RemoteArtistMeta>) -> String {
    match meta {
        None => "Remote artist: not requested".to_string(),
        Some(RemoteArtistMeta::NoSuitable) => "Remote artist: no suitable".to_string(),
        Some(RemoteArtistMeta::Mapped { id, name: None }) => format!("Remote artist id: {}", id),
        Some(RemoteArtistMeta::Mapped {
            id,
            name: Some(name),
        }) => format!("Remote artist: {} ({})", name, id),
    }
}

/// How the mapping of an album directory is shown in the info panel.
fn remote_album_meta_line(meta: &Option<RemoteAlbumMeta>) -> String {
    match meta {
        None => "Remote album: not requested".to_string(),
        Some(RemoteAlbumMeta::NoSuitable) => "Remote album: no suitable".to_string(),
        Some(RemoteAlbumMeta::Mapped { id, name: None }) => format!("Remote album id: {}", id),
        Some(RemoteAlbumMeta::Mapped {
            id,
            name: Some(name),
        }) => format!("Remote album: {} ({})", name, id),
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
        FsEntry::FsDirectory(d) => {
            info.push(TextRow {
                text: d.relative_path.display().to_string().into(),
            });

            // an album belongs to an artist as well, but only an artist
            // directory can be mapped onto the remote source
            if d.is_artist() {
                info.push(TextRow {
                    text: remote_meta_line(d.remote_artist_meta()).into(),
                });
            }

            if d.is_album() {
                info.push(TextRow {
                    text: remote_album_meta_line(d.remote_album_meta()).into(),
                });
            }
        }
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

    let found_problems = match entry {
        FsEntry::FsMusicFile(mf) => mf.find_problems(),
        FsEntry::FsDirectory(d) => d.find_problems(),
        FsEntry::FsFile(f) => f.find_problems(),
    };

    for problem in found_problems {
        problems.push(TextRow {
            text: problem.to_string().into(),
        });
    }

    for action in backend.get_suitable_actions(entry) {
        actions.push(ActionRow {
            label: action.to_string().into(),
            action: action.to_string().into(),
        });
    }

    (info, problems, actions)
}
