//! Typed view over a WebDAV `response` element.

use chrono::{DateTime, Utc};
use percent_encoding::percent_decode_str;

use crate::files::dav::{self, DavResponse, NS_DAV, NS_NC, NS_OC, parse_http_date, parse_iso_date};
use crate::files::kind::MediaKind;
use crate::files::path;
use crate::sharing::ShareType;

/// The permission string from `oc:permissions`. One letter per capability,
/// unordered, any of them absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavPermissions {
    raw: String,
}

impl DavPermissions {
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    fn has(&self, c: char) -> bool {
        self.raw.contains(c)
    }

    pub fn is_shared(&self) -> bool {
        self.has('S')
    }
    pub fn is_shareable(&self) -> bool {
        self.has('R')
    }
    pub fn is_mounted(&self) -> bool {
        self.has('M')
    }
    pub fn is_readable(&self) -> bool {
        self.has('G')
    }
    pub fn is_deletable(&self) -> bool {
        self.has('D')
    }
    pub fn is_renameable(&self) -> bool {
        self.has('N')
    }
    pub fn is_moveable(&self) -> bool {
        self.has('V')
    }
    pub fn is_writeable(&self) -> bool {
        self.has('W')
    }
    pub fn can_create_file(&self) -> bool {
        self.has('C')
    }
    pub fn can_create_folder(&self) -> bool {
        self.has('K')
    }
}

/// properties come back depends on the request and the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileEntry {
    /// Path relative to the user's files root, with a leading `/` and no
    /// trailing slash. The root itself is `/`.
    pub path: String,
    pub href: String,
    pub is_directory: bool,
    /// `oc:fileid`, stable across moves and renames within the instance.
    pub file_id: Option<u64>,
    /// `d:getetag`, quotes included, since conditional requests echo them.
    pub etag: Option<String>,
    pub content_type: Option<String>,
    /// Size in bytes: `d:getcontentlength` for files, `oc:size` for folders,
    /// where it is a recursive total.
    pub size: Option<u64>,
    pub last_modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub permissions: Option<DavPermissions>,
    pub favorite: bool,
    pub has_preview: Option<bool>,
    pub owner_id: Option<String>,
    pub owner_display_name: Option<String>,
    /// `oc:share-types`: the kinds of share currently active on this resource.
    pub share_types: Vec<ShareType>,
    pub comments_unread: Option<u64>,
    pub comments_count: Option<u64>,
    pub mount_type: Option<String>,
    pub contained_file_count: Option<u64>,
    pub contained_folder_count: Option<u64>,
}

impl FileEntry {
    pub fn name(&self) -> &str {
        self.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
    }

    pub fn parent(&self) -> Option<&str> {
        let trimmed = self.path.trim_end_matches('/');
        let idx = trimmed.rfind('/')?;
        if idx == 0 {
            Some("/")
        } else {
            Some(&trimmed[..idx])
        }
    }

    /// Classify this entry as playable media, by content type where there is
    /// one and by name otherwise. Directories are never media.
    pub fn media_kind(&self) -> Option<MediaKind> {
        if self.is_directory {
            return None;
        }
        MediaKind::detect(self.name(), self.content_type.as_deref())
    }

    /// Whether this entry is the listed directory itself, which a `Depth: 1`
    /// PROPFIND always returns first.
    pub fn is_root_of(&self, listed_path: &str) -> bool {
        path::normalise(listed_path) == self.path
    }

