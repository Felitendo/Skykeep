//! What is on the desktop's clipboard, for Drive's Paste: files copied in the file manager
//! (Files, Dolphin, Thunar) are there as a `text/uri-list`, which the window's own clipboard
//! does not hand a page outside an editable field. Read here through GTK, the window's
//! toolkit, which speaks Wayland's clipboard and X11's alike.

use serde::Serialize;

/// The files on the clipboard, as paths on this computer, and its text when it holds no files.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Clipboard {
    files: Vec<String>,
    text: Option<String>,
}

#[tauri::command]
pub async fn clipboard_read(app: tauri::AppHandle) -> Result<Clipboard, String> {
    #[cfg(target_os = "linux")]
    {
        read(&app).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Ok(Clipboard::default())
    }
}

/// Asked on the thread GTK lives on, without waiting there: the answer comes back to it.
#[cfg(target_os = "linux")]
async fn read(app: &tauri::AppHandle) -> Result<Clipboard, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        clipboard.request_uris(move |clipboard, uris| {
            let files: Vec<String> = uris
                .iter()
                .filter_map(|uri| glib::filename_from_uri(uri).ok())
                .map(|(path, _)| path.to_string_lossy().into_owned())
                .collect();
            if files.is_empty() {
                clipboard.request_text(move |_, text| {
                    let _ = tx.send(Clipboard {
                        files,
                        text: text.map(str::to_string),
                    });
                });
            } else {
                let _ = tx.send(Clipboard { files, text: None });
            }
        });
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "The clipboard couldn’t be read.".to_string())
}
