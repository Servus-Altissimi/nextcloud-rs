//! File access over WebDAV.
//!
//! Personal files live at `remote.php/dav/files/{user}/`. RFC 4918 verbs work
//! as written; the Nextcloud extras (file ids, share state, previews) arrive as
//! vendor properties.
//!
//! ```no_run
//! # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
//! let files = nc.files();
//!
//! files.create_dir_all("/Reports/2026").await?;
//! files.upload("/Reports/2026/q1.txt", "hello".as_bytes().to_vec()).await?;
//!
//! for e in files.list("/Reports/2026").await? {
//!     println!("{} {}", e.name(), e.size.unwrap_or(0));
//! }
//!
//! let body = files.download("/Reports/2026/q1.txt").await?;
//! assert_eq!(&body[..], b"hello");
//! # Ok(())
//! # }
//! ```

pub mod dav;
mod entry;
mod kind;
pub mod path;
mod trashbin;
mod upload;
mod versions;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use percent_encoding::percent_decode_str;
use reqwest::{Method, RequestBuilder, Response};
use url::Url;

use crate::client::{DavRoot, Nextcloud};
use crate::error::{Error, Result};
use crate::ocs::truncate;

pub use dav::{
    DavResponse, Depth, NS_DAV, NS_NC, NS_OC, NS_OCS, PropValue, parse_multistatus, prop_key,
};
pub use entry::{DavPermissions, FileEntry};
pub use kind::MediaKind;
pub use trashbin::{Trashbin, TrashbinEntry};
pub use upload::{
    CHUNKED_UPLOAD_THRESHOLD, ChunkedUpload, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE,
};
pub use versions::{FileVersion, Versions};

/// The properties requested by default.
///
/// Unsupported ones come back in a `404` propstat and are dropped, so
/// over-requesting is harmless.
pub const DEFAULT_PROPS: &[(&str, &str)] = &[
    (NS_DAV, "getlastmodified"),
    (NS_DAV, "getetag"),
    (NS_DAV, "getcontenttype"),
    (NS_DAV, "getcontentlength"),
    (NS_DAV, "resourcetype"),
    (NS_DAV, "creationdate"),
    (NS_OC, "fileid"),
    (NS_OC, "permissions"),
    (NS_OC, "size"),
    (NS_OC, "favorite"),
    (NS_OC, "owner-id"),
    (NS_OC, "owner-display-name"),
    (NS_OC, "share-types"),
    (NS_OC, "comments-unread"),
    (NS_NC, "has-preview"),
    (NS_NC, "mount-type"),
    (NS_NC, "contained-folder-count"),
    (NS_NC, "contained-file-count"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
}

impl ArchiveFormat {
    fn accept(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "application/zip",
            ArchiveFormat::Tar => "application/x-tar",
        }
    }
}

/// A slice of a file, with the context a `Range` response needs.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeRead {
    pub bytes: Bytes,
    pub start: u64,
    pub end: u64,
    pub total: u64,
    /// The file's content type, when the server reported one.
    pub content_type: Option<String>,
}

impl RangeRead {
    pub fn content_range(&self) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, self.total)
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether nothing was read, which happens for a zero-length file.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Whether the slice covers the whole file, making a `200` as correct as a `206`.
    pub fn is_whole_file(&self) -> bool {
        self.start == 0 && self.total > 0 && self.end + 1 >= self.total
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UploadResult {
    pub etag: Option<String>,
    pub file_id: Option<String>,
}

pub(crate) fn dav_method(name: &'static str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("WebDAV method names are valid tokens")
}

pub(crate) fn xml_request(
    nc: &Nextcloud,
    method: &'static str,
    url: Url,
    depth: Option<Depth>,
    body: String,
) -> RequestBuilder {
    let mut rb = nc
        .request(dav_method(method), url)
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(body);
    if let Some(depth) = depth {
        rb = rb.header("Depth", depth.header());
    }
    rb
}

/// Send a request and parse the `207 Multi-Status` body it answers with.
pub(crate) async fn multistatus(
    rb: RequestBuilder,
    method: &'static str,
    path: &str,
) -> Result<Vec<DavResponse>> {
    let resp = send_dav(rb, method, path).await?;
    parse_multistatus(&resp.bytes().await?)
}

pub struct Files<'a> {
    pub(crate) nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn files(&self) -> Files<'_> {
        Files { nc: self }
    }
}

