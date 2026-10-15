// Spike 003: StatusNotifierItem tray icon + desktop notification, on KineticWE / Noctalia.
//
// Step 1: register a StatusNotifierItem with ksni (title, tooltip, generated ARGB32 pixmap,
//         left-click activate, submenu with one entry per fake device).
// Step 2: send a real desktop notification via notify-rust (urgency normal, short timeout,
//         one action button), and report the server's GetCapabilities reply.
// Step 3: stay alive so a screenshot can catch the tray item + the notification OSD, then exit.

use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{Icon, ToolTip, Tray};
use notify_rust::{Notification, Timeout, Urgency};
use std::time::Duration;

/// One fake device: (display name, battery percent, state label).
struct Device {
    name: &'static str,
    percent: u8,
    state: &'static str,
}

fn devices() -> Vec<Device> {
    vec![
        Device { name: "G703 LIGHTSPEED", percent: 88, state: "discharging" },
        Device { name: "G915 TKL", percent: 95, state: "discharging" },
        Device { name: "G733", percent: 0, state: "offline" },
    ]
}

/// Build a 22x22 ARGB32 battery pictogram. No icon-theme dependency.
/// Renders a green battery body with a terminal nub, on a transparent background.
fn battery_icon() -> Vec<Icon> {
    const W: i32 = 22;
    const H: i32 = 22;
    let mut data: Vec<u8> = Vec::with_capacity((W * H * 4) as usize);

    // battery body: x 2..17, y 6..16 ; nub: x 17..20, y 9..13
    let (body_x0, body_x1) = (2, 17);
    let (body_y0, body_y1) = (6, 16);
    let (nub_x0, nub_x1) = (17, 20);
    let (nub_y0, nub_y1) = (9, 13);

    for y in 0..H {
        for x in 0..W {
            let in_body = x >= body_x0 && x < body_x1 && y >= body_y0 && y < body_y1;
            let in_nub = x >= nub_x0 && x < nub_x1 && y >= nub_y0 && y < nub_y1;
            // 1px border of the body drawn in a lighter green.
            let on_border = in_body
                && (x == body_x0 || x == body_x1 - 1 || y == body_y0 || y == body_y1 - 1);

            let (a, r, g, b) = if in_nub {
                (255u8, 160u8, 90u8, 60u8)
            } else if on_border {
                (255, 220, 240, 220)
            } else if in_body {
                (255, 60, 190, 90)
            } else {
                (0, 0, 0, 0)
            };
            // ARGB32, network byte order (big endian): A R G B
            data.extend_from_slice(&[a, r, g, b]);
        }
    }

    vec![Icon { width: W, height: H, data }]
}

struct SolaarTray {
    id: String,
    icon_name: String,
    devs: Vec<Device>,
    clicks: u32,
}

