// The production artist selector: the one that asks a living user, in a modal
// window of its own. The unit tests wire a mock in its place.

use std::{cell::RefCell, sync::mpsc};

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use crate::{
    ArtistOption, ArtistSelectWindow,
    remote_tags::{ArtistCandidate, ArtistSelector, Selection},
};

// Keeps the artist window alive while it is shown; lives on the UI thread only.
thread_local! {
    static ARTIST_WINDOW: RefCell<Option<ArtistSelectWindow>> = RefCell::new(None);
}

enum ModalChoice {
    Option(usize),
    NoSuitable,
    Cancel,
}

// Opens the modal on the UI thread and parks the calling worker thread until
// the user answers.
pub struct ModalArtistSelector {}

impl ArtistSelector for ModalArtistSelector {
    fn select(&self, local_name: &str, candidates: &[ArtistCandidate]) -> Selection {
        let (sender, receiver) = mpsc::channel::<ModalChoice>();

        let options: Vec<ArtistOption> = candidates
            .iter()
            .enumerate()
            .map(|(index, c)| ArtistOption {
                // the id tells two artists of the same name apart
                label: format!(
                    "{} ({}) — {}%",
                    c.artist.name, c.artist.id, c.likeness_percent
                )
                .into(),
                index: index as i32,
            })
            .collect();

        let local_name_owned = local_name.to_string();
        let opened = slint::invoke_from_event_loop(move || {
            open_artist_window(local_name_owned, options, sender)
        });

        if opened.is_err() {
            return Selection::Cancelled;
        }

        match receiver.recv() {
            Ok(ModalChoice::Option(index)) => match candidates.get(index) {
                Some(candidate) => Selection::Chosen(candidate.artist.clone()),
                None => Selection::Cancelled,
            },
            Ok(ModalChoice::NoSuitable) => Selection::NoSuitable,
            Ok(ModalChoice::Cancel) | Err(_) => Selection::Cancelled,
        }
    }
}

fn open_artist_window(
    local_name: String,
    options: Vec<ArtistOption>,
    sender: mpsc::Sender<ModalChoice>,
) {
    let window = match ArtistSelectWindow::new() {
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
    ARTIST_WINDOW.with(|w| *w.borrow_mut() = Some(window));
}
