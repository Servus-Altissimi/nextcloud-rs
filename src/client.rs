//! The [`Nextcloud`] client: construction, URL layout, and request helpers.

use std::time::Duration;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::{Method, RequestBuilder};
use url::Url;

use crate::auth::{Credentials, LoginCredentials};
use crate::error::{Error, Result};

/// Bytes escaped inside a WebDAV path segment. `/` is included: segments are
/// escaped one by one and joined, so a slash in a name is not a separator.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'%')
    .add(b'/')
    .add(b'\\')
    .add(b'^')
    .add(b'[')
    .add(b']')
    .add(b'|')
    .add(b'+');

pub(crate) fn encode_segment(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// Percent-encode a `/`-separated path, preserving separators and dropping
/// empty segments so `"/a//b/"` and `"a/b"` agree.
pub(crate) fn encode_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Normalise a host or URL into a base URL with a trailing slash, which
/// `Url::join` needs.
pub(crate) fn normalise_base(input: &str) -> Result<Url> {
    let mut s = input.trim().to_string();
    if s.is_empty() {
        return Err(Error::UnexpectedResponse("empty server URL".into()));
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("https://{s}");
    }
    // trailing index.php: copy-paste out of the browser bar
    if let Some(stripped) = s.strip_suffix("/index.php") {
        s = stripped.to_string();
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    Ok(Url::parse(&s)?)
}

/// Which personal WebDAV tree a path is relative to. All four live under
/// `remote.php/dav/` and embed the user id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DavRoot {
    Files,
    Uploads,
    Trashbin,
    Versions,
}

impl DavRoot {
    fn prefix(self) -> &'static str {
        match self {
            DavRoot::Files => "files",
            DavRoot::Uploads => "uploads",
            DavRoot::Trashbin => "trashbin",
            DavRoot::Versions => "versions",
        }
    }
}

/// An async Nextcloud client. Cheap to clone: the inner [`reqwest::Client`]
/// shares one connection pool.
///
/// ```no_run
/// # async fn demo() -> Result<(), nextcloud::Error> {
/// let nc = nextcloud::Nextcloud::builder("http://127.0.0.1:8080")
///     .basic_auth("alice", "app-password")
///     .build()?;
///
/// for entry in nc.files().list("/Documents").await? {
///     println!("{} ({} bytes)", entry.name(), entry.size.unwrap_or(0));
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Nextcloud {
    pub(crate) http: reqwest::Client,
    pub(crate) base: Url,
    pub(crate) creds: Credentials,
    pub(crate) user_id: Option<String>,
}

impl Nextcloud {
    pub fn builder(server: impl Into<String>) -> NextcloudBuilder {
        NextcloudBuilder {
            server: server.into(),
            creds: Credentials::Anonymous,
            user_id: None,
            user_agent: crate::DEFAULT_USER_AGENT.to_string(),
            timeout: Some(Duration::from_secs(60)),
            http: None,
        }
    }

    /// Build a client from a completed [`LoginFlowV2`](crate::LoginFlowV2)
    /// handshake, using the server URL and login name as reported.
    pub fn from_login(creds: LoginCredentials) -> Result<Self> {
        Nextcloud::builder(creds.server.clone())
            .user_id(creds.login_name.clone())
            .credentials(Credentials::from(creds))
            .build()
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    pub fn set_user_id(&mut self, user_id: impl Into<String>) {
        self.user_id = Some(user_id.into());
    }

    pub(crate) fn require_user_id(&self) -> Result<&str> {
        self.user_id.as_deref().ok_or(Error::MissingUserId)
    }

    pub(crate) fn dav_url(&self, root: DavRoot, rel_path: &str) -> Result<Url> {
        let user = self.require_user_id()?;
        let mut path = format!("remote.php/dav/{}/{}", root.prefix(), encode_segment(user));
        let rel = encode_path(rel_path);
        if !rel.is_empty() {
            path.push('/');
            path.push_str(&rel);
        }
        Ok(self.base.join(&path)?)
    }

    pub(crate) fn url(&self, rel: &str) -> Result<Url> {
        Ok(self.base.join(rel.trim_start_matches('/'))?)
    }

    pub(crate) fn request(&self, method: Method, url: Url) -> RequestBuilder {
        self.creds.apply(self.http.request(method, url))
    }

    /// The underlying HTTP client, for endpoints this crate does not model.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    /// Ask the server who the credentials belong to, and adopt the returned id
    /// as the WebDAV user id. Also a cheap credential check.
    pub async fn whoami(&mut self) -> Result<crate::provisioning::User> {
        let data = self
            .ocs_call(Method::GET, "ocs/v2.php/cloud/user", &[], &[])
            .await?;
        let user: crate::provisioning::User = serde_json::from_value(data)?;
        self.user_id = Some(user.id.clone());
        Ok(user)
    }
}

#[derive(Debug)]
pub struct NextcloudBuilder {
    server: String,
    creds: Credentials,
    user_id: Option<String>,
    user_agent: String,
    timeout: Option<Duration>,
    http: Option<reqwest::Client>,
}

impl NextcloudBuilder {
    /// Authenticate with HTTP Basic, ideally with an app password. The
    /// username doubles as the WebDAV user id unless
    /// [`user_id`](Self::user_id) overrides it.
    pub fn basic_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.creds = Credentials::basic(username, password);
        self
    }

