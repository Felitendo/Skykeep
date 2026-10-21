//! Pomander as six apps on one engine: Notes, Reminders, Calendar, Contacts, Drive and Photos,
//! each a window of its own, opened from its own launcher entry (`pomander --app notes`) — one
//! process, one sign-in, one library and one sync behind all of them, as Microsoft 365 is one
//! install with Word and Excel each in the Start menu.
//!
//! Each window is known to the desktop as its app: on Wayland it says its own app ID, so a
//! dock shows Notes and Reminders as two apps with their own icons, each matched to its own
//! launcher entry — not six windows under one Pomander icon. (Measured: GTK 3.24's
//! `gdk_wayland_window_set_application_id` sets it per window, within one process.)

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The apps: the key the window and the engine use, and the name a person sees.
pub const APPS: [(&str, &str); 6] = [
    ("notes", "Notes"),
    ("reminders", "Reminders"),
    ("calendar", "Calendar"),
    ("contacts", "Contacts"),
    ("drivews", "Drive"),
    ("photos", "Photos"),
];

/// The app a command line asks for: `--app notes` (or `--app=notes`); the Drive app answers
/// to "drive" as well as its engine's own "drivews".
pub fn asked(args: &[String]) -> Option<&'static str> {
    let named = args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--app=")
            .map(ToString::to_string)
            .or_else(|| (a == "--app").then(|| args.get(i + 1).cloned()).flatten())
    })?;
    let named = if named == "drive" {
        "drivews".to_string()
    } else {
        named
    };
    APPS.iter()
        .find(|(key, _)| *key == named)
        .map(|(key, _)| *key)
}

/// What a quick-capture launcher action asks for: `--new reminder` or `--new note` (or
/// `--new=note`).
pub fn new_asked(args: &[String]) -> Option<&'static str> {
    let named = args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--new=")
            .map(ToString::to_string)
            .or_else(|| (a == "--new").then(|| args.get(i + 1).cloned()).flatten())
    })?;
    match named.as_str() {
        "reminder" => Some("reminder"),
        "note" => Some("note"),
        _ => None,
    }
}

/// The calendar and contact files a command line hands over — opened from a file manager,
/// as a path (`%F`, Windows) or a `file://` URI (`%U`) — made absolute against `cwd`, the
/// directory the launch came from. Only files of the kinds Pomander opens are taken; any
/// other argument is someone else's business.
pub fn files_asked(args: &[String], cwd: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut skip_next = false;
    let mut out = Vec::new();
    for a in args.iter().skip(1) {
        if std::mem::take(&mut skip_next) {
            continue;
        }
        if a == "--app" || a == "--new" {
            skip_next = true;
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        let path = match a.strip_prefix("file://") {
            Some(uri) => {
                // `file:///home/a/b.ics`, or `file://localhost/…`; on Windows `file:///C:/…`.
                let uri = uri.strip_prefix("localhost").unwrap_or(uri);
                let decoded = percent_decoded(uri);
                let decoded = if cfg!(windows) {
                    decoded.trim_start_matches('/').to_string()
                } else {
                    decoded
                };
                std::path::PathBuf::from(decoded)
            }
            None if a.contains("://") => continue,
            None => std::path::PathBuf::from(a),
        };
        let wanted = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "ics" | "ical" | "icalendar" | "vcf" | "vcard"
            )
        });
        if wanted {
            out.push(if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            });
        }
    }
    out
}

/// `%20` and the rest of a URI's escapes turned back into the bytes they stand for.
fn percent_decoded(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && s.is_char_boundary(i + 3) {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn label(key: &str) -> String {
    format!("app-{key}")
}

/// The app ID a window of `key` is known by: the app's own launcher entry's name, as the
/// desktop matches them (`<identifier>.Notes.desktop`).
pub fn app_id(app: &AppHandle, key: &str) -> String {
    let name = APPS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, n)| *n);
    format!("{}.{name}", app.config().identifier)
}

/// The app's window, brought to the front, or made: its own window, showing that app alone.
pub fn open(app: &AppHandle, key: &str) {
    let Some((key, name)) = APPS.iter().find(|(k, _)| *k == key).copied() else {
        return;
    };
    if let Some(w) = app.get_webview_window(&label(key)) {
        w.unminimize().ok();
        show(&w);
        return;
    }
    let made = WebviewWindowBuilder::new(
        app,
        label(key),
        WebviewUrl::App(format!("index.html?app={key}").into()),
    )
    .title(name)
    .inner_size(1040.0, 720.0)
    .min_inner_size(480.0, 400.0)
    // Shown by its own window_ready, once it has drawn: never a blank frame.
    .visible(false)
    .build();
    let window = match made {
        Ok(w) => w,
        Err(e) => {
            quiet!("the {name} window could not be made: {e}");
            return;
        }
    };
    if crate::tiling_compositor() {
        window.set_decorations(false).ok();
    }
    // Shown by its window_ready once drawn; if that never comes (a page slow to load, a
    // keyring that does not answer), shown anyway — never a launch that opens nothing.
    let w = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if !w.is_visible().unwrap_or(true) {
            show(&w);
        }
    });
}

