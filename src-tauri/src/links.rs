//! Where a navigation in a window may go. Much of what the window shows was written by someone
//! else — a shared note, a shared list, an invitation, a public calendar — so a link's address
//! is theirs to choose, and the computer's opener (ShellExecute, xdg-open) runs whatever an
//! address names: a `file:` link to a share, a `search-ms:` query, a `.desktop` file, a UNC
//! path that hands over the person's Windows password hash. So only the kinds a person means by
//! "a link" leave Skykeep; the page keeps the same list (src/lib/links.ts).

use tauri::Url;

/// The kinds of address handed to the computer's opener. Anything else is refused.
const OUTSIDE: &[&str] = &["http", "https", "mailto", "tel", "geo"];

/// What becomes of a navigation.
#[derive(Debug, PartialEq, Eq)]
pub enum Navigation {
    /// The app's own page: followed in the window.
    Inside,
    /// A link Skykeep opens: refused in the window and handed to the computer.
    Outside,
    /// Neither: refused, and not opened anywhere. `data:`, `blob:` and `javascript:` would put
    /// a page of someone else's making in the window (a convincing "sign in again"), and
    /// `quince:` is the app's own data, which is fetched, never navigated to.
    Refused,
}

/// Where `url` may go. `dev` is a development build, whose page comes from the dev server on
/// localhost; a release counts no other program's localhost as its own.
pub fn navigation(url: &Url, dev: bool) -> Navigation {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");
    let web = matches!(scheme, "http" | "https");
    // The app's own page: `tauri://localhost` on Linux and macOS, `http(s)://tauri.localhost`
    // on Windows; a blank page is nothing at all.
    if scheme == "tauri" && host == "localhost"
        || web && host == "tauri.localhost"
        || web && dev && host == "localhost"
        || scheme == "about" && url.path() == "blank"
    {
        return Navigation::Inside;
    }
    if !OUTSIDE.contains(&scheme) {
        return Navigation::Refused;
    }
    // A web address names a host; one that names none (`http:///x`) is not followed.
    if web && host.is_empty() {
        return Navigation::Refused;
    }
    Navigation::Outside
}

/// What every `quince://` answer says about itself. The scheme is the app's own, so a page it
/// served would be trusted with the app's commands; what it serves is pictures, media and files,
/// some of them someone else's bytes. So: never read as another type than the one given
/// (`nosniff`), and, were one ever opened as a page, nothing in it may run or load
/// (`sandbox`, `default-src 'none'`).
pub const GUARDS: [(&str, &str); 2] = [
    ("x-content-type-options", "nosniff"),
    (
        "content-security-policy",
        "default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; sandbox",
    ),
];

/// A response with the guards on it, in place of any it set itself.
pub fn guard<T>(mut response: tauri::http::Response<T>) -> tauri::http::Response<T> {
    for (name, value) in GUARDS {
        response.headers_mut().insert(
            tauri::http::HeaderName::from_static(name),
            tauri::http::HeaderValue::from_static(value),
        );
    }
    response
}

/// The scheme's responder, which guards every answer it gives.
pub struct Guarded(pub tauri::UriSchemeResponder);

impl Guarded {
    pub fn respond<T: Into<std::borrow::Cow<'static, [u8]>>>(
        self,
        response: tauri::http::Response<T>,
    ) {
        self.0.respond(guard(response));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(s: &str, dev: bool) -> Navigation {
        navigation(&Url::parse(s).unwrap(), dev)
    }

    #[test]
    fn the_apps_own_page_stays_in_the_window() {
        assert_eq!(
            nav("tauri://localhost/index.html", false),
            Navigation::Inside
        );
        assert_eq!(nav("http://tauri.localhost/", false), Navigation::Inside);
        assert_eq!(
            nav("https://tauri.localhost/?app=notes", false),
            Navigation::Inside
        );
        assert_eq!(nav("about:blank", false), Navigation::Inside);
        assert_eq!(nav("http://localhost:1420/", true), Navigation::Inside);
    }

    #[test]
    fn web_mail_calls_and_places_go_to_the_computer() {
        for s in [
            "https://example.org/agenda",
            "http://example.org/",
            "mailto:someone@example.org",
            "tel:+15550100",
            "geo:0,0?q=Main%20Street",
            "HTTPS://EXAMPLE.ORG/",
        ] {
            assert_eq!(nav(s, false), Navigation::Outside, "{s}");
        }
        // Another program's server on this computer is not the app's page in a release.
        assert_eq!(nav("http://localhost:8080/", false), Navigation::Outside);
    }

    #[test]
    fn every_answer_is_guarded_whatever_it_set() {
        let r = tauri::http::Response::builder()
            .header("Content-Type", "text/html")
            .header("X-Content-Type-Options", "sniff-away")
            .body(Vec::<u8>::new())
            .unwrap();
        let r = guard(r);
        assert_eq!(r.headers()["x-content-type-options"], "nosniff");
        let csp = r.headers()["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("default-src 'none'") && csp.contains("sandbox"),
            "{csp}"
        );
        assert!(!csp.contains("script-src"), "{csp}");
        assert_eq!(
            r.headers().get_all("x-content-type-options").iter().count(),
            1
        );
    }

    #[test]
    fn every_other_kind_is_refused_not_opened() {
        for s in [
            "file:///home/someone/x.desktop",
            "file://attacker.example/DavWWWRoot/a.lnk",
            "file://server/share/report.url",
            "search-ms:query=x&crumb=location:%5C%5Cattacker.example%5Cshare",
            "search:query=x",
            "ms-msdt:/id PCWDiagnostic",
            "ms-settings:display",
            "smb://attacker.example/share",
            "javascript:alert(1)",
            "data:text/html,<h1>Sign in to iCloud</h1>",
            "blob:tauri://localhost/0b1c",
            "quince://photo/ABC",
            "quince://localhost/note-file/x",
            "applenotes:note/0b1c2d3e-0000-0000-0000-000000000000",
            "vscode://file/x",
            "about:srcdoc",
            "tauri://elsewhere/",
            "ftp://example.org/x",
            "webcal://example.org/feed.ics",
        ] {
            assert_eq!(nav(s, false), Navigation::Refused, "{s}");
        }
    }
}
