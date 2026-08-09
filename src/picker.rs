// Asking the user to choose between several options, and telling them when
// something went wrong.
//
// 'fetch_tags' only knows the Prompt trait, so its flow can be driven by canned
// answers in tests. The implementation here opens a PickerWindow per question.

use std::{cell::RefCell, rc::Rc, sync::mpsc};

use slint::ComponentHandle;

use crate::PickerWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Picked(usize),
    Skip,
    Cancel,
}

pub trait Prompt {
    /// Ask which of 'items' it is; blocks until the user answers.
    fn ask(&self, heading: String, message: String, items: Vec<String>) -> Reply;

    /// Tell the user something; does not wait for it to be dismissed.
    fn notify(&self, heading: &str, message: String);
}

pub struct WindowPrompt {}

impl WindowPrompt {
    pub fn new() -> WindowPrompt {
        WindowPrompt {}
    }
}

// Every window lives on the UI thread, the asking flow lives on its own one, so
// both calls hop threads and 'ask' waits for the answer to come back.
impl Prompt for WindowPrompt {
    fn ask(&self, heading: String, message: String, items: Vec<String>) -> Reply {
        let (tx, rx) = mpsc::channel();

        let posted =
            slint::invoke_from_event_loop(move || open_picker_window(heading, message, items, tx));
        if posted.is_err() {
            return Reply::Cancel;
        }

        rx.recv().unwrap_or(Reply::Cancel)
    }

    fn notify(&self, heading: &str, message: String) {
        let heading = heading.to_string();
        slint::invoke_from_event_loop(move || open_message_window(heading, message)).ok();
    }
}

fn open_message_window(heading: String, message: String) {
    let window = match PickerWindow::new() {
        Ok(w) => w,
        Err(_) => return,
    };

    window.set_heading(heading.into());
    window.set_message(message.into());
    window.set_message_only(true);
    keep_alive_until_dismissed(window, None);
}

fn open_picker_window(
    heading: String,
    message: String,
    items: Vec<String>,
    reply: mpsc::Sender<Reply>,
) {
    let window = match PickerWindow::new() {
        Ok(w) => w,
        Err(_) => {
            reply.send(Reply::Cancel).ok();
            return;
        }
    };

    let items: Vec<slint::SharedString> =
        items.into_iter().map(slint::SharedString::from).collect();

    window.set_heading(heading.into());
    window.set_message(message.into());
    window.set_items(slint::ModelRc::new(slint::VecModel::from(items)));
    window.set_message_only(false);
    keep_alive_until_dismissed(window, Some(reply));
}

/// Show the window and let it own itself until one of its buttons is pressed:
/// the flow that opened it lives on another thread and can not hold it.
fn keep_alive_until_dismissed(window: PickerWindow, reply: Option<mpsc::Sender<Reply>>) {
    let holder: Rc<RefCell<Option<PickerWindow>>> = Rc::new(RefCell::new(None));

    let dismiss = {
        let holder = holder.clone();
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                window.hide().ok();
            }
            holder.borrow_mut().take();
        }
    };

    // The asking side takes the first answer only, later sends are no-ops.
    let answer = |reply: &Option<mpsc::Sender<Reply>>, value: Reply| {
        if let Some(tx) = reply {
            tx.send(value).ok();
        }
    };

    {
        let (dismiss, reply) = (dismiss.clone(), reply.clone());
        window.on_picked(move |index| {
            answer(&reply, Reply::Picked(index.max(0) as usize));
            dismiss();
        });
    }
    {
        let (dismiss, reply) = (dismiss.clone(), reply.clone());
        window.on_skipped(move || {
            answer(&reply, Reply::Skip);
            dismiss();
        });
    }
    {
        let (dismiss, reply) = (dismiss.clone(), reply.clone());
        window.on_cancelled(move || {
            answer(&reply, Reply::Cancel);
            dismiss();
        });
    }
    {
        let (dismiss, reply) = (dismiss.clone(), reply.clone());
        window.window().on_close_requested(move || {
            answer(&reply, Reply::Cancel);
            dismiss();
            slint::CloseRequestResponse::HideWindow
        });
    }

    window.show().ok();
    *holder.borrow_mut() = Some(window);
}
