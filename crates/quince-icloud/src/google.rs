//! Signing in to Google Calendar, the way a desktop app is meant to (OAuth 2.0 with PKCE and a
//! loopback redirect).
//!
//! Google's CalDAV refuses passwords. The person signs in on Google's own page in their browser;
//! Google sends the browser back to a port on this computer with a code, which is traded for a
//! token. The OAuth client is the person's own, from their Google Cloud project: Skykeep ships
//! none, so nothing about it is shared between the people who use it.

use crate::{Error, Result};
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use reqwest::{Client as Http, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// What an account's client reaches: the calendars, and where its tokens are renewed.
pub const HOSTS: [&str; 2] = ["apidata.googleusercontent.com", "oauth2.googleapis.com"];

/// What an account is known by, in place of a server the person named.
pub const SERVER: &str = "https://apidata.googleusercontent.com";

const CALENDARS: &str = "https://www.googleapis.com/auth/calendar";

/// The person's own OAuth client. A desktop client's secret is no secret (Google's words: such
/// apps "cannot keep secrets confidential"), but it is kept in the keyring all the same.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthClient {
    pub id: String,
    #[serde(default)]
    pub secret: Option<String>,
}

/// What the keyring holds for a Google account.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub client: OAuthClient,
    pub refresh_token: String,
}

/// A client as typed, checked enough to say a wrong paste at once.
pub fn client_from(id: &str, secret: Option<&str>) -> Result<OAuthClient> {
    let id = id.trim();
    if !id.ends_with(".apps.googleusercontent.com") {
        return Err(Error::Refused(
            "That isn’t an OAuth client ID. It ends in .apps.googleusercontent.com.".into(),
        ));
    }
    Ok(OAuthClient {
        id: id.to_string(),
        secret: secret
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
    })
}

fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// A sign-in under way: the page to open, and the port Google sends the browser back to.
pub struct Flow {
    pub login: String,
    listener: TcpListener,
    redirect: String,
    verifier: String,
    state: String,
}

/// Open a port on this computer alone and make the page that leads back to it.
pub async fn begin(client: &OAuthClient) -> Result<Flow> {
    begin_at(AUTH_URL, client).await
}

async fn begin_at(auth_url: &str, client: &OAuthClient) -> Result<Flow> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let redirect = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    let verifier = random();
    let state = random();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let scope = format!("{CALENDARS} openid email");
    let login = Url::parse_with_params(
        auth_url,
        [
            ("client_id", client.id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            // Without it, a second sign-in comes back with no refresh token.
            ("prompt", "consent"),
        ],
    )
    .map_err(|_| Error::Protocol("the sign-in address can't be made".into()))?
    .to_string();
    Ok(Flow {
        login,
        listener,
        redirect,
        verifier,
        state,
    })
}

const CAME_BACK: &str = "<!doctype html><meta charset=utf-8><title>Skykeep</title>\
<body style=\"font:16px system-ui;margin:20vh auto;max-width:30em;text-align:center\">\
<h1 style=\"font-size:20px\">You’re signed in</h1><p>You can close this page and go back to Skykeep.";

const WENT_WRONG: &str = "<!doctype html><meta charset=utf-8><title>Skykeep</title>\
<body style=\"font:16px system-ui;margin:20vh auto;max-width:30em;text-align:center\">\
<h1 style=\"font-size:20px\">The sign-in didn’t finish</h1><p>Go back to Skykeep to try again.";

