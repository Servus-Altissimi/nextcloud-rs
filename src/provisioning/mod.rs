//! The Provisioning API: users, groups, and apps.
//!
//! Base path: `/ocs/v1.php/cloud`. Almost every endpoint needs admin rights;
//! subadmins may manage only their own groups. POST bodies are
//! `application/x-www-form-urlencoded`.

mod apps;
mod groups;

use std::collections::HashMap;

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::ocs::{Form, paged_query};
use crate::serde_util::{flexible_bool, opt_flexible_i64, opt_flexible_string};

pub use apps::{AppFilter, AppInfo, Apps};
pub use groups::Groups;

pub(crate) const BASE: &str = "ocs/v1.php/cloud";

/// Storage quota figures, in bytes. `quota` carries sentinels: `-3` unlimited,
/// `-1` and `-2` unknown or unset, depending on the backend.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Quota {
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub free: Option<i64>,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub used: Option<i64>,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub total: Option<i64>,
    #[serde(default)]
    pub relative: Option<f64>,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub quota: Option<i64>,
}

impl Quota {
    /// Whether the account has an unlimited quota (`-3`).
    pub fn is_unlimited(&self) -> bool {
        self.quota == Some(-3)
    }
}

/// A user account. Profile fields grow with each release and apps add their
/// own, so unmodelled keys land in [`extra`](Self::extra).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct User {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_true", deserialize_with = "flexible_bool")]
    pub enabled: bool,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub displayname: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub email: Option<String>,
    #[serde(default)]
    pub quota: Quota,
    /// Last sign-in, as a Unix timestamp in milliseconds.
    #[serde(default, rename = "lastLogin", deserialize_with = "opt_flexible_i64")]
    pub last_login: Option<i64>,
    /// The user backend, e.g. `Database` or `LDAP`.
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub backend: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub subadmin: Vec<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub phone: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub address: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub website: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub twitter: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub organisation: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub role: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub language: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub locale: Option<String>,
    /// Any field this crate does not model.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

fn default_true() -> bool {
    true
}

impl User {
    /// Last sign-in as a UTC datetime. `0` on the wire means never.
    pub fn last_login_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let ms = self.last_login.filter(|v| *v > 0)?;
        chrono::DateTime::from_timestamp_millis(ms)
    }

    /// The best display name available, falling back to the account id.
    pub fn display_name_or_id(&self) -> &str {
        self.displayname.as_deref().unwrap_or(&self.id)
    }
}

/// A field [`Users::edit`] can change. [`UserField::Other`] passes through any
/// key the server accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserField {
    Email,
    /// Storage quota, e.g. `"5 GB"` or `"none"`.
    Quota,
    DisplayName,
    Phone,
    Address,
    Website,
    Twitter,
    Password,
    Other(String),
}

impl UserField {
    pub fn as_str(&self) -> &str {
        match self {
            UserField::Email => "email",
            UserField::Quota => "quota",
            UserField::DisplayName => "displayname",
            UserField::Phone => "phone",
            UserField::Address => "address",
            UserField::Website => "website",
            UserField::Twitter => "twitter",
            UserField::Password => "password",
            UserField::Other(s) => s,
        }
    }
}

/// Parameters for creating an account. A password or an email address is
/// required: without a password the server mails a set-up link.
#[derive(Clone, Debug)]
pub struct NewUser {
    user_id: String,
    password: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    groups: Vec<String>,
    subadmin: Vec<String>,
    quota: Option<String>,
    language: Option<String>,
}

