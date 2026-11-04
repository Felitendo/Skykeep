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
            let files = handed_over(
                uris.iter()
                    .filter_map(|uri| glib::filename_from_uri(uri).ok())
                    .map(|(path, _)| path),
            );
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

/// Files copied in the file manager were handed over by the person, as a drop is: granted for
/// reading (grants.rs), so Drive's Paste can upload them. Answered as the window names them.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn handed_over(paths: impl Iterator<Item = std::path::PathBuf>) -> Vec<String> {
    paths
        .map(|path| {
            crate::grants::grant_read(&path);
            path.to_string_lossy().into_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// Files on the clipboard can be uploaded by Paste: what clipboard_read hands the window
    /// is granted for reading, and nothing else is.
    #[test]
    fn files_on_the_clipboard_may_be_pasted_and_no_others() {
        let dir = tempfile::tempdir().unwrap();
        let (copied, other) = (dir.path().join("copied.jpg"), dir.path().join("other.jpg"));
        std::fs::write(&copied, b"x").unwrap();
        std::fs::write(&other, b"x").unwrap();
        let named = super::handed_over(std::iter::once(copied.clone()));
        assert_eq!(named, vec![copied.to_string_lossy().into_owned()]);
        assert!(crate::grants::take_read(&named[0]).is_ok());
        assert!(crate::grants::take_read(&other.to_string_lossy()).is_err());
    }
}
