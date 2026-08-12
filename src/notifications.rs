//! The Notifications app API (v2).
//!
//! Base path: `/ocs/v2.php/apps/notifications/api/v2`.
//!
//! Needs the Notifications app, which
//! [`Capabilities::has_app`](crate::Capabilities::has_app) reports.
//! [`Capabilities::poll_interval`](crate::Capabilities::poll_interval) is the
//! server's suggested polling floor.

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::serde_util::{flexible_bool, flexible_i64, flexible_string, opt_flexible_string};

const BASE: &str = "ocs/v2.php/apps/notifications/api/v2";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct NotificationAction {
    #[serde(default, deserialize_with = "flexible_string")]
    pub label: String,
    #[serde(default, deserialize_with = "flexible_string")]
    pub link: String,
    /// `GET`, `POST`, `DELETE`, `PUT`, or `WEB`, which means open in a browser.
    #[serde(rename = "type", default, deserialize_with = "flexible_string")]
    pub action_type: String,
    #[serde(default, deserialize_with = "flexible_bool")]
    pub primary: bool,
}

impl NotificationAction {
    pub fn is_web_link(&self) -> bool {
        self.action_type.eq_ignore_ascii_case("WEB")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Notification {
    #[serde(default, deserialize_with = "flexible_i64")]
    pub notification_id: i64,
    #[serde(default, deserialize_with = "flexible_string")]
    pub app: String,
    #[serde(default, deserialize_with = "flexible_string")]
    pub user: String,
    /// ISO 8601 publication time, as sent.
    #[serde(default, deserialize_with = "flexible_string")]
    pub datetime: String,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub object_type: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub object_id: Option<String>,
    #[serde(default, deserialize_with = "flexible_string")]
    pub subject: String,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub message: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub link: Option<String>,
    /// Subject with `{placeholder}` markers, paired with `subjectRichParameters`.
    #[serde(
        default,
        rename = "subjectRich",
        deserialize_with = "opt_flexible_string"
    )]
    pub subject_rich: Option<String>,
    #[serde(
        default,
        rename = "messageRich",
        deserialize_with = "opt_flexible_string"
    )]
    pub message_rich: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub icon: Option<String>,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
}

impl Notification {
    /// Publication time parsed from the ISO 8601 `datetime` field.
    pub fn published_at(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.datetime)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    }

    pub fn primary_action(&self) -> Option<&NotificationAction> {
        self.actions.iter().find(|a| a.primary)
    }
}

pub struct Notifications<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications { nc: self }
    }
}

impl Notifications<'_> {
    pub async fn list(&self) -> Result<Vec<Notification>> {
        self.nc
            .ocs_typed(Method::GET, &format!("{BASE}/notifications"), &[], &[])
            .await
    }

    pub async fn get(&self, notification_id: i64) -> Result<Notification> {
        self.nc
            .ocs_typed(
                Method::GET,
                &format!("{BASE}/notifications/{notification_id}"),
                &[],
                &[],
            )
            .await
    }

    pub async fn dismiss(&self, notification_id: i64) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::DELETE,
                &format!("{BASE}/notifications/{notification_id}"),
                &[],
                &[],
            )
            .await
    }

    pub async fn dismiss_all(&self) -> Result<()> {
        self.nc
            .ocs_unit(Method::DELETE, &format!("{BASE}/notifications"), &[], &[])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Notification {
        serde_json::from_value(serde_json::json!({
            "notification_id": 42,
            "app": "files_sharing",
            "user": "alice",
            "datetime": "2026-08-11T10:30:00+00:00",
            "object_type": "remote_share",
            "object_id": "17",
            "subject": "Bob shared a folder with you",
            "message": "",
            "link": "https://cloud.example.com/apps/files",
            "subjectRich": "{user} shared {file} with you",
            "actions": [
                {"label": "Accept", "link": "https://x/accept", "type": "POST", "primary": true},
                {"label": "Decline", "link": "https://x/decline", "type": "DELETE", "primary": false}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn fields() {
        let n = sample();
        assert_eq!(n.notification_id, 42);
        assert_eq!(n.app, "files_sharing");
        assert_eq!(n.subject, "Bob shared a folder with you");
        assert_eq!(n.object_id.as_deref(), Some("17"));
        assert_eq!(
            n.subject_rich.as_deref(),
            Some("{user} shared {file} with you")
        );
    }

    #[test]
    fn empty_message() {
        assert_eq!(sample().message, None);
    }

    #[test]
    fn datetime() {
        let n = sample();
        // 2026-08-11T10:30:00Z
        assert_eq!(n.published_at().unwrap().timestamp(), 1786444200);
    }

    #[test]
    fn bad_datetime() {
        let n = Notification {
            datetime: "not a date".into(),
            ..sample()
        };
        assert!(n.published_at().is_none());
    }

    #[test]
    fn primary_action() {
        let n = sample();
        assert_eq!(n.primary_action().unwrap().label, "Accept");
        assert_eq!(n.actions.len(), 2);
    }

    #[test]
    fn web_action() {
        let n = sample();
        assert!(!n.actions[0].is_web_link());

        let web: NotificationAction = serde_json::from_value(serde_json::json!({
            "label": "Open", "link": "https://x", "type": "WEB", "primary": 1
        }))
        .unwrap();
        assert!(web.is_web_link());
        assert!(web.primary);
    }

    #[test]
    fn no_actions() {
        let n: Notification = serde_json::from_value(serde_json::json!({
            "notification_id": "7", "app": "core", "subject": "hi"
        }))
        .unwrap();
        assert_eq!(n.notification_id, 7);
        assert!(n.actions.is_empty());
        assert!(n.primary_action().is_none());
    }
}
