//! A file's own bytes written through the folder: kept on this computer first, then sent to
//! Apple behind the app writing them.
//!
//! An app saving a file never waits on the network. What it writes goes into a private copy;
//! a few seconds after the last program closes it, the copy goes to Apple. Replacing a file
//! is done the safe way round — the new one sent, the old one moved into Recently Deleted
//! (where it can be put back for thirty days), the new one given its name — and a file
//! changed on another device meanwhile is never overwritten: this computer's version is kept
//! beside it as a conflicted copy. What is waiting to go is written down, so a restart, a
//! crash or an hour offline loses nothing: it goes when it can.

use crate::Remote;
use quince_core::drive::DriveItem;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// How long a file must be left alone after its last writer closes before it is sent: one
/// save is often several writes and closes, and a scratch file often lives a second.
pub const SETTLE: Duration = Duration::from_secs(2);

/// One file written through the folder and not yet at Apple as written.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    /// The folder it goes into (Apple's id).
    pub folder: String,
    /// Its name, extension and all, as the folder shows it.
    pub name: String,
    /// The Apple file it will replace, as it was when this computer's copy was taken: its
    /// version tells whether it changed elsewhere since. `None` for a new file.
    pub replaces: Option<DriveItem>,
    /// Programs that have it open now (not kept across restarts).
    #[serde(skip)]
    pub open: u32,
    /// Written since it was last sent.
    pub dirty: bool,
    /// Bumped on every write: a send is only counted done for the bytes it sent.
    #[serde(skip)]
    pub generation: u64,
    pub modified: SystemTime,
    /// A send of it is waiting or under way (one at a time; not kept across restarts).
    #[serde(skip)]
    pub sender: bool,
    /// Since when nothing has touched it: it is sent once left alone (SETTLE).
    #[serde(skip)]
    pub quiet_since: Option<std::time::Instant>,
    /// On its way to Apple right now.
    #[serde(skip)]
    pub sending: bool,
    /// Why the last try did not go, said once it has failed (cleared when it goes).
    #[serde(skip)]
    pub failure: Option<String>,
}

/// What is waiting to go, as a person is told it: how many files, how many on their way now,
/// and the reason one would not go, if one would not.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub files: u32,
    pub sending: u32,
    pub failure: Option<String>,
}

/// Names programs use for their own scratch files, which never leave this computer: Vim's
/// swap files and its "4913" probe, Emacs's locks and backups, LibreOffice's lock files,
/// GLib's atomic-save temporaries, and anything ending "~".
#[must_use]
pub fn stays_local(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name == "4913"
        || name.ends_with('~')
        || name.starts_with(".#")
        || name.starts_with(".~lock.")
        || name.starts_with(".goutputstream-")
        || [".swp", ".swo", ".swx", ".tmp", ".part", ".crdownload"]
            .iter()
            .any(|s| lower.ends_with(s))
        || (name.starts_with('#') && name.ends_with('#'))
}

/// A conflicted copy's name, as Dropbox and OneDrive name theirs: "Report (conflicted copy
/// 2026-10-20 18:40).docx" — beside the other device's version, never over it.
#[must_use]
pub fn conflicted(name: &str, when: SystemTime) -> String {
    let secs = when
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 as a calendar date (UTC), without a date crate for one label.
    let (y, m, d) = civil(i64::try_from(days).unwrap_or(0));
    let stamp = format!(
        "{y:04}-{m:02}-{d:02} {:02}.{:02}",
        rest / 3600,
        (rest % 3600) / 60
    );
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (conflicted copy {stamp}).{ext}"),
        _ => format!("{name} (conflicted copy {stamp})"),
    }
}

/// Days since 1970-01-01 as (year, month, day): Howard Hinnant's civil_from_days.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// What is waiting to go, by inode, with its bytes in a folder of its own on disk.
pub struct Writes {
    dir: PathBuf,
    pub(crate) items: Mutex<HashMap<u64, Pending>>,
    /// What this computer last sent under each name ("folder/name"): the next save of that
    /// name replaces it as its own, never mistaking it for a change made elsewhere.
    pub(crate) sent: Mutex<HashMap<String, DriveItem>>,
    /// One send at a time per name ("folder/name"): a replace is three steps at Apple (send,
    /// old into Recently Deleted, rename), and a second save of the same name arriving
    /// between them took the name and left the first as "name 2" — measured live.
    lanes: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl Writes {
    /// # Errors
    /// When the folder cannot be made.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            items: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashMap::new()),
            lanes: Mutex::new(HashMap::new()),
        })
    }

    /// Where a pending file's bytes are: a folder of its own, the file named as it will be
    /// sent (Apple names an upload after the file).
    #[must_use]
    pub fn staging(&self, ino: u64, name: &str) -> PathBuf {
        self.dir.join(ino.to_string()).join(name)
    }

    /// The bytes moved when the file is renamed before it is sent.
    ///
    /// # Errors
    /// When the disk refuses.
    pub fn rename_staging(&self, ino: u64, from: &str, to: &str) -> std::io::Result<()> {
        if from == to {
            return Ok(());
        }
        std::fs::rename(self.staging(ino, from), self.staging(ino, to))
    }

    /// The bytes let go of, once sent or given up.
    pub fn drop_staging(&self, ino: u64) {
        let _ = std::fs::remove_dir_all(self.dir.join(ino.to_string()));
    }

    /// What is waiting to go: scratch files, which never go, are not counted.
    #[must_use]
    pub fn waiting(&self) -> Waiting {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut w = Waiting::default();
        for p in items.values().filter(|p| p.dirty && !stays_local(&p.name)) {
            w.files += 1;
            w.sending += u32::from(p.sending);
            if w.failure.is_none() {
                w.failure.clone_from(&p.failure);
            }
        }
        w
    }

    /// The lane sends under one name wait in, one after another.
    pub fn lane(&self, key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut lanes = self
            .lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::sync::Arc::clone(lanes.entry(key.to_string()).or_default())
    }

    fn journal(&self) -> PathBuf {
        self.dir.join("pending.json")
    }

    /// What is waiting written down, whole or not at all, so a restart finds it.
    pub fn save(&self) {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keep: Vec<(u64, &Pending)> = items
            .iter()
            .filter(|(_, p)| p.dirty && !stays_local(&p.name))
            .map(|(i, p)| (*i, p))
            .collect();
        if let Ok(json) = serde_json::to_vec(&keep) {
            let part = self.journal().with_extension("json.part");
            if std::fs::write(&part, json).is_ok() {
                let _ = std::fs::rename(&part, self.journal());
            }
        }
    }

    /// What was waiting when the folder last went away, with where its bytes are.
    #[must_use]
    pub fn load(&self) -> Vec<(u64, Pending)> {
        std::fs::read(self.journal())
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(u64, Pending)>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(ino, p)| self.staging(*ino, &p.name).exists())
            .collect()
    }
}

