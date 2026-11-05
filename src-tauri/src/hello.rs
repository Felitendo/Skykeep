//! Windows Hello — face, fingerprint or PIN — to unlock the Hidden album on Windows, as Face ID
//! unlocks it on an iPhone. The prompt is owned by Skykeep's own window
//! (`IUserConsentVerifierInterop::RequestVerificationForWindowAsync`, reached through the
//! activation factory): asked for the plain WinRT way, a desktop app's prompt has no owner and
//! can open behind the window or at the corner of the screen (bitwarden/desktop#641). Where
//! Hello isn't set up, or isn't Windows, it isn't offered; the Apple Account password always is.

/// What asking Windows Hello came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Hello {
    Verified,
    /// The person cancelled, or Windows Hello said no.
    #[cfg_attr(not(windows), allow(dead_code))]
    Declined,
    /// Not set up on this computer (or not Windows): the password is asked instead.
    Unavailable,
}

/// Whether Windows Hello can be asked on this computer.
pub async fn available() -> bool {
    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(|| {
            use windows::Security::Credentials::UI::{
                UserConsentVerifier, UserConsentVerifierAvailability,
            };
            UserConsentVerifier::CheckAvailabilityAsync()
                .and_then(|op| op.join())
                .is_ok_and(|a| a == UserConsentVerifierAvailability::Available)
        })
        .await
        .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Windows Hello asked, its prompt owned by the window `hwnd`, saying `message`.
pub async fn verify(hwnd: isize, message: String) -> Hello {
    #[cfg(windows)]
    {
        if !available().await {
            return Hello::Unavailable;
        }
        tokio::task::spawn_blocking(move || {
            use windows::core::{factory, HSTRING};
            use windows::Security::Credentials::UI::{
                UserConsentVerificationResult, UserConsentVerifier,
            };
            use windows::Win32::Foundation::HWND;
            use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
            use windows_future::IAsyncOperation;
            let asked = || -> windows::core::Result<UserConsentVerificationResult> {
                let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
                // SAFETY: `hwnd` is this app's own window, alive while the prompt is up (the
                // window waits on the answer); the call only reads it as the prompt's owner.
                let op: IAsyncOperation<UserConsentVerificationResult> = unsafe {
                    interop.RequestVerificationForWindowAsync(
                        HWND(hwnd as *mut core::ffi::c_void),
                        &HSTRING::from(message.as_str()),
                    )
                }?;
                op.join()
            };
            match asked() {
                Ok(UserConsentVerificationResult::Verified) => Hello::Verified,
                Ok(
                    UserConsentVerificationResult::DeviceNotPresent
                    | UserConsentVerificationResult::NotConfiguredForUser
                    | UserConsentVerificationResult::DisabledByPolicy,
                ) => Hello::Unavailable,
                Ok(_) | Err(_) => Hello::Declined,
            }
        })
        .await
        .unwrap_or(Hello::Declined)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, message);
        Hello::Unavailable
    }
}
