use slint::{ModelRc, VecModel};
use std::time::{SystemTime, UNIX_EPOCH};

slint::include_modules!();

fn ms_since_epoch() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;

    let devices = vec![
        Device {
            icon: "M".into(),
            name: "G703 LIGHTSPEED".into(),
            kind: "Mouse".into(),
            connection: "Lightspeed".into(),
            percent: 88,
            millivolts: 4051,
            status: "Discharging".into(),
            updated: "12s ago".into(),
        },
        Device {
            icon: "K".into(),
            name: "G915 TKL".into(),
            kind: "Keyboard".into(),
            connection: "Lightspeed".into(),
            percent: 95,
            millivolts: 4122,
            status: "Discharging".into(),
            updated: "12s ago".into(),
        },
        Device {
            icon: "H".into(),
            name: "G733".into(),
            kind: "Headset".into(),
            connection: "USB".into(),
            percent: 0,
            millivolts: 0,
            status: "Offline".into(),
            updated: "3h ago".into(),
        },
    ];

    ui.set_devices(ModelRc::new(VecModel::from(devices)));

    ui.show()?;
    // Emitted the instant the toolkit has created and mapped the window surface.
    eprintln!("WINDOW_SHOWN_MS={}", ms_since_epoch());

    slint::run_event_loop().unwrap();
    Ok(())
}
