//! Skykeep installed as an MSIX package — from the Microsoft Store, or a test install of the same
//! package (packaging/windows/msix). Such a copy has a package identity: Windows knows it by its
//! package family name, updates it through the Store, and knows each of its seven Start-menu
//! entries by `<family name>!<application id>` alone. The manifest gives each entry the ID the
//! installers already use — `io.github.yashwanthkamireddi.Skykeep`, `….Skykeep.Notes` and so on —
//! so inside a package every app ID is the usual one with the family name in front.
//!
//! Elsewhere (Linux, the .exe and the .msi) there is no package and nothing here changes a thing.

use std::path::Path;

/// The package family name this process runs under, if it runs from a package.
#[cfg(windows)]
pub fn family_name() -> Option<String> {
    use std::sync::OnceLock;
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY
        .get_or_init(|| {
            use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
            use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;
            let mut len = 0u32;
            // SAFETY: a length query with no buffer; Windows writes only `len`. A process with
            // no package answers APPMODEL_ERROR_NO_PACKAGE.
            if unsafe { GetCurrentPackageFamilyName(&mut len, None) } != ERROR_INSUFFICIENT_BUFFER {
                return None;
            }
            let mut buf = vec![0u16; len as usize];
            // SAFETY: a buffer of the `len` units Windows asked for, NUL included.
            let done = unsafe {
                GetCurrentPackageFamilyName(&mut len, Some(windows::core::PWSTR(buf.as_mut_ptr())))
            };
            if done.is_err() {
                return None;
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let name = String::from_utf16_lossy(&buf[..end]);
            (!name.is_empty()).then_some(name)
        })
        .clone()
}

#[cfg(not(windows))]
pub fn family_name() -> Option<String> {
    None
}

/// The app ID Windows knows an entry by: `<family>!<id>` in a package, `id` itself otherwise.
pub fn app_id(family: Option<&str>, id: &str) -> String {
    match family {
        Some(f) => format!("{f}!{id}"),
        None => id.to_string(),
    }
}

/// Whether a program at `exe` sits where Windows keeps installed packages
/// (`C:\Program Files\WindowsApps\<package>\skykeep.exe`, or an app execution alias under
/// `…\Microsoft\WindowsApps`): a copy only the Store, or whoever installed the package, may
/// replace — even started some way that left it without its package identity.
pub fn in_package_folder(exe: &Path) -> bool {
    exe.to_string_lossy()
        .split(['\\', '/'])
        .any(|part| part.eq_ignore_ascii_case("WindowsApps"))
}

/// Whether this copy is an MSIX package's: it runs with a package identity, or from where
/// packages are installed.
pub fn packaged() -> bool {
    family_name().is_some()
        || std::env::current_exe().is_ok_and(|exe| cfg!(windows) && in_package_folder(&exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_in_and_out_of_a_package() {
        let id = "io.github.yashwanthkamireddi.Skykeep.Notes";
        assert_eq!(app_id(None, id), id);
        assert_eq!(
            app_id(Some("Skykeep_8wekyb3d8bbwe"), id),
            "Skykeep_8wekyb3d8bbwe!io.github.yashwanthkamireddi.Skykeep.Notes"
        );
    }

    #[test]
    fn where_packages_are_installed() {
        let store = Path::new(
            r"C:\Program Files\WindowsApps\Skykeep_0.1.2.0_x64__8wekyb3d8bbwe\skykeep.exe",
        );
        assert!(in_package_folder(store));
        let alias = Path::new(r"C:\Users\a\AppData\Local\Microsoft\WindowsApps\skykeep.exe");
        assert!(in_package_folder(alias));
        let nsis = Path::new(r"C:\Users\a\AppData\Local\Skykeep\skykeep.exe");
        assert!(!in_package_folder(nsis));
        let msi = Path::new(r"C:\Program Files\Skykeep\skykeep.exe");
        assert!(!in_package_folder(msi));
        // A folder merely named like it is not the one.
        let lookalike = Path::new(r"C:\Users\a\MyWindowsAppsBackup\skykeep.exe");
        assert!(!in_package_folder(lookalike));
    }
}
