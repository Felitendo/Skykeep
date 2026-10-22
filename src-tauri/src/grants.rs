//! Which files the window may name: only those the person handed over.
//!
//! Commands that read or write a file on this computer take its path from the window. The
//! window is Skykeep's own page, but it is still a web page, so it is never trusted to name a
//! file on its own: a path is accepted only once the person has chosen it — in an open or
//! save dialog Skykeep shows from here ([`pick_open`], [`pick_save`]), by dropping it on the
//! window, or by opening it with Skykeep from the system. Each of those records a grant here,
//! and each command that touches a file asks for one first ([`take_read`], [`take_write`]).
//!
//! - A grant lapses after [`LASTS`]: a choice is acted on at once, not kept for later.
//! - A write grant is used up by the one write it was given for.
//! - A folder granted for reading covers what is in it (a folder dropped on Drive uploads
//!   whole); a folder granted for writing is the folder itself, not what is in it.
//! - Every path is compared resolved — `..` and links followed — so a granted folder cannot be
//!   walked out of by a name that climbs, or by a link inside it that points elsewhere.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

/// How long a choice stays good for.
pub const LASTS: Duration = Duration::from_secs(10 * 60);

const REFUSED: &str = "Skykeep only opens or saves files you choose. Choose the file again.";

#[derive(Default)]
struct Registry {
    /// Resolved path → when it was granted, and whether it is a folder (covering its contents).
    reads: HashMap<PathBuf, (Instant, bool)>,
    /// Resolved path → when it was granted. Removed by the write it is for.
    writes: HashMap<PathBuf, Instant>,
}

impl Registry {
    fn prune(&mut self, now: Instant) {
        let live = |at: &Instant| now.saturating_duration_since(*at) < LASTS;
        self.reads.retain(|_, (at, _)| live(at));
        self.writes.retain(|_, at| live(at));
    }

    fn grant_read(&mut self, path: &Path, now: Instant) {
        if let Some(p) = resolved(path) {
            let dir = p.is_dir();
            self.reads.insert(p, (now, dir));
        }
    }

    fn grant_write(&mut self, path: &Path, now: Instant) {
        if let Some(p) = resolved(path) {
            self.writes.insert(p, now);
        }
    }

    fn take_read(&mut self, path: &Path, now: Instant) -> bool {
        self.prune(now);
        let Some(p) = resolved(path) else {
            return false;
        };
        p.ancestors().any(|a| match self.reads.get(a) {
            Some((_, dir)) => a == p || *dir,
            None => false,
        })
    }

    fn take_write(&mut self, path: &Path, now: Instant) -> bool {
        self.prune(now);
        resolved(path).is_some_and(|p| self.writes.remove(&p).is_some())
    }
}

static GRANTS: Mutex<Option<Registry>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    let mut g = GRANTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(g.get_or_insert_with(Registry::default))
}

/// A path as the file system resolves it: absolute, `..` and links followed. A file not
/// there yet (a save's) is its folder resolved, plus its own plain name. `None` for anything
/// else: a relative path, or one whose last part is not a plain name.
fn resolved(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    if let Ok(p) = std::fs::canonicalize(path) {
        return Some(p);
    }
    let name = match path.components().next_back()? {
        Component::Normal(name) => name,
        _ => return None,
    };
    let parent = std::fs::canonicalize(path.parent()?).ok()?;
    Some(parent.join(name))
}

/// The person chose `path` to be read (a file, or a folder and what is in it).
pub fn grant_read(path: &Path) {
    with(|r| r.grant_read(path, Instant::now()));
}

/// The person chose `path` to be written, once.
pub fn grant_write(path: &Path) {
    with(|r| r.grant_write(path, Instant::now()));
}

/// Whether `path` may be read: chosen by the person, a short while ago.
pub fn take_read(path: &str) -> Result<(), String> {
    if with(|r| r.take_read(Path::new(path), Instant::now())) {
        Ok(())
    } else {
        Err(REFUSED.into())
    }
}

/// Whether `path` may be written: chosen by the person for this, a short while ago. Uses the
/// grant up.
pub fn take_write(path: &str) -> Result<(), String> {
    if with(|r| r.take_write(Path::new(path), Instant::now())) {
        Ok(())
    } else {
        Err(REFUSED.into())
    }
}

/// One file type offered in a dialog: "Contacts" and `["vcf", "csv"]`.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

fn dialog(
    window: &tauri::Window,
    title: Option<String>,
    filters: &[Filter],
) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let mut d = window.app_handle().dialog().file();
    if let Some(title) = title {
        d = d.set_title(title);
    }
    for f in filters {
        let ext: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        d = d.add_filter(&f.name, &ext);
    }
    d
}

fn paths(chosen: Option<Vec<tauri_plugin_dialog::FilePath>>) -> Option<Vec<PathBuf>> {
    chosen.map(|v| v.into_iter().filter_map(|f| f.into_path().ok()).collect())
}

