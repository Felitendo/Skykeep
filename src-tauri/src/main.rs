// Quince — desktop shell.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use quince_auth::{Client, SignInOutcome, TwoFactorOptions};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, State};
use tokio::sync::Mutex;

struct Auth(Mutex<Client>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// A stored session exists and is worth trying to resume.
    has_stored_session: bool,
    /// Apple's service map is loaded, i.e. we are actually signed in right now.
    signed_in: bool,
}

/// Errors cross to the UI as plain sentences a person can act on.
type UiResult<T> = Result<T, String>;

fn state_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join("quince"))
}

#[tauri::command]
async fn auth_status(auth: State<'_, Auth>) -> UiResult<Status> {
    let client = auth.0.lock().await;
    Ok(Status {
        has_stored_session: client.has_stored_session(),
        signed_in: client.session().webservices.is_some(),
    })
}

/// Try the stored session. `false` just means a sign-in is needed — not an error.
#[tauri::command]
async fn auth_resume(auth: State<'_, Auth>) -> UiResult<bool> {
    let mut client = auth.0.lock().await;
    client.resume().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_sign_in(
    apple_id: String,
    password: String,
    auth: State<'_, Auth>,
) -> UiResult<&'static str> {
    let mut client = auth.0.lock().await;
    match client.sign_in(&apple_id, &password).await {
        Ok(SignInOutcome::Complete) => Ok("complete"),
        Ok(SignInOutcome::NeedsTwoFactor) => Ok("needs_two_factor"),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn auth_submit_code(code: String, auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.0.lock().await;
    client.submit_two_factor(&code).await.map_err(|e| e.to_string())
}

/// Where Apple can send a code, and whether it already pushed one.
/// A `trustedDeviceCount` of zero means nothing was sent and waiting is pointless.
#[tauri::command]
async fn auth_two_factor_options(auth: State<'_, Auth>) -> UiResult<TwoFactorOptions> {
    let mut client = auth.0.lock().await;
    client.two_factor_options().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_resend_code(auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.0.lock().await;
    client.resend_to_devices().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_send_sms(phone_id: u64, auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.0.lock().await;
    client.send_sms(phone_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_submit_sms_code(
    phone_id: u64,
    code: String,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let mut client = auth.0.lock().await;
    client.submit_sms_code(phone_id, &code).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_sign_out(auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.0.lock().await;
    client.sign_out().map_err(|e| e.to_string())
}

/// Services Apple advertises for this account, for the sidebar.
#[tauri::command]
async fn account_services(auth: State<'_, Auth>) -> UiResult<Vec<String>> {
    let client = auth.0.lock().await;
    Ok(client
        .session()
        .webservices
        .as_ref()
        .and_then(|w| w.as_object())
        .map(|o| {
            let mut keys: Vec<String> = o.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = state_dir(&app.handle());
            let client = Client::new(&dir)?;
            app.manage(Auth(Mutex::new(client)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            auth_status,
            auth_resume,
            auth_sign_in,
            auth_submit_code,
            auth_two_factor_options,
            auth_resend_code,
            auth_send_sms,
            auth_submit_sms_code,
            auth_sign_out,
            account_services,
        ])
        .run(tauri::generate_context!())
        .expect("Quince failed to start");
}
