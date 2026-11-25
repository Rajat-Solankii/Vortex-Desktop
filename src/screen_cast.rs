use eframe::egui::{Id, Ui};
use std::process::Command;

pub fn button(ui: &mut Ui) {
    let response = ui
        .button("Cast screen")
        .on_hover_text("Mirror Vortex to a compatible TV or wireless display");
    let notice = Id::new("screen cast status");
    if response.clicked() {
        let result = open_controls();
        ui.ctx().data_mut(|data| {
            data.remove::<String>(notice);
            if let Err(error) = result {
                data.insert_temp(notice, error);
            }
        });
    }
    if let Some(message) = ui.ctx().data(|data| data.get_temp::<String>(notice)) {
        ui.label(crate::style::muted(message));
    }
}

#[cfg(target_os = "windows")]
fn open_controls() -> Result<(), String> {
    Command::new("explorer.exe")
        .arg("ms-settings-connectabledevices:devicediscovery")
        .spawn()
        .map(|_| ())
        .map_err(|_| "Open Windows Cast with Win+K.".to_owned())
}

#[cfg(target_os = "macos")]
fn open_controls() -> Result<(), String> {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Displays-Settings.extension")
        .spawn()
        .map(|_| ())
        .map_err(|_| "Open Control Center and choose Screen Mirroring.".to_owned())
}

#[cfg(target_os = "linux")]
fn open_controls() -> Result<(), String> {
    Command::new("gnome-network-displays")
        .spawn()
        .map(|_| ())
        .map_err(|_| "Install GNOME Network Displays to open wireless display controls.".to_owned())
}
