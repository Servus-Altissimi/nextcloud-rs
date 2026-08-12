//! The Files Sharing OCS API.
//!
//! Base path: `/ocs/v2.php/apps/files_sharing/api/v1`.

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Nextcloud;
use crate::error::Result;
use crate::ocs::{Form, bool_str};
use crate::serde_util::{
    flexible_bool, flexible_i64, flexible_string, opt_flexible_i64, opt_flexible_string,
};

const BASE: &str = "ocs/v2.php/apps/files_sharing/api/v1";

/// Who or what a share targets. Apps add their own codes, so unmodelled ones
/// are kept in [`ShareType::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShareType {
    User,
    Group,
    PublicLink,
    Email,
    FederatedCloud,
    Circle,
    TalkConversation,
    Other(i64),
}

impl ShareType {
    pub fn from_code(code: i64) -> Self {
        match code {
            0 => ShareType::User,
            1 => ShareType::Group,
            3 => ShareType::PublicLink,
            4 => ShareType::Email,
            6 => ShareType::FederatedCloud,
            7 => ShareType::Circle,
            10 => ShareType::TalkConversation,
            other => ShareType::Other(other),
        }
    }

    pub fn code(self) -> i64 {
        match self {
            ShareType::User => 0,
            ShareType::Group => 1,
            ShareType::PublicLink => 3,
            ShareType::Email => 4,
            ShareType::FederatedCloud => 6,
            ShareType::Circle => 7,
            ShareType::TalkConversation => 10,
            ShareType::Other(code) => code,
        }
    }

    /// Whether this share type needs a `shareWith` recipient. Public links
    /// are the only kind that does not.
    pub fn needs_recipient(self) -> bool {
        !matches!(self, ShareType::PublicLink)
    }
}

impl<'de> Deserialize<'de> for ShareType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let code = crate::serde_util::flexible_i64(d)?;
        Ok(ShareType::from_code(code))
    }
}

impl Serialize for ShareType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_i64(self.code())
    }
}

/// The share permission bitmask. Values combine: `READ | UPDATE` is `3`.
/// Private shares default to [`ALL`](Self::ALL), public links to
/// [`READ`](Self::READ).
///
/// ```
/// use nextcloud::SharePermissions;
/// let p = SharePermissions::READ | SharePermissions::UPDATE;
/// assert_eq!(p.bits(), 3);
/// assert!(p.can_read() && p.can_update() && !p.can_delete());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct SharePermissions(i64);

impl SharePermissions {
    pub const READ: Self = Self(1);
    pub const UPDATE: Self = Self(2);
    pub const CREATE: Self = Self(4);
    pub const DELETE: Self = Self(8);
    pub const SHARE: Self = Self(16);
    pub const ALL: Self = Self(31);

    pub const fn from_bits(bits: i64) -> Self {
        Self(bits)
    }
    pub const fn bits(self) -> i64 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
    pub const fn can_read(self) -> bool {
        self.contains(Self::READ)
    }
    pub const fn can_update(self) -> bool {
        self.contains(Self::UPDATE)
    }
    pub const fn can_create(self) -> bool {
        self.contains(Self::CREATE)
    }
    pub const fn can_delete(self) -> bool {
        self.contains(Self::DELETE)
    }
    pub const fn can_share(self) -> bool {
        self.contains(Self::SHARE)
    }
}

impl std::ops::BitOr for SharePermissions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for SharePermissions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl<'de> Deserialize<'de> for SharePermissions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(SharePermissions(crate::serde_util::flexible_i64(d)?))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Share {
    /// Share id. A string on the wire even though it is numeric.
    #[serde(deserialize_with = "flexible_string")]
    pub id: String,
    pub share_type: ShareType,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub uid_owner: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub displayname_owner: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub uid_file_owner: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub displayname_file_owner: Option<String>,
    #[serde(default)]
    pub permissions: SharePermissions,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub stime: Option<i64>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub parent: Option<String>,
    /// Expiry, formatted `YYYY-MM-DD HH:MM:SS`, kept as sent.
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub expiration: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub token: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub path: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub item_type: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub mimetype: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub file_source: Option<i64>,
    #[serde(default, deserialize_with = "opt_flexible_i64")]
    pub file_parent: Option<i64>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub file_target: Option<String>,
    /// Recipient identifier: user id, group id, or email address.
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub share_with: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub share_with_displayname: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub note: Option<String>,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "flexible_bool")]
    pub hide_download: bool,
    #[serde(default, deserialize_with = "flexible_bool")]
    pub send_password_by_talk: bool,
    #[serde(default, deserialize_with = "opt_flexible_string")]
    pub password: Option<String>,
    #[serde(default, deserialize_with = "flexible_bool")]
    pub mail_send: bool,
    #[serde(default, deserialize_with = "flexible_i64")]
    pub storage: i64,
}