async fn answer(socket: &mut tokio::net::TcpStream, status: &str, page: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         Connection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = socket.write_all(reply.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// The first line of what a connection sent, or nothing if it sends nothing soon.
async fn request_line(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    let reading = async {
        while bytes.len() < 8192 && !bytes.windows(2).any(|w| w == b"\r\n") {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), reading)
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    text.lines().next().map(ToString::to_string)
}

/// Wait for Google to send the browser back, and answer the code it carries. Anything that
/// doesn't carry this sign-in's own `state` (a browser's request for its icon, another program
/// trying the port) is turned away, and the wait goes on.
pub async fn wait(flow: &Flow) -> Result<String> {
    loop {
        let (mut socket, _) = flow.listener.accept().await?;
        let Some(line) = request_line(&mut socket).await else {
            continue;
        };
        let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let params: Vec<(String, String)> = Url::parse(&format!("http://127.0.0.1{target}"))
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default();
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        if get("state").as_deref() != Some(flow.state.as_str()) {
            answer(&mut socket, "404 Not Found", WENT_WRONG).await;
            continue;
        }
        if let Some(reason) = get("error") {
            answer(&mut socket, "200 OK", WENT_WRONG).await;
            return Err(Error::Refused(if reason == "access_denied" {
                "The sign-in was cancelled on Google’s page.".into()
            } else {
                format!("Google stopped the sign-in ({reason}).")
            }));
        }
        let Some(code) = get("code") else {
            answer(&mut socket, "400 Bad Request", WENT_WRONG).await;
            continue;
        };
        answer(&mut socket, "200 OK", CAME_BACK).await;
        return Ok(code);
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize, Default)]
struct TokenError {
    #[serde(default)]
    error: String,
}

/// What signing in came to. Not `Debug`: it holds the tokens.
pub struct SignedIn {
    pub email: String,
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: String,
}

/// The address inside an ID token. It came straight from Google over TLS, so its signature
/// need not be checked (OpenID Connect Core, 3.1.3.7).
fn email_of(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("email")?.as_str().map(ToString::to_string)
}

async fn token_request(http: &Http, url: &str, form: &[(&str, &str)]) -> Result<TokenAnswer> {
    let response = http.post(url).form(form).send().await?;
    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .map_err(|_| Error::Protocol("Google's answer can't be read".into()));
    }
    let said: TokenError = response.json().await.unwrap_or_default();
    Err(match said.error.as_str() {
        // Revoked, or a client still in testing after its seven days.
        "invalid_grant" => Error::ReauthRequired,
        "invalid_client" | "unauthorized_client" => Error::Refused(
            "Google didn’t accept that OAuth client. Check its ID and secret.".into(),
        ),
        "invalid_request" => Error::Refused(
            "Google didn’t accept that sign-in. If the client has a secret, enter it too.".into(),
        ),
        _ if status.is_server_error() || status.as_u16() == 429 => Error::Busy,
        _ => Error::Refused(format!(
            "Google turned the sign-in down (it answered {}).",
            status.as_u16()
        )),
    })
}

/// Trade the code for tokens.
pub async fn exchange(
    http: &Http,
    token_url: &str,
    flow: &Flow,
    client: &OAuthClient,
    code: &str,
) -> Result<SignedIn> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client.id.as_str()),
        ("code", code),
        ("code_verifier", flow.verifier.as_str()),
        ("redirect_uri", flow.redirect.as_str()),
    ];
    if let Some(secret) = &client.secret {
        form.push(("client_secret", secret.as_str()));
    }
    let answer = token_request(http, token_url, &form).await?;
    // On Google's page each permission is its own tick box.
    if !answer
        .scope
        .as_deref()
        .is_some_and(|s| s.split_whitespace().any(|one| one == CALENDARS))
    {
        return Err(Error::Refused(
            "Skykeep wasn’t given access to your calendars. Sign in again and tick that box."
                .into(),
        ));
    }
    let email = answer
        .id_token
        .as_deref()
        .and_then(email_of)
        .ok_or_else(|| Error::Protocol("Google didn't say which account this is".into()))?;
    let refresh_token = answer
        .refresh_token
        .ok_or_else(|| Error::Protocol("Google gave no lasting sign-in".into()))?;
    Ok(SignedIn {
        email,
        access_token: answer.access_token,
        expires_in: answer.expires_in,
        refresh_token,
    })
}

/// A new access token, and how many seconds it lasts. `ReauthRequired`: the sign-in is over.
pub async fn refresh(http: &Http, token_url: &str, kept: &Kept) -> Result<(String, i64)> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("client_id", kept.client.id.as_str()),
        ("refresh_token", kept.refresh_token.as_str()),
    ];
    if let Some(secret) = &kept.client.secret {
        form.push(("client_secret", secret.as_str()));
    }
    let answer = token_request(http, token_url, &form).await?;
    Ok((answer.access_token, answer.expires_in))
}

/// Let go of the sign-in at Google. What can't be done now is for the person to do in their
/// Google account's settings.
pub async fn revoke(http: &Http, revoke_url: &str, token: &str) -> Result<()> {
    let response = http
        .post(revoke_url)
        .form(&[("token", token)])
        .send()
        .await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::Refused(
            "Google didn’t let go of the sign-in.".into(),
        ))
    }
}