    /// Build an entry from a parsed DAV response. `root_url_path`, e.g.
    /// `/remote.php/dav/files/alice`, is stripped to give a relative path.
    pub(crate) fn from_dav(resp: &DavResponse, root_url_path: &str) -> Self {
        let is_directory = resp.is_collection();

        let decoded = percent_decode_str(&resp.href)
            .decode_utf8_lossy()
            .into_owned();
        let relative = decoded
            .strip_prefix(root_url_path)
            .unwrap_or(&decoded)
            .to_string();

        let share_types = resp
            .prop(NS_OC, "share-types")
            .map(|p| {
                p.children
                    .iter()
                    .filter_map(|(_, v)| v.parse::<i64>().ok())
                    .map(ShareType::from_code)
                    .collect()
            })
            .unwrap_or_default();

        FileEntry {
            path: path::normalise(&relative),
            href: resp.href.clone(),
            is_directory,
            file_id: resp.parsed(NS_OC, "fileid"),
            etag: resp.text(NS_DAV, "getetag").map(str::to_owned),
            content_type: resp.text(NS_DAV, "getcontenttype").map(str::to_owned),
            size: dav::size_of(resp, is_directory),
            last_modified: resp
                .text(NS_DAV, "getlastmodified")
                .and_then(parse_http_date),
            created: resp.text(NS_DAV, "creationdate").and_then(parse_iso_date),
            permissions: resp.text(NS_OC, "permissions").map(DavPermissions::new),
            favorite: resp
                .text(NS_OC, "favorite")
                .is_some_and(|v| v == "1" || v == "true"),
            has_preview: resp
                .text(NS_NC, "has-preview")
                .map(|v| v == "true" || v == "1"),
            owner_id: resp.text(NS_OC, "owner-id").map(str::to_owned),
            owner_display_name: resp.text(NS_OC, "owner-display-name").map(str::to_owned),
            share_types,
            comments_unread: resp.parsed(NS_OC, "comments-unread"),
            comments_count: resp.parsed(NS_OC, "comments-count"),
            mount_type: resp.text(NS_NC, "mount-type").map(str::to_owned),
            contained_file_count: resp.parsed(NS_NC, "contained-file-count"),
            contained_folder_count: resp.parsed(NS_NC, "contained-folder-count"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::dav::parse_multistatus;

    const ROOT: &str = "/remote.php/dav/files/alice";

    const XML: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/alice/Music/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/></d:resourcetype>
      <d:getlastmodified>Sun, 06 Nov 1994 08:49:37 GMT</d:getlastmodified>
      <oc:fileid>10</oc:fileid>
      <oc:size>2048</oc:size>
      <oc:permissions>RGDNVCK</oc:permissions>
      <nc:contained-file-count>3</nc:contained-file-count>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alice/Music/a%20song%20%231.mp3</d:href>
    <d:propstat><d:prop>
      <d:resourcetype/>
      <d:getcontentlength>512</d:getcontentlength>
      <d:getcontenttype>audio/mpeg</d:getcontenttype>
      <oc:fileid>11</oc:fileid>
      <oc:favorite>1</oc:favorite>
      <oc:owner-id>alice</oc:owner-id>
      <oc:share-types><oc:share-type>3</oc:share-type></oc:share-types>
      <nc:has-preview>false</nc:has-preview>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

    fn entries() -> Vec<FileEntry> {
        parse_multistatus(XML.as_bytes())
            .unwrap()
            .iter()
            .map(|r| FileEntry::from_dav(r, ROOT))
            .collect()
    }

    #[test]
    fn href_to_path() {
        let e = entries();
        assert_eq!(e[0].path, "/Music");
        assert_eq!(e[1].path, "/Music/a song #1.mp3");
        assert!(e[1].href.contains("%20"));
    }

    #[test]
    fn sizes() {
        let e = entries();
        assert_eq!(e[0].size, Some(2048));
        assert_eq!(e[1].size, Some(512));
    }

    #[test]
    fn name_and_parent() {
        let e = entries();
        assert_eq!(e[0].name(), "Music");
        assert_eq!(e[1].name(), "a song #1.mp3");
        assert_eq!(e[1].parent(), Some("/Music"));
        assert_eq!(e[0].parent(), Some("/"));
    }

    #[test]
    fn is_root() {
        let e = entries();
        assert!(e[0].is_root_of("/Music"));
        assert!(e[0].is_root_of("Music/"));
        assert!(!e[1].is_root_of("/Music"));
    }

    #[test]
    fn metadata() {
        let e = entries();
        assert!(e[0].is_directory);
        assert!(!e[1].is_directory);
        assert_eq!(e[0].file_id, Some(10));
        assert_eq!(e[0].contained_file_count, Some(3));
        assert_eq!(e[1].content_type.as_deref(), Some("audio/mpeg"));
        assert!(e[1].favorite);
        assert!(!e[0].favorite);
        assert_eq!(e[1].has_preview, Some(false));
        assert_eq!(e[1].share_types, vec![ShareType::PublicLink]);
        assert_eq!(e[1].owner_id.as_deref(), Some("alice"));
        assert!(e[0].last_modified.is_some());
    }

    #[test]
    fn permissions() {
        let p = entries()[0].permissions.clone().unwrap();
        assert!(p.is_shareable());
        assert!(p.is_deletable());
        assert!(p.can_create_folder());
        assert!(p.can_create_file());
        assert!(!p.is_shared());
        assert!(!p.is_mounted());
        assert_eq!(p.as_str(), "RGDNVCK");
    }

    #[test]
    fn normalise_twice() {
        assert_eq!(path::normalise(""), "/");
        assert_eq!(path::normalise("/"), "/");
        assert_eq!(path::normalise("a/b"), "/a/b");
        assert_eq!(path::normalise("/a/b/"), "/a/b");
        assert_eq!(path::normalise(&path::normalise("/a/b/")), "/a/b");
    }

    #[test]
    fn all_permission_letters() {
        let all = DavPermissions::new("SRMGDNVWCK");
        assert!(all.is_shared());
        assert!(all.is_shareable());
        assert!(all.is_mounted());
        assert!(all.is_readable());
        assert!(all.is_deletable());
        assert!(all.is_renameable());
        assert!(all.is_moveable());
        assert!(all.is_writeable());
        assert!(all.can_create_file());
        assert!(all.can_create_folder());

        let none = DavPermissions::new("");
        assert!(!none.is_readable());
        assert!(!none.can_create_file());
    }

    #[test]
    fn kind_from_mime_then_name() {
        let mut entry = FileEntry {
            path: "/Music/track.flac".into(),
            content_type: Some("application/octet-stream".into()),
            ..Default::default()
        };
        assert_eq!(entry.media_kind(), Some(MediaKind::Audio));

        entry.content_type = Some("video/mp4".into());
        assert_eq!(entry.media_kind(), Some(MediaKind::Video));

        entry.is_directory = true;
        assert_eq!(entry.media_kind(), None);
    }
}
