//! The device thread — the only place a HID++ handle is ever open.
//!
//! A sweep of two receivers takes seconds, so nothing here may run on the UI thread. The
//! thread owns `core::Monitor` (and with it the policy that decides which alerts are worth a
//! notification) and never shares it: commands come in on a channel, results go out on two.
//!
//! Between sweeps the application holds no device open at all: [`core::Monitor::refresh`]
//! opens what it needs, reads, and closes it again before returning.

use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Instant;

use embr_core::{Alert, Monitor};
use ksni::Handle;

use crate::messages::{Command, UiState, dispatch_alert, thresholds_text};
use crate::tray::EmbrTray;

/// Whatever the configuration file says, the window never asks for a faster cadence than
/// this: a sweep is seconds of HID++ traffic and the devices are not ours alone.
pub const MIN_REFRESH_SECONDS: u64 = 5;

/// A hidden switch, for proving the notification path while notifications are off (the
/// default): `EMBR_DEBUG_ALERT=1 embr` sends one synthetic low-battery alert
/// through the very same code a real one takes.
const DEBUG_ALERT_VARIABLE: &str = "EMBR_DEBUG_ALERT";

pub struct DeviceWorker {
    monitor: Monitor,
    states: Sender<UiState>,
    alerts: Sender<Alert>,
    tray: Option<Handle<EmbrTray>>,
    debug_alert: bool,
}

/// Start the device thread. Errors are the caller's to report — a window with no device
/// thread is still a window, and saying so beats a panic.
pub fn spawn(
    commands: Receiver<Command>,
    states: Sender<UiState>,
    alerts: Sender<Alert>,
    tray: Option<Handle<EmbrTray>>,
) -> std::io::Result<thread::JoinHandle<()>> {
    let worker = DeviceWorker {
        monitor: Monitor::from_disk(),
        states,
        alerts,
        tray,
        debug_alert: std::env::var_os(DEBUG_ALERT_VARIABLE).is_some(),
    };
    thread::Builder::new()
        .name("devices".to_string())
        .spawn(move || worker.run(commands))
}

impl DeviceWorker {
    fn run(mut self, commands: Receiver<Command>) {
        let config = self.monitor.config();
        eprintln!(
            "[devices] configuration: refresh every {}s, notifications {}",
            config.general.refresh_seconds,
            if config.notifications.enabled {
                "on"
            } else {
                "off"
            }
        );
        if self.debug_alert {
            eprintln!(
                "[devices] {DEBUG_ALERT_VARIABLE} is set: one synthetic alert will be sent after the first sweep"
            );
        }

        let mut first = true;
        loop {
            // The first thing this thread does is the first sweep, so the window has real
            // data as soon as the hardware has answered.
            let command = if first {
                first = false;
                Command::Refresh
            } else {
                match commands.recv() {
                    Ok(command) => command,
                    // The window is gone. There is nobody left to tell, so stop.
                    Err(_) => break,
                }
            };

            // A sweep takes seconds. Everything that arrived while the last one ran is taken
            // in the same batch, so a slow device cannot build an unbounded backlog of
            // refreshes — and a dozen queued "Refresh" clicks cost one sweep, not a dozen.
            let mut batch = vec![command];
            while let Ok(command) = commands.try_recv() {
                batch.push(command);
            }

            let mut refresh = false;
            let mut settings_changed = false;
            for command in batch {
                match command {
                    Command::Refresh => refresh = true,
                    Command::Snooze(key) => {
                        self.monitor.snooze(&key);
                        eprintln!("[devices] snoozed {}", self.name_of(&key));
                    }
                    Command::SetNotifications(enabled) => {
                        match self.monitor.set_notifications_enabled(enabled) {
                            Ok(()) => eprintln!(
                                "[devices] battery notifications {}",
                                if enabled { "on" } else { "off" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                }
            }

            if refresh {
                self.sweep();
            } else if settings_changed {
                self.publish();
            }
        }
        eprintln!("[devices] the device thread is stopping");
    }

    /// One discovery pass: read every device, publish it, and hand out what is worth saying.
    fn sweep(&mut self) {
        let started = Instant::now();
        let alerts = self.monitor.refresh();
        eprintln!(
            "[devices] {} device(s) read in {} ms",
            self.monitor.snapshot().devices.len(),
            started.elapsed().as_millis()
        );
        self.publish();
        self.update_tray();

        if self.debug_alert {
            let name = self
                .monitor
                .snapshot()
                .devices
                .first()
                .map(|device| device.name.clone())
                .unwrap_or_else(|| "Debug device".to_string());
            eprintln!("[notify] {DEBUG_ALERT_VARIABLE} is set — sending a synthetic alert");
            dispatch_alert(
                &self.alerts,
                Alert::Low {
                    key: "debug".to_string(),
                    name,
                    threshold: 20,
                    percent: 19,
                    estimated: true,
                },
            );
        }

        let enabled = self.monitor.config().notifications.enabled;
        for alert in alerts {
            if enabled {
                dispatch_alert(&self.alerts, alert);
            } else {
                // Notifications are opt-in and this is the default state, so the honest thing
                // is to say what the user would have been told.
                let (title, body) = alert.title_and_body();
                eprintln!("[notify] notifications are off — would have shown: {title}: {body}");
            }
        }
    }

    /// Hand the window everything it draws, plus the settings that came with it.
    fn publish(&self) {
        let config = self.monitor.config();
        let state = UiState {
            snapshot: self.monitor.snapshot().clone(),
            notifications_enabled: config.notifications.enabled,
            thresholds_text: thresholds_text(&config.notifications.thresholds),
            refresh_seconds: config.general.refresh_seconds.max(MIN_REFRESH_SECONDS),
            notice: self.monitor.notices.first().cloned(),
        };
        if self.states.send(state).is_err() {
            eprintln!("[devices] the window has closed");
        }
    }

    fn update_tray(&self) {
        let Some(tray) = &self.tray else { return };
        let snapshot = self.monitor.snapshot();
        tray.update(|item| item.observe(snapshot));
    }

    fn name_of(&self, key: &str) -> String {
        self.monitor
            .snapshot()
            .devices
            .iter()
            .find(|device| device.key == key)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| key.to_string())
    }
}
