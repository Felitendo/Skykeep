//! Open at Login: Quince started in the background when the person logs in — its window
//! hidden, everything behind it running — so reminders ring, changes sync and the folders are
//! up without the window ever being opened. Kept the standard way, as an XDG autostart entry
//! (`~/.config/autostart`), which the desktops run themselves; nothing of Quince's own runs it.
//!
//! Some compositors run no autostart entries (Hyprland, Sway and the like, unless started
//! through uwsm). There Settings says so, with the one line that does the same in their
//! own startup — written by the person, never by Quince into their config.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

/// Started by the login entry: the window stays hidden unless something needs the person.
static BACKGROUND: AtomicBool = AtomicBool::new(false);
/// Whether the first window_ready has been answered: only that one may keep it hidden.
static ASKED: AtomicBool = AtomicBool::new(false);

/// What the login entry passes to start hidden.
const FLAG: &str = "--background";

/// Read once at launch, from the command line.
pub fn note_launch() {
    BACKGROUND.store(std::env::args().any(|a| a == FLAG), Ordering::SeqCst);
}

/// Whether this run was started at login, hidden.
pub fn background() -> bool {
    BACKGROUND.load(Ordering::SeqCst)
}

/// Whether the window should stay hidden now: started at login, and nothing needs the person
/// (a code to enter, a sign-in). Only the first answer can be yes; after that the window
/// shows as usual.
pub fn stay_hidden(needs_you: bool) -> bool {
    background() && !ASKED.swap(true, Ordering::SeqCst) && !needs_you
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginItem {
    pub on: bool,
    /// The command the entry runs, for a startup file that does not read autostart entries.
    pub command: String,
    /// Whether this desktop runs autostart entries at login. `false`: the entry alone does
    /// nothing here, and `command` has to go in the compositor's own startup.
    pub honoured: bool,
}

fn entry(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .config_dir()
        .ok()
        .map(|c| c.join("autostart").join("dev.quince.client.desktop"))
}

/// The command that starts this Quince hidden: the AppImage itself when run from one (its
/// own binary lives in a mount that is gone after a restart), else the program running now.
fn command() -> String {
    let program = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("quince-app"));
    format!("{} {FLAG}", quoted(&program.to_string_lossy()))
}

/// A program path as the Desktop Entry spec wants it in Exec: in double quotes, with `"`,
/// `` ` ``, `$` and `\` escaped.
fn quoted(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn desktop_file(command: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Quince\nComment=Keeps iCloud in sync in the background\nExec={command}\nIcon=dev.quince.client\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// Desktops that run autostart entries themselves.
const RUNS_AUTOSTART: [&str; 10] = [
    "GNOME",
    "KDE",
    "XFCE",
    "X-Cinnamon",
    "MATE",
    "LXQt",
    "LXDE",
    "Budgie",
    "COSMIC",
    "Unity",
];

/// Whether autostart entries run at login here: a desktop that runs them, or a session that
/// started systemd's autostart target (uwsm, and the desktops that use it).
fn honoured() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop
        .split(':')
        .any(|d| RUNS_AUTOSTART.iter().any(|k| d.eq_ignore_ascii_case(k)))
    {
        return true;
    }
    std::process::Command::new("systemctl")
        .args([
            "--user",
            "is-active",
            "--quiet",
            "xdg-desktop-autostart.target",
        ])
        .status()
        .is_ok_and(|s| s.success())
}

#[tauri::command]
pub async fn login_item_status(app: AppHandle) -> Result<LoginItem, String> {
    let on = entry(&app).is_some_and(|p| p.exists());
    let honoured = tauri::async_runtime::spawn_blocking(honoured)
        .await
        .unwrap_or(false);
    Ok(LoginItem {
        on,
        command: command(),
        honoured,
    })
}

#[tauri::command]
pub async fn login_item_set(on: bool, app: AppHandle) -> Result<LoginItem, String> {
    let path = entry(&app).ok_or("There is no configuration folder to keep it in.")?;
    if on {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, desktop_file(&command())).map_err(|e| e.to_string())?;
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    login_item_status(app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Exec line survives any path: spaces quoted, the spec's special characters escaped.
    #[test]
    fn the_exec_line_is_quoted_as_the_spec_says() {
        assert_eq!(
            quoted("/opt/Quince App/quince"),
            "\"/opt/Quince App/quince\""
        );
        assert_eq!(quoted("/a/$HOME\"x`\\"), "\"/a/\\$HOME\\\"x\\`\\\\\"");
        let file = desktop_file("\"/usr/bin/quince-app\" --background");
        assert!(file.contains("Exec=\"/usr/bin/quince-app\" --background\n"));
        assert!(file.starts_with("[Desktop Entry]\nType=Application\n"));
    }
}
