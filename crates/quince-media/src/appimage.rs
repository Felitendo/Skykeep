//! HEIC photos from the AppImage, on a computer that has no HEVC decoder of its own.
//!
//! libheif reads no HEIC by itself: its HEVC decoder is a plugin (libde265's), which it looks
//! for in the directory it was built with — `/usr/lib/x86_64-linux-gnu/libheif/plugins` on the
//! Ubuntu the AppImage is built on. The AppImage carries libheif, but the plugin directory it
//! then searches is the host's: empty on a clean Debian or Fedora (every HEIC failed with
//! "Unsupported codec"), and elsewhere a different libheif's plugins. So the AppImage carries
//! the decoder too (`bundle.linux.appimage.files` in tauri.conf.json) and libheif is pointed at
//! that copy with `LIBHEIF_PLUGIN_PATH`, which it reads when it is first initialised.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where the AppImage keeps libheif's plugins, under its AppDir (tauri.conf.json).
pub const PLUGINS: &str = "usr/lib/libheif/plugins";

/// Points libheif at the AppImage's own plugins when this program runs from an AppImage.
///
/// Only then: `APPDIR` (set by the AppImage runtime and its AppRun) must be an absolute path
/// that this very executable lies inside, and the plugin directory must be a real directory
/// inside it — so a stray `APPDIR` in an ordinary install's environment, or one naming some
/// other directory, never decides what code libheif loads. A `LIBHEIF_PLUGIN_PATH` the person
/// set themselves is left alone.
///
/// Call it before any thread starts and before libheif is first used: it sets the environment.
pub fn use_bundled_heif_plugins() {
    if std::env::var_os("LIBHEIF_PLUGIN_PATH").is_some() {
        return;
    }
    let Some(appdir) = std::env::var_os("APPDIR") else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Some(dir) = heif_plugin_dir(&appdir, &exe) {
        // Before any thread starts (the caller's promise): nothing else reads the environment.
        std::env::set_var("LIBHEIF_PLUGIN_PATH", dir);
    }
}

/// The AppImage's plugin directory for the AppDir `appdir` and the running executable `exe`,
/// when `exe` is inside `appdir` and the directory is there; both resolved through symlinks.
#[must_use]
pub fn heif_plugin_dir(appdir: &OsStr, exe: &Path) -> Option<PathBuf> {
    let appdir = Path::new(appdir);
    if !appdir.is_absolute() {
        return None;
    }
    let root = appdir.canonicalize().ok()?;
    let exe = exe.canonicalize().ok()?;
    if !exe.starts_with(&root) {
        return None;
    }
    let dir = root.join(PLUGINS).canonicalize().ok()?;
    // libheif splits the variable at ':' — a path with one would name two other places.
    let usable =
        dir.starts_with(&root) && dir.is_dir() && !dir.as_os_str().to_string_lossy().contains(':');
    usable.then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An AppDir laid out as the AppImage's: the program in usr/bin, the plugins under usr/lib.
    fn appdir() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::create_dir_all(root.path().join(PLUGINS)).unwrap();
        let exe = root.path().join("usr/bin/skykeep");
        std::fs::write(&exe, b"").unwrap();
        (root, exe)
    }

    #[test]
    fn the_appimages_own_plugins() {
        let (root, exe) = appdir();
        let dir = heif_plugin_dir(root.path().as_os_str(), &exe).unwrap();
        assert_eq!(dir, root.path().canonicalize().unwrap().join(PLUGINS));
    }

    #[test]
    fn not_for_a_program_outside_the_appdir() {
        let (root, _) = appdir();
        let (_other, elsewhere) = appdir();
        assert_eq!(heif_plugin_dir(root.path().as_os_str(), &elsewhere), None);
    }

    #[test]
    fn not_for_a_relative_appdir() {
        let (_root, exe) = appdir();
        assert_eq!(heif_plugin_dir(OsStr::new("usr/.."), &exe), None);
        assert_eq!(heif_plugin_dir(OsStr::new(""), &exe), None);
    }

    #[test]
    fn not_when_the_appimage_carries_none() {
        let (root, exe) = appdir();
        std::fs::remove_dir(root.path().join(PLUGINS)).unwrap();
        assert_eq!(heif_plugin_dir(root.path().as_os_str(), &exe), None);
        // A file where the directory should be is not one either.
        std::fs::write(root.path().join(PLUGINS), b"").unwrap();
        assert_eq!(heif_plugin_dir(root.path().as_os_str(), &exe), None);
    }

    #[test]
    fn not_through_a_link_out_of_the_appdir() {
        let (root, exe) = appdir();
        let outside = tempfile::tempdir().unwrap();
        let plugins = root.path().join(PLUGINS);
        std::fs::remove_dir(&plugins).unwrap();
        std::os::unix::fs::symlink(outside.path(), &plugins).unwrap();
        assert_eq!(heif_plugin_dir(root.path().as_os_str(), &exe), None);
    }
}
