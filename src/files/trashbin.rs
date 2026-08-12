//! The trash bin.
//!
//! Deleted files move to `remote.php/dav/trashbin/{user}/trash`, keeping their
//! origin in `nc:trashbin-filename`, `nc:trashbin-original-location` and
//! `nc:trashbin-deletion-time`. Restoring is a `MOVE` into the sibling
//! `restore` collection. Deleting from the trash is permanent.

use chrono::{DateTime, Utc};
use reqwest::Method;
use url::Url;

use crate::client::{DavRoot, Nextcloud};
use crate::error::Result;
use crate::files::dav::{self, DavResponse, Depth, NS_DAV, NS_NC, NS_OC};
use crate::files::{dav_method, multistatus, send_dav, xml_request};

const TRASH_PROPS: &[(&str, &str)] = &[
    (NS_DAV, "resourcetype"),
    (NS_DAV, "getcontentlength"),
    (NS_DAV, "getcontenttype"),
    (NS_DAV, "getlastmodified"),
    (NS_OC, "fileid"),
    (NS_OC, "size"),
    (NS_NC, "trashbin-filename"),
    (NS_NC, "trashbin-original-location"),
    (NS_NC, "trashbin-deletion-time"),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrashbinEntry {
    pub href: String,
    /// The name inside the trash, which encodes the deletion time.
    pub trash_name: String,
    pub original_name: Option<String>,
    pub original_location: Option<String>,
    pub deleted_at: Option<i64>,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub file_id: Option<u64>,
    pub content_type: Option<String>,
}

impl TrashbinEntry {
    pub fn deleted_at_utc(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(self.deleted_at?, 0)
    }

    /// The original name if the server reported one, else the trash name.
    pub fn display_name(&self) -> &str {
        self.original_name.as_deref().unwrap_or(&self.trash_name)
    }

    fn from_dav(resp: &DavResponse) -> Self {
        let is_directory = resp.is_collection();

        TrashbinEntry {
            href: resp.href.clone(),
            trash_name: dav::href_leaf(&resp.href),
            original_name: resp.text(NS_NC, "trashbin-filename").map(str::to_owned),
            original_location: resp
                .text(NS_NC, "trashbin-original-location")
                .map(str::to_owned),
            deleted_at: resp.parsed(NS_NC, "trashbin-deletion-time"),
            is_directory,
            size: dav::size_of(resp, is_directory),
            file_id: resp.parsed(NS_OC, "fileid"),
            content_type: resp.text(NS_DAV, "getcontenttype").map(str::to_owned),
        }
    }
}

pub struct Trashbin<'a> {
    pub(crate) nc: &'a Nextcloud,
}

impl Trashbin<'_> {
    /// Resolve an entry's href against the server root. Hrefs are absolute
    /// server paths, so this holds for a subdirectory install too.
    fn entry_url(&self, entry: &TrashbinEntry) -> Result<Url> {
        Ok(self.nc.base.join(&entry.href)?)
    }

    pub async fn list(&self) -> Result<Vec<TrashbinEntry>> {
        let url = self.nc.dav_url(DavRoot::Trashbin, "trash")?;
        let body = dav::propfind_body(TRASH_PROPS);
        let rb = xml_request(self.nc, "PROPFIND", url, Some(Depth::One), body);

        let responses = multistatus(rb, "PROPFIND", "trashbin/trash").await?;

        // first response is the collection itself
        Ok(responses
            .iter()
            .map(TrashbinEntry::from_dav)
            .filter(|e| e.trash_name != "trash")
            .collect())
    }

    pub async fn restore(&self, entry: &TrashbinEntry) -> Result<()> {
        let source = self.entry_url(entry)?;
        let destination = self
            .nc
            .dav_url(DavRoot::Trashbin, &format!("restore/{}", entry.trash_name))?;

        let rb = self
            .nc
            .request(dav_method("MOVE"), source)
            .header("Destination", destination.to_string());

        send_dav(rb, "MOVE", &entry.trash_name).await?;
        Ok(())
    }

    pub async fn delete_permanently(&self, entry: &TrashbinEntry) -> Result<()> {
        let url = self.entry_url(entry)?;
        let rb = self.nc.request(Method::DELETE, url);
        send_dav(rb, "DELETE", &entry.trash_name).await?;
        Ok(())
    }

    /// Empty the trash. Irreversible.
    pub async fn empty(&self) -> Result<()> {
        let url = self.nc.dav_url(DavRoot::Trashbin, "trash")?;
        let rb = self.nc.request(Method::DELETE, url);
        send_dav(rb, "DELETE", "trashbin/trash").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::dav::parse_multistatus;

    const XML: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/trashbin/alice/trash/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/trashbin/alice/trash/notes.txt.d1700000000</d:href>
    <d:propstat><d:prop>
      <d:resourcetype/>
      <d:getcontentlength>99</d:getcontentlength>
      <d:getcontenttype>text/plain</d:getcontenttype>
      <oc:fileid>77</oc:fileid>
      <nc:trashbin-filename>notes.txt</nc:trashbin-filename>
      <nc:trashbin-original-location>Documents/notes.txt</nc:trashbin-original-location>
      <nc:trashbin-deletion-time>1700000000</nc:trashbin-deletion-time>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

    fn parse() -> Vec<TrashbinEntry> {
        parse_multistatus(XML.as_bytes())
            .unwrap()
            .iter()
            .map(TrashbinEntry::from_dav)
            .filter(|e| e.trash_name != "trash")
            .collect()
    }

    #[test]
    fn skips_collection() {
        assert_eq!(parse().len(), 1);
    }

    #[test]
    fn original_location() {
        let e = &parse()[0];
        assert_eq!(e.original_name.as_deref(), Some("notes.txt"));
        assert_eq!(e.original_location.as_deref(), Some("Documents/notes.txt"));
        assert_eq!(e.deleted_at, Some(1700000000));
        assert_eq!(e.deleted_at_utc().unwrap().timestamp(), 1700000000);
    }

    #[test]
    fn trash_vs_display_name() {
        let e = &parse()[0];
        assert_eq!(e.trash_name, "notes.txt.d1700000000");
        assert_eq!(e.display_name(), "notes.txt");
    }

    #[test]
    fn display_name_fallback() {
        let xml = r#"<d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/remote.php/dav/trashbin/alice/trash/x.d1</d:href>
          <d:propstat><d:prop><d:resourcetype/></d:prop>
          <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let e = TrashbinEntry::from_dav(&parse_multistatus(xml.as_bytes()).unwrap()[0]);
        assert_eq!(e.display_name(), "x.d1");
    }

    #[test]
    fn metadata() {
        let e = &parse()[0];
        assert!(!e.is_directory);
        assert_eq!(e.size, Some(99));
        assert_eq!(e.file_id, Some(77));
        assert_eq!(e.content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn href_subdir() {
        let nc = Nextcloud::builder("https://example.com/nextcloud")
            .basic_auth("alice", "pw")
            .build()
            .unwrap();
        let tb = Trashbin { nc: &nc };
        let entry = TrashbinEntry {
            href: "/nextcloud/remote.php/dav/trashbin/alice/trash/a.d1".into(),
            ..Default::default()
        };
        assert_eq!(
            tb.entry_url(&entry).unwrap().as_str(),
            "https://example.com/nextcloud/remote.php/dav/trashbin/alice/trash/a.d1"
        );
    }
}
