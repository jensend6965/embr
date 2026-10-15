//! Desktop notifications, on their own thread.
//!
//! `Notification::show()` blocks until the notification is closed, and reaching the message
//! bus is a D-Bus round trip that can hang for as long as the bus feels like it. Neither may
//! touch the window, so every notice is handed to a thread here.
//!
//! Two kinds arrive on the one channel: a battery alert from the policy, which offers the
//! "Show devices" action, and a sentence from the application itself, which does not.

use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use notify_rust::{Notification, Timeout, Urgency};

use crate::messages::{Notice, UiRequest};

const APP_NAME: &str = "Embr";
/// The identifier of the notification's action button, as the notification server sees it.
const SHOW_DEVICES_ACTION: &str = "show";
const NOTIFICATION_TIMEOUT_MS: u32 = 8000;

/// Start the notification thread. Failing to start it is not fatal: the window and the tray
/// keep working, the user simply hears nothing.
pub fn spawn(
    notices: Receiver<Notice>,
    ui: Sender<UiRequest>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("notifications".to_string())
        .spawn(move || {
            for notice in notices {
                let ui = ui.clone();
                // One thread per notice: a notification left open on screen must not hold up
                // the next one. Alerts are rate-limited in `core`, so this cannot pile up.
                let unshown = notice.clone();
                let started = thread::Builder::new()
                    .name("notification".to_string())
                    .spawn(move || send(&notice, &ui));
                if let Err(problem) = started {
                    eprintln!(
                        "[notify] could not start a notification thread ({problem}); not shown: {}",
                        spoken(&unshown)
                    );
                }
            }
            eprintln!("[notify] the notification thread is stopping");
        })
}

/// What a notice says, as one string, for the log.
fn spoken(notice: &Notice) -> String {
    match notice {
        Notice::Alert(alert) => {
            let (title, body) = alert.title_and_body();
            format!("{title}: {body}")
        }
        Notice::Text { title, body } => format!("{title}: {body}"),
    }
}

fn send(notice: &Notice, ui: &Sender<UiRequest>) {
    let (title, body, actionable) = match notice {
        Notice::Alert(alert) => {
            let (title, body) = alert.title_and_body();
            (title, body, true)
        }
        Notice::Text { title, body } => (title.clone(), body.clone(), false),
    };

    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(&title)
        .body(&body)
        // An alert is about a battery, and a note is the app talking: they get the icon that
        // says which without being read.
        .icon(if actionable {
            "battery-caution"
        } else {
            "dialog-information"
        })
        .urgency(Urgency::Normal)
        .timeout(Timeout::Milliseconds(NOTIFICATION_TIMEOUT_MS));
    // Only an alert has somewhere to go: "Show devices" is meaningless for "still running".
    if actionable && actions_supported() {
        notification.action(SHOW_DEVICES_ACTION, "Show devices");
    }

    match notification.show() {
        Ok(handle) => {
            // The id is what the server will name in the ActionInvoked signal, so logging it
            // is what makes an action verifiable from outside the app.
            eprintln!("[notify] shown as id {}: {title}: {body}", handle.id());
            // Blocks this thread until the notification is dismissed or acted on; that is
            // exactly why it is not the UI thread.
            handle.wait_for_action(|action| {
                if action == SHOW_DEVICES_ACTION {
                    eprintln!("[notify] \"Show devices\" chosen");
                    if ui.send(UiRequest::ShowWindow).is_err() {
                        eprintln!("[notify] the window is gone");
                    }
                }
            });
        }
        Err(problem) => {
            eprintln!("[notify] could not show the notification ({problem}): {title}: {body}");
        }
    }
}

/// Asking the server once is enough, and `OnceLock` keeps the answer where the first caller
/// found it.
fn actions_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| match notify_rust::get_capabilities() {
        Ok(capabilities) => {
            let supported = capabilities
                .iter()
                .any(|capability| capability == "actions");
            eprintln!(
                "[notify] the notification server advertises actions = {supported}; server = {:?}",
                notify_rust::get_server_information()
                    .map(|information| information.name)
                    .unwrap_or_else(|_| "unknown".to_string())
            );
            supported
        }
        Err(problem) => {
            eprintln!(
                "[notify] could not ask the notification server for its capabilities ({problem}); sending without an action"
            );
            false
        }
    })
}
