//! Advanced Data Protection: asking Apple to let this session read end-to-end encrypted data.
//!
//! With Advanced Data Protection on (Apple's iCloud Data Protection, `iCDPEnabled`) and
//! "Access iCloud Data on the Web" allowed, a web session reaches Calendar and Contacts as
//! usual, but Notes, Reminders, Photos and iCloud Drive only with a per-service key Apple
//! releases as a `X-APPLE-WEBAUTH-PCS-*` cookie. Apple releases it only after the person
//! approves the session on one of their trusted devices, and for an hour at a time
//! (support.apple.com/guide/security/sec973254c5f). Without it, iCloud Drive answers
//! `423 Missing PCS cookies from the request` and CloudKit `403 ACCESS_DENIED, private db
//! access disabled for this account` — issue #4.
//!
//! Three `POST`s to the setup service do it, as icloud.com does them (foxt/icloud.js#4,
//! rclone's `iclouddrive` backend, icloudpy#214, timlaing/pyicloud):
//!
//! * `requestWebAccessState`, empty — `isWebAccessAllowed`, `isICDRSDisabled` (recovery off:
//!   ADP is in effect), `isDeviceConsentedForPCS`, `deviceConsentForPCSExpiry` (epoch ms).
//! * `enableDeviceConsentForPCS`, empty — puts the approval on the person's devices;
//!   `isDeviceConsentNotificationSent` says whether Apple could.
//! * `requestPCS`, `{"appName", "derivedFromUserAction"}` — once approved, attaches the
//!   service's cookies: `status: "success"` with "Cookies attached." or "Cookies already
//!   present."; `status: "failure"` with "Requested the device to upload cookies." or
//!   "Cookies not available yet on server." while the device is still sending the key.
//!   `derivedFromUserAction` is true on the first ask only: repeating it notifies again.
//!
//! Nothing here waits for the person. [`Client::data_access`] takes one step and says where
//! it got to; whoever holds the client releases it between steps and follows
//! [`APPROVAL_SCHEDULE`] with [`follow`], so a sign-in, a sync or a sign-out is never held
//! up behind someone reaching for their phone.

use crate::{Client, Error, Result};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

/// One service Advanced Data Protection keeps end-to-end encrypted, and how Apple names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protected {
    /// The app's own name for the service, as the sync reports name it.
    pub service: &'static str,
    /// Where Apple's service map lists it.
    pub webservice: &'static str,
    /// What `requestPCS` calls it — not the webservice key. A wrong one answers `success` and
    /// sets nothing, so where the cookies are known the answer is checked against them.
    pub app_name: &'static str,
    /// The cookies `requestPCS` sets for it, where the references have recorded them.
    pub cookies: &'static [&'static str],
}

/// Every service Skykeep reads that Advanced Data Protection covers. Calendar and Contacts are
/// not in it: Apple keeps their keys for itself under ADP, so they need no approval.
///
/// The app names are icloud.com's own: `photos` and `iclouddrive` (rclone, icloudpy#214, with
/// the cookies each sets), `notes3` and `reminders` (timlaing/pyicloud, which records no
/// cookie names for them).
pub const PROTECTED: [Protected; 4] = [
    Protected {
        service: "notes",
        webservice: "ckdatabasews",
        app_name: "notes3",
        cookies: &[],
    },
    Protected {
        service: "reminders",
        webservice: "ckdatabasews",
        app_name: "reminders",
        cookies: &[],
    },
    Protected {
        service: "photos",
        webservice: "ckdatabasews",
        app_name: "photos",
        cookies: &["X-APPLE-WEBAUTH-PCS-Photos", "X-APPLE-WEBAUTH-PCS-Sharing"],
    },
    Protected {
        service: "drive",
        webservice: "drivews",
        app_name: "iclouddrive",
        cookies: &["X-APPLE-WEBAUTH-PCS-Documents"],
    },
];

/// The protected service behind one of the app's names.
#[must_use]
pub fn protected(service: &str) -> Option<&'static Protected> {
    PROTECTED.iter().find(|p| p.service == service)
}