    pub fn bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.creds = Credentials::bearer(token);
        self
    }

    pub fn credentials(mut self, creds: Credentials) -> Self {
        self.creds = creds;
        self
    }

    /// Override the user id embedded in personal WebDAV paths, for when the
    /// login name differs from the account id (LDAP, email-address logins).
    pub fn user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set the `User-Agent`, which names this client on the Login Flow grant
    /// screen and in the connected-devices list.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// Per-request timeout, 60s by default. `None` disables it, for large
    /// uploads and downloads.
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Supply a preconfigured HTTP client, e.g. with a proxy or a custom root
    /// certificate. Overrides `user_agent` and `timeout`.
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    pub fn build(self) -> Result<Nextcloud> {
        let base = normalise_base(&self.server)?;

        let http = match self.http {
            Some(c) => c,
            None => {
                let builder = reqwest::Client::builder();

                // neither knob exists on wasm; the browser owns both
                #[cfg(not(target_arch = "wasm32"))]
                let builder = {
                    let mut b = builder.user_agent(self.user_agent);
                    if let Some(t) = self.timeout {
                        b = b.timeout(t);
                    }
                    b
                };
                #[cfg(target_arch = "wasm32")]
                let _ = (&self.user_agent, self.timeout);

                builder.build()?
            }
        };

        let user_id = self
            .user_id
            .or_else(|| self.creds.login_name().map(str::to_owned));

        Ok(Nextcloud {
            http,
            base,
            creds: self.creds,
            user_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_base_url() {
        assert_eq!(
            normalise_base("cloud.example.com").unwrap().as_str(),
            "https://cloud.example.com/"
        );
        assert_eq!(
            normalise_base("https://cloud.example.com")
                .unwrap()
                .as_str(),
            "https://cloud.example.com/"
        );
        assert_eq!(
            normalise_base("https://example.com/nc/index.php")
                .unwrap()
                .as_str(),
            "https://example.com/nc/"
        );
    }

    #[test]
    fn dav_url_subdir() {
        let nc = Nextcloud::builder("https://example.com/nextcloud")
            .basic_auth("alice", "pw")
            .build()
            .unwrap();
        assert_eq!(
            nc.dav_url(DavRoot::Files, "/a.txt").unwrap().as_str(),
            "https://example.com/nextcloud/remote.php/dav/files/alice/a.txt"
        );
    }

    #[test]
    fn dav_url_escaping() {
        let nc = Nextcloud::builder("https://cloud.example.com")
            .basic_auth("alice", "pw")
            .build()
            .unwrap();

        assert_eq!(
            nc.dav_url(DavRoot::Files, "/holiday photos/a b.jpg")
                .unwrap()
                .as_str(),
            "https://cloud.example.com/remote.php/dav/files/alice/holiday%20photos/a%20b.jpg"
        );
        assert_eq!(
            nc.dav_url(DavRoot::Files, "").unwrap().as_str(),
            "https://cloud.example.com/remote.php/dav/files/alice"
        );
        assert_eq!(
            nc.dav_url(DavRoot::Files, "//a///b/").unwrap().as_str(),
            "https://cloud.example.com/remote.php/dav/files/alice/a/b"
        );
        assert_eq!(
            nc.dav_url(DavRoot::Files, "/track #1.mp3")
                .unwrap()
                .as_str(),
            "https://cloud.example.com/remote.php/dav/files/alice/track%20%231.mp3"
        );
    }

    #[test]
    fn dav_url_without_user() {
        let nc = Nextcloud::builder("https://cloud.example.com")
            .build()
            .unwrap();
        assert!(matches!(
            nc.dav_url(DavRoot::Files, "/a").unwrap_err(),
            Error::MissingUserId
        ));
    }
}
