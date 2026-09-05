// The production track selector: the one that asks a living user, in a modal
// window of its own. The unit tests wire a mock in its place.

use std::{cell::RefCell, sync::mpsc};

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use crate::{
    TrackOption, TrackSelectWindow,
    remote_tags::{LocalTrack, TrackCandidate, TrackSelection, TrackSelector},
};

// Keeps the track window alive while it is shown; lives on the UI thread only.
thread_local! {
    static TRACK_WINDOW: RefCell<Option<TrackSelectWindow>> = RefCell::new(None);
}

enum ModalChoice {
    Option(usize),
    Skip,
    Cancel,
}

// Opens the modal on the UI thread and parks the calling worker thread until
// the user answers. One album can open it several times, once per file that
// was not matched on its own.
pub struct ModalTrackSelector {
    // shown above the list, so that the user knows what is being installed
    pub album_name: String,
}

impl TrackSelector for ModalTrackSelector {
    fn select(&self, local: &LocalTrack, candidates: &[TrackCandidate]) -> TrackSelection {
        let (sender, receiver) = mpsc::channel::<ModalChoice>();

        let options: Vec<TrackOption> = candidates
            .iter()
            .enumerate()
            .map(|(index, c)| TrackOption {
                label: option_label(c).into(),
                index: index as i32,
            })
            .collect();

        let local_name = local.name.clone();
        let album_name = self.album_name.clone();
        let opened = slint::invoke_from_event_loop(move || {
            open_track_window(local_name, album_name, options, sender)
        });

        if opened.is_err() {
            return TrackSelection::Cancelled;
        }

        match receiver.recv() {
            Ok(ModalChoice::Option(index)) => match candidates.get(index) {
                Some(candidate) => TrackSelection::Chosen(candidate.track.clone()),
                None => TrackSelection::Skip,
            },
            Ok(ModalChoice::Skip) => TrackSelection::Skip,
            Ok(ModalChoice::Cancel) | Err(_) => TrackSelection::Cancelled,
        }
    }
}

// The numbers are shown beside the percentage: they are what the install is
// after, and a set of several discs holds more than one track of a number.
fn option_label(candidate: &TrackCandidate) -> String {
    let number = match candidate.track.disc_number {
        0 => format!("{:02}", candidate.track.track_number),
        disc => format!("{}-{:02}", disc, candidate.track.track_number),
    };

    format!(
        "[{}] {} — {}%",
        number, candidate.track.name, candidate.likeness_percent
    )
}

fn open_track_window(
    local_name: String,
    album_name: String,
    options: Vec<TrackOption>,
    sender: mpsc::Sender<ModalChoice>,
) {
    let window = match TrackSelectWindow::new() {
        Ok(w) => w,
        Err(_) => {
            let _ = sender.send(ModalChoice::Cancel);
            return;
        }
    };

    window.set_local_name(local_name.into());
    window.set_album_name(album_name.into());
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

    let skip_sender = sender.clone();
    let skip_answer = answer.clone();
    window.on_skip(move || skip_answer(&skip_sender, ModalChoice::Skip));

    let cancel_sender = sender.clone();
    let cancel_answer = answer.clone();
    window.on_cancel(move || cancel_answer(&cancel_sender, ModalChoice::Cancel));

    // closing the window with its own button skips this one file, it does not
    // drop the whole install
    window.window().on_close_requested(move || {
        let _ = sender.send(ModalChoice::Skip);
        CloseRequestResponse::HideWindow
    });

    let _ = window.show();
    TRACK_WINDOW.with(|w| *w.borrow_mut() = Some(window));
}
