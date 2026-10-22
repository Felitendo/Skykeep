//! Apple's own names for the labels on a card's numbers, emails, links, addresses, dates and
//! related people.
//!
//! A label reaches this app in one of two spellings. Contacts on the phone and the Mac keep
//! Apple's internal token, `_$!<Mobile>!$_`, and a vCard carries it as `X-ABLabel`. Apple's web
//! service (`/co/`, what this app reads and writes) hands the same label out in its own upper
//! case — `MOBILE` — and takes it back that way; icloud.com's editor writes nothing else
//! (contacts2 bundle, 2026-10). Anything that is neither is a custom label, kept as typed.
//!
//! So a label is compared and shown through here, by what it means, and a label already on a
//! card is written back exactly as it came: neither spelling is ever rewritten into the other.

/// One label Apple knows: how `/co/` spells it, Apple's internal token, and how a card shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    pub wire: &'static str,
    pub token: &'static str,
    pub shown: &'static str,
}

const fn k(wire: &'static str, token: &'static str, shown: &'static str) -> Known {
    Known { wire, token, shown }
}

/// Every label Apple names, from icloud.com's editor and the Contacts framework's constants.
pub const KNOWN: &[Known] = &[
    k("MOBILE", "_$!<Mobile>!$_", "mobile"),
    k("IPHONE", "iPhone", "iPhone"),
    k("APPLE WATCH", "_$!<AppleWatch>!$_", "Apple Watch"),
    k("HOME", "_$!<Home>!$_", "home"),
    k("WORK", "_$!<Work>!$_", "work"),
    k("SCHOOL", "_$!<School>!$_", "school"),
    k("MAIN", "_$!<Main>!$_", "main"),
    k("HOME FAX", "_$!<HomeFAX>!$_", "home fax"),
    k("WORK FAX", "_$!<WorkFAX>!$_", "work fax"),
    k("OTHER FAX", "_$!<OtherFAX>!$_", "other fax"),
    k("PAGER", "_$!<Pager>!$_", "pager"),
    k("PHONE", "_$!<Phone>!$_", "phone"),
    k("OTHER", "_$!<Other>!$_", "other"),
    k("ICLOUD", "iCloud", "iCloud"),
    k("HOMEPAGE", "_$!<HomePage>!$_", "homepage"),
    k("BLOG", "_$!<Blog>!$_", "blog"),
    k("ANNIVERSARY", "_$!<Anniversary>!$_", "anniversary"),
    k("BIRTHDAY", "_$!<Birthday>!$_", "birthday"),
    k("MOTHER", "_$!<Mother>!$_", "mother"),
    k("FATHER", "_$!<Father>!$_", "father"),
    k("PARENT", "_$!<Parent>!$_", "parent"),
    k("BROTHER", "_$!<Brother>!$_", "brother"),
    k("SISTER", "_$!<Sister>!$_", "sister"),
    k("SON", "_$!<Son>!$_", "son"),
    k("DAUGHTER", "_$!<Daughter>!$_", "daughter"),
    k("CHILD", "_$!<Child>!$_", "child"),
    k("FRIEND", "_$!<Friend>!$_", "friend"),
    k("SPOUSE", "_$!<Spouse>!$_", "spouse"),
    k("PARTNER", "_$!<Partner>!$_", "partner"),
    k("ASSISTANT", "_$!<Assistant>!$_", "assistant"),
    k("MANAGER", "_$!<Manager>!$_", "manager"),
];

