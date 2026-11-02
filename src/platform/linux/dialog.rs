//! Modal alerts through `zenity`; the update progress window has no Linux twin.

pub fn show_progress_dialog(_title: &str, _text: &str) {}

pub fn close_progress_dialog() {}

/// Returns the index of the chosen button. The first button is the default
/// action, the last one is Cancel, and any in between become extra buttons.
/// Closing the dialog, `Esc`, or a missing `zenity` all resolve to the last
/// button so a dismissal never triggers a side effect.
pub fn alert_choice(title: &str, text: &str, buttons: &[&str]) -> usize {
    let zenity_bin = if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" };
    let mut cmd = std::process::Command::new(zenity_bin);
    cmd.args(zenity_args(title, text, buttons));
    cmd.stdout(std::process::Stdio::piped());
    match cmd.output() {
        Ok(out) => choice_index(out.status.success(), &String::from_utf8_lossy(&out.stdout), buttons),
        Err(_) => buttons.len().saturating_sub(1),
    }
}

fn zenity_args(title: &str, text: &str, buttons: &[&str]) -> Vec<String> {
    let mut args: Vec<String> =
        ["--question", "--title", title, "--text", text].into_iter().map(String::from).collect();
    if let Some(first) = buttons.first() {
        args.extend(["--ok-label".into(), first.to_string()]);
    }
    if buttons.len() > 1 {
        args.extend(["--cancel-label".into(), buttons[buttons.len() - 1].to_string()]);
    }
    if buttons.len() > 2 {
        for extra in &buttons[1..buttons.len() - 1] {
            args.extend(["--extra-button".into(), extra.to_string()]);
        }
    }
    args
}

/// zenity exits 0 for the OK button and 1 for everything else; an extra button
/// additionally prints its label on stdout.
fn choice_index(ok: bool, stdout: &str, buttons: &[&str]) -> usize {
    let cancel = buttons.len().saturating_sub(1);
    if ok {
        return 0;
    }
    let label = stdout.trim_end_matches(['\n', '\r']);
    if buttons.len() > 2
        && let Some(i) = buttons[1..cancel].iter().position(|b| *b == label)
    {
        return i + 1;
    }
    cancel
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

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATE: [&str; 3] = ["Update Now", "View Release", "Cancel"];

    #[test]
    fn three_buttons_map_to_ok_extra_and_cancel() {
        let args = zenity_args("t", "x", &UPDATE);
        let tail = &args[5..];
        assert_eq!(
            tail,
            ["--ok-label", "Update Now", "--cancel-label", "Cancel", "--extra-button", "View Release"]
        );
    }

    #[test]
    fn ok_extra_and_dismissal_resolve_to_distinct_indices() {
        assert_eq!(choice_index(true, "", &UPDATE), 0);
        assert_eq!(choice_index(false, "View Release\n", &UPDATE), 1);
        // Cancel button, Esc, window close and a missing zenity all land here.
        assert_eq!(choice_index(false, "", &UPDATE), 2);
    }

    #[test]
    fn two_button_dialog_dismissal_is_not_the_confirm_choice() {
        assert_eq!(choice_index(false, "", &["Install", "Cancel"]), 1);
        assert_eq!(choice_index(true, "", &["Install", "Cancel"]), 0);
    }

    #[test]
    fn unknown_stdout_does_not_pick_a_button() {
        assert_eq!(choice_index(false, "Cancel", &UPDATE), 2);
        assert_eq!(choice_index(false, "Update Now", &UPDATE), 2);
    }
}