impl<'a> Files<'a> {
    /// The decoded URL path of the files root, used to make hrefs relative.
    pub(crate) fn root_path(&self) -> Result<String> {
        let url = self.nc.dav_url(DavRoot::Files, "")?;
        Ok(percent_decode_str(url.path())
            .decode_utf8_lossy()
            .into_owned())
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.nc.dav_url(DavRoot::Files, path)
    }

    /// Run a PROPFIND for properties outside [`DEFAULT_PROPS`].
    pub async fn propfind_raw(
        &self,
        path: &str,
        depth: Depth,
        props: &[(&str, &str)],
    ) -> Result<Vec<DavResponse>> {
        let body = dav::propfind_body(props);
        let rb = xml_request(self.nc, "PROPFIND", self.url(path)?, Some(depth), body);
        multistatus(rb, "PROPFIND", path).await
    }

    pub async fn propfind(
        &self,
        path: &str,
        depth: Depth,
        props: &[(&str, &str)],
    ) -> Result<Vec<FileEntry>> {
        let root = self.root_path()?;
        let responses = self.propfind_raw(path, depth, props).await?;
        Ok(responses
            .iter()
            .map(|r| FileEntry::from_dav(r, &root))
            .collect())
    }

    /// List a directory's immediate children, without the directory itself.
    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>> {
        let entries = self.list_with_self(path).await?;
        Ok(entries
            .into_iter()
            .filter(|e| !e.is_root_of(path))
            .collect())
    }

    /// List a directory, with the directory itself as the first entry.
    pub async fn list_with_self(&self, path: &str) -> Result<Vec<FileEntry>> {
        self.propfind(path, Depth::One, DEFAULT_PROPS).await
    }

    pub async fn stat(&self, path: &str) -> Result<FileEntry> {
        let mut entries = self.propfind(path, Depth::Zero, DEFAULT_PROPS).await?;
        if entries.is_empty() {
            return Err(Error::UnexpectedResponse(format!(
                "PROPFIND {path} returned an empty multistatus"
            )));
        }
        Ok(entries.remove(0))
    }

    /// Whether a path exists. Only a 404 answers `false`; other failures
    /// propagate, so a permission problem never reads as absence.
    pub async fn exists(&self, path: &str) -> Result<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub async fn download(&self, path: &str) -> Result<Bytes> {
        let rb = self.nc.request(Method::GET, self.url(path)?);
        let resp = send_dav(rb, "GET", path).await?;
        Ok(resp.bytes().await?)
    }

    pub async fn download_range(&self, path: &str, start: u64, end: u64) -> Result<Bytes> {
        let rb = self
            .nc
            .request(Method::GET, self.url(path)?)
            .header("Range", format!("bytes={start}-{end}"));
        let resp = send_dav(rb, "GET", path).await?;
        Ok(resp.bytes().await?)
    }