impl Share {
    pub fn is_public_link(&self) -> bool {
        self.share_type == ShareType::PublicLink
    }

    pub fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self.stime?, 0)
    }
}

/// Parameters for creating a share.
///
/// ```no_run
/// # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
/// use nextcloud::{NewShare, SharePermissions};
///
/// // Share a folder with a user, read plus update.
/// let s = nc.shares().create(
///     NewShare::with_user("/Documents", "bob")
///         .permissions(SharePermissions::READ | SharePermissions::UPDATE),
/// ).await?;
///
/// // Or mint a password-protected public link that expires.
/// let link = nc.shares().create(
///     NewShare::public_link("/Documents/report.pdf")
///         .password("hunter2")
///         .expire_date("2026-12-31"),
/// ).await?;
/// println!("{}", link.url.unwrap_or_default());
/// # let _ = s;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct NewShare {
    path: String,
    share_type: ShareType,
    share_with: Option<String>,
    permissions: Option<SharePermissions>,
    public_upload: Option<bool>,
    password: Option<String>,
    expire_date: Option<String>,
    note: Option<String>,
    label: Option<String>,
    send_password_by_talk: Option<bool>,
    send_mail: Option<bool>,
    attributes: Option<String>,
}

impl NewShare {
    pub fn new(path: impl Into<String>, share_type: ShareType) -> Self {
        Self {
            path: path.into(),
            share_type,
            share_with: None,
            permissions: None,
            public_upload: None,
            password: None,
            expire_date: None,
            note: None,
            label: None,
            send_password_by_talk: None,
            send_mail: None,
            attributes: None,
        }
    }

    pub fn with_user(path: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self::new(path, ShareType::User).share_with(user_id)
    }

    pub fn with_group(path: impl Into<String>, group_id: impl Into<String>) -> Self {
        Self::new(path, ShareType::Group).share_with(group_id)
    }

    pub fn public_link(path: impl Into<String>) -> Self {
        Self::new(path, ShareType::PublicLink)
    }

    pub fn with_email(path: impl Into<String>, email: impl Into<String>) -> Self {
        Self::new(path, ShareType::Email).share_with(email)
    }

    pub fn share_with(mut self, who: impl Into<String>) -> Self {
        self.share_with = Some(who.into());
        self
    }

    pub fn permissions(mut self, p: SharePermissions) -> Self {
        self.permissions = Some(p);
        self
    }

    pub fn public_upload(mut self, yes: bool) -> Self {
        self.public_upload = Some(yes);
        self
    }

    pub fn password(mut self, pw: impl Into<String>) -> Self {
        self.password = Some(pw.into());
        self
    }

