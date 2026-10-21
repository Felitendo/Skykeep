//! Pomander talks only to Apple, by construction: the client cannot resolve, so cannot reach,
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
        "setup.icloud.com.cn",
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
        said.contains("Pomander talks only to Apple"),
        "refused by the resolver, not the network: {said}"
    );
}