/// How long a closed app window is kept, put away, before its memory is given back.
const KEPT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// When each put-away app window was closed: shown again, it is taken off the list, so a
/// window closed, reopened and closed again is kept the full while from its last close.
static PUT_AWAY: std::sync::Mutex<Vec<(String, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

/// A closed app window, kept hidden for a while and then closed for good if it was not opened
/// again meanwhile.
pub fn put_away(window: &tauri::Window) {
    let label = window.label().to_string();
    let at = std::time::Instant::now();
    {
        let mut list = PUT_AWAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        list.retain(|(l, _)| *l != label);
        list.push((label.clone(), at));
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(KEPT).await;
        let still = PUT_AWAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|(l, t)| *l == label && *t == at);
        if still {
            PUT_AWAY
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|(l, _)| *l != label);
            if let Some(w) = app.get_webview_window(&label) {
                if !w.is_visible().unwrap_or(true) {
                    w.destroy().ok();
                }
            }
        }
    });
}

/// An app's window put on screen, and — now that it is there — told its own app ID: GTK
/// takes a window's app ID only once the window is shown (measured: set before, it was
/// silently dropped and the window went on saying the program's name).
pub fn show(window: &tauri::WebviewWindow) {
    PUT_AWAY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|(l, _)| l != window.label());
    window.show().ok();
    window.set_focus().ok();
    #[cfg(target_os = "linux")]
    if let Some(key) = window.label().strip_prefix("app-") {
        let id = app_id(window.app_handle(), key);
        let w = window.clone();
        window
            .app_handle()
            .run_on_main_thread(move || own_app_id(&w, &id))
            .ok();
    }
}

/// On Wayland, the window says it is its app, not Pomander as a whole.
#[cfg(target_os = "linux")]
fn own_app_id(window: &tauri::WebviewWindow, id: &str) {
    use glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    extern "C" {
        fn gdk_wayland_window_set_application_id(
            window: *mut gdk::ffi::GdkWindow,
            id: *const std::ffi::c_char,
        );
    }
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    let Some(gdk_window) = gtk_window.window() else {
        return;
    };
    // X11 has no app ID; the call is Wayland's alone.
    if gdk_window.display().type_().name() != "GdkWaylandDisplay" {
        return;
    }
    let Ok(id) = std::ffi::CString::new(id) else {
        return;
    };
    let raw: *mut gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
    // SAFETY: a live, realized GdkWindow of a Wayland display, and a NUL-terminated id that
    // GDK copies before returning.
    unsafe { gdk_wayland_window_set_application_id(raw, id.as_ptr()) };
}

/// Open an app's window from the interface (the tray, a search result for another app).
#[tauri::command]
pub fn app_open(key: String, app: AppHandle) {
    open(&app, &key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_launcher_entry_names_its_app() {
        assert_eq!(asked(&args(&["pomander", "--app", "notes"])), Some("notes"));
        assert_eq!(
            asked(&args(&["pomander", "--app=reminders"])),
            Some("reminders")
        );
        assert_eq!(
            asked(&args(&["pomander", "--app", "drive"])),
            Some("drivews")
        );
        assert_eq!(asked(&args(&["pomander", "--background"])), None);
        assert_eq!(asked(&args(&["pomander", "--app", "mail"])), None);
        assert_eq!(asked(&args(&["pomander", "--app"])), None);
    }

    #[test]
    fn a_launcher_action_asks_for_a_quick_capture() {
        assert_eq!(
            new_asked(&args(&["pomander", "--new", "reminder"])),
            Some("reminder")
        );
        assert_eq!(new_asked(&args(&["pomander", "--new=note"])), Some("note"));
        assert_eq!(new_asked(&args(&["pomander", "--new", "event"])), None);
        assert_eq!(new_asked(&args(&["pomander", "--new"])), None);
        assert_eq!(new_asked(&args(&["pomander", "--app", "notes"])), None);
    }

    #[test]
    fn opened_files_are_taken_as_paths_or_uris() {
        let cwd = std::path::Path::new("/home/a/Downloads");
        let got = files_asked(
            &args(&[
                "pomander",
                "--app",
                "calendar",
                "invite.ics",
                "/tmp/Card.VCF",
                "file:///home/a/My%20Events.ics",
                "notes.txt",
                "https://example.com/x.ics",
            ]),
            cwd,
        );
        assert_eq!(
            got,
            vec![
                std::path::PathBuf::from("/home/a/Downloads/invite.ics"),
                std::path::PathBuf::from("/tmp/Card.VCF"),
                std::path::PathBuf::from("/home/a/My Events.ics"),
            ]
        );
        assert!(files_asked(&args(&["pomander", "--app", "x.ics"]), cwd).is_empty());
    }
}
