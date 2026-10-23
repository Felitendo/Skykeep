//! Apple's client reaches only Apple, by construction: it cannot resolve, so cannot reach, any
//! host that is not Apple's — GitHub's release servers, which the updater uses, included.

#[test]
fn only_apples_hosts_count_as_apples() {
    use quince_auth::is_apple_host;
    for ok in [
        "idmsa.apple.com",
        "setup.icloud.com",
        "p177-ckdatabasews.icloud.com",
        "cvws.icloud-content.com",
        "p34-webcourier.push.apple.com",
        "www.icloud.com.",
        "localhost",
    ] {
        assert!(is_apple_host(ok), "{ok}");
    }
    for no in [
        "example.com",
        "icloud.com.evil.example",
        "notapple.com",
        "apple.com.evil.example",
        "evilicloud.com",
        "tile.openstreetmap.org",
        "setup.icloud.com.cn",
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    ] {
        assert!(!is_apple_host(no), "{no}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_refuses_anything_but_apple_before_connecting() {
    let client = quince_auth::apple_only().build().unwrap();
    // GitHub's release servers are the updater's own client's, never Apple's.
    for url in [
        "https://example.com/",
        "https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/latest.json",
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/2",
        "https://objects.githubusercontent.com/x",
    ] {
        let err = client
            .get(url)
            .send()
            .await
            .expect_err("a request to a host that is not Apple's");
        let said = format!("{err:?}");
        assert!(
            said.contains("Apple's client reaches only Apple"),
            "{url}: refused by the resolver, not the network: {said}"
        );
    }
}

/// And the other way round: the updater's client cannot reach Apple.
#[tokio::test(flavor = "multi_thread")]
async fn the_updater_client_cannot_reach_apple() {
    use quince_auth::releases::{is_release_host, releases_only};
    for host in quince_auth::APPLE_DOMAINS {
        assert!(!is_release_host(host), "{host}");
    }
    let client = releases_only(reqwest_releases::Client::builder())
        .build()
        .unwrap();
    for url in [
        "https://www.icloud.com/",
        "https://idmsa.apple.com/appleauth",
    ] {
        let err = client.get(url).send().await.expect_err(url);
        assert!(
            format!("{err:?}").contains("not one of GitHub's release servers"),
            "{url}: {err:?}"
        );
    }
}

/// An address in the URL is never looked up, so the resolver alone let it through: the URL
/// itself is checked, a redirect's included, and only this computer's addresses pass.
#[tokio::test(flavor = "multi_thread")]
async fn an_address_is_refused_as_a_name_is() {
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    let client = quince_auth::apple_only().build().unwrap();
    for url in [
        "https://93.184.215.14/",
        "http://1.1.1.1:80/x",
        "https://[2606:4700:4700::1111]/",
        "https://0x5db8d70e/",
    ] {
        let err = client.get(url).send().await.expect_err(url);
        assert!(
            format!("{err:?}").contains("Apple's client reaches only Apple"),
            "{url}: {err:?}"
        );
    }
    // This computer still answers (the tests' mock servers), and a redirect away is refused.
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/away"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://1.1.1.1/"))
        .mount(&server)
        .await;
    let ok = client.get(format!("{}/ok", server.uri())).send().await;
    assert_eq!(ok.unwrap().status(), 200);
    let away = client.get(format!("{}/away", server.uri())).send().await;
    assert!(format!("{:?}", away.expect_err("redirected away"))
        .contains("Apple's client reaches only Apple"));
}
