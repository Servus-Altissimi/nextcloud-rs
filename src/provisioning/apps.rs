//! App management, part of the Provisioning API.

use std::collections::HashMap;

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::provisioning::BASE;
use crate::serde_util::opt_flexible_string;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppFilter {
    Enabled,
    Disabled,
}

impl AppFilter {
    fn as_str(self) -> &'static str {
        match self {
            AppFilter::Enabled => "enabled",
            AppFilter::Disabled => "disabled",
        }
    }
}

/// Metadata from an app's `info.xml`.
///
/// The shape varies between apps and server versions, so unmodelled keys are
/// kept in [`extra`](Self::extra).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AppInfo {
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub summary: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub licence: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub website: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub bugs: Option<String>,
    /// Anything not modelled above, including `author`, `category` and
    /// `dependencies`, whose shapes differ per app.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Deserialize)]
struct AppIdList {
    #[serde(default)]
    apps: Vec<String>,
}

pub struct Apps<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn apps(&self) -> Apps<'_> {
        Apps { nc: self }
    }
}

impl Apps<'_> {
    /// List app ids. `filter` narrows the result to enabled or disabled apps;
    /// passing `None` lists everything installed.
    pub async fn list(&self, filter: Option<AppFilter>) -> Result<Vec<String>> {
        let query: Vec<(&str, String)> = filter
            .map(|f| vec![("filter", f.as_str().to_string())])
            .unwrap_or_default();

        let list: AppIdList = self
            .nc
            .ocs_typed(Method::GET, &format!("{BASE}/apps"), &query, &[])
            .await?;
        Ok(list.apps)
    }

    pub async fn info(&self, app_id: &str) -> Result<AppInfo> {
        self.nc
            .ocs_typed(Method::GET, &format!("{BASE}/apps/{app_id}"), &[], &[])
            .await
    }

    pub async fn enable(&self, app_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(Method::POST, &format!("{BASE}/apps/{app_id}"), &[], &[])
            .await
    }

    pub async fn disable(&self, app_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(Method::DELETE, &format!("{BASE}/apps/{app_id}"), &[], &[])
            .await
    }

    pub async fn is_enabled(&self, app_id: &str) -> Result<bool> {
        Ok(self
            .list(Some(AppFilter::Enabled))
            .await?
            .iter()
            .any(|a| a == app_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_strings() {
        assert_eq!(AppFilter::Enabled.as_str(), "enabled");
        assert_eq!(AppFilter::Disabled.as_str(), "disabled");
    }

    #[test]
    fn app_list() {
        let list: AppIdList =
            serde_json::from_value(serde_json::json!({"apps": ["files", "music"]})).unwrap();
        assert_eq!(list.apps, vec!["files", "music"]);
    }

    #[test]
    fn app_info_extra_keys() {
        let info: AppInfo = serde_json::from_value(serde_json::json!({
            "id": "music",
            "name": "Music",
            "version": "1.0.0",
            "author": ["Someone"],
            "category": "multimedia"
        }))
        .unwrap();
        assert_eq!(info.id.as_deref(), Some("music"));
        assert_eq!(info.version.as_deref(), Some("1.0.0"));
        assert!(info.extra.contains_key("author"));
        assert!(info.extra.contains_key("category"));
    }
}