/// Where letting this session read end-to-end encrypted data has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DataAccess {
    /// Nothing waits on an approval: the account has no Advanced Data Protection, or every
    /// service it covers already has its key here.
    Open,
    /// The approval is on the person's devices, or approved and the keys on their way.
    /// Asked again after a wait ([`APPROVAL_SCHEDULE`]).
    Waiting,
    /// An approval is needed and none is on its way: asking was left to the person (a
    /// background pass never puts a prompt on their phone), or an earlier one ran out.
    Needed,
    /// Apple could not put the approval on any of the person's devices.
    NotSent,
    /// "Access iCloud Data on the Web" is off: no approval opens anything until it is on.
    WebAccessOff,
}

/// What `requestPCS` answers while the device is still uploading the key: not yet, not no.
const PENDING: [&str; 2] = [
    "Requested the device to upload cookies.",
    "Cookies not available yet on server.",
];

/// The web session cookie `requestPCS` stands on; without a live one Apple answers 500.
const WEB_AUTH_COOKIE: &str = "X-APPLE-WEBAUTH-TOKEN";

/// How long a key is taken to last when Apple gave no time for it: Apple's own hour.
const KEY_HOUR: u64 = 60 * 60;

/// An approval asked for within this long is not asked for again: a second push to the
/// person's phone while they reach for it is noise, and pushes count toward Apple's limits.
const ASK_AGAIN_AFTER: u64 = 2 * 60;

/// How long an approval put on the person's devices is waited for: [`APPROVAL_SCHEDULE`]'s
/// five minutes, and a little over for the requests themselves.
const APPROVAL_WINDOW: u64 = 6 * 60;

/// How long to wait before each step while an approval is on its way, then stop: every 5
/// seconds for the first minute (as icloudpy and pyicloud poll), every 10 for four more (as
/// rclone does), five minutes in all (rclone's bound) — 36 requests at most, never hammering.
/// After that the person is asked to try again.
pub const APPROVAL_SCHEDULE: fn(u32) -> Option<Duration> = |step| match step {
    0..=11 => Some(Duration::from_secs(5)),
    12..=35 => Some(Duration::from_secs(10)),
    _ => None,
};

/// Follow an approval from [`DataAccess::Waiting`] to where it lands: `step` is asked again
/// after each of `schedule`'s waits — the caller takes and releases the client inside it —
/// and the first answer that is not `Waiting` is the outcome. Run out, it is
/// [`DataAccess::Needed`]: the person is asked to try again rather than waited on for ever.
pub async fn follow<F, Fut>(
    schedule: fn(u32) -> Option<Duration>,
    mut step: F,
) -> Result<DataAccess>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<DataAccess>>,
{
    let mut n = 0;
    while let Some(wait) = schedule(n) {
        n += 1;
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        match step().await? {
            DataAccess::Waiting => {}
            other => return Ok(other),
        }
    }
    Ok(DataAccess::Needed)
}

/// What this run of the app knows about approvals, beside what the session keeps.
#[derive(Debug, Default)]
pub(crate) struct Pcs {
    /// When an approval was last put on the person's devices.
    asked_at: Option<u64>,
    /// Services already asked for with `derivedFromUserAction`, so the ones after are not
    /// another notification. Forgotten once the key is here.
    announced: HashSet<&'static str>,
    /// Services Apple refused for want of their key even though one was held — it ran out,
    /// or Apple let it go: asked for again whatever the cookies say.
    stale: HashSet<&'static str>,
}

/// One answer from the setup service: its status and JSON body.
struct Answer {
    status: StatusCode,
    body: Value,
}

impl Answer {
    fn flag(&self, key: &str) -> Option<bool> {
        self.body.get(key).and_then(Value::as_bool)
    }
}

