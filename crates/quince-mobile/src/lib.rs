//! Skykeep's engine for Android, as the Kotlin app sees it.
//!
//! Coarse calls only: each crosses the bridge once and does its work here, in Rust, so the
//! app never drives the engine a step at a time. The engine's own crates stay as they are on
//! desktop; this crate only adapts them to what Android gives (its Keystore, through
//! [`SecretStore`]) and what it can call (UniFFI's generated Kotlin).

use std::sync::OnceLock;

uniffi::setup_scaffolding!();

/// Why a call failed, in words the app can show.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("{message}")]
    Failed { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for EngineError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        EngineError::Failed { message: e.reason }
    }
}

/// The app's secret store: Android's Keystore, kept by the Kotlin side, where the session's
/// keys live as they live in the keyring on desktop.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn get(&self, service: String, name: String) -> Result<Option<String>, EngineError>;
    fn set(&self, service: String, name: String, value: String) -> Result<(), EngineError>;
    fn delete(&self, service: String, name: String) -> Result<(), EngineError>;
}

/// The Kotlin store, seen by the engine as its platform's secrets.
struct Bridged(std::sync::Arc<dyn SecretStore>);

impl quince_auth::PlatformSecrets for Bridged {
    fn get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        self.0
            .get(service.into(), name.into())
            .map_err(|e| e.to_string())
    }
    fn set(&self, service: &str, name: &str, value: &str) -> Result<(), String> {
        self.0
            .set(service.into(), name.into(), value.into())
            .map_err(|e| e.to_string())
    }
    fn delete(&self, service: &str, name: &str) -> Result<(), String> {
        self.0
            .delete(service.into(), name.into())
            .map_err(|e| e.to_string())
    }
}

/// One runtime for the life of the process: Android starts and stops activities freely, and
/// the engine's work must not start and stop with them.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("skykeep-engine")
            .enable_all()
            .build()
            .expect("the engine's runtime")
    })
}

/// The engine's version, the same as the desktop app's.
#[uniffi::export]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hands the engine Android's secret store. Once per process, before anything signs in.
#[uniffi::export]
pub fn start(secrets: std::sync::Arc<dyn SecretStore>) -> Result<(), EngineError> {
    quince_auth::use_platform_secrets(Box::new(Bridged(secrets)))
        .map_err(|message| EngineError::Failed { message })
}

/// Whether Apple's sign-in service can be reached from this phone, through the engine's own
/// Apple-only client: the network, DNS and TLS checked before a sign-in is offered.
#[uniffi::export]
pub fn apple_reachable() -> Result<bool, EngineError> {
    runtime().block_on(async {
        let http = quince_auth::apple_only()
            .build()
            .map_err(|e| EngineError::Failed {
                message: e.to_string(),
            })?;
        Ok(http
            .get("https://setup.icloud.com/setup/ws/1/validate")
            .send()
            .await
            .is_ok())
    })
}
