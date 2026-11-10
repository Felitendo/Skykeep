//! iCloud Drive on the phone, for Android's Files app (the app's DocumentsProvider): folders read
//! from Apple and kept in the phone's library, a file fetched whole when opened, and a file
//! written by another app sent back through the desktop's own write-back
//! (`quince_vfs::writes::send`): into that same file when it is still the version opened, beside
//! it as a conflicted copy when it changed elsewhere meanwhile, never over another device's work.

use crate::{failed, runtime, Account, EngineError};
use quince_core::drive::DriveItem;
use quince_vfs::{Fut, Remote};
use std::path::Path;
use std::sync::Arc;

/// An iCloud Drive item, as the Files app lists it.
#[derive(uniffi::Record)]
pub struct DItem {
    /// Apple's id: `FOLDER::…` or `FILE::…`.
    pub id: String,
    pub parent_id: Option<String>,
    /// The name with its extension, as a file manager shows it.
    pub name: String,
    pub folder: bool,
    pub size: Option<i64>,
    /// Unix seconds.
    pub modified_at: Option<i64>,
    /// How many items a folder holds, when Apple says.
    pub children: Option<i64>,
}

fn ditem(i: &DriveItem) -> DItem {
    DItem {
        id: i.drivewsid.clone(),
        parent_id: i.parent_id.clone(),
        name: i.display_name(),
        folder: i.is_container(),
        size: i.size,
        modified_at: i.modified_at,
        children: i.child_count,
    }
}

/// The engine's way to Apple for the desktop's file tree (`quince_vfs::Remote`), on the phone:
/// the same calls the desktop's makes (src-tauri/src/mount.rs), with the phone's session and
/// library.
#[derive(Clone)]
struct PhoneRemote(Arc<Session>);

/// What every call to Apple needs, shared by the futures the file tree keeps.
struct Session {
    http: reqwest::Client,
    drivews: String,
    docws: String,
    setup: String,
    params: Vec<(String, String)>,
    library: Arc<quince_sync::library_lock::LibraryLock>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl std::ops::Deref for PhoneRemote {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0
    }
}

impl Session {
    /// iCloud with no room for `grows` more bytes: said as that, never an opaque refusal.
    async fn room_for(&self, grows: u64) -> Result<(), String> {
        if grows == 0 {
            return Ok(());
        }
        if let Ok(s) =
            quince_icloud::drive::storage(&self.http, &self.setup, self.params.clone()).await
        {
            if s.total_bytes > 0 && s.used_bytes.saturating_add(grows) > s.total_bytes {
                return Err("Your iCloud storage is full.".into());
            }
        }
        Ok(())
    }
}

impl Remote for PhoneRemote {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let folder = folder.to_string();
        let me = self.clone();
        Box::pin(async move {
            let items = quince_icloud::drive::fetch_folder(
                &me.http,
                &me.drivews,
                me.params.clone(),
                &folder,
            )
            .await
            .map_err(text)?;
            let mut l = me.library.lock().await;
            quince_icloud::drive::store_folder(&mut l, &folder, &items).map_err(text)?;
            Ok(items)
        })
    }

    fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
        // Files are fetched whole for the Files app (`drive_download`), never read in pieces.
        Box::pin(async { Err("read in parts is not used on the phone".into()) })
    }

    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        let (parent, name) = (parent.drivewsid.clone(), name.to_string());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::create_folder(
                &me.http,
                &me.drivews,
                me.params.clone(),
                &parent,
                &name,
            )
            .await
            .map_err(text)
        })
    }

    fn rename(&self, item: &DriveItem, name: &str) -> Fut<()> {
        let (item, name) = (item.clone(), name.to_string());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::rename(&me.http, &me.drivews, me.params.clone(), &item, &name)
                .await
                .map_err(text)
        })
    }

    fn move_to(&self, item: &DriveItem, folder: &DriveItem) -> Fut<()> {
        let (item, to) = (item.clone(), folder.drivewsid.clone());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::move_to(&me.http, &me.drivews, me.params.clone(), &item, &to)
                .await
                .map_err(text)
        })
    }

    fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
        let (folder, path) = (folder.drivewsid.clone(), path.to_path_buf());
        let me = self.clone();
        Box::pin(async move {
            let len = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
            me.room_for(len).await?;
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            quince_icloud::drive::upload(
                &me.http,
                &me.docws,
                &me.drivews,
                me.params.clone(),
                &folder,
                &path,
                progress,
            )
            .await
            .map_err(text)
        })
    }

    fn replace(&self, item: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
        let (item, path) = (item.clone(), path.to_path_buf());
        let me = self.clone();
        Box::pin(async move {
            let len = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
            let was = item.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0);
            me.room_for(len.saturating_sub(was)).await?;
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            match quince_icloud::drive::replace_contents(
                &me.http,
                &me.docws,
                &me.drivews,
                me.params.clone(),
                &item,
                &path,
                progress,
            )
            .await
            {
                Ok(now) => Ok(Some(now)),
                // Changed on another device first: nothing written; the write-back keeps both.
                Err(quince_icloud::Error::Conflict) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
    }

    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let item = item.clone();
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::delete(&me.http, &me.drivews, me.params.clone(), &item)
                .await
                .map_err(text)
        })
    }

    fn space(&self) -> Fut<(u64, u64)> {
        let me = self.clone();
        Box::pin(async move {
            let s = quince_icloud::drive::storage(&me.http, &me.setup, me.params.clone())
                .await
                .map_err(text)?;
            Ok((s.used_bytes, s.total_bytes))
        })
    }
}

