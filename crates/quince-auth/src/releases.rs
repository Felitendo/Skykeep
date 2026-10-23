//! The updater's client: GitHub's release servers, and nothing else.
//!
//! Skykeep checks for a new version of itself and downloads it from the project's GitHub
//! Releases. That client is built here, beside Apple's, with the same two locks: a resolver
//! that looks up no other name, and a check of every request's own URL — a redirect's
//! included — so an address in a URL, a proxy or a redirect cannot carry it anywhere else.
//! Apple's client is not widened by it (it still refuses GitHub), and this one cannot reach
//! Apple.
//!
//! It is built on the reqwest the Tauri updater plugin uses (0.13; Apple's is 0.12), since the
//! plugin makes the requests and hands its client builder over to be locked
//! (`UpdaterBuilder::configure_client`).

use reqwest_releases as reqwest;
use std::sync::Arc;

/// Every host a release is fetched from. `latest.json` and each download are asked of
/// `github.com`, which answers with a redirect to the file itself on GitHub's download host:
/// `release-assets.githubusercontent.com` today (checked 2026-10-23 on a public release),
/// `objects.githubusercontent.com` before it, kept because GitHub still sends some downloads
/// there. `api.github.com` is not needed: the updater reads a file, not the API.
pub const RELEASE_HOSTS: [&str; 3] = [
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// Where a request that is not GitHub's release servers is pointed: a name the resolver
/// refuses, so it fails before anything leaves the machine.
const REFUSED: &str = "http://refused.skykeep.invalid";

/// Whether `host` is one of [`RELEASE_HOSTS`], exactly: no host under them, and no other.
#[must_use]
pub fn is_release_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    RELEASE_HOSTS.contains(&host.as_str())
}

/// Whether a request's URL may be sent: HTTPS, to a release host by name. Never an address,
/// never this computer, never plain HTTP.
#[must_use]
pub fn url_is_release(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        // An address never matches a name in the list, so an address in a URL is refused.
        && url.host_str().is_some_and(is_release_host)
}

/// Lock a client builder to GitHub's release servers. The updater plugin's own settings (its
/// user agent, its timeout) are kept; what it can reach is narrowed to [`RELEASE_HOSTS`].
pub fn releases_only(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    // The TLS the plugin is built with has no provider of its own; the plugin sets ring as
    // the default before it builds a client, and so does this, for a client built here.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let mut builder = builder
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .dns_resolver(Arc::new(ReleasesOnly))
        // Checked for every request and every redirect: one that is not to a release host is
        // sent to a "proxy" the resolver will not find, so the refusal reads the same.
        .proxy(reqwest::Proxy::custom(|url| {
            (!url_is_release(url)).then(|| REFUSED.to_string())
        }));
    // A proxy set above turns the system's off; the system's HTTPS proxy is put back for the
    // release hosts, as Apple's client puts it back for Apple's.
    if let Some(via) = crate::env_proxy("https") {
        builder = builder.proxy(
            reqwest::Proxy::custom(move |url| (url.scheme() == "https").then(|| via.clone()))
                .no_proxy(reqwest::NoProxy::from_env()),
        );
    }
    builder
}

/// The updater client's resolver: a release host is looked up as the system looks up any
/// name; any other is never looked up at all.
struct ReleasesOnly;

impl reqwest::dns::Resolve for ReleasesOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if !is_release_host(&host) {
                let refused: Box<dyn std::error::Error + Send + Sync> = format!(
                    "{host} is not one of GitHub's release servers, so updates cannot reach it"
                )
                .into();
                return Err(refused);
            }
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let addrs: reqwest::dns::Addrs = Box::new(found.collect::<Vec<_>>().into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_release_hosts_count() {
        for ok in [
            "github.com",
            "GitHub.com.",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
        ] {
            assert!(is_release_host(ok), "{ok}");
        }
        for no in [
            "api.github.com",
            "gist.github.com",
            "raw.githubusercontent.com",
            "githubusercontent.com",
            "evil-github.com",
            "github.com.evil.example",
            "github.io",
            "apple.com",
            "setup.icloud.com",
            "localhost",
            "example.com",
        ] {
            assert!(!is_release_host(no), "{no}");
        }
    }

    #[test]
    fn a_url_must_be_https_to_a_release_host_by_name() {
        let ok = |u: &str| url_is_release(&reqwest::Url::parse(u).unwrap());
        assert!(ok(
            "https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/latest.json"
        ));
        assert!(ok(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2"
        ));
        assert!(!ok("http://github.com/x"), "plain HTTP");
        assert!(!ok("https://github.com:8443/x"), "another port");
        assert!(!ok("https://140.82.112.3/x"), "an address, not a name");
        assert!(!ok("https://[::1]/x"), "this computer");
        assert!(!ok("https://localhost/x"), "this computer by name");
        assert!(!ok("https://api.github.com/repos/x/y/releases"));
        assert!(
            !ok("https://www.icloud.com/"),
            "Apple's hosts are not the updater's"
        );
    }

    /// Refused before connecting: by name (the resolver), by address and by scheme (the URL
    /// check). Nothing here reaches the network.
    #[tokio::test]
    async fn the_updater_client_refuses_everything_else() {
        let http = releases_only(reqwest::Client::builder()).build().unwrap();
        for url in [
            "https://example.com/latest.json",
            "https://www.icloud.com/",
            "https://idmsa.apple.com/",
            "https://93.184.215.14/latest.json",
            "https://[2606:4700:4700::1111]/",
            "http://github.com/latest.json",
            "https://api.github.com/repos/x/y/releases/latest",
        ] {
            let err = http.get(url).send().await.expect_err(url);
            let text = format!("{err:?}");
            assert!(
                text.contains("not one of GitHub's release servers")
                    || text.contains("refused.skykeep.invalid")
                    || text.contains("BadScheme"),
                "{url} was not refused by the lock: {text}"
            );
        }
    }
}