/// The open dialog, shown from here: what the person chose, granted for reading (a folder
/// for writing into as well, once — "Choose where to export"). `None` when cancelled.
#[tauri::command]
pub async fn pick_open(
    window: tauri::Window,
    filters: Option<Vec<Filter>>,
    multiple: Option<bool>,
    directory: Option<bool>,
    title: Option<String>,
) -> Result<Option<Vec<String>>, String> {
    let (multiple, directory) = (multiple.unwrap_or(false), directory.unwrap_or(false));
    #[allow(unused_mut)]
    let mut d = dialog(&window, title, &filters.unwrap_or_default());
    #[cfg(any(windows, target_os = "macos"))]
    {
        d = d.set_parent(&window);
    }
    let chosen = tauri::async_runtime::spawn_blocking(move || match (directory, multiple) {
        (true, true) => paths(d.blocking_pick_folders()),
        (true, false) => paths(d.blocking_pick_folder().map(|f| vec![f])),
        (false, true) => paths(d.blocking_pick_files()),
        (false, false) => paths(d.blocking_pick_file().map(|f| vec![f])),
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(chosen.map(|chosen| {
        chosen
            .into_iter()
            .map(|p| {
                grant_read(&p);
                if directory {
                    grant_write(&p);
                }
                p.to_string_lossy().into_owned()
            })
            .collect()
    }))
}

/// The save dialog, shown from here, offering `default_name`: where the person chose,
/// granted for one write. `None` when cancelled.
#[tauri::command]
pub async fn pick_save(
    window: tauri::Window,
    default_name: Option<String>,
    filters: Option<Vec<Filter>>,
    title: Option<String>,
) -> Result<Option<String>, String> {
    let mut d = dialog(&window, title, &filters.unwrap_or_default()).set_parent(&window);
    // A name, never a place: the page does not get to pick the folder the dialog opens in.
    if let Some(name) = default_name
        .as_deref()
        .and_then(|n| Path::new(n).file_name())
    {
        d = d.set_file_name(name.to_string_lossy());
    }
    let chosen = tauri::async_runtime::spawn_blocking(move || d.blocking_save_file())
        .await
        .map_err(|e| e.to_string())?;
    Ok(chosen.and_then(|f| f.into_path().ok()).map(|p| {
        grant_write(&p);
        p.to_string_lossy().into_owned()
    }))
}

/// Files dropped on a window from this computer: the person handed them over.
pub fn dropped(event: &tauri::DragDropEvent) {
    if let tauri::DragDropEvent::Drop { paths, .. } = event {
        for p in paths {
            grant_read(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(r: &mut Registry) -> Instant {
        let now = Instant::now();
        r.prune(now);
        now
    }

    #[test]
    fn a_granted_write_is_taken_once_and_an_ungranted_path_never() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Note.md");
        let mut r = Registry::default();
        let now = at(&mut r);
        assert!(!r.take_write(&file, now), "nothing granted yet");
        r.grant_write(&file, now);
        assert!(r.take_write(&file, now));
        assert!(!r.take_write(&file, now), "a write grant is used up");
        assert!(!r.take_write(&dir.path().join("Other.md"), now));
        assert!(
            !r.take_read(&file, now),
            "a write grant is not a read grant"
        );
    }

    #[test]
    fn a_read_grant_lasts_a_while_then_lapses() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.jpg");
        std::fs::write(&file, b"x").unwrap();
        let mut r = Registry::default();
        let now = at(&mut r);
        r.grant_read(&file, now);
        assert!(r.take_read(&file, now));
        assert!(r.take_read(&file, now), "reads may be asked again");
        assert!(r.take_read(&file, now + LASTS - Duration::from_secs(1)));
        assert!(!r.take_read(&file, now + LASTS), "lapsed");
        r.grant_write(&file, now);
        assert!(!r.take_write(&file, now + LASTS + Duration::from_secs(1)));
    }

    #[test]
    fn a_folder_covers_its_contents_for_reading_not_its_neighbours() {
        let base = tempfile::tempdir().unwrap();
        let holiday = base.path().join("Holiday");
        std::fs::create_dir_all(holiday.join("Day 1")).unwrap();
        std::fs::write(holiday.join("Day 1/a.jpg"), b"x").unwrap();
        std::fs::write(base.path().join("secret.txt"), b"x").unwrap();
        let mut r = Registry::default();
        let now = at(&mut r);
        r.grant_read(&holiday, now);
        assert!(r.take_read(&holiday.join("Day 1/a.jpg"), now));
        assert!(!r.take_read(&base.path().join("secret.txt"), now));
        // A file's grant covers nothing below or beside it.
        let file = holiday.join("Day 1/a.jpg");
        let mut r = Registry::default();
        r.grant_read(&file, now);
        assert!(!r.take_read(&holiday.join("Day 1"), now));
        // A folder granted for writing is the folder, not a file in it.
        r.grant_write(&holiday, now);
        assert!(!r.take_write(&holiday.join("Day 1/a.jpg"), now));
        assert!(r.take_write(&holiday, now));
    }

    #[test]
    fn climbing_out_and_links_out_are_refused() {
        let base = tempfile::tempdir().unwrap();
        let granted = base.path().join("granted");
        std::fs::create_dir(&granted).unwrap();
        let outside = base.path().join("outside.txt");
        std::fs::write(&outside, b"x").unwrap();
        let mut r = Registry::default();
        let now = at(&mut r);
        r.grant_read(&granted, now);
        assert!(!r.take_read(&granted.join("../outside.txt"), now), "..");
        assert!(
            r.take_read(&granted.join("../granted/."), now),
            "the same folder, spelled oddly"
        );
        #[cfg(unix)]
        {
            let link = granted.join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(!r.take_read(&link, now), "a link inside pointing outside");
        }
        // A save's grant is for the place it names, however it is spelled.
        std::fs::create_dir(granted.join("sub")).unwrap();
        r.grant_write(&granted.join("new.md"), now);
        assert!(!r.take_write(&granted.join("../new.md"), now));
        assert!(
            r.take_write(&granted.join("sub/../new.md"), now),
            "the same place, spelled oddly"
        );
        #[cfg(unix)]
        {
            // A save's place that became a link to somewhere else is not the place granted.
            let save = granted.join("later.md");
            r.grant_write(&save, now);
            std::os::unix::fs::symlink(&outside, &save).unwrap();
            assert!(!r.take_write(&save, now));
        }
        assert!(
            !r.take_write(Path::new("new.md"), now),
            "relative paths are never granted"
        );
        assert!(!r.take_write(&granted.join(".."), now));
    }
}