/// Where this account's calendars are found from: its principal, as Google documents it, then
/// the root of its CalDAV.
pub fn starts(email: &str) -> Result<Vec<Url>> {
    let root = Url::parse(&format!("{SERVER}/caldav/v2/"))
        .map_err(|_| Error::Protocol("the address can't be made".into()))?;
    let mut principal = root.clone();
    principal
        .path_segments_mut()
        .map_err(|()| Error::Protocol("the address can't be made".into()))?
        .pop_if_empty()
        .push(email)
        .push("user");
    Ok(vec![principal, root])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> OAuthClient {
        OAuthClient {
            id: "abc.apps.googleusercontent.com".into(),
            secret: Some("shh".into()),
        }
    }

    fn id_token(email: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::json!({ "email": email }).to_string());
        format!("header.{payload}.signature")
    }

    async fn knock(flow: &Flow, query: &str) -> reqwest::Response {
        let port = flow
            .redirect
            .trim_end_matches('/')
            .rsplit(':')
            .next()
            .unwrap();
        Http::new()
            .get(format!("http://127.0.0.1:{port}/{query}"))
            .send()
            .await
            .unwrap()
    }

    fn param(url: &str, name: &str) -> String {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn the_page_asks_for_calendars_a_lasting_sign_in_and_proof_of_this_computer() {
        let flow = begin(&client()).await.unwrap();
        assert!(flow.login.starts_with(AUTH_URL));
        assert_eq!(param(&flow.login, "client_id"), client().id);
        assert_eq!(param(&flow.login, "code_challenge_method"), "S256");
        assert_eq!(param(&flow.login, "access_type"), "offline");
        assert!(param(&flow.login, "scope").contains(CALENDARS));
        assert!(param(&flow.login, "redirect_uri").starts_with("http://127.0.0.1:"));
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(flow.verifier.as_bytes()));
        assert_eq!(param(&flow.login, "code_challenge"), challenge);
        assert!((43..=128).contains(&flow.verifier.len()));
        assert!(!flow.login.contains("shh"), "the secret stays off the page");
    }

    /// Only the browser coming back from this sign-in counts.
    #[tokio::test]
    async fn the_port_answers_only_this_sign_ins_own_return() {
        let flow = begin(&client()).await.unwrap();
        let state = flow.state.clone();
        let (code, ()) = tokio::join!(wait(&flow), async {
            assert_eq!(knock(&flow, "favicon.ico").await.status(), 404);
            assert_eq!(knock(&flow, "?code=stolen&state=wrong").await.status(), 404);
            let page = knock(&flow, &format!("?code=good&state={state}")).await;
            assert_eq!(page.status(), 200);
            assert!(page.text().await.unwrap().contains("close this page"));
        });
        assert_eq!(code.unwrap(), "good");
    }

    #[tokio::test]
    async fn a_sign_in_cancelled_on_googles_page_says_so() {
        let flow = begin(&client()).await.unwrap();
        let state = flow.state.clone();
        let (said, _) = tokio::join!(wait(&flow), async {
            knock(&flow, &format!("?error=access_denied&state={state}")).await
        });
        assert!(said.unwrap_err().to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn the_code_is_traded_with_its_proof_for_tokens_and_the_address() {
        let server = MockServer::start().await;
        let flow = begin(&client()).await.unwrap();
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains(format!(
                "code_verifier={}",
                flow.verifier
            )))
            .and(body_string_contains("client_secret=shh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "A", "expires_in": 3599, "refresh_token": "R",
                "scope": format!("openid {CALENDARS} email"),
                "id_token": id_token("felix@example.org"),
            })))
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());
        let signed = exchange(&Http::new(), &url, &flow, &client(), "code")
            .await
            .unwrap();
        assert_eq!(signed.email, "felix@example.org");
        assert_eq!(signed.access_token, "A");
        assert_eq!(signed.refresh_token, "R");
    }

    /// Google's page lets a person leave the calendars box unticked.
    #[tokio::test]
    async fn a_sign_in_without_the_calendars_is_said_to_be_one() {
        let server = MockServer::start().await;
        let flow = begin(&client()).await.unwrap();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "A", "expires_in": 3599, "refresh_token": "R",
                "scope": "openid email", "id_token": id_token("a@b.c"),
            })))
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());
        let said = exchange(&Http::new(), &url, &flow, &client(), "c")
            .await
            .err()
            .unwrap();
        assert!(said.to_string().contains("calendars"), "{said}");
    }

    #[tokio::test]
    async fn a_sign_in_that_has_ended_asks_for_a_new_one_and_a_live_one_renews() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("refresh_token=dead"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({ "error": "invalid_grant" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("refresh_token=live"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "access_token": "new", "expires_in": 3599 }),
                ),
            )
            .mount(&server)
            .await;
        let url = format!("{}/token", server.uri());
        let kept = |token: &str| Kept {
            client: client(),
            refresh_token: token.into(),
        };
        assert!(matches!(
            refresh(&Http::new(), &url, &kept("dead")).await,
            Err(Error::ReauthRequired)
        ));
        let (token, lasts) = refresh(&Http::new(), &url, &kept("live")).await.unwrap();
        assert_eq!((token.as_str(), lasts), ("new", 3599));
    }

    #[test]
    fn a_wrong_paste_is_said_at_once() {
        assert!(client_from("not-a-client", None).is_err());
        let c = client_from(" x.apps.googleusercontent.com ", Some("  ")).unwrap();
        assert_eq!(c.id, "x.apps.googleusercontent.com");
        assert_eq!(c.secret, None);
    }

    #[test]
    fn the_calendars_are_looked_for_under_the_accounts_own_address() {
        let found = starts("felix@example.org").unwrap();
        assert_eq!(found[0].host_str(), Some(HOSTS[0]));
        assert!(
            found[0].path().ends_with("/felix@example.org/user"),
            "{}",
            found[0]
        );
        assert_eq!(found[1].path(), "/caldav/v2/");
    }
}
