//! Server version and capability discovery.
//!
//! `GET /ocs/v2.php/cloud/capabilities`. The payload is a typed `version` block
//! plus a `capabilities` map keyed by app id. Each app defines its own schema
//! there, so the map stays raw JSON behind typed accessors.
//!
//! ```no_run
//! # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
//! let caps = nc.capabilities().await?;
//! println!("Nextcloud {}", caps.version.string);
//!
//! if caps.has_app("files_sharing") {
//!     let public = caps.get("files_sharing", "public");
//!     println!("public link sharing config: {public:?}");
//! }
//! # Ok(())
//! # }
//! ```

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::serde_util::{flexible_bool, flexible_i64, flexible_string};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ServerVersion {
    #[serde(default, deserialize_with = "flexible_i64")]
    pub major: i64,
    #[serde(default, deserialize_with = "flexible_i64")]
    pub minor: i64,
    #[serde(default, deserialize_with = "flexible_i64")]
    pub micro: i64,
    #[serde(default, deserialize_with = "flexible_string")]
    pub string: String,
    #[serde(default, deserialize_with = "flexible_string")]
    pub edition: String,
    #[serde(
        default,
        rename = "extendedSupport",
        deserialize_with = "flexible_bool"
    )]
    pub extended_support: bool,
}

impl ServerVersion {
    /// Whether the server is at least `major.minor`.
    pub fn at_least(&self, major: i64, minor: i64) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Capabilities {
    #[serde(default)]
    pub version: ServerVersion,
    /// Per-app capability objects, keyed by app id, as raw JSON.
    #[serde(default)]
    pub capabilities: Value,
}

impl Capabilities {
    /// Whether an app reported capabilities, meaning it is installed and enabled.
    pub fn has_app(&self, app: &str) -> bool {
        self.capabilities.get(app).is_some()
    }

    pub fn get(&self, app: &str, key: &str) -> Option<&Value> {
        self.capabilities.get(app)?.get(key)
    }

    /// Read a boolean capability, accepting `true`, `1` and `"1"`.
    pub fn get_bool(&self, app: &str, key: &str) -> Option<bool> {
        match self.get(app, key)? {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => Some(n.as_i64()? != 0),
            Value::String(s) => Some(matches!(s.as_str(), "1" | "true" | "yes")),
            _ => None,
        }
    }

    pub fn app(&self, app: &str) -> Option<&Value> {
        self.capabilities.get(app)
    }

    /// `core.pollinterval`: how often, in seconds, the server suggests polling
    /// for notifications.
    pub fn poll_interval(&self) -> Option<i64> {
        self.get("core", "pollinterval")?.as_i64()
    }

    /// `core.webdav-root`, the legacy entry point. [`Files`](crate::files::Files)
    /// uses `remote.php/dav/files/{user}` instead.
    pub fn webdav_root(&self) -> Option<&str> {
        self.get("core", "webdav-root")?.as_str()
    }
}

impl Nextcloud {
    /// Fetch the server's version and capabilities. Unauthenticated on most
    /// instances, so it doubles as a reachability check.
    pub async fn capabilities(&self) -> Result<Capabilities> {
        self.ocs_typed(Method::GET, "ocs/v2.php/cloud/capabilities", &[], &[])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Capabilities {
        serde_json::from_value(serde_json::json!({
            "version": {
                "major": 31, "minor": 0, "micro": 2,
                "string": "31.0.2", "edition": "", "extendedSupport": false
            },
            "capabilities": {
                "core": { "pollinterval": 60, "webdav-root": "remote.php/webdav" },
                "files_sharing": { "api_enabled": true, "public": { "enabled": "1" } },
                "theming": { "name": "Nextcloud" }
            }
        }))
        .unwrap()
    }

    #[test]
    fn version() {
        let c = sample();
        assert_eq!(c.version.major, 31);
        assert_eq!(c.version.string, "31.0.2");
        assert!(!c.version.extended_support);
    }

    #[test]
    fn at_least() {
        let c = sample();
        assert!(c.version.at_least(31, 0));
        assert!(c.version.at_least(30, 9));
        assert!(!c.version.at_least(31, 1));
        assert!(!c.version.at_least(32, 0));
    }

    #[test]
    fn has_app() {
        let c = sample();
        assert!(c.has_app("files_sharing"));
        assert!(c.has_app("theming"));
        assert!(!c.has_app("spreed"));
    }

    #[test]
    fn core_helpers() {
        let c = sample();
        assert_eq!(c.poll_interval(), Some(60));
        assert_eq!(c.webdav_root(), Some("remote.php/webdav"));
    }

    #[test]
    fn php_bools() {
        let c = sample();
        assert_eq!(c.get_bool("files_sharing", "api_enabled"), Some(true));
        let public = c.get("files_sharing", "public").unwrap();
        assert_eq!(public.get("enabled").unwrap(), "1");
    }

    #[test]
    fn missing_keys() {
        let c = sample();
        assert!(c.get("nope", "nope").is_none());
        assert!(c.get("core", "nope").is_none());
        assert!(c.get_bool("core", "nope").is_none());
    }

    #[test]
    fn empty_doc() {
        let c = Capabilities::default();
        assert!(!c.has_app("core"));
        assert!(c.poll_interval().is_none());
    }
}
