//! The User Status app API.
//!
//! Base paths: `.../user_status` for the authenticated account, `.../statuses`
//! for reading other accounts.
//!
//! Needs the `user_status` app, which
//! [`Capabilities::has_app`](crate::Capabilities::has_app) reports.

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::ocs::{Form, paged_query};
use crate::serde_util::{opt_flexible_i64, opt_flexible_string};

const OWN: &str = "ocs/v2.php/apps/user_status/api/v1/user_status";
const OTHERS: &str = "ocs/v2.php/apps/user_status/api/v1/statuses";

/// Availability. A payload omitting the field reads as
/// [`Offline`](Self::Offline).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StatusType {
    Online,
    Away,
    /// Do not disturb; suppresses notifications.
    DoNotDisturb,
    /// Appear offline while still connected.
    Invisible,
    #[default]
    Offline,
    /// A value this crate does not model.
    Other(String),
}

impl StatusType {
    pub fn as_str(&self) -> &str {
        match self {
            StatusType::Online => "online",
            StatusType::Away => "away",
            StatusType::DoNotDisturb => "dnd",
            StatusType::Invisible => "invisible",
            StatusType::Offline => "offline",
            StatusType::Other(s) => s,
        }
    }
}

impl From<&str> for StatusType {
    fn from(s: &str) -> Self {
        match s {
            "online" => StatusType::Online,
            "away" => StatusType::Away,
            "dnd" => StatusType::DoNotDisturb,
            "invisible" => StatusType::Invisible,
            "offline" => StatusType::Offline,
            other => StatusType::Other(other.to_string()),
        }
    }
}

impl std::str::FromStr for StatusType {
    type Err = std::convert::Infallible;

    /// Never fails: an unrecognised value becomes [`StatusType::Other`].
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(StatusType::from(s))
    }
}

impl<'de> Deserialize<'de> for StatusType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = Option::<String>::deserialize(d)?;
        Ok(StatusType::from(s.as_deref().unwrap_or("offline")))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct UserStatus {
    #[serde(rename = "userId", default, deserialize_with = "opt_flexible_string")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub status: StatusType,
    /// Whether the status was set automatically rather than by the user.
    #[serde(rename = "statusIsUserDefined", default)]
    pub status_is_user_defined: Option<bool>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub message: Option<String>,
    /// Identifier of the predefined message, when one is in use.
    #[serde(
        rename = "messageId",
        default,
        deserialize_with = "opt_flexible_string"
    )]
    pub message_id: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub icon: Option<String>,
    /// Unix timestamp at which the message auto-clears.
    #[serde(rename = "clearAt", default, deserialize_with = "opt_flexible_i64")]
    pub clear_at: Option<i64>,
}

impl UserStatus {
    pub fn clear_at_utc(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(self.clear_at?, 0)
    }
}

pub struct UserStatuses<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn user_status(&self) -> UserStatuses<'_> {
        UserStatuses { nc: self }
    }
}

impl UserStatuses<'_> {
    pub async fn get_own(&self) -> Result<UserStatus> {
        self.nc.ocs_typed(Method::GET, OWN, &[], &[]).await
    }

    pub async fn set_status(&self, status: StatusType) -> Result<UserStatus> {
        let form = [("statusType", status.as_str().to_string())];
        self.nc
            .ocs_typed(Method::PUT, &format!("{OWN}/status"), &[], &form)
            .await
    }

    /// Set a predefined message such as `meeting`. `clear_at` is the Unix
    /// timestamp at which it disappears.
    pub async fn set_predefined_message(
        &self,
        message_id: &str,
        clear_at: Option<i64>,
    ) -> Result<UserStatus> {
        let mut form = Form::new();
        form.set("messageId", message_id);
        form.opt_display("clearAt", clear_at);
        let form = form.finish();
        self.nc
            .ocs_typed(
                Method::PUT,
                &format!("{OWN}/message/predefined"),
                &[],
                &form,
            )
            .await
    }

    pub async fn set_custom_message(
        &self,
        message: Option<&str>,
        icon: Option<&str>,
        clear_at: Option<i64>,
    ) -> Result<UserStatus> {
        let mut form = Form::new();
        form.opt_display("message", message);
        form.opt_display("statusIcon", icon);
        form.opt_display("clearAt", clear_at);
        let form = form.finish();
        self.nc
            .ocs_typed(Method::PUT, &format!("{OWN}/message/custom"), &[], &form)
            .await
    }

    pub async fn clear_message(&self) -> Result<()> {
        self.nc
            .ocs_unit(Method::DELETE, &format!("{OWN}/message"), &[], &[])
            .await
    }

    pub async fn list(&self, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<UserStatus>> {
        let query = paged_query(None, limit, offset);
        self.nc.ocs_typed(Method::GET, OTHERS, &query, &[]).await
    }

    pub async fn get(&self, user_id: &str) -> Result<UserStatus> {
        self.nc
            .ocs_typed(Method::GET, &format!("{OTHERS}/{user_id}"), &[], &[])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_type_codes() {
        for t in [
            StatusType::Online,
            StatusType::Away,
            StatusType::DoNotDisturb,
            StatusType::Invisible,
            StatusType::Offline,
        ] {
            assert_eq!(StatusType::from(t.as_str()), t);
        }
        assert_eq!(StatusType::DoNotDisturb.as_str(), "dnd");
    }

    #[test]
    fn unknown_status_type() {
        let t = StatusType::from("busy-in-a-new-release");
        assert_eq!(t, StatusType::Other("busy-in-a-new-release".into()));
        assert_eq!(t.as_str(), "busy-in-a-new-release");
    }

    #[test]
    fn deserialise() {
        let s: UserStatus = serde_json::from_value(serde_json::json!({
            "userId": "alice",
            "message": "In a meeting",
            "messageId": "meeting",
            "icon": "📅",
            "clearAt": 1700000000,
            "status": "dnd",
            "statusIsUserDefined": true
        }))
        .unwrap();

        assert_eq!(s.user_id.as_deref(), Some("alice"));
        assert_eq!(s.status, StatusType::DoNotDisturb);
        assert_eq!(s.icon.as_deref(), Some("📅"));
        assert_eq!(s.clear_at_utc().unwrap().timestamp(), 1700000000);
        assert_eq!(s.status_is_user_defined, Some(true));
    }

    #[test]
    fn no_message() {
        let s: UserStatus = serde_json::from_value(serde_json::json!({
            "userId": "bob", "status": "online", "message": null, "clearAt": null
        }))
        .unwrap();
        assert_eq!(s.status, StatusType::Online);
        assert!(s.message.is_none());
        assert!(s.clear_at_utc().is_none());
    }

    #[test]
    fn missing_status_defaults_offline() {
        let s: UserStatus = serde_json::from_value(serde_json::json!({"userId": "x"})).unwrap();
        assert_eq!(s.status, StatusType::Offline);
    }
}