impl Client {
    /// The services Advanced Data Protection covers that this session cannot read yet: it
    /// holds no key for them, or the one it held ran out. Asks Apple nothing.
    #[must_use]
    pub fn data_access_missing(&self) -> Vec<&'static str> {
        // Web access off, or terms waiting at icloud.com: no key is released either way (Apple
        // answers requestPCS "Missing X-APPLE-WEBAUTH-TOKEN cookie"), so none is asked for.
        if self.session.gate.web_access_allowed() == Some(false) || self.waits_on_icloud_com() {
            return Vec::new();
        }
        self.protected_here()
            .filter(|p| !self.holds_key(p))
            .map(|p| p.service)
            .collect()
    }

    /// Apple refused `service` for want of its key (see `quince_icloud::Error::
    /// NeedsDeviceApproval`): the next [`Self::data_access`] asks for it again, whatever the
    /// cookies held say.
    pub fn data_access_lapsed(&mut self, service: &str) {
        let Some(p) = protected(service) else { return };
        // Apple's own refusal is proof the account has Advanced Data Protection, whatever its
        // flags said (iCDPEnabled can read false with it on: rclone#9658).
        self.session.data_protection_seen = Some(true);
        self.pcs.stale.insert(p.service);
        self.session.pcs_until.remove(p.app_name);
    }

    /// Forget every approval, for a sign-out or a new sign-in: the keys belong to the web
    /// session they were released to, so a new one asks for its own, whatever cookies the
    /// old one left in the jar.
    pub(crate) fn data_access_forget(&mut self) {
        self.pcs = Pcs {
            stale: PROTECTED.iter().map(|p| p.service).collect(),
            ..Pcs::default()
        };
        self.session.pcs_until.clear();
        // Perhaps another account: what Apple showed of the last one says nothing of this one.
        self.session.data_protection_seen = None;
    }

    /// Whether this account has Advanced Data Protection in effect, on evidence: Apple's own
    /// word from `requestWebAccessState` or a refusal for want of a key, which outrank the
    /// sign-in's `iCDPEnabled`. `pcsRequired` alone is not evidence (icloudpy#214): an account
    /// without ADP can carry it, and reading it as ADP held such an account's Notes back.
    fn data_protection_on(&self) -> bool {
        match self.session.data_protection_seen {
            Some(seen) => seen,
            None => self.session.data_protection == Some(true),
        }
    }

    /// The protected services Apple offers this account, when it has Advanced Data Protection.
    fn protected_here(&self) -> impl Iterator<Item = &'static Protected> + '_ {
        let adp = self.data_protection_on();
        PROTECTED.iter().filter(move |p| {
            adp && self
                .session
                .webservices
                .as_ref()
                .and_then(|w| w.get(p.webservice))
                .is_some()
        })
    }

    /// Whether this session holds a key for `p` that has not run out.
    fn holds_key(&self, p: &Protected) -> bool {
        if self.pcs.stale.contains(p.service) {
            return false;
        }
        let now = crate::unix_now();
        // Every key is held only until the time recorded when it arrived — its cookie's
        // expiry, the approval's, and Apple's hour, whichever is first — so a pass stops asking
        // for the service before Apple would refuse it, never after.
        let recorded = self
            .session
            .pcs_until
            .get(p.app_name)
            .is_some_and(|until| *until > now);
        if !recorded {
            return false;
        }
        if p.cookies.is_empty() {
            return true;
        }
        let jar = self
            .jar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held: HashSet<&str> = jar.iter_unexpired().map(|c| c.name()).collect();
        p.cookies.iter().all(|name| held.contains(name))
    }

    /// When the keys just attached run out: whichever comes first of the earliest PCS cookie's
    /// expiry, the approval's (`deviceConsentForPCSExpiry`) and Apple's hour from now — the
    /// hour Apple's security guide gives an approval, and what people report seeing.
    fn keys_until(&self, consent_until: Option<u64>) -> u64 {
        let now = crate::unix_now();
        let jar = self
            .jar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cookie = jar
            .iter_unexpired()
            .filter(|c| c.name().starts_with("X-APPLE-WEBAUTH-PCS-"))
            .filter_map(|c| match c.expires {
                cookie_store::CookieExpiration::AtUtc(t) => u64::try_from(t.unix_timestamp()).ok(),
                cookie_store::CookieExpiration::SessionEnd => None,
            })
            .min();
        [cookie, consent_until, Some(now + KEY_HOUR)]
            .into_iter()
            .flatten()
            .filter(|t| *t > now)
            .min()
            .unwrap_or(now + KEY_HOUR)
    }

    /// Where letting this session read end-to-end encrypted data stands, from what is held
    /// here alone: no request, no prompt, no notification on anyone's phone. What a background
    /// pass goes by — only the person's own action asks Apple ([`Self::data_access`]).
    #[must_use]
    pub fn data_access_standing(&self) -> DataAccess {
        if self.session.gate.web_access_allowed() == Some(false) {
            return DataAccess::WebAccessOff;
        }
        if self.data_access_missing().is_empty() {
            DataAccess::Open
        } else {
            DataAccess::Needed
        }
    }

    /// One `POST` to the setup service. Apple's words stay out of every error: what it says
    /// about an account's keys is the account's.
    async fn setup_post(&mut self, endpoint: &str, body: Option<Value>) -> Result<Answer> {
        let mut request = self
            .http
            .post(format!("{}/{endpoint}", self.endpoints.setup))
            .query(&self.service_params());
        request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(body.to_string()),
            None => request.body(""),
        };
        let response = request.send().await?;
        self.capture(&response);
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        quiet!("quince: {endpoint} answered {status}");
        match status.as_u16() {
            401 | 421 | 450 => Err(Error::ReauthRequired),
            // "Not now": Apple busy, or asking for fewer requests. The caller waits.
            429 | 502 | 503 | 504 => Err(Error::Unavailable(status.as_u16())),
            _ => Ok(Answer { status, body }),
        }
    }

    /// Take one step toward reading this account's end-to-end encrypted services, and say
    /// where it got to. Asks Apple nothing when nothing is missing.
    ///
    /// Called only for something the person did — signing in, pressing Sync Now — or to follow
    /// the approval that asked for: every key Apple releases shows a notice on their devices,
    /// so a background pass reads [`Self::data_access_standing`] instead and asks nothing.
    /// `ask` lets this put the approval itself on their devices when none is given; following
    /// one already asked for, it is `false`.
    pub async fn data_access(&mut self, ask: bool) -> Result<DataAccess> {
        if self.session.gate.web_access_allowed() == Some(false) {
            return Ok(DataAccess::WebAccessOff);
        }
        let missing: Vec<&'static Protected> = self
            .data_access_missing()
            .into_iter()
            .filter_map(protected)
            .collect();
        if missing.is_empty() {
            return Ok(DataAccess::Open);
        }
        let state = self.setup_post("requestWebAccessState", None).await?;
        if !state.status.is_success() {
            return Err(Error::Protocol(format!(
                "Apple didn't say whether this session may read your encrypted data (HTTP {})",
                state.status
            )));
        }
        if state.flag("isWebAccessAllowed") == Some(false) {
            self.session.gate.web_access_off();
            self.keep();
            return Ok(DataAccess::WebAccessOff);
        }
        // An account that said it has Advanced Data Protection, or was refused for want of a
        // key, and an answer with no word of recovery either way: not something to read as
        // "nothing to approve", which would leave its services failing without saying why.
        let said_adp = self.data_protection_on();
        if said_adp && state.flag("isICDRSDisabled").is_none() {
            return Err(Error::Protocol(format!(
                "Apple didn't say whether this session may read your encrypted data (HTTP {})",
                state.status
            )));
        }
        if state.flag("isICDRSDisabled") != Some(true) {
            // Recovery is on, so Apple holds the keys itself: no approval opens anything more,
            // and nothing is held back for one — remembered, so it is not asked again.
            self.session.data_protection_seen = Some(false);
            self.keep();
            return Ok(DataAccess::Open);
        }
        self.session.data_protection_seen = Some(true);
        let consent_until = state
            .body
            .get("deviceConsentForPCSExpiry")
            .and_then(Value::as_u64)
            .map(|ms| ms / 1000)
            .filter(|s| *s > 0);

        if state.flag("isDeviceConsentedForPCS") != Some(true) {
            let now = crate::unix_now();
            let since_asked = self.pcs.asked_at.map(|at| now.saturating_sub(at));
            let ask_now = ask && since_asked.is_none_or(|s| s >= ASK_AGAIN_AFTER);
            if !ask_now {
                // One already on the person's devices is waited for, as long as the schedule
                // runs; after that it is theirs to ask for again.
                return Ok(match since_asked {
                    Some(s) if s < APPROVAL_WINDOW => DataAccess::Waiting,
                    _ => DataAccess::Needed,
                });
            }
            let sent = self.setup_post("enableDeviceConsentForPCS", None).await?;
            if sent.flag("isWebAccessAllowed") == Some(false) {
                self.session.gate.web_access_off();
                self.keep();
                return Ok(DataAccess::WebAccessOff);
            }
            if sent.flag("isDeviceConsentNotificationSent") != Some(true) {
                return Ok(DataAccess::NotSent);
            }
            self.pcs.asked_at = Some(now);
            // A new approval: each service's first ask may say so again.
            self.pcs.announced.clear();
            return Ok(DataAccess::Waiting);
        }

        // Approved: ask for each missing service's key.
        self.pcs.asked_at = None;
        let mut waiting = false;
        for p in missing {
            let first = self.pcs.announced.insert(p.service);
            let answer = self
                .setup_post(
                    "requestPCS",
                    Some(json!({ "appName": p.app_name, "derivedFromUserAction": first })),
                )
                .await?;
            let message = answer
                .body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("");
            if answer.flag("isWebAccessAllowed") == Some(false) {
                self.session.gate.web_access_off();
                self.keep();
                return Ok(DataAccess::WebAccessOff);
            }
            // Recovery on after all (Apple's "User has not disabled iCloud DRS"): no key needed.
            if answer.status.as_u16() == 412 || message.contains("iCloud DRS") {
                self.session.data_protection_seen = Some(false);
                self.keep();
                return Ok(DataAccess::Open);
            }
            if answer.status.as_u16() == 500 && message.contains(WEB_AUTH_COOKIE) {
                return Err(Error::ReauthRequired);
            }
            if answer.status.is_success()
                && answer.body.get("status").and_then(Value::as_str) == Some("success")
            {
                if !self.cookies_arrived(p) {
                    return Err(Error::Protocol(format!(
                        "Apple said the {} key was attached, but none arrived",
                        p.service
                    )));
                }
                let until = self.keys_until(consent_until);
                self.session.pcs_until.insert(p.app_name.to_string(), until);
                self.pcs.stale.remove(p.service);
                self.pcs.announced.remove(p.service);
                continue;
            }
            if PENDING.contains(&message) {
                waiting = true;
                continue;
            }
            return Err(Error::Protocol(format!(
                "Apple didn't release the {} key (HTTP {})",
                p.service, answer.status
            )));
        }
        // The cookies the answers set are in the jar: kept, sealed, for the next start.
        self.keep();
        Ok(if waiting {
            DataAccess::Waiting
        } else {
            DataAccess::Open
        })
    }

    /// Whether the cookies `requestPCS` is known to set for `p` are here now.
    fn cookies_arrived(&self, p: &Protected) -> bool {
        if p.cookies.is_empty() {
            return true;
        }
        let jar = self
            .jar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held: HashSet<&str> = jar.iter_unexpired().map(|c| c.name()).collect();
        p.cookies.iter().all(|name| held.contains(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_is_bounded_and_never_hammers() {
        let waits: Vec<Duration> = (0..).map_while(APPROVAL_SCHEDULE).collect();
        assert_eq!(waits.len(), 36, "36 asks at most");
        let total: u64 = waits.iter().map(Duration::as_secs).sum();
        assert_eq!(total, 5 * 60, "five minutes in all, as rclone allows");
        assert!(
            waits.iter().all(|w| w.as_secs() >= 5),
            "never faster than every 5 seconds"
        );
    }

    #[test]
    fn every_protected_service_has_its_app_name() {
        let names: Vec<&str> = PROTECTED.iter().map(|p| p.app_name).collect();
        assert_eq!(names, ["notes3", "reminders", "photos", "iclouddrive"]);
        assert!(protected("calendar").is_none() && protected("contacts").is_none());
    }

    #[tokio::test]
    async fn following_stops_at_the_first_answer_that_is_not_waiting() {
        let mut asked = 0;
        let got = follow(
            |n| (n < 10).then_some(Duration::ZERO),
            || {
                asked += 1;
                let now = asked;
                async move {
                    Ok(if now < 3 {
                        DataAccess::Waiting
                    } else {
                        DataAccess::Open
                    })
                }
            },
        )
        .await
        .unwrap();
        assert_eq!((got, asked), (DataAccess::Open, 3));

        let ran_out = follow(
            |n| (n < 4).then_some(Duration::ZERO),
            || async { Ok(DataAccess::Waiting) },
        )
        .await
        .unwrap();
        assert_eq!(
            ran_out,
            DataAccess::Needed,
            "run out, the person is asked to try again"
        );
    }
}
