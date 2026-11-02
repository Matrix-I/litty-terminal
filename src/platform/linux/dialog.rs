//! Modal alerts through `zenity`; the update progress window has no Linux twin.

pub fn show_progress_dialog(_title: &str, _text: &str) {}

pub fn close_progress_dialog() {}

pub fn alert_choice(title: &str, text: &str, buttons: &[&str]) -> usize {
    let zenity_bin = if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" };
    let mut cmd = std::process::Command::new(zenity_bin);
    cmd.args(["--question", "--title", title, "--text", text]);
    if let Some(first) = buttons.first() {
        cmd.args(["--ok-label", first]);
    }
    if let Some(second) = buttons.get(1) {
        cmd.args(["--cancel-label", second]);
    }
    match cmd.status() {
        Ok(status) if status.success() => 0,
        _ => 1,
    }
}

pub fn confirm(title: &str, text: &str, button: &str) -> bool {
    alert_choice(title, text, &[button, "Cancel"]) == 0
}

pub fn alert(title: &str, text: &str) {
    let zenity_bin = if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" };
    let _ = std::process::Command::new(zenity_bin)
        .args(["--info", "--title", title, "--text", text])
        .status();
}