impl NewUser {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            password: None,
            display_name: None,
            email: None,
            groups: Vec::new(),
            subadmin: Vec::new(),
            quota: None,
            language: None,
        }
    }

    /// Set an initial password. Omitted, the server emails an invitation.
    pub fn password(mut self, pw: impl Into<String>) -> Self {
        self.password = Some(pw.into());
        self
    }
    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.groups.push(group.into());
        self
    }
    pub fn subadmin_of(mut self, group: impl Into<String>) -> Self {
        self.subadmin.push(group.into());
        self
    }
    pub fn quota(mut self, quota: impl Into<String>) -> Self {
        self.quota = Some(quota.into());
        self
    }
    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }

    fn to_form(&self) -> Vec<(&'static str, String)> {
        let mut f = Form::new();
        f.set("userid", self.user_id.clone());
        f.opt("password", self.password.as_ref());
        f.opt("displayName", self.display_name.as_ref());
        f.opt("email", self.email.as_ref());
        f.opt("quota", self.quota.as_ref());
        f.opt("language", self.language.as_ref());
        // repeated groups[] keys are how PHP receives an array
        for g in &self.groups {
            f.set("groups[]", g.clone());
        }
        for g in &self.subadmin {
            f.set("subadmin[]", g.clone());
        }
        f.finish()
    }
}

/// The `{"users": [...]}` wrapper every account listing comes back in.
#[derive(Deserialize)]
pub(crate) struct UserIdList {
    #[serde(default)]
    pub(crate) users: Vec<String>,
}

/// The `{"groups": [...]}` wrapper every group listing comes back in.
#[derive(Deserialize)]
pub(crate) struct GroupIdList {
    #[serde(default)]
    pub(crate) groups: Vec<String>,
}

pub struct Users<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn users(&self) -> Users<'_> {
        Users { nc: self }
    }
}

