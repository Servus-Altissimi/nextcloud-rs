//! File version history.
//!
//! Versions live at `remote.php/dav/versions/{user}/versions/{fileid}`, keyed
//! by `oc:fileid`, so history survives renames. Each version is named for the
//! Unix timestamp at which it was superseded. Restoring is a `MOVE` into the
//! sibling `restore` collection.

use chrono::{DateTime, Utc};
use url::Url;

use crate::client::{DavRoot, Nextcloud};
use crate::error::Result;
use crate::files::dav::{self, DavResponse, Depth, NS_DAV, NS_NC, parse_http_date};
use crate::files::{dav_method, multistatus, send_dav, xml_request};

const VERSION_PROPS: &[(&str, &str)] = &[
    (NS_DAV, "getcontentlength"),
    (NS_DAV, "getcontenttype"),
    (NS_DAV, "getlastmodified"),
    (NS_DAV, "getetag"),
    (NS_DAV, "resourcetype"),
    (NS_NC, "version-label"),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileVersion {
    pub href: String,
    /// The version identifier, which is the Unix timestamp of the revision.
    pub version_id: String,
    pub size: Option<u64>,
    pub content_type: Option<String>,
    pub last_modified: Option<DateTime<Utc>>,
    pub etag: Option<String>,
    /// `nc:version-label`, set when a user has named this revision.
    pub label: Option<String>,
}

impl FileVersion {
    pub fn timestamp(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(self.version_id.parse().ok()?, 0)
    }

    fn from_dav(resp: &DavResponse) -> Self {
        FileVersion {
            href: resp.href.clone(),
            version_id: dav::href_leaf(&resp.href),
            size: resp.parsed(NS_DAV, "getcontentlength"),
            content_type: resp.text(NS_DAV, "getcontenttype").map(str::to_owned),
            last_modified: resp
                .text(NS_DAV, "getlastmodified")
                .and_then(parse_http_date),
            etag: resp.text(NS_DAV, "getetag").map(str::to_owned),
            label: resp.text(NS_NC, "version-label").map(str::to_owned),
        }
    }
}

pub struct Versions<'a> {
    pub(crate) nc: &'a Nextcloud,
}

impl Versions<'_> {
    fn version_url(&self, version: &FileVersion) -> Result<Url> {
        Ok(self.nc.base.join(&version.href)?)
    }

    /// List the superseded revisions of a file, by
    /// [`file_id`](crate::files::FileEntry::file_id). The current one is not
    /// included.
    pub async fn list(&self, file_id: u64) -> Result<Vec<FileVersion>> {
        let rel = format!("versions/{file_id}");
        let url = self.nc.dav_url(DavRoot::Versions, &rel)?;
        let body = dav::propfind_body(VERSION_PROPS);
        let rb = xml_request(self.nc, "PROPFIND", url, Some(Depth::One), body);

        let responses = multistatus(rb, "PROPFIND", &rel).await?;

        // the file's own collection leads the listing
        Ok(responses
            .iter()
            .map(FileVersion::from_dav)
            .filter(|v| v.version_id != file_id.to_string())
            .collect())
    }

    pub async fn download(&self, version: &FileVersion) -> Result<bytes::Bytes> {
        let url = self.version_url(version)?;
        let rb = self.nc.request(reqwest::Method::GET, url);
        let resp = send_dav(rb, "GET", &version.version_id).await?;
        Ok(resp.bytes().await?)
    }

    /// Restore a revision. The content it replaces becomes a new entry in the
    /// history.
    pub async fn restore(&self, version: &FileVersion) -> Result<()> {
        let source = self.version_url(version)?;
        let destination = self.nc.dav_url(
            DavRoot::Versions,
            &format!("restore/{}", version.version_id),
        )?;

        let rb = self
            .nc
            .request(dav_method("MOVE"), source)
            .header("Destination", destination.to_string());

        send_dav(rb, "MOVE", &version.version_id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::dav::parse_multistatus;

    const XML: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:nc="http://nextcloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/versions/alice/versions/42/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/versions/alice/versions/42/1700000000</d:href>
    <d:propstat><d:prop>
      <d:getcontentlength>1024</d:getcontentlength>
      <d:getcontenttype>text/plain</d:getcontenttype>
      <d:getlastmodified>Sun, 06 Nov 1994 08:49:37 GMT</d:getlastmodified>
      <nc:version-label>before edit</nc:version-label>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

    fn versions() -> Vec<FileVersion> {
        parse_multistatus(XML.as_bytes())
            .unwrap()
            .iter()
            .map(FileVersion::from_dav)
            .filter(|v| v.version_id != "42")
            .collect()
    }

    #[test]
    fn skips_collection() {
        assert_eq!(versions().len(), 1);
    }

    #[test]
    fn version_id() {
        assert_eq!(versions()[0].version_id, "1700000000");
    }

    #[test]
    fn version_timestamp() {
        let ts = versions()[0].timestamp().unwrap();
        assert_eq!(ts.timestamp(), 1700000000);
    }

    #[test]
    fn odd_version_id() {
        let v = FileVersion {
            version_id: "not-a-timestamp".into(),
            ..Default::default()
        };
        assert!(v.timestamp().is_none());
    }

    #[test]
    fn metadata() {
        let v = &versions()[0];
        assert_eq!(v.size, Some(1024));
        assert_eq!(v.content_type.as_deref(), Some("text/plain"));
        assert_eq!(v.label.as_deref(), Some("before edit"));
        assert!(v.last_modified.is_some());
    }
}
