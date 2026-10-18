//! CloudKit's push channel: how a change made on a phone reaches this computer in about a
//! second, rather than at the next timed pass.
//!
//! As icloud.com's CloudKit JS does it, measured against the real service:
//!
//! - `POST {ckdevice}/device/1/<container>/production/tokens/create` with
//!   `{apnsEnvironment: "production", clientId}` answers `{apnsToken, webcourierURL}`.
//! - The same token is added to more containers with `.../tokens/register` and
//!   `{apnsEnvironment, apnsToken, clientId}`, so one connection carries Notes and Reminders.
//! - `GET webcourierURL` is parked: Apple holds it open and answers when something changed,
//!   with `{"aps": {...}, "ck": {"cid": "<container>", "met": {"zid": "<zone>", ...}}}`.
//!   It is parked again straight away for the next one.
//!
//! No subscription has to be made: the account's own devices keep a database subscription
//! (`DatabaseSubscription-Private`) that fires for every change. That includes this app's own
//! writes, which is harmless: they come back as a quick delta with nothing new.

use crate::{Error, Result};
use serde_json::{json, Value};

/// A push token, and the address to wait on for what it carries.
#[derive(Debug, Clone)]
pub struct Channel {
    pub token: String,
    pub url: String,
}

fn query(params: &[(String, String)]) -> Vec<(String, String)> {
    let mut q = params.to_vec();
    q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
    q.push(("ckjsVersion".into(), "2.6.1".into()));
    q
}

async fn post(
    http: &reqwest::Client,
    url: &str,
    params: &[(String, String)],
    body: &Value,
) -> Result<Value> {
    let response = http
        .post(url)
        .query(&query(params))
        .json(body)
        .send()
        .await?;
    let status = response.status();
    if status.as_u16() == 450 || status.as_u16() == 401 {
        return Err(Error::ReauthRequired);
    }
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "the push channel answered {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("the push channel sent invalid JSON: {e}")))
}

/// A token for the first container that also carries the rest, ready to wait on.
pub async fn open(
    http: &reqwest::Client,
    ckdevice_root: &str,
    params: &[(String, String)],
    client_id: &str,
    containers: &[&str],
) -> Result<Channel> {
    let root = ckdevice_root.trim_end_matches('/');
    let (first, rest) = containers
        .split_first()
        .ok_or_else(|| Error::Protocol("no container to listen to".into()))?;
    let made = post(
        http,
        &format!("{root}/device/1/{first}/production/tokens/create"),
        params,
        &json!({ "apnsEnvironment": "production", "clientId": client_id }),
    )
    .await?;
    let field = |key: &str| {
        made.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Protocol(format!("the push token came without {key}")))
    };
    let channel = Channel {
        token: field("apnsToken")?,
        url: field("webcourierURL")?,
    };
    for container in rest {
        post(
            http,
            &format!("{root}/device/1/{container}/production/tokens/register"),
            params,
            &json!({ "apnsEnvironment": "production", "apnsToken": channel.token, "clientId": client_id }),
        )
        .await?;
    }
    Ok(channel)
}

/// Wait on the channel for one push, for at most `longest`. `Ok(None)` is a wait that ended
/// with nothing to say — the connection closed or the time ran out — and is simply parked
/// again. Otherwise the container the change was in.
pub async fn wait(
    http: &reqwest::Client,
    channel: &Channel,
    longest: std::time::Duration,
) -> Result<Option<String>> {
    let response = match http.get(&channel.url).timeout(longest).send().await {
        Ok(response) => response,
        Err(e) if e.is_timeout() => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "the push channel answered {status}"
        )));
    }
    let text = response.text().await.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(None);
    }
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("the push channel sent invalid JSON: {e}")))?;
    Ok(container_of(&body))
}

/// Which container a push is about.
#[must_use]
pub fn container_of(push: &Value) -> Option<String> {
    push.pointer("/ck/cid")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// The push Apple actually sent for a Reminders change.
    fn measured() -> Value {
        json!({"aps":{"content-available":1},"ck":{"ce":1,"ckuserid":"_u","aux":{},"nid":"n",
               "met":{"zid":"Reminders","dbs":1,"zoid":"_u","sid":"DatabaseSubscription-Private"},
               "cid":"com.apple.reminders"}})
    }

    #[test]
    fn a_push_names_its_container() {
        assert_eq!(
            container_of(&measured()).as_deref(),
            Some("com.apple.reminders")
        );
        assert_eq!(container_of(&json!({"aps":{}})), None);
    }

    #[tokio::test]
    async fn one_token_is_made_and_shared_with_the_other_containers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/device/1/com.apple.notes/production/tokens/create"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apnsEnvironment": "production", "apnsToken": "T1",
                "webcourierURL": format!("{}/aps?tok=T1", server.uri())
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(
                "/device/1/com.apple.reminders/production/tokens/register",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/aps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(measured()))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let channel = open(
            &http,
            &server.uri(),
            &[],
            "c1",
            &["com.apple.notes", "com.apple.reminders"],
        )
        .await
        .unwrap();
        assert_eq!(channel.token, "T1");
        let registered: Vec<Request> = server.received_requests().await.unwrap();
        let body: Value = registered
            .iter()
            .find(|r| r.url.path().ends_with("/register"))
            .unwrap()
            .body_json()
            .unwrap();
        assert_eq!(body["apnsToken"], "T1", "the one token, not a second");
        let got = wait(&http, &channel, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(got.as_deref(), Some("com.apple.reminders"));
    }

    #[tokio::test]
    async fn a_wait_that_runs_out_is_nothing_not_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/aps"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)))
            .mount(&server)
            .await;
        let channel = Channel {
            token: "T".into(),
            url: format!("{}/aps", server.uri()),
        };
        let got = wait(
            &reqwest::Client::new(),
            &channel,
            std::time::Duration::from_millis(300),
        )
        .await;
        assert!(matches!(got, Ok(None)), "{got:?}");
    }
}
