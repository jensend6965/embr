//! The device thread — the only place a HID++ handle is ever open.
//!
//! A sweep of two receivers takes seconds, so nothing here may run on the UI thread. The thread
//! owns
//! `core::Monitor` (and with it the policy that decides which alerts are worth a notification)
//! and never shares it: commands come in on a channel, results go out on two.
//!
//! It also owns the one thing that knows *which* tray icons should exist — that is a decision
//! about the devices — while the window applies their style when the picker moves. See
//! `embr_desktop::tray::Icons` for why that split exists (a style change once waited 827 ms for a
//! sweep to finish).
//!
//! Between sweeps the application holds no device open at all: [`Monitor::refresh`] opens what it
//! needs, reads, and closes it again before returning.

use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use embr_core::{Monitor, Patience, default_path, thresholds_text};
use embr_desktop::tray::{Icons, labels};
use embr_desktop::{Notice, dispatch_alert};

use crate::messages::{Command, State};

/// A hidden switch, for proving the notification path while notifications are off (the default):
/// `EMBR_DEBUG_ALERT=1 embr` sends one synthetic low-battery alert through the very same code
/// a real one takes.
const DEBUG_ALERT_VARIABLE: &str = "EMBR_DEBUG_ALERT";

pub struct DeviceWorker {
    monitor: Monitor,
    states: Sender<State>,
    notices: Sender<Notice>,
    tray: Icons,
    debug_alert: bool,
}