    pub fn expire_date(mut self, date: impl Into<String>) -> Self {
        self.expire_date = Some(date.into());
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn send_password_by_talk(mut self, yes: bool) -> Self {
        self.send_password_by_talk = Some(yes);
        self
    }

    pub fn send_mail(mut self, yes: bool) -> Self {
        self.send_mail = Some(yes);
        self
    }

    /// The raw `attributes` JSON: download restrictions, file request settings.
    pub fn attributes(mut self, json: impl Into<String>) -> Self {
        self.attributes = Some(json.into());
        self
    }

    fn to_form(&self) -> Vec<(&'static str, String)> {
        let mut f = Form::new();
        f.set("path", self.path.clone());
        f.set("shareType", self.share_type.code().to_string());
        f.opt("shareWith", self.share_with.as_ref());
        f.opt_display("permissions", self.permissions.map(SharePermissions::bits));
        f.opt_bool("publicUpload", self.public_upload);
        f.opt("password", self.password.as_ref());
        f.opt("expireDate", self.expire_date.as_ref());
        f.opt("note", self.note.as_ref());
        f.opt("label", self.label.as_ref());
        f.opt_bool("sendPasswordByTalk", self.send_password_by_talk);
        f.opt_bool("sendMail", self.send_mail);
        f.opt("attributes", self.attributes.as_ref());
        f.finish()
    }
}

/// Fields to change on an existing share. Unset fields are left alone.
#[derive(Clone, Debug, Default)]
pub struct ShareUpdate {
    permissions: Option<SharePermissions>,
    password: Option<String>,
    public_upload: Option<bool>,
    expire_date: Option<String>,
    note: Option<String>,
    label: Option<String>,
    hide_download: Option<bool>,
    send_password_by_talk: Option<bool>,
    attributes: Option<String>,
}

impl ShareUpdate {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn permissions(mut self, p: SharePermissions) -> Self {
        self.permissions = Some(p);
        self
    }
    pub fn password(mut self, pw: impl Into<String>) -> Self {
        self.password = Some(pw.into());
        self
    }
    pub fn clear_password(mut self) -> Self {
        self.password = Some(String::new());
        self
    }
    pub fn public_upload(mut self, yes: bool) -> Self {
        self.public_upload = Some(yes);
        self
    }
    pub fn expire_date(mut self, date: impl Into<String>) -> Self {
        self.expire_date = Some(date.into());
        self
    }
    pub fn clear_expire_date(mut self) -> Self {
        self.expire_date = Some(String::new());
        self
    }
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn hide_download(mut self, yes: bool) -> Self {
        self.hide_download = Some(yes);
        self
    }
    pub fn send_password_by_talk(mut self, yes: bool) -> Self {
        self.send_password_by_talk = Some(yes);
        self
    }
    pub fn attributes(mut self, json: impl Into<String>) -> Self {
        self.attributes = Some(json.into());
        self
    }

    fn to_form(&self) -> Vec<(&'static str, String)> {
        let mut f = Form::new();
        f.opt_display("permissions", self.permissions.map(SharePermissions::bits));
        f.opt("password", self.password.as_ref());
        f.opt_bool("publicUpload", self.public_upload);
        f.opt("expireDate", self.expire_date.as_ref());
        f.opt("note", self.note.as_ref());
        f.opt("label", self.label.as_ref());
        f.opt_bool("hideDownload", self.hide_download);
        f.opt_bool("sendPasswordByTalk", self.send_password_by_talk);
        f.opt("attributes", self.attributes.as_ref());
        f.finish()
    }

    pub fn is_empty(&self) -> bool {
        self.to_form().is_empty()
    }
}

pub struct Shares<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn shares(&self) -> Shares<'_> {
        Shares { nc: self }
    }
}

