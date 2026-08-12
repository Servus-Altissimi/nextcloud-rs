//! Credentials and the interactive Login Flow v2 handshake.
//!
//! Three credential shapes work: basic auth with an app password (per-client,
//! revocable, unaffected by 2FA), basic auth with the account password (only
//! without 2FA), and bearer tokens for OIDC-fronted instances. [`LoginFlowV2`]
//! mints an app password through the browser.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

#[derive(Clone, Debug, Default)]
pub enum Credentials {
    /// Send no credentials. Only public endpoints will answer.
    #[default]
    Anonymous,
    /// HTTP Basic. `password` should be an app password where possible.
    Basic {
        /// Login name, which is not always the same as the display name.
        username: String,
        password: String,
    },
    /// `Authorization: Bearer <token>`, for OIDC-fronted instances.
    Bearer(String),
}

impl Credentials {
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Credentials::Basic {
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn bearer(token: impl Into<String>) -> Self {
        Credentials::Bearer(token.into())
    }

    /// The login name, when these credentials carry one.
    ///
    /// Used to seed the WebDAV user id, since personal DAV endpoints embed it.
    pub fn login_name(&self) -> Option<&str> {
        match self {
            Credentials::Basic { username, .. } => Some(username),
            _ => None,
        }
    }

    pub(crate) fn apply(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Credentials::Anonymous => rb,
            Credentials::Basic { username, password } => rb.basic_auth(username, Some(password)),
            Credentials::Bearer(token) => rb.bearer_auth(token),
        }
    }
}

/// The credentials handed back once the user finishes Login Flow v2.
///
/// Serialisable, to store and rebuild a session with
/// [`Nextcloud::from_login`](crate::Nextcloud::from_login). The app password
/// authenticates as the user until revoked.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LoginCredentials {
    /// Canonical server URL as the server reports it. Prefer this over the URL
    /// the user typed; it accounts for redirects and trailing paths.
    pub server: String,
    #[serde(rename = "loginName")]
    pub login_name: String,
    #[serde(rename = "appPassword")]
    pub app_password: String,
}

impl From<LoginCredentials> for Credentials {
    fn from(c: LoginCredentials) -> Self {
        Credentials::basic(c.login_name, c.app_password)
    }
}

#[derive(Debug, Deserialize)]
struct LoginFlowInit {
    poll: LoginFlowPoll,
    login: String,
}

#[derive(Debug, Deserialize)]
struct LoginFlowPoll {
    token: String,
    endpoint: String,
}

/// An in-progress Login Flow v2 handshake.
///
/// `POST <server>/index.php/login/v2` opens it, the returned `login` URL goes
/// to a browser, and the poll `token` is posted to the poll `endpoint` about
/// once a second: 404 until the user finishes, then one 200 with the
/// credentials. The token lives 20 minutes and the payload is served once.
///
/// ```no_run
/// # async fn demo() -> Result<(), nextcloud::Error> {
/// let flow = nextcloud::LoginFlowV2::start("http://127.0.0.1:8080").await?;
/// println!("open this in a browser: {}", flow.login_url());
/// let creds = flow.wait_for_completion().await?;
/// let nc = nextcloud::Nextcloud::from_login(creds)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct LoginFlowV2 {
    http: reqwest::Client,
    login_url: String,
    poll_endpoint: Url,
    poll_token: String,
}

impl LoginFlowV2 {
    /// Begin a handshake against `server`.
    ///
    /// The client's `User-Agent` is what the grant screen names;
    /// [`start_with_client`](Self::start_with_client) sets it.
    pub async fn start(server: &str) -> Result<Self> {
        let builder = reqwest::Client::builder();

        // wasm: the UA is the browser's and the builder has no such method
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.user_agent(crate::DEFAULT_USER_AGENT);

        Self::start_with_client(server, builder.build()?).await
    }

