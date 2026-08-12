//! Error types for every layer of the client.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Anything that can go wrong talking to a Nextcloud server.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    /// Transport-level failure: DNS, TLS, connection reset, timeout.
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    /// The OCS envelope came back with a non-success `statuscode`, which is
    /// not the HTTP status. See [`ocs::status`](crate::ocs::status).
    #[error("OCS error {code}: {message}")]
    Ocs { code: i32, message: String },

    #[error("WebDAV {method} {path} failed with HTTP {status}")]
    Dav {
        method: String,
        path: String,
        status: u16,
        body: String,
    },

    #[error("XML parse error: {0}")]
    Xml(String),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// A response was structurally valid but not what the endpoint promises.
    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),

    /// Login Flow v2 is still pending, which the server signals with a 404.
    /// The expected state while polling.
    #[error("login flow not completed yet")]
    LoginFlowPending,

    /// Login Flow v2 exceeded the server's 20 minute token lifetime.
    #[error("login flow timed out")]
    LoginFlowTimeout,

    /// A WebDAV path was requested with no user id known. Set one with
    /// [`NextcloudBuilder::user_id`](crate::NextcloudBuilder::user_id), use
    /// basic auth, or call [`Nextcloud::whoami`](crate::Nextcloud::whoami).
    #[error("no user id configured for WebDAV request")]
    MissingUserId,

    /// A local IO failure, e.g. reading a file being uploaded.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether the failure is an authentication or authorisation problem,
    /// reported through either protocol.
    pub fn is_auth_error(&self) -> bool {
        match self {
            Error::Ocs { code, .. } => matches!(code, 401 | 403 | 997),
            Error::Dav { status, .. } => matches!(status, 401 | 403),
            _ => false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        match self {
            Error::Ocs { code, .. } => matches!(code, 404 | 998),
            Error::Dav { status, .. } => *status == 404,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dav(status: u16) -> Error {
        Error::Dav {
            method: "GET".into(),
            path: "/a".into(),
            status,
            body: String::new(),
        }
    }

    fn ocs(code: i32) -> Error {
        Error::Ocs {
            code,
            message: "nope".into(),
        }
    }

    #[test]
    fn auth_failures() {
        for code in [401, 403, 997] {
            assert!(ocs(code).is_auth_error(), "ocs {code}");
        }
        for status in [401, 403] {
            assert!(dav(status).is_auth_error(), "dav {status}");
        }
        assert!(!ocs(404).is_auth_error());
        assert!(!dav(500).is_auth_error());
        assert!(!Error::LoginFlowTimeout.is_auth_error());
    }

    #[test]
    fn not_found_variants() {
        // 998 = legacy "not found"
        for code in [404, 998] {
            assert!(ocs(code).is_not_found(), "ocs {code}");
        }
        assert!(dav(404).is_not_found());
        assert!(!dav(403).is_not_found());
        assert!(!ocs(400).is_not_found());
        assert!(!Error::MissingUserId.is_not_found());
    }

    #[test]
    fn error_messages() {
        assert!(dav(404).to_string().contains("WebDAV GET /a failed"));
        assert!(ocs(998).to_string().contains("OCS error 998"));
        assert_eq!(
            Error::MissingUserId.to_string(),
            "no user id configured for WebDAV request"
        );
    }
}