impl Shares<'_> {
    pub async fn list(&self) -> Result<Vec<Share>> {
        self.nc
            .ocs_typed(Method::GET, &format!("{BASE}/shares"), &[], &[])
            .await
    }

    /// List shares on one path. `reshares` includes other users' shares of the
    /// same item; `subfiles` lists a folder's children instead of the folder.
    pub async fn list_for_path(
        &self,
        path: &str,
        reshares: bool,
        subfiles: bool,
    ) -> Result<Vec<Share>> {
        let query = [
            ("path", path.to_string()),
            ("reshares", bool_str(reshares)),
            ("subfiles", bool_str(subfiles)),
        ];
        self.nc
            .ocs_typed(Method::GET, &format!("{BASE}/shares"), &query, &[])
            .await
    }

    pub async fn list_shared_with_me(&self) -> Result<Vec<Share>> {
        let query = [("shared_with_me", "true".to_string())];
        self.nc
            .ocs_typed(Method::GET, &format!("{BASE}/shares"), &query, &[])
            .await
    }

    /// Fetch a single share. The endpoint answers with a one-element list.
    ///
    /// The raw payload is taken rather than the typed one, whose empty-array
    /// normalisation would turn an empty list into a decode failure.
    pub async fn get(&self, share_id: &str) -> Result<Share> {
        let data = self
            .nc
            .ocs_call(Method::GET, &format!("{BASE}/shares/{share_id}"), &[], &[])
            .await?;
        let list: Vec<Share> = serde_json::from_value(data)?;
        list.into_iter().next().ok_or_else(|| {
            crate::Error::UnexpectedResponse(format!("share {share_id} returned no entries"))
        })
    }

    pub async fn create(&self, share: NewShare) -> Result<Share> {
        let form = share.to_form();
        self.nc
            .ocs_typed(Method::POST, &format!("{BASE}/shares"), &[], &form)
            .await
    }

    /// Update a share. The server applies one field at a time internally, so a
    /// multi-field update can apply partially if one field is rejected.
    pub async fn update(&self, share_id: &str, update: ShareUpdate) -> Result<Share> {
        let form = update.to_form();
        self.nc
            .ocs_typed(
                Method::PUT,
                &format!("{BASE}/shares/{share_id}"),
                &[],
                &form,
            )
            .await
    }

    /// Delete a share. For a share received from someone else this declines it.
    pub async fn delete(&self, share_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::DELETE,
                &format!("{BASE}/shares/{share_id}"),
                &[],
                &[],
            )
            .await
    }

    pub async fn send_email(&self, share_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::POST,
                &format!("{BASE}/shares/{share_id}/send-email"),
                &[],
                &[],
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_type_codes() {
        for code in [0, 1, 3, 4, 6, 7, 10] {
            assert_eq!(ShareType::from_code(code).code(), code);
        }
    }

    #[test]
    fn unknown_share_type() {
        let t = ShareType::from_code(99);
        assert_eq!(t, ShareType::Other(99));
        assert_eq!(t.code(), 99);
    }

    #[test]
    fn needs_recipient() {
        assert!(!ShareType::PublicLink.needs_recipient());
        assert!(ShareType::User.needs_recipient());
        assert!(ShareType::Email.needs_recipient());
    }

    #[test]
    fn permission_bits() {
        let p = SharePermissions::READ | SharePermissions::UPDATE | SharePermissions::SHARE;
        assert_eq!(p.bits(), 19);
        assert!(p.can_read() && p.can_update() && p.can_share());
        assert!(!p.can_create() && !p.can_delete());
        assert!(SharePermissions::ALL.contains(p));
        assert_eq!(SharePermissions::ALL.bits(), 31);
    }

    #[test]
    fn create_form() {
        let form = NewShare::public_link("/a.txt")
            .password("pw")
            .expire_date("2026-01-01")
            .permissions(SharePermissions::READ)
            .public_upload(false)
            .to_form();

        let get = |k: &str| form.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("path"), Some("/a.txt"));
        assert_eq!(get("shareType"), Some("3"));
        assert_eq!(get("password"), Some("pw"));
        assert_eq!(get("expireDate"), Some("2026-01-01"));
        assert_eq!(get("permissions"), Some("1"));
        assert_eq!(get("publicUpload"), Some("false"));
        assert_eq!(get("shareWith"), None);
    }

    #[test]
    fn create_form_user_share() {
        let form = NewShare::with_user("/a", "bob").to_form();
        assert!(form.contains(&("shareWith", "bob".to_string())));
        assert!(form.contains(&("shareType", "0".to_string())));
    }

    #[test]
    fn update_form() {
        assert!(ShareUpdate::new().is_empty());
        let form = ShareUpdate::new()
            .permissions(SharePermissions::READ)
            .to_form();
        assert_eq!(form.len(), 1);
        assert_eq!(form[0], ("permissions", "1".to_string()));
    }

    #[test]
    fn update_form_clearing() {
        let form = ShareUpdate::new()
            .clear_password()
            .clear_expire_date()
            .to_form();
        assert!(form.contains(&("password", String::new())));
        assert!(form.contains(&("expireDate", String::new())));
    }

    #[test]
    fn share_deserialise() {
        // ids and permissions arrive as strings, bools as 0/1
        let json = serde_json::json!({
            "id": "17",
            "share_type": "3",
            "uid_owner": "alice",
            "permissions": "1",
            "stime": 1700000000,
            "token": "abc",
            "url": "https://cloud.example.com/s/abc",
            "hide_download": 0,
            "mail_send": 1,
            "note": "",
            "storage": "2"
        });
        let s: Share = serde_json::from_value(json).unwrap();
        assert_eq!(s.id, "17");
        assert_eq!(s.share_type, ShareType::PublicLink);
        assert!(s.is_public_link());
        assert_eq!(s.permissions, SharePermissions::READ);
        assert!(!s.hide_download);
        assert!(s.mail_send);
        assert_eq!(s.note, None);
        assert!(s.created_at().is_some());
    }
}