    /// Read a byte range, with the total length and content type a `206` needs.
    ///
    /// `end` is inclusive and clamped to the last byte; `None` reads to the end
    /// of the file. `Ok(None)` means `start` is at or past the end, the
    /// `416 Range Not Satisfiable` case.
    ///
    /// ```no_run
    /// # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
    /// // Answer `Range: bytes=0-`, capped at 2 MiB.
    /// if let Some(read) = nc.files().read_range("/film.mp4", 0, Some(2 * 1024 * 1024 - 1)).await? {
    ///     println!("{} of {} bytes", read.bytes.len(), read.total);
    ///     println!("Content-Range: {}", read.content_range());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn read_range(
        &self,
        path: &str,
        start: u64,
        end: Option<u64>,
    ) -> Result<Option<RangeRead>> {
        let entry = self.stat(path).await?;
        let total = entry.size.unwrap_or(0);
        let content_type = entry.content_type.clone();

        if total == 0 {
            return Ok(Some(RangeRead {
                bytes: Bytes::new(),
                start: 0,
                end: 0,
                total: 0,
                content_type,
            }));
        }

        if start >= total {
            return Ok(None);
        }

        let last = end.unwrap_or(total - 1).min(total - 1);
        let bytes = self.download_range(path, start, last).await?;

        Ok(Some(RangeRead {
            bytes,
            start,
            end: last,
            total,
            content_type,
        }))
    }

    pub async fn download_stream(
        &self,
        path: &str,
    ) -> Result<impl Stream<Item = Result<Bytes>> + use<>> {
        let rb = self.nc.request(Method::GET, self.url(path)?);
        let resp = send_dav(rb, "GET", path).await?;
        Ok(resp.bytes_stream().map(|r| r.map_err(Error::from)))
    }

    pub async fn download_folder(&self, path: &str, format: ArchiveFormat) -> Result<Bytes> {
        let rb = self
            .nc
            .request(Method::GET, self.url(path)?)
            .header("Accept", format.accept());
        let resp = send_dav(rb, "GET", path).await?;
        Ok(resp.bytes().await?)
    }

    /// Upload a file, replacing it if it exists.
    ///
    /// Servers cap request bodies (`client_max_body_size` in nginx,
    /// `upload_max_filesize` in PHP); [`upload_large`](Self::upload_large)
    /// chunks around that.
    pub async fn upload(&self, path: &str, body: impl Into<reqwest::Body>) -> Result<UploadResult> {
        self.upload_with(path, body, None, None).await
    }

    /// Upload with an explicit modification time and content type.
    ///
    /// `mtime` is a Unix timestamp sent as `X-OC-MTime`, which the server
    /// records in place of the upload time.
    pub async fn upload_with(
        &self,
        path: &str,
        body: impl Into<reqwest::Body>,
        mtime: Option<i64>,
        content_type: Option<&str>,
    ) -> Result<UploadResult> {
        let mut rb = self.nc.request(Method::PUT, self.url(path)?).body(body);
        if let Some(m) = mtime {
            rb = rb.header("X-OC-MTime", m.to_string());
        }
        if let Some(ct) = content_type {
            rb = rb.header("Content-Type", ct);
        }
        let resp = send_dav(rb, "PUT", path).await?;
        Ok(upload_result(&resp))
    }

    /// Upload a file, chunking above [`CHUNKED_UPLOAD_THRESHOLD`].
    ///
    /// ```no_run
    /// # async fn demo(nc: &nextcloud::Nextcloud, data: Vec<u8>) -> Result<(), nextcloud::Error> {
    /// nc.files().upload_auto("/Videos/holiday.mp4", data).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn upload_auto(&self, path: &str, data: impl AsRef<[u8]>) -> Result<UploadResult> {
        let data = data.as_ref();
        if upload::wants_chunking(data.len() as u64) {
            self.upload_large(path, data, DEFAULT_CHUNK_SIZE as usize)
                .await
        } else {
            self.upload(path, data.to_vec()).await
        }
    }

    /// Upload a large file using the chunked protocol.
    ///
    /// `chunk_size` is clamped to [`MIN_CHUNK_SIZE`]..=[`MAX_CHUNK_SIZE`].
    pub async fn upload_large(
        &self,
        path: &str,
        data: impl AsRef<[u8]>,
        chunk_size: usize,
    ) -> Result<UploadResult> {
        let data = data.as_ref();
        let mut upload = self.chunked_upload(path, Some(data.len() as u64)).await?;

        for chunk in data.chunks(upload::clamp_chunk_size(chunk_size)) {
            if let Err(e) = upload.put_chunk(chunk.to_vec()).await {
                // an abandoned session is held server-side for 24h
                let _ = upload.abort().await;
                return Err(e);
            }
        }
        upload.finish().await
    }

    /// Begin a chunked upload session targeting `dest_path`.
    ///
    /// `total_length` lets the server reject an over-quota upload up front.
    pub async fn chunked_upload(
        &self,
        dest_path: &str,
        total_length: Option<u64>,
    ) -> Result<ChunkedUpload<'a>> {
        ChunkedUpload::create(self.nc, dest_path, total_length).await
    }

    pub async fn create_dir(&self, path: &str) -> Result<()> {
        let rb = self.nc.request(dav_method("MKCOL"), self.url(path)?);
        send_dav(rb, "MKCOL", path).await?;
        Ok(())
    }

    /// Create a directory and any missing parents. Idempotent.
    pub async fn create_dir_all(&self, path: &str) -> Result<()> {
        let mut current = String::new();
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(segment);
            match self.create_dir(&current).await {
                Ok(()) => {}
                // MKCOL on an existing collection answers 405
                Err(Error::Dav { status: 405, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let rb = self.nc.request(Method::DELETE, self.url(path)?);
        send_dav(rb, "DELETE", path).await?;
        Ok(())
    }

    /// Move or rename. `overwrite` maps to the WebDAV `Overwrite` header.
    pub async fn move_to(&self, from: &str, to: &str, overwrite: bool) -> Result<()> {
        let rb = self
            .nc
            .request(dav_method("MOVE"), self.url(from)?)
            .header("Destination", self.url(to)?.to_string())
            .header("Overwrite", if overwrite { "T" } else { "F" });
        send_dav(rb, "MOVE", from).await?;
        Ok(())
    }

    pub async fn copy_to(&self, from: &str, to: &str, overwrite: bool) -> Result<()> {
        let rb = self
            .nc
            .request(dav_method("COPY"), self.url(from)?)
            .header("Destination", self.url(to)?.to_string())
            .header("Overwrite", if overwrite { "T" } else { "F" });
        send_dav(rb, "COPY", from).await?;
        Ok(())
    }

    /// Mark or unmark a file as a favourite, via PROPPATCH on `oc:favorite`.
    pub async fn set_favorite(&self, path: &str, favorite: bool) -> Result<()> {
        let body = dav::proppatch_body(NS_OC, "favorite", if favorite { "1" } else { "0" });
        let rb = xml_request(self.nc, "PROPPATCH", self.url(path)?, None, body);
        send_dav(rb, "PROPPATCH", path).await?;
        Ok(())
    }

    /// List favourites under `path` via the `oc:filter-files` REPORT. `"/"`
    /// searches the whole tree.
    pub async fn favorites(&self, path: &str) -> Result<Vec<FileEntry>> {
        let body = dav::favorites_body(DEFAULT_PROPS);
        let rb = xml_request(self.nc, "REPORT", self.url(path)?, None, body);

        let responses = multistatus(rb, "REPORT", path).await?;
        let root = self.root_path()?;
        Ok(responses
            .iter()
            .map(|r| FileEntry::from_dav(r, &root))
            .filter(|e| !e.is_root_of(path))
            .collect())
    }

    pub fn trashbin(&self) -> Trashbin<'a> {
        Trashbin { nc: self.nc }
    }

    pub fn versions(&self) -> Versions<'a> {
        Versions { nc: self.nc }
    }
}

