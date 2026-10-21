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
        w.show().ok();
        w.unminimize().ok();
        w.set_focus().ok();
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
    #[cfg(target_os = "linux")]
    {
        let id = app_id(app, key);
        let w = window.clone();
        app.run_on_main_thread(move || own_app_id(&w, &id)).ok();
    }
}

/// On Wayland, the window says it is its app, not Pomander as a whole. Set before it is first
/// shown, so a dock and the compositor's rules see it as that app from the start.
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
    gtk_window.realize();
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
}