/// Start the device thread. Errors are the caller's to report — a window with no device thread is
/// still a window, and saying so beats a panic.
pub fn spawn(
    commands: Receiver<Command>,
    states: Sender<State>,
    notices: Sender<Notice>,
    tray: Icons,
) -> std::io::Result<JoinHandle<()>> {
    let worker = DeviceWorker {
        monitor: Monitor::from_disk(),
        states,
        notices,
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
            // The first thing this thread does is the first read, with the waits a device coming
            // back from sleep needs — because the first read here *is* the window opening, and
            // `core`'s own rule for that is a read at once rather than at the next scheduled
            // sweep. A scheduled ping is 700 ms, which is not enough for this desk's devices when
            // they have gone quiet (measured: a scheduled sweep wrote all three off as *Not
            // answering* while the same devices answered a patient one seconds later).
            let command = if first {
                first = false;
                Command::Rescan
            } else {
                match commands.recv() {
                    Ok(command) => command,
                    // The window is gone. There is nobody left to tell, so stop.
                    Err(_) => break,
                }
            };

            // A sweep takes seconds. Everything that arrived while the last one ran is taken in
            // the same batch, so a slow device cannot build a backlog of refreshes — and a dozen
            // queued clicks cost one sweep, not a dozen.
            let mut batch = vec![command];
            while let Ok(command) = commands.try_recv() {
                batch.push(command);
            }

            let mut refresh = false;
            let mut rescan = false;
            let mut watch = false;
            let mut settings_changed = false;
            let mut asked: Vec<String> = Vec::new();
            for command in batch {
                match command {
                    Command::Refresh => refresh = true,
                    Command::Watch => watch = true,
                    Command::Rescan => {
                        refresh = true;
                        rescan = true;
                    }
                    Command::Pages(key) => {
                        if !asked.contains(&key) {
                            asked.push(key);
                        }
                    }
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
                    Command::SetDeviceAlertLevel { key, level } => {
                        match self.monitor.set_device_alert_level(&key, level) {
                            Ok(()) => eprintln!(
                                "[devices] {} now alerts at {}",
                                self.name_of(&key),
                                match level {
                                    Some(level) => format!("{level}%"),
                                    None => "the shared thresholds".to_string(),
                                }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the level was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                        // A level chosen for a device that is *already* below it has to be
                        // measured against a fresh reading: the policy only speaks when it
                        // observes.
                        refresh = true;
                    }
                    Command::SetTrayTextColor(color) => {
                        match self.monitor.set_tray_text_color(color) {
                            Ok(()) => eprintln!(
                                "[devices] the tray icon's text is drawn {} ({})",
                                color.label(),
                                color.as_str()
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetTrayStyle(style) => {
                        match self.monitor.set_tray_style(style) {
                            Ok(()) => eprintln!(
                                "[devices] the tray icon is drawn as {} ({})",
                                style.label(),
                                style.as_str()
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetTrayDevice { key, on } => {
                        match self.monitor.set_tray_device(&key, on) {
                            Ok(()) => eprintln!(
                                "[devices] {} {} a tray icon of its own",
                                self.name_of(&key),
                                if on { "gets" } else { "gives up" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetWatchDevices(enabled) => {
                        match self.monitor.set_watch_devices(enabled) {
                            Ok(()) => eprintln!(
                                "[devices] devices will {} be checked between sweeps",
                                if enabled { "" } else { "not" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetCloseToTray(enabled) => {
                        match self.monitor.set_close_to_tray(enabled) {
                            Ok(()) => eprintln!(
                                "[devices] closing the window will {}",
                                if enabled {
                                    "leave Embr running"
                                } else {
                                    "quit"
                                }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetTheme(theme) => {
                        match self.monitor.set_theme(theme) {
                            Ok(()) => eprintln!(
                                "[devices] the window follows the {} colour scheme ({})",
                                theme.label(),
                                theme.as_str()
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetShowVoltage(show) => {
                        match self.monitor.set_show_voltage(show) {
                            Ok(()) => eprintln!(
                                "[devices] the voltage is {} beside the percentage",
                                if show { "shown" } else { "no longer shown" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetLevelColors(colors) => {
                        match self.monitor.set_level_colors(colors) {
                            Ok(()) => eprintln!(
                                "[devices] the level bars are {} coloured by their level",
                                if colors { "" } else { "no longer" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetMarkEstimates(mark) => {
                        match self.monitor.set_mark_estimates(mark) {
                            Ok(()) => eprintln!(
                                "[devices] a percentage read from voltage is {} marked with a ~",
                                if mark { "" } else { "no longer" }
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the choice was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                    Command::SetRefreshSeconds(seconds) => {
                        match self.monitor.set_refresh_seconds(seconds) {
                            Ok(()) => eprintln!(
                                "[devices] a fresh reading every {}s while the window is open",
                                seconds.max(embr_core::MIN_REFRESH_SECONDS)
                            ),
                            Err(problem) => eprintln!(
                                "[devices] the interval was applied but not saved: {problem}"
                            ),
                        }
                        settings_changed = true;
                    }
                }
            }

            if refresh {
                self.sweep(if rescan {
                    Patience::Asked
                } else {
                    Patience::Scheduled
                });
            } else if watch {
                // Not a sweep: every device is asked the one question that can have changed,
                // with deadlines short enough (see `Patience::Watch`) to do this every few
                // seconds.
                self.sweep(Patience::Watch);
            }

            // After the sweep, so that a device the window just asked about has been read at
            // least once and is in the report the feature set is read from.
            for key in &asked {
                self.report_pages(key);
            }

            if refresh || watch || !asked.is_empty() {
                self.publish();
                self.update_tray();
            } else if settings_changed {
                // A settings change reaches the window and the tray now, from the readings
                // already in hand: a sweep would take seconds and change nothing.
                self.publish();
                self.update_tray();
            }
        }
        eprintln!("[devices] the device thread is stopping");
    }

    /// One discovery pass: read every device, publish it, and hand out what is worth saying.
    fn sweep(&mut self, patience: Patience) {
        let started = Instant::now();
        let alerts = self.monitor.refresh_with(patience);
        eprintln!(
            "[devices] {} device(s) read in {} ms{}",
            self.monitor.snapshot().devices.len(),
            started.elapsed().as_millis(),
            match patience {
                Patience::Asked => " (asked for, so the slow ones were given time)",
                Patience::Watch => " (a watch pass)",
                Patience::Scheduled => "",
            }
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
                &self.notices,
                embr_core::Alert::Low {
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
                dispatch_alert(&self.notices, alert);
            } else {
                // Notifications are opt-in and this is the default state, so the honest thing
                // is to say what the user would have been told.
                let (title, body) = alert.title_and_body();
                eprintln!("[notify] notifications are off — would have shown: {title}: {body}");
            }
        }
    }

    /// Ask one device what it advertises and keep the answer.
    fn report_pages(&mut self, key: &str) {
        match self.monitor.read_pages(key) {
            Some(pages) => {
                let names: Vec<&str> = pages.pages.iter().map(|page| page.label()).collect();
                eprintln!(
                    "[devices] {key} advertises {} page(s) from {} slot(s): {}{}",
                    pages.pages.len(),
                    pages.declared,
                    names.join(", "),
                    if pages.complete() {
                        String::new()
                    } else {
                        // Not a detail: a partial list means the rail is missing pages the device
                        // really has, and the window says so too.
                        format!(" — {} slot(s) did not answer", pages.silent)
                    }
                );
            }
            None => eprintln!(
                "[devices] {} did not answer about its features; the rail will say so",
                self.name_of(key)
            ),
        }
    }

    /// Hand the window everything it draws, plus the settings that came with it.
    fn publish(&self) {
        let config = self.monitor.config();
        let state = State {
            snapshot: self.monitor.snapshot().clone(),
            notifications_enabled: config.notifications.enabled,
            thresholds_text: thresholds_text(&config.notifications.thresholds),
            refresh_seconds: config.general.refresh_seconds,
            tray_look: config.ui.tray_look(),
            snooze_minutes: config.notifications.snooze_minutes,
            tray_devices: config.ui.tray_devices.clone(),
            close_to_tray: config.general.close_to_tray,
            theme: config.ui.theme(),
            show_voltage: config.ui.show_voltage,
            level_colors: config.ui.level_colors,
            mark_estimates: config.ui.mark_estimates,
            watch_devices: config.general.watch_devices,
            config_path: default_path().display().to_string(),
            notice: self.monitor.notices.first().cloned(),
        };
        if self.states.send(state).is_err() {
            eprintln!("[devices] the window has closed");
        }
    }

    /// Keep the tray icons in step with the configuration and the readings.
    fn update_tray(&self) {
        let config = self.monitor.config();
        let snapshot = self.monitor.snapshot();
        // Which icons should exist is `core`'s decision (nothing chosen means one icon for the
        // worst device); the set itself is this call's business.
        let wanted = embr_core::tray_icons(snapshot, &config.ui.tray_devices);
        self.tray
            .sync(&wanted, &labels(snapshot), config.ui.tray_look());
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
