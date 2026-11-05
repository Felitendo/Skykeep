//! Where a session stands, in one place: each fact that decides what the person sees, the
//! states it can be in, and the only ways it moves between them.
//!
//! Two machines, each with its own lifetime:
//!
//! * [`SignIn`]: a sign-in in flight this run. The password is held here only until Apple has
//!   accepted it all the way, the code it wants (and whether it has been asked for) with it.
//!   Never written anywhere.
//! * [`Gate`]: what Apple has said holds the account's data back from this session. Kept
//!   with the session, read again from every answer that carries it.
//!
//! Advanced Data Protection's approval has its own explicit states, [`crate::pcs::DataAccess`],
//! read once the gate is open: nothing is approved while Apple waits at icloud.com.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A sign-in in flight this run.
///
/// ```text
///            start                    wants_code(lapse)
///   Idle ───────────────► Proving ─────────────────────► Code { sent, lapse }
///    ▲                      │  │                           │   │      ▲  │
///    │       fail           │  │ finish                    │   │ sent │  │ start
///    ├──────────────────────┘  │                           │   └──────┘  ▼
///    │       finish            ▼                           │          Proving
///    └─────────────────────────┴───────────────────────────┘
/// ```
///
/// `finish` is a completed sign-in: the password proved itself and is handed over to be kept.
/// `fail` is Apple, or this computer, ending the attempt before that.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) enum SignIn {
    /// No sign-in in flight: signed in, or not, but no password waiting to be kept.
    #[default]
    Idle,
    /// A password is being tried.
    Proving { apple_id: String, password: String },
    /// Apple accepted the password and wants a code before it trusts this computer.
    Code {
        apple_id: String,
        password: String,
        /// Apple has been asked to send it, to the devices or by text, so the code screen
        /// opening does not ask for another.
        sent: bool,
        /// A lapsed session met it signing itself back in, with nobody watching: nothing has
        /// been asked of Apple, and the window shows the code screen next.
        lapse: bool,
    },
}

impl SignIn {
    /// A password starts being tried, whatever came before: a new attempt starts clean.
    pub(crate) fn start(&mut self, apple_id: &str, password: &str) {
        *self = SignIn::Proving {
            apple_id: apple_id.trim().to_string(),
            password: password.to_string(),
        };
    }

    /// Apple accepted the password and wants a code. `lapse` when no one is watching.
    pub(crate) fn wants_code(&mut self, lapse: bool) {
        *self = match std::mem::take(self) {
            SignIn::Proving { apple_id, password } => SignIn::Code {
                apple_id,
                password,
                sent: false,
                lapse,
            },
            SignIn::Code {
                apple_id,
                password,
                sent,
                ..
            } => SignIn::Code {
                apple_id,
                password,
                sent,
                lapse,
            },
            SignIn::Idle => SignIn::Idle,
        };
    }

    /// Apple has been asked to send the code.
    pub(crate) fn sent(&mut self) {
        if let SignIn::Code { sent, .. } = self {
            *sent = true;
        }
    }

    /// The attempt ended before Apple accepted it: nothing is kept.
    pub(crate) fn fail(&mut self) {
        *self = SignIn::Idle;
    }

    /// The sign-in completed: the Apple ID and password to keep, if one was in flight.
    pub(crate) fn finish(&mut self) -> Option<(String, String)> {
        match std::mem::take(self) {
            SignIn::Idle => None,
            SignIn::Proving { apple_id, password }
            | SignIn::Code {
                apple_id, password, ..
            } => Some((apple_id, password)),
        }
    }

    /// A lapsed session wants a code before it carries on: the code screen is next.
    pub(crate) fn awaiting_code(&self) -> bool {
        matches!(self, SignIn::Code { lapse: true, .. })
    }

    /// The code this sign-in wants has already been asked for.
    pub(crate) fn code_sent(&self) -> bool {
        matches!(self, SignIn::Code { sent: true, .. })
    }

    /// The password in flight, if any.
    pub(crate) fn password(&self) -> Option<&str> {
        match self {
            SignIn::Idle => None,
            SignIn::Proving { password, .. } | SignIn::Code { password, .. } => Some(password),
        }
    }
}

/// What holds the account's data back from this session, as Apple last said it. At most one
/// applies, in this order: each before the next is all the person can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Nothing Apple has said holds the data back.
    Open,
    /// "Access iCloud Data on the Web" is off: nothing can be read until it is on.
    WebAccessOff,
    /// New iCloud terms to agree to at icloud.com (`termsUpdateNeeded`). Until then Apple opens
    /// only part of a web session, and every service fails (issue #5).
    Terms,
    /// The account to finish at icloud.com (`isRepairNeeded`).
    Repair,
}

impl Standing {
    /// Waiting on something only icloud.com can do.
    #[must_use]
    pub fn at_icloud_com(self) -> bool {
        matches!(self, Standing::Terms | Standing::Repair)
    }
}

/// The facts behind [`Standing`], kept with the session under the names they have always had,
/// so a session kept by an earlier version reads the same.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// `dsInfo.isWebAccessAllowed`; `None` until Apple has said.
    #[serde(default)]
    web_access_allowed: Option<bool>,
    #[serde(default)]
    terms_update_needed: bool,
    #[serde(default)]
    repair_needed: bool,
}

impl Gate {
    /// Where the account stands.
    #[must_use]
    pub fn standing(&self) -> Standing {
        if self.web_access_allowed == Some(false) {
            Standing::WebAccessOff
        } else if self.terms_update_needed {
            Standing::Terms
        } else if self.repair_needed {
            Standing::Repair
        } else {
            Standing::Open
        }
    }