impl Tray for SolaarTray {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        "Solaar".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        battery_icon()
    }

    fn tool_tip(&self) -> ToolTip {
        let (p703, p915) = (self.devs[0].percent, self.devs[1].percent);
        ToolTip {
            icon_name: self.icon_name.clone(),
            icon_pixmap: battery_icon(),
            title: "Solaar".into(),
            description: format!("Solaar — G703 {}%, G915 {}%", p703, p915),
        }
    }

    /// Left click on the tray icon.
    fn activate(&mut self, x: i32, y: i32) {
        self.clicks += 1;
        println!(
            "[1] TRAY  left-click (Activate) #{} received at ({}, {}); firing notify-send",
            self.clicks, x, y
        );
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "Solaar", "Solaar", "Left-click activation received"])
            .spawn();
    }

    /// Middle click.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        println!("[1] TRAY  middle-click (SecondaryActivate) received");
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: "Show devices".into(),
                activate: Box::new(|t: &mut Self| {
                    t.clicks += 1;
                    println!("[1] TRAY  menu: \"Show devices\" activated");
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];

        // One submenu entry per fake device.
        let submenu: Vec<MenuItem<Self>> = self
            .devs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let label = if d.state == "offline" {
                    format!("{} — offline", d.name)
                } else {
                    format!("{} — {}%", d.name, d.percent)
                };
                StandardItem {
                    label,
                    enabled: d.state != "offline",
                    activate: Box::new(move |t: &mut Self| {
                        t.clicks += 1;
                        println!("[1] TRAY  menu: device #{} selected", i);
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        items.push(
            SubMenu {
                label: "Devices".into(),
                submenu,
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit spike".into(),
                activate: Box::new(|_: &mut Self| {
                    println!("[1] TRAY  menu: quit requested");
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

fn main() {
    let stay: u64 = std::env::var("SOLAAR_TRAY_STAY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let tray_id = std::env::var("SOLAAR_TRAY_ID").unwrap_or_else(|_| "solaar".into());
    let tray_icon_name = std::env::var("SOLAAR_TRAY_ICON_NAME").unwrap_or_default();
    let skip_notif = std::env::var("SOLAAR_SKIP_NOTIF").is_ok();

    println!(
        "[0] ENV   pid={} WAYLAND_DISPLAY={:?} XDG_CURRENT_DESKTOP={:?}",
        std::process::id(),
        std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
    );
    println!("[0] ENV   tray id={:?} icon_name={:?} stay={}s", tray_id, tray_icon_name, stay);

    // ---------------------------------------------------------------- step 1: tray icon
    println!(
        "[1] TRAY  registering StatusNotifierItem (title=\"Solaar\", 22x22 ARGB32 pixmap, submenu)"
    );
    let devs = devices();
    for d in &devs {
        println!("[1] TRAY    device: {} — {} ({})", d.name, d.percent, d.state);
    }
    let service = ksni::TrayService::new(SolaarTray {
        id: tray_id.clone(),
        icon_name: tray_icon_name.clone(),
        devs,
        clicks: 0,
    });
    let _handle = service.handle();
    service.spawn();
    println!(
        "[1] TRAY  TrayService::spawn() returned; D-Bus object owned as org.kde.StatusNotifierItem-{}-1",
        std::process::id()
    );
    println!("[1] TRAY  ksni Handle acquired (update()/shutdown() available): tray thread is running");

    // ---------------------------------------------------------------- step 2: notification
    std::thread::sleep(Duration::from_millis(1500));

    println!("[2] NOTIF querying server capabilities");
    match notify_rust::get_server_information() {
        Ok(i) => println!(
            "[2] NOTIF GetServerInformation -> name={:?} vendor={:?} version={:?} spec={:?}",
            i.name, i.vendor, i.version, i.spec_version
        ),
        Err(e) => println!("[2] NOTIF GetServerInformation FAILED: {e}"),
    }
    let caps: Vec<String> = match notify_rust::get_capabilities() {
        Ok(c) => {
            let mut v: Vec<String> = c.iter().cloned().collect();
            v.sort();
            println!("[2] NOTIF GetCapabilities -> {:?}", v);
            v
        }
        Err(e) => {
            println!("[2] NOTIF GetCapabilities FAILED: {e}");
            vec![]
        }
    };
    let actions_supported = caps.iter().any(|c| c == "actions");
    println!("[2] NOTIF server advertises 'actions' = {}", actions_supported);

    let summary = "Solaar battery";
    let body = "G703 LIGHTSPEED Wireless Gaming Mouse: 20% (discharging)";
    println!(
        "[2] NOTIF sending: summary={:?} body={:?} urgency=normal timeout=6s action=(\"show\", \"Show devices\")",
        summary, body
    );

    // show() blocks until the notification is closed -> run it on its own thread.
    let notif_thread = if skip_notif {
        println!("[2] NOTIF send SKIPPED (SOLAAR_SKIP_NOTIF set) — tray-only observation run");
        None
    } else {
        Some(std::thread::spawn(move || {
            let mut n = Notification::new();
            n.appname("Solaar")
                .summary(summary)
                .body(body)
                .icon("battery-caution")
                .urgency(Urgency::Normal)
                .timeout(Timeout::Milliseconds(6000));
            if actions_supported {
                n.action("show", "Show devices");
            }
            match n.show() {
                Ok(h) => {
                    println!(
                        "[2] NOTIF notification accepted by the server (Notify() returned an id)"
                    );
                    h.wait_for_action(|a| println!("[2] NOTIF ACTION INVOKED: {:?}", a));
                    println!("[2] NOTIF notification closed by server/user");
                }
                Err(e) => println!("[2] NOTIF show() FAILED: {e}"),
            }
        }))
    };

    println!(
        "[2] NOTIF dispatched on background thread; staying alive {}s for observation",
        stay
    );
    println!("[3] OBSERVE  waiting {}s — screenshot window is NOW", stay);
    std::thread::sleep(Duration::from_secs(stay));

    let _ = notif_thread;
    println!("[3] EXIT   clean exit after {}s", stay);
}