impl Account {
    fn drive(&self) -> Result<PhoneRemote, EngineError> {
        let (library, _) = self.library()?;
        let c = self.client.lock().map_err(failed)?;
        let s = c.session();
        let need = |name: &str| {
            s.service_url(name)
                .ok_or_else(|| failed("Apple hasn't offered iCloud Drive for this account."))
        };
        Ok(PhoneRemote(Arc::new(Session {
            drivews: need("drivews")?,
            docws: need("docws")?,
            setup: c.setup_url().to_string(),
            http: c.http().clone(),
            params: c.service_params(),
            library,
        })))
    }

    fn known(&self, id: &str) -> Result<DriveItem, EngineError> {
        // The top of iCloud Drive is never one of a folder's items, so never in the library: it
        // is known by its id. Without this, nothing could be made or moved at the top.
        if id == quince_core::drive::ROOT {
            return Ok(DriveItem {
                drivewsid: id.to_string(),
                parent_id: None,
                name: "iCloud Drive".into(),
                extension: None,
                kind: "FOLDER".into(),
                size: None,
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: None,
            });
        }
        let (lock, _) = self.library()?;
        runtime()
            .block_on(async { lock.lock().await.drive_item(id) })
            .map_err(failed)?
            .ok_or_else(|| failed("That item isn't in iCloud Drive any more."))
    }
}

#[uniffi::export]
impl Account {
    /// The top of iCloud Drive's id.
    pub fn drive_root(&self) -> String {
        quince_core::drive::ROOT.to_string()
    }

    /// A folder's items: as the phone last saw them, or (with `fresh`, or when never seen) read
    /// from Apple now and kept.
    pub fn drive_children(&self, folder: String, fresh: bool) -> Result<Vec<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        let kept = runtime()
            .block_on(async { lock.lock().await.drive_children(&folder) })
            .map_err(failed)?;
        if !fresh && !kept.is_empty() {
            return Ok(kept.iter().map(ditem).collect());
        }
        let remote = self.drive()?;
        let items = runtime().block_on(remote.list(&folder)).map_err(failed)?;
        Ok(items.iter().map(ditem).collect())
    }

    /// One item, as the phone last saw it.
    pub fn drive_item(&self, id: String) -> Result<Option<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.drive_item(&id) })
            .map_err(failed)?
            .as_ref()
            .map(ditem))
    }

    /// Items whose name has `query`, from what the phone has seen.
    pub fn drive_search(&self, query: String) -> Result<Vec<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.search_drive(&query, 200) })
            .map_err(failed)?
            .iter()
            .map(ditem)
            .collect())
    }

    /// A file fetched whole to `path` (the app's cache), for the Files app to open.
    pub fn drive_download(&self, id: String, path: String) -> Result<(), EngineError> {
        let item = self.known(&id)?;
        let remote = self.drive()?;
        let partial = format!("{path}.part");
        runtime()
            .block_on(quince_icloud::drive::download_to(
                &remote.http,
                &remote.docws,
                remote.params.clone(),
                &item,
                Path::new(&partial),
            ))
            .map_err(failed)?;
        // Whole or not at all: an app never opens half a file.
        std::fs::rename(&partial, &path).map_err(failed)
    }

    /// A file written by another app sent to iCloud: `staged` holds its bytes, named as the file
    /// is to be called, for `folder`; `base` is the file it was opened from (none for a new
    /// file). Through the desktop's write-back: into that same file when it is still that
    /// version, as a conflicted copy beside it when it changed elsewhere. Answers the file.
    pub fn drive_write_back(
        &self,
        folder: String,
        staged: String,
        base: Option<String>,
    ) -> Result<DItem, EngineError> {
        let remote = self.drive()?;
        let folder_item = self.known(&folder)?;
        let base_item = base.map(|b| self.known(&b)).transpose()?;
        let sent = runtime()
            .block_on(quince_vfs::writes::send(
                &remote,
                &folder_item,
                Path::new(&staged),
                base_item.as_ref(),
                None,
            ))
            .map_err(failed)?;
        // Kept, so the Files app lists it at once.
        let _ = runtime().block_on(remote.list(&folder));
        Ok(ditem(&sent.item))
    }

    /// A new folder in `parent`.
    pub fn drive_mkdir(&self, parent: String, name: String) -> Result<DItem, EngineError> {
        let remote = self.drive()?;
        let parent_item = self.known(&parent)?;
        let made = runtime()
            .block_on(remote.mkdir(&parent_item, &name))
            .map_err(failed)?;
        let _ = runtime().block_on(remote.list(&parent));
        Ok(ditem(&made))
    }

    /// Renamed in its folder.
    pub fn drive_rename(&self, id: String, name: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        runtime()
            .block_on(remote.rename(&item, &name))
            .map_err(failed)?;
        if let Some(parent) = item.parent_id {
            let _ = runtime().block_on(remote.list(&parent));
        }
        Ok(())
    }

    /// Moved into another folder.
    pub fn drive_move(&self, id: String, folder: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        let to = self.known(&folder)?;
        runtime()
            .block_on(remote.move_to(&item, &to))
            .map_err(failed)?;
        for f in [item.parent_id.clone().unwrap_or_default(), folder] {
            if !f.is_empty() {
                let _ = runtime().block_on(remote.list(&f));
            }
        }
        Ok(())
    }

    /// Deleted: into iCloud Drive's Recently Deleted, where it can be put back for 30 days.
    pub fn drive_delete(&self, id: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        runtime().block_on(remote.delete(&item)).map_err(failed)?;
        if let Some(parent) = item.parent_id {
            let _ = runtime().block_on(remote.list(&parent));
        }
        Ok(())
    }

    /// iCloud storage, `[used, total]` in bytes.
    pub fn drive_space(&self) -> Result<Vec<u64>, EngineError> {
        let remote = self.drive()?;
        let (used, total) = runtime().block_on(remote.space()).map_err(failed)?;
        Ok(vec![used, total])
    }
}
