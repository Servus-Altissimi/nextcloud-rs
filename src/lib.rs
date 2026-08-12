//! __ _  ____  _  _  ____  ___  __     __   _  _  ____      ____  ____
//!(  ( \(  __)( \/ )(_  _)/ __)(  )   /  \ / )( \(    \ ___(  _ \/ ___)
//!/    / ) _)  )  (   )( ( (__ / (_/\(  O )) \/ ( ) D ((___))   /\___ \
//!\_)__)(____)(_/\_) (__) \___)\____/ \__/ \____/(____/    (__\_)(____/

//! An async Rust client for the Nextcloud server APIs.
//!
//! [`files`] is WebDAV, [`preview`] plain `index.php` routes, [`auth`] JSON
//! over HTTP, and the rest OCS.
//!
//! # Getting started
//!
//! ```no_run
//! # async fn demo() -> Result<(), nextcloud::Error> {
//! use nextcloud::Nextcloud;
//!
//! let nc = Nextcloud::builder("http://127.0.0.1:8080")
//!     .basic_auth("alice", "app-password")
//!     .build()?;
//!
//! for entry in nc.files().list("/").await? {
//!     println!("{}", entry.name());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Authentication
//!
//! App passwords are revocable, scoped to one client, and unaffected by 2FA.
//! [`LoginFlowV2`] mints one through the browser:
//!
//! ```no_run
//! # async fn demo() -> Result<(), nextcloud::Error> {
//! let flow = nextcloud::LoginFlowV2::start("http://127.0.0.1:8080").await?;
//! println!("Authorise at: {}", flow.login_url());
//!
//! let creds = flow.wait_for_completion().await?;
//! let nc = nextcloud::Nextcloud::from_login(creds)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Error handling
//!
//! OCS reports failures in the body: a v1 endpoint answers `200 OK` with
//! `ocs.meta.statuscode: 998` for a missing resource. WebDAV uses the status
//! line. [`Error::is_not_found`] and [`Error::is_auth_error`] span both.
//!
//! Behaviour varies with server version and installed apps, which
//! [`Nextcloud::capabilities`] reports. [`ocs_raw`](Nextcloud::ocs_raw) reaches
//! endpoints this crate does not model.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod auth;
pub mod capabilities;
mod client;
pub mod error;
pub mod files;
pub mod notifications;
pub mod ocs;
pub mod preview;
pub mod provisioning;
mod serde_util;
pub mod sharing;
pub mod user_status;

pub use auth::{Credentials, LoginCredentials, LoginFlowV2};
pub use capabilities::{Capabilities, ServerVersion};
pub use client::{Nextcloud, NextcloudBuilder};
pub use error::{Error, Result};
pub use files::{
    ArchiveFormat, ChunkedUpload, DavPermissions, Depth, FileEntry, FileVersion, Files, MediaKind,
    RangeRead, TrashbinEntry, UploadResult, path,
};
pub use notifications::{Notification, NotificationAction, Notifications};
pub use ocs::{OcsMeta, OcsResponse};
pub use preview::{Preview, PreviewMode, PreviewOptions, Previews};
pub use provisioning::{AppFilter, AppInfo, Apps, Groups, NewUser, Quota, User, UserField, Users};
pub use sharing::{NewShare, Share, SharePermissions, ShareType, ShareUpdate, Shares};
pub use user_status::{StatusType, UserStatus, UserStatuses};

/// The `User-Agent` sent unless [`NextcloudBuilder::user_agent`] overrides it.
/// It names this client on the grant screen and in connected devices.
pub const DEFAULT_USER_AGENT: &str = concat!("nextcloud-rs/", env!("CARGO_PKG_VERSION"));