    /// Whether web access is allowed, as Apple last said; `None` until it has.
    #[must_use]
    pub fn web_access_allowed(&self) -> Option<bool> {
        self.web_access_allowed
    }

    #[must_use]
    pub fn terms_update_needed(&self) -> bool {
        self.terms_update_needed
    }

    #[must_use]
    pub fn repair_needed(&self) -> bool {
        self.repair_needed
    }

    /// Read an account answer (`accountLogin`): each fact it carries replaces the one kept, so
    /// agreeing at icloud.com clears it at the next answer. One it leaves out is left as it was.
    pub fn absorb(&mut self, data: &Value) {
        if let Some(allowed) = data
            .pointer("/dsInfo/isWebAccessAllowed")
            .and_then(Value::as_bool)
        {
            self.web_access_allowed = Some(allowed);
        }
        if let Some(needed) = data.get("termsUpdateNeeded").and_then(Value::as_bool) {
            self.terms_update_needed = needed;
        }
        if let Some(needed) = data.get("isRepairNeeded").and_then(Value::as_bool) {
            self.repair_needed = needed;
        }
    }

    /// Apple said, outside an account answer, that web access is off.
    pub fn web_access_off(&mut self) {
        self.web_access_allowed = Some(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proving() -> SignIn {
        let mut s = SignIn::default();
        s.start(" someone@example.invalid ", "hunter2");
        s
    }

    #[test]
    fn a_sign_in_without_a_code_hands_its_password_over_once() {
        let mut s = proving();
        assert_eq!(s.password(), Some("hunter2"));
        assert!(!s.awaiting_code() && !s.code_sent());
        assert_eq!(
            s.finish(),
            Some(("someone@example.invalid".into(), "hunter2".into()))
        );
        assert_eq!(s, SignIn::Idle);
        assert_eq!(s.finish(), None, "kept once");
    }

    #[test]
    fn a_code_is_asked_for_once_and_survives_until_the_sign_in_completes() {
        let mut s = proving();
        s.sent();
        assert!(!s.code_sent(), "nothing to send before Apple wants a code");
        s.wants_code(false);
        assert!(!s.code_sent() && !s.awaiting_code());
        s.sent();
        assert!(s.code_sent());
        s.wants_code(false);
        assert!(s.code_sent(), "asked again, still sent");
        assert!(s.finish().is_some());
        assert!(!s.code_sent());
    }

    #[test]
    fn a_lapse_that_wants_a_code_shows_the_code_screen_with_nothing_sent() {
        let mut s = proving();
        s.wants_code(true);
        assert!(s.awaiting_code());
        assert!(
            !s.code_sent(),
            "nothing pushed to a phone from the background"
        );
        s.sent();
        assert!(s.awaiting_code() && s.code_sent());
        // The next attempt starts clean.
        s.start("someone@example.invalid", "hunter2");
        assert!(!s.awaiting_code() && !s.code_sent());
    }

    #[test]
    fn a_failed_attempt_keeps_nothing() {
        let mut s = proving();
        s.wants_code(false);
        s.fail();
        assert_eq!(s, SignIn::Idle);
        assert_eq!(s.password(), None);
        let mut idle = SignIn::Idle;
        idle.wants_code(true);
        assert_eq!(idle, SignIn::Idle, "no code without a password in flight");
    }

    #[test]
    fn standing_reads_apple_answers_in_order_and_clears_with_them() {
        let mut g = Gate::default();
        assert_eq!(g.standing(), Standing::Open);
        g.absorb(&json!({ "isRepairNeeded": true }));
        assert_eq!(g.standing(), Standing::Repair);
        g.absorb(&json!({ "termsUpdateNeeded": true }));
        assert_eq!(g.standing(), Standing::Terms, "terms before repair");
        assert!(g.standing().at_icloud_com());
        g.absorb(&json!({ "dsInfo": { "isWebAccessAllowed": false } }));
        assert_eq!(g.standing(), Standing::WebAccessOff, "web access first");
        assert!(!g.standing().at_icloud_com());
        // An answer that leaves a fact out leaves it as it was.
        g.absorb(&json!({ "dsInfo": { "isWebAccessAllowed": true } }));
        assert_eq!(g.standing(), Standing::Terms);
        g.absorb(&json!({ "termsUpdateNeeded": false, "isRepairNeeded": false }));
        assert_eq!(g.standing(), Standing::Open);
        g.web_access_off();
        assert_eq!(g.standing(), Standing::WebAccessOff);
        assert_eq!(g.web_access_allowed(), Some(false));
    }

    #[test]
    fn the_gate_keeps_the_names_a_kept_session_has_always_used() {
        #[derive(Serialize, Deserialize)]
        struct Holder {
            #[serde(flatten)]
            gate: Gate,
        }
        let old = r#"{"web_access_allowed":false,"terms_update_needed":true,"repair_needed":true}"#;
        let read: Holder = serde_json::from_str(old).unwrap();
        assert_eq!(read.gate.standing(), Standing::WebAccessOff);
        assert!(read.gate.terms_update_needed() && read.gate.repair_needed());
        let written = serde_json::to_value(&read).unwrap();
        assert_eq!(written, serde_json::from_str::<Value>(old).unwrap());
        let none: Holder = serde_json::from_str("{}").unwrap();
        assert_eq!(none.gate, Gate::default());
    }
}