impl Users<'_> {
    pub async fn list(
        &self,
        search: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<String>> {
        let query = paged_query(search, limit, offset);
        let list: UserIdList = self
            .nc
            .ocs_typed(Method::GET, &format!("{BASE}/users"), &query, &[])
            .await?;
        Ok(list.users)
    }

    pub async fn get(&self, user_id: &str) -> Result<User> {
        let mut user: User = self
            .nc
            .ocs_typed(Method::GET, &format!("{BASE}/users/{user_id}"), &[], &[])
            .await?;
        // some versions omit id
        if user.id.is_empty() {
            user.id = user_id.to_string();
        }
        Ok(user)
    }

    pub async fn create(&self, user: NewUser) -> Result<()> {
        let form = user.to_form();
        self.nc
            .ocs_unit(Method::POST, &format!("{BASE}/users"), &[], &form)
            .await
    }

    /// Change one field. The endpoint takes one key/value pair per request.
    pub async fn edit(&self, user_id: &str, field: UserField, value: &str) -> Result<()> {
        let form = [
            ("key", field.as_str().to_string()),
            ("value", value.to_string()),
        ];
        self.nc
            .ocs_unit(Method::PUT, &format!("{BASE}/users/{user_id}"), &[], &form)
            .await
    }

    pub async fn disable(&self, user_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::PUT,
                &format!("{BASE}/users/{user_id}/disable"),
                &[],
                &[],
            )
            .await
    }

    pub async fn enable(&self, user_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::PUT,
                &format!("{BASE}/users/{user_id}/enable"),
                &[],
                &[],
            )
            .await
    }

    /// Delete an account and its data. Irreversible.
    pub async fn delete(&self, user_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(Method::DELETE, &format!("{BASE}/users/{user_id}"), &[], &[])
            .await
    }

    pub async fn groups(&self, user_id: &str) -> Result<Vec<String>> {
        let list: GroupIdList = self
            .nc
            .ocs_typed(
                Method::GET,
                &format!("{BASE}/users/{user_id}/groups"),
                &[],
                &[],
            )
            .await?;
        Ok(list.groups)
    }

    pub async fn add_to_group(&self, user_id: &str, group_id: &str) -> Result<()> {
        let form = [("groupid", group_id.to_string())];
        self.nc
            .ocs_unit(
                Method::POST,
                &format!("{BASE}/users/{user_id}/groups"),
                &[],
                &form,
            )
            .await
    }

    pub async fn remove_from_group(&self, user_id: &str, group_id: &str) -> Result<()> {
        let form = [("groupid", group_id.to_string())];
        self.nc
            .ocs_unit(
                Method::DELETE,
                &format!("{BASE}/users/{user_id}/groups"),
                &[],
                &form,
            )
            .await
    }

    pub async fn subadmin_groups(&self, user_id: &str) -> Result<Vec<String>> {
        self.nc
            .ocs_typed(
                Method::GET,
                &format!("{BASE}/users/{user_id}/subadmins"),
                &[],
                &[],
            )
            .await
    }

    pub async fn promote_subadmin(&self, user_id: &str, group_id: &str) -> Result<()> {
        let form = [("groupid", group_id.to_string())];
        self.nc
            .ocs_unit(
                Method::POST,
                &format!("{BASE}/users/{user_id}/subadmins"),
                &[],
                &form,
            )
            .await
    }

    pub async fn demote_subadmin(&self, user_id: &str, group_id: &str) -> Result<()> {
        let form = [("groupid", group_id.to_string())];
        self.nc
            .ocs_unit(
                Method::DELETE,
                &format!("{BASE}/users/{user_id}/subadmins"),
                &[],
                &form,
            )
            .await
    }

    pub async fn resend_welcome_email(&self, user_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::POST,
                &format!("{BASE}/users/{user_id}/welcome"),
                &[],
                &[],
            )
            .await
    }

    /// List the profile fields the current user may edit. The singular `user`
    /// in the path is not a typo.
    pub async fn editable_fields(&self) -> Result<Vec<String>> {
        self.nc
            .ocs_typed(Method::GET, "ocs/v1.php/cloud/user/fields", &[], &[])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_field_names() {
        assert_eq!(UserField::DisplayName.as_str(), "displayname");
        assert_eq!(UserField::Email.as_str(), "email");
        assert_eq!(UserField::Quota.as_str(), "quota");
        assert_eq!(UserField::Password.as_str(), "password");
        assert_eq!(UserField::Other("locale".into()).as_str(), "locale");
    }

    #[test]
    fn array_form_keys() {
        let form = NewUser::new("bob")
            .group("staff")
            .group("editors")
            .subadmin_of("staff")
            .to_form();

        let groups: Vec<_> = form
            .iter()
            .filter(|(k, _)| *k == "groups[]")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(groups, vec!["staff", "editors"]);
        assert!(form.contains(&("subadmin[]", "staff".to_string())));
    }

    #[test]
    fn create_form() {
        let form = NewUser::new("bob")
            .display_name("Bob B")
            .email("bob@example.com")
            .password("pw")
            .quota("5 GB")
            .to_form();

        assert!(form.contains(&("userid", "bob".to_string())));
        assert!(form.contains(&("displayName", "Bob B".to_string())));
        assert!(form.contains(&("email", "bob@example.com".to_string())));
        assert!(form.contains(&("quota", "5 GB".to_string())));
    }

    #[test]
    fn create_form_omits_none() {
        let form = NewUser::new("bob").to_form();
        assert_eq!(form, vec![("userid", "bob".to_string())]);
    }

    #[test]
    fn user_deserialise() {
        let json = serde_json::json!({
            "id": "alice",
            "enabled": true,
            "displayname": "Alice",
            "email": "alice@example.com",
            "lastLogin": 1700000000000i64,
            "backend": "Database",
            "groups": ["staff"],
            "quota": { "free": 100, "used": 50, "total": 150, "relative": 33.3, "quota": -3 },
            "profile_enabled": "1"
        });
        let u: User = serde_json::from_value(json).unwrap();
        assert_eq!(u.id, "alice");
        assert!(u.enabled);
        assert_eq!(u.groups, vec!["staff"]);
        assert!(u.quota.is_unlimited());
        assert_eq!(u.quota.used, Some(50));
        assert_eq!(u.last_login_utc().unwrap().timestamp(), 1700000000);
        assert!(u.extra.contains_key("profile_enabled"));
    }

    #[test]
    fn last_login_zero() {
        let u = User {
            last_login: Some(0),
            ..Default::default()
        };
        assert!(u.last_login_utc().is_none());
    }

    #[test]
    fn display_name_fallback() {
        let u = User {
            id: "alice".into(),
            ..Default::default()
        };
        assert_eq!(u.display_name_or_id(), "alice");
    }
}
