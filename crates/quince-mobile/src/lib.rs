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
/// (`reason`, not `message`: Kotlin exceptions already have a `message`.)
pub enum EngineError {
    #[error("{reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for EngineError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        EngineError::Failed { reason: e.reason }
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
        .map_err(|reason| EngineError::Failed { reason })
}

/// Whether Apple's sign-in service can be reached from this phone, through the engine's own
/// Apple-only client: the network, DNS and TLS checked before a sign-in is offered.
#[uniffi::export]
pub fn apple_reachable() -> Result<bool, EngineError> {
    runtime().block_on(async {
        let http = quince_auth::apple_only()
            .build()
            .map_err(|e| EngineError::Failed {
                reason: e.to_string(),
            })?;
        Ok(http
            .get("https://setup.icloud.com/setup/ws/1/validate")
            .send()
            .await
            .is_ok())
    })
}

/// What signing in decided.
#[derive(uniffi::Enum)]
pub enum SignIn {
    /// Signed in: Apple already trusted this phone.
    Complete,
    /// Apple wants a six-digit code (see [`Account::code_options`]).
    NeedsCode,
}

/// A phone number Apple will send a code to, already masked by Apple.
#[derive(uniffi::Record)]
pub struct TrustedPhone {
    pub id: u64,
    pub masked_number: String,
}

/// Where Apple is willing to send a code.
#[derive(uniffi::Record)]
pub struct CodeOptions {
    /// Devices Apple pushed the code to; zero means nothing arrives on its own.
    pub trusted_devices: u64,
    pub phones: Vec<TrustedPhone>,
    /// Apple refuses more codes for a while.
    pub too_many_codes_sent: bool,
    /// The account signs in with security keys, which Skykeep cannot answer yet.
    pub security_key: bool,
}

/// A way to reach someone: `phone`, `email`, `url` or `address`, with Apple's label.
#[derive(uniffi::Record)]
pub struct Method {
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
}

/// A person in the iCloud address book, as Android's Contacts will hold them.
#[derive(uniffi::Record)]
pub struct Person {
    /// Apple's id for the card: the raw contact's source id, so a sync finds it again.
    pub guid: String,
    /// Changes whenever the card does: an unchanged card is not rewritten.
    pub etag: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub is_company: bool,
    pub birthday: Option<String>,
    pub notes: Option<String>,
    pub methods: Vec<Method>,
}

fn failed(e: impl std::fmt::Display) -> EngineError {
    EngineError::Failed {
        reason: e.to_string(),
    }
}

/// One Apple Account, signed in or being signed in: the desktop app's own sign-in (SRP,
/// two-factor, the session sealed in the Keystore through [`start`]), unchanged.
#[derive(uniffi::Object)]
pub struct Account {
    client: std::sync::Mutex<quince_auth::Client>,
}

#[uniffi::export]
impl Account {
    /// The account whose state lives in `state_dir` (the app's private files).
    #[uniffi::constructor]
    pub fn new(state_dir: String) -> Result<std::sync::Arc<Self>, EngineError> {
        let client = quince_auth::Client::new(std::path::Path::new(&state_dir)).map_err(failed)?;
        Ok(std::sync::Arc::new(Self {
            client: std::sync::Mutex::new(client),
        }))
    }

    /// Signs in again with the session kept from last time; false when there is none to use.
    pub fn resume(&self) -> Result<bool, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.resume()).map_err(failed)
    }

    pub fn sign_in(&self, apple_id: String, password: String) -> Result<SignIn, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        match runtime()
            .block_on(c.sign_in(&apple_id, &password))
            .map_err(failed)?
        {
            quince_auth::SignInOutcome::Complete => Ok(SignIn::Complete),
            quince_auth::SignInOutcome::NeedsTwoFactor => Ok(SignIn::NeedsCode),
        }
    }

    pub fn code_options(&self) -> Result<CodeOptions, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        let o = runtime().block_on(c.two_factor_options()).map_err(failed)?;
        Ok(CodeOptions {
            trusted_devices: o.trusted_device_count,
            phones: o
                .phones
                .into_iter()
                .map(|p| TrustedPhone {
                    id: p.id,
                    masked_number: p.masked_number,
                })
                .collect(),
            too_many_codes_sent: o.too_many_codes_sent,
            security_key: o.security_key,
        })
    }

    /// The code shown on a trusted device.
    pub fn submit_code(&self, code: String) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime()
            .block_on(c.submit_two_factor(&code))
            .map_err(failed)
    }

    pub fn resend_to_devices(&self) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.resend_to_devices()).map_err(failed)
    }

    pub fn send_sms(&self, phone_id: u64) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.send_sms(phone_id)).map_err(failed)
    }

    pub fn submit_sms_code(&self, phone_id: u64, code: String) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime()
            .block_on(c.submit_sms_code(phone_id, &code))
            .map_err(failed)
    }

    /// The Apple Account signed in, as Android's account list names it.
    pub fn apple_id(&self) -> Option<String> {
        let c = self.client.lock().ok()?;
        c.session().apple_id.clone()
    }

    /// The whole iCloud address book, read as desktop reads it (Apple's `/co/startup`).
    pub fn contacts(&self) -> Result<Vec<Person>, EngineError> {
        let c = self.client.lock().map_err(failed)?;
        let root = c
            .session()
            .service_url("contacts")
            .ok_or_else(|| failed("Apple hasn't offered a contacts service for this account."))?;
        let (book, _) = runtime()
            .block_on(quince_icloud::contacts::fetch_book_with_tokens(
                c.http(),
                &root,
                c.service_params(),
            ))
            .map_err(failed)?;
        Ok(book
            .contacts
            .into_iter()
            .map(|p| Person {
                guid: p.guid,
                etag: p.etag,
                first_name: p.first_name,
                middle_name: p.middle_name,
                last_name: p.last_name,
                company: p.company,
                is_company: p.is_company,
                birthday: p.birthday,
                notes: p.notes,
                methods: p
                    .methods
                    .into_iter()
                    .map(|m| Method {
                        kind: m.kind,
                        label: m.label,
                        value: m.value,
                    })
                    .collect(),
            })
            .collect())
    }

    /// Signs out here: the session forgotten on this phone, its secrets deleted.
    pub fn sign_out(&self) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        c.sign_out().map_err(failed)
    }
}
