// The production album selector: the one that asks a living user, in a modal
// window of its own. The unit tests wire a mock in its place.

use std::{cell::RefCell, sync::mpsc};

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use crate::{
    AlbumOption, AlbumSelectWindow,
    remote_tags::{AlbumCandidate, AlbumSelection, AlbumSelector},
};

// Keeps the album window alive while it is shown; lives on the UI thread only.
thread_local! {
    static ALBUM_WINDOW: RefCell<Option<AlbumSelectWindow>> = RefCell::new(None);
}

enum ModalChoice {
    Option(usize),
    NoSuitable,
    Cancel,
}

// Opens the modal on the UI thread and parks the calling worker thread until
// the user answers.
pub struct ModalAlbumSelector {}

impl AlbumSelector for ModalAlbumSelector {
    fn select(&self, local_name: &str, candidates: &[AlbumCandidate]) -> AlbumSelection {
        let (sender, receiver) = mpsc::channel::<ModalChoice>();

        let options: Vec<AlbumOption> = candidates
            .iter()
            .enumerate()
            .map(|(index, c)| AlbumOption {
                label: option_label(c).into(),
                index: index as i32,
            })
            .collect();

        let local_name_owned = local_name.to_string();
        let opened = slint::invoke_from_event_loop(move || {
            open_album_window(local_name_owned, options, sender)
        });

        if opened.is_err() {
            return AlbumSelection::Cancelled;
        }

        match receiver.recv() {
            Ok(ModalChoice::Option(index)) => match candidates.get(index) {
                Some(candidate) => AlbumSelection::Chosen(candidate.album.clone()),
                None => AlbumSelection::Cancelled,
            },
            Ok(ModalChoice::NoSuitable) => AlbumSelection::NoSuitable,
            Ok(ModalChoice::Cancel) | Err(_) => AlbumSelection::Cancelled,
        }
    }
}

// The year and the track count are shown beside the percentage: the count is
// what ordered the list, and the percentage alone cannot tell two pressings of
// one album apart. The id tells two albums of the very same name apart.
fn option_label(candidate: &AlbumCandidate) -> String {
    let year = match candidate.album.year.is_empty() {
        true => "no year".to_string(),
        false => candidate.album.year.clone(),
    };

    let tracks = match candidate.album.track_count {
        0 => "no track count".to_string(),
        count => format!("{} tracks", count),
    };

    format!(
        "{} — {}, {} ({}) — {}%",
        candidate.album.name, year, tracks, candidate.album.id, candidate.likeness_percent
    )
}

fn open_album_window(
    local_name: String,
    options: Vec<AlbumOption>,
    sender: mpsc::Sender<ModalChoice>,
) {
    let window = match AlbumSelectWindow::new() {
        Ok(w) => w,
        Err(_) => {
            let _ = sender.send(ModalChoice::Cancel);
            return;
        }
    };

    window.set_local_name(local_name.into());
    window.set_options(ModelRc::new(VecModel::from(options)));

    let weak = window.as_weak();
    let answer = move |sender: &mpsc::Sender<ModalChoice>, choice: ModalChoice| {
        let _ = sender.send(choice);
        if let Some(w) = weak.upgrade() {
            let _ = w.hide();
        }
    };

    let chosen_sender = sender.clone();
    let chosen_answer = answer.clone();
    window.on_chosen(move |index| {
        chosen_answer(&chosen_sender, ModalChoice::Option(index.max(0) as usize))
    });

    let no_suitable_sender = sender.clone();
    let no_suitable_answer = answer.clone();
    window.on_no_suitable(move || no_suitable_answer(&no_suitable_sender, ModalChoice::NoSuitable));

    let cancel_sender = sender.clone();
    let cancel_answer = answer.clone();
    window.on_cancel(move || cancel_answer(&cancel_sender, ModalChoice::Cancel));

    // closing the window with its own button is a cancel too
    window.window().on_close_requested(move || {
        let _ = sender.send(ModalChoice::Cancel);
        CloseRequestResponse::HideWindow
    });

    let _ = window.show();
    ALBUM_WINDOW.with(|w| *w.borrow_mut() = Some(window));
}