/// The labels icloud.com's editor offers for each kind of detail, in its order, `OTHER` last
/// as it adds it. Instant messaging is labelled by its service; a social profile by its network.
#[must_use]
pub fn offered(kind: &str) -> &'static [&'static str] {
    match kind {
        "phone" => &[
            "MOBILE",
            "IPHONE",
            "APPLE WATCH",
            "HOME",
            "WORK",
            "SCHOOL",
            "MAIN",
            "HOME FAX",
            "WORK FAX",
            "PAGER",
            "OTHER",
        ],
        "email" => &["HOME", "WORK", "SCHOOL", "ICLOUD", "OTHER"],
        "url" => &["HOMEPAGE", "HOME", "WORK", "SCHOOL", "OTHER"],
        "address" => &["HOME", "WORK", "SCHOOL", "OTHER"],
        "date" => &["ANNIVERSARY", "OTHER"],
        "related" => &[
            "MOTHER",
            "FATHER",
            "PARENT",
            "BROTHER",
            "SISTER",
            "SON",
            "DAUGHTER",
            "CHILD",
            "FRIEND",
            "SPOUSE",
            "PARTNER",
            "ASSISTANT",
            "MANAGER",
            "OTHER",
        ],
        "im" => &[
            "AIM",
            "YAHOO",
            "MSN",
            "ICQ",
            "JABBER",
            "SKYPE",
            "GOOGLETALK",
            "FACEBOOK",
            "QQ",
            "GADUGADU",
        ],
        "profile" => &[
            "TWITTER",
            "FACEBOOK",
            "FLICKR",
            "LINKEDIN",
            "MYSPACE",
            "SINAWEIBO",
        ],
        _ => &[],
    }
}

/// The label Apple means, in either spelling and any case; `None` for a custom label.
#[must_use]
pub fn known(label: &str) -> Option<&'static Known> {
    let label = label.trim();
    KNOWN.iter().find(|k| {
        k.wire.eq_ignore_ascii_case(label)
            || k.token.eq_ignore_ascii_case(label)
            || k.shown.eq_ignore_ascii_case(label)
    })
}

/// How a card shows a label: Apple's own word for one it knows, a custom one as typed.
#[must_use]
pub fn shown(label: &str) -> String {
    known(label).map_or_else(|| label.trim().to_string(), |k| k.shown.to_string())
}

/// What a label means, for comparing two: Apple's spelling of one it knows, a custom one
/// folded. `MOBILE`, `mobile` and `_$!<Mobile>!$_` are one label.
#[must_use]
pub fn meaning(label: Option<&str>) -> String {
    match label.map(str::trim).filter(|l| !l.is_empty()) {
        None => String::new(),
        Some(l) => known(l).map_or_else(|| crate::fold::fold(l), |k| k.wire.to_string()),
    }
}

/// Whether two labels say the same thing, however each is spelled.
#[must_use]
pub fn same(a: Option<&str>, b: Option<&str>) -> bool {
    meaning(a) == meaning(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_of_a_label_mean_one_thing() {
        assert!(same(Some("MOBILE"), Some("_$!<Mobile>!$_")));
        assert!(same(Some("mobile"), Some("MOBILE")));
        assert!(same(Some("HOME FAX"), Some("_$!<HomeFAX>!$_")));
        assert!(!same(Some("HOME"), Some("WORK")));
        assert!(same(None, Some("  ")));
    }

    #[test]
    fn a_label_is_shown_in_apples_words_and_a_custom_one_as_typed() {
        assert_eq!(shown("_$!<Mobile>!$_"), "mobile");
        assert_eq!(shown("IPHONE"), "iPhone");
        assert_eq!(shown("APPLE WATCH"), "Apple Watch");
        assert_eq!(shown("ICLOUD"), "iCloud");
        assert_eq!(shown("_$!<Anniversary>!$_"), "anniversary");
        assert_eq!(shown("Grandma's landline"), "Grandma's landline");
        assert_eq!(meaning(Some("Grandma's Landline")), "grandma's landline");
    }

    #[test]
    fn every_label_offered_is_one_apple_knows_or_a_service() {
        for kind in ["phone", "email", "url", "address", "date", "related"] {
            for label in offered(kind) {
                assert!(known(label).is_some(), "{kind}: {label}");
            }
        }
    }
}