/// How a send ended: the file as Apple now has it, and whether it went as a conflicted copy.
pub struct Sent {
    pub item: DriveItem,
    pub conflicted: bool,
}

/// One pending file sent to Apple, the safe way round (see the module's notes). `folder` is
/// the folder's item; `staged` the bytes, named as the file is to be called; `base` the
/// version of Apple's file this computer's copy was taken from; `ours` what this computer
/// itself last sent under that name.
///
/// What it replaces is found when it is sent, not when it was opened: by the version it was
/// taken from, else by its name. It is replaced only if it is that version or this
/// computer's own last send; anything else is a change made elsewhere, and both are kept.
///
/// # Errors
/// When Apple cannot be reached or refuses; nothing is lost — the copy stays, and the send
/// is tried again.
pub async fn send(
    remote: &dyn Remote,
    folder: &DriveItem,
    staged: &Path,
    base: Option<&DriveItem>,
    ours: Option<&DriveItem>,
) -> Result<Sent, String> {
    let name = staged
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("no name")?
        .to_string();
    let now = remote.list(&folder.drivewsid).await?;
    let current = base
        .and_then(|b| now.iter().find(|i| i.drivewsid == b.drivewsid))
        .or_else(|| now.iter().find(|i| i.display_name() == name))
        .cloned();
    let same = |a: &DriveItem, b: &DriveItem| {
        a.drivewsid == b.drivewsid && a.etag == b.etag && a.size == b.size
    };
    let changed_elsewhere = current
        .as_ref()
        .is_some_and(|c| !(base.is_some_and(|b| same(b, c)) || ours.is_some_and(|o| same(o, c))));
    if changed_elsewhere {
        // Both kept: this computer's beside the other device's, never over it.
        let copy = staged.with_file_name(conflicted(&name, SystemTime::now()));
        std::fs::copy(staged, &copy).map_err(|e| e.to_string())?;
        let sent = remote.upload(folder, &copy).await;
        let _ = std::fs::remove_file(&copy);
        return Ok(Sent {
            item: sent?,
            conflicted: true,
        });
    }
    let new = remote.upload(folder, staged).await?;
    if let Some(old) = current {
        // The old one into Recently Deleted, then the new one given its name: if anything
        // fails between, both are still there to be found — nothing is ever lost.
        remote.delete(&old).await?;
        if new.display_name() != name {
            remote.rename(&new, &name).await?;
        }
        let after = remote.list(&folder.drivewsid).await?;
        let item = after
            .into_iter()
            .find(|i| i.drivewsid == new.drivewsid)
            .unwrap_or(new);
        return Ok(Sent {
            item,
            conflicted: false,
        });
    }
    Ok(Sent {
        item: new,
        conflicted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_files_never_leave() {
        for n in [
            ".report.docx.swp",
            "4913",
            "notes.txt~",
            ".#notes.txt",
            ".~lock.Deck.odp#",
            ".goutputstream-ABC123",
            "x.crdownload",
        ] {
            assert!(stays_local(n), "{n}");
        }
        for n in ["Report.docx", "notes.txt", "photo.jpg", "a~b.txt"] {
            assert!(!stays_local(n), "{n}");
        }
    }

    #[test]
    fn a_conflicted_copy_is_named_beside() {
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_521_600); // 2026-10-20 18:40 UTC
        assert_eq!(
            conflicted("Report.docx", when),
            "Report (conflicted copy 2026-10-20 18.40).docx"
        );
        assert_eq!(
            conflicted("Makefile", when),
            "Makefile (conflicted copy 2026-10-20 18.40)"
        );
    }

    #[test]
    fn what_waits_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let w = Writes::new(dir.path()).unwrap();
        let p = Pending {
            folder: "F".into(),
            name: "a.txt".into(),
            replaces: None,
            open: 1,
            dirty: true,
            generation: 3,
            modified: SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            failure: None,
        };
        std::fs::create_dir_all(w.staging(7, "a.txt").parent().unwrap()).unwrap();
        std::fs::write(w.staging(7, "a.txt"), b"hi").unwrap();
        w.items.lock().unwrap().insert(7, p.clone());
        w.items.lock().unwrap().insert(
            8,
            Pending {
                name: ".a.txt.swp".into(),
                ..p.clone()
            },
        );
        w.save();
        let again = Writes::new(dir.path()).unwrap().load();
        assert_eq!(again.len(), 1, "the scratch file is not kept");
        assert_eq!(again[0].0, 7);
        assert_eq!(again[0].1.name, "a.txt");
        assert_eq!(again[0].1.open, 0, "nothing is open after a restart");
    }
}
