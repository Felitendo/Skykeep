//! Skykeep talks only to Apple, by construction: the client cannot resolve, so cannot reach,
//! any host that is not Apple's.

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
    ] {
        assert!(!is_apple_host(no), "{no}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_refuses_anything_but_apple_before_connecting() {
    let client = quince_auth::apple_only().build().unwrap();
    let err = client
        .get("https://example.com/")
        .send()
        .await
        .expect_err("a request to a host that is not Apple's");
    let said = format!("{err:?}");
    assert!(
        said.contains("Skykeep talks only to Apple"),
        "refused by the resolver, not the network: {said}"
    );
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
            format!("{err:?}").contains("Skykeep talks only to Apple"),
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
    assert!(
        format!("{:?}", away.expect_err("redirected away")).contains("Skykeep talks only to Apple")
    );
}