    pub async fn start_with_client(server: &str, http: reqwest::Client) -> Result<Self> {
        let base = crate::client::normalise_base(server)?;
        let url = base.join("index.php/login/v2")?;

        let resp = http
            .post(url)
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?;

        let init: LoginFlowInit = resp.json().await?;

        // overwrite.cli.url can name an origin we cannot reach from here
        Ok(Self {
            http,
            login_url: rebase_on(&base, &init.login)?.to_string(),
            poll_endpoint: rebase_on(&base, &init.poll.endpoint)?,
            poll_token: init.poll.token,
        })
    }

    pub fn login_url(&self) -> &str {
        &self.login_url
    }

    /// The opaque poll token. Exposed so a flow can be persisted and resumed.
    pub fn poll_token(&self) -> &str {
        &self.poll_token
    }

    /// Poll once. [`Error::LoginFlowPending`] until the user authorises.
    pub async fn poll_once(&self) -> Result<LoginCredentials> {
        let resp = self
            .http
            .post(self.poll_endpoint.clone())
            .header("Accept", "application/json")
            .form(&[("token", &self.poll_token)])
            .send()
            .await?;

        match resp.status().as_u16() {
            200 => Ok(resp.json().await?),
            404 => Err(Error::LoginFlowPending),
            other => Err(Error::UnexpectedResponse(format!(
                "login flow poll returned HTTP {other}"
            ))),
        }
    }

    /// Poll every second until the user finishes, or the 20 minute token
    /// lifetime expires.
    pub async fn wait_for_completion(&self) -> Result<LoginCredentials> {
        self.wait_with(Duration::from_secs(1), Duration::from_secs(20 * 60))
            .await
    }

    /// Poll on a custom interval up to a custom deadline.
    ///
    /// Elapsed time accumulates from the interval, since `Instant` is
    /// unavailable on `wasm32-unknown-unknown`.
    pub async fn wait_with(
        &self,
        interval: Duration,
        timeout: Duration,
    ) -> Result<LoginCredentials> {
        let mut waited = Duration::ZERO;
        loop {
            match self.poll_once().await {
                Ok(creds) => return Ok(creds),
                Err(Error::LoginFlowPending) => {
                    if waited >= timeout {
                        return Err(Error::LoginFlowTimeout);
                    }
                    sleep(interval).await;
                    waited += interval;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Hang an advertised URL's path and query off `base`.
fn rebase_on(base: &Url, advertised: &str) -> Result<Url> {
    let advertised = Url::parse(advertised)?;
    let mut out = base.clone();
    out.set_path(advertised.path());
    out.set_query(advertised.query());
    Ok(out)
}

/// Sleep, on whichever executor this target has.
#[cfg(not(target_arch = "wasm32"))]
async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_advertised_url() {
        let base = Url::parse("http://localhost:8080/").unwrap();
        let out = rebase_on(&base, "https://cloud.example.com/login/v2/poll").unwrap();
        assert_eq!(out.as_str(), "http://localhost:8080/login/v2/poll");
    }

    #[test]
    fn rebase_keeps_subdir() {
        let base = Url::parse("https://example.com/nextcloud/").unwrap();
        let out = rebase_on(&base, "https://internal.lan/nextcloud/login/v2/flow/abc").unwrap();
        assert_eq!(
            out.as_str(),
            "https://example.com/nextcloud/login/v2/flow/abc"
        );
    }

    #[test]
    fn rebase_with_query() {
        let base = Url::parse("https://a.example/").unwrap();
        let out = rebase_on(&base, "https://b.example/login/v2/poll?x=1").unwrap();
        assert_eq!(out.as_str(), "https://a.example/login/v2/poll?x=1");
    }

    #[test]
    fn rebase_noop() {
        let base = Url::parse("https://cloud.example.com/").unwrap();
        let out = rebase_on(&base, "https://cloud.example.com/login/v2/poll").unwrap();
        assert_eq!(out.as_str(), "https://cloud.example.com/login/v2/poll");
    }
}

#[cfg(target_arch = "wasm32")]
async fn sleep(duration: Duration) {
    gloo_timers::future::TimeoutFuture::new(duration.as_millis() as u32).await;
}