pub(crate) fn upload_result(resp: &Response) -> UploadResult {
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    UploadResult {
        etag: header("OC-ETag").or_else(|| header("ETag")),
        file_id: header("OC-FileId"),
    }
}

/// Send a request and turn a non-2xx status into [`Error::Dav`]. Per-resource
/// failures inside a 207 body are the caller's to inspect.
pub(crate) async fn send_dav(rb: RequestBuilder, method: &str, path: &str) -> Result<Response> {
    let resp = rb.send().await?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Dav {
        method: method.to_string(),
        path: path.to_string(),
        status: status.as_u16(),
        body: truncate(&body, 500),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_headers() {
        assert_eq!(Depth::Zero.header(), "0");
        assert_eq!(Depth::One.header(), "1");
        assert_eq!(Depth::Infinity.header(), "infinity");
        assert_eq!(Depth::default(), Depth::One);
    }

    #[test]
    fn dav_methods() {
        for m in ["PROPFIND", "PROPPATCH", "MKCOL", "MOVE", "COPY", "REPORT"] {
            assert_eq!(dav_method(m).as_str(), m);
        }
    }

    #[test]
    fn archive_accept() {
        assert_eq!(ArchiveFormat::Zip.accept(), "application/zip");
        assert_eq!(ArchiveFormat::Tar.accept(), "application/x-tar");
    }

    #[test]
    fn default_props() {
        for required in [
            (NS_OC, "fileid"),
            (NS_OC, "permissions"),
            (NS_DAV, "getetag"),
            (NS_DAV, "resourcetype"),
        ] {
            assert!(DEFAULT_PROPS.contains(&required), "missing {required:?}");
        }
    }

    #[test]
    fn range_header() {
        let read = RangeRead {
            bytes: Bytes::from_static(b"0123456789"),
            start: 10,
            end: 19,
            total: 100,
            content_type: Some("video/mp4".into()),
        };
        assert_eq!(read.content_range(), "bytes 10-19/100");
        assert_eq!(read.len(), 10);
        assert!(!read.is_empty());
        assert!(!read.is_whole_file());
    }

    #[test]
    fn whole_range() {
        let whole = RangeRead {
            bytes: Bytes::from_static(b"abc"),
            start: 0,
            end: 2,
            total: 3,
            content_type: None,
        };
        assert!(whole.is_whole_file());
        assert_eq!(whole.content_range(), "bytes 0-2/3");
    }

    #[test]
    fn empty_file_range() {
        let empty = RangeRead {
            bytes: Bytes::new(),
            start: 0,
            end: 0,
            total: 0,
            content_type: None,
        };
        assert!(empty.is_empty());
        assert!(!empty.is_whole_file());
    }

    #[test]
    fn root_href_decoded() {
        let nc = Nextcloud::builder("https://cloud.example.com")
            .basic_auth("user name", "pw")
            .build()
            .unwrap();
        assert_eq!(
            nc.files().root_path().unwrap(),
            "/remote.php/dav/files/user name"
        );
    }
}
