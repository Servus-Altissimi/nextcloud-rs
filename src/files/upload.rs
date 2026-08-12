//! Chunked upload (the "v2" protocol).
//!
//! Parts are staged in `remote.php/dav/uploads/{user}/{uploadid}` and assembled
//! by the server, which sidesteps the request size limits in the web server and
//! PHP. `MKCOL` the session with a `Destination` header naming the eventual
//! file, `PUT` each chunk as `{00001..}`, then `MOVE` `.file` to assemble.
//!
//! Chunks assemble in numerical order whatever order they arrive in, and each
//! must be between [`MIN_CHUNK_SIZE`] and [`MAX_CHUNK_SIZE`] except the last.
//! An abandoned session expires after 24 hours; [`ChunkedUpload::abort`] clears
//! it immediately.

use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::Method;
use url::Url;

use crate::client::{DavRoot, Nextcloud};
use crate::error::{Error, Result};
use crate::files::{dav_method, send_dav};

pub const MIN_CHUNK_SIZE: u64 = 5 * 1024 * 1024;
pub const MAX_CHUNK_SIZE: u64 = 5 * 1024 * 1024 * 1024;
/// Default of 10 MiB: above the minimum, cheap to retry.
pub const DEFAULT_CHUNK_SIZE: u64 = 10 * 1024 * 1024;

const MAX_CHUNKS: u32 = 10_000;

const _: () = assert!(DEFAULT_CHUNK_SIZE >= MIN_CHUNK_SIZE && DEFAULT_CHUNK_SIZE <= MAX_CHUNK_SIZE);

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Name a fresh upload session.
///
/// A timestamp plus a process-local counter, which avoids a `getrandom` wasm
/// backend for an id that only has to be unique among one account's uploads. A
/// collision surfaces as a `405` from MKCOL.
fn new_session_id() -> String {
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("nextcloud-rs-{stamp:x}-{counter:x}")
}

/// [`finish`](Self::finish) or [`abort`](Self::abort) leaves it on the server.
pub struct ChunkedUpload<'a> {
    nc: &'a Nextcloud,
    upload_id: String,
    destination: Url,
    /// The destination path, kept for error messages.
    dest_path: String,
    total_length: Option<u64>,
    next_index: u32,
    mtime: Option<i64>,
}

impl<'a> ChunkedUpload<'a> {
    pub(crate) async fn create(
        nc: &'a Nextcloud,
        dest_path: &str,
        total_length: Option<u64>,
    ) -> Result<Self> {
        let upload_id = new_session_id();
        let destination = nc.dav_url(DavRoot::Files, dest_path)?;
        let session_url = nc.dav_url(DavRoot::Uploads, &upload_id)?;

        let rb = nc
            .request(dav_method("MKCOL"), session_url)
            .header("Destination", destination.to_string());
        send_dav(rb, "MKCOL", &upload_id).await?;

        Ok(Self {
            nc,
            upload_id,
            destination,
            dest_path: dest_path.to_string(),
            total_length,
            next_index: 1,
            mtime: None,
        })
    }

    pub fn set_mtime(&mut self, unix_timestamp: i64) {
        self.mtime = Some(unix_timestamp);
    }

    pub fn upload_id(&self) -> &str {
        &self.upload_id
    }

    pub fn chunks_sent(&self) -> u32 {
        self.next_index - 1
    }

    /// Upload the next chunk, numbering from 1. Use
    /// [`put_chunk_at`](Self::put_chunk_at) to place one explicitly.
    pub async fn put_chunk(&mut self, data: impl Into<reqwest::Body>) -> Result<()> {
        let index = self.next_index;
        self.put_chunk_at(index, data).await?;
        self.next_index += 1;
        Ok(())
    }

    /// Upload a chunk at an explicit 1-based index, which sets assembly order.
    pub async fn put_chunk_at(&self, index: u32, data: impl Into<reqwest::Body>) -> Result<()> {
        if index == 0 || index > MAX_CHUNKS {
            return Err(Error::UnexpectedResponse(format!(
                "chunk index {index} is outside the permitted range 1..={MAX_CHUNKS}"
            )));
        }

        let rel = format!("{}/{:05}", self.upload_id, index);
        let url = self.nc.dav_url(DavRoot::Uploads, &rel)?;

        let mut rb = self
            .nc
            .request(Method::PUT, url)
            .header("Destination", self.destination.to_string())
            .body(data);

        if let Some(total) = self.total_length {
            rb = rb.header("OC-Total-Length", total.to_string());
        }

        send_dav(rb, "PUT", &rel).await?;
        Ok(())
    }

    /// Assemble the chunks into the destination file. The `MOVE` reports the
    /// same identity headers a plain `PUT` does.
    pub async fn finish(self) -> Result<crate::files::UploadResult> {
        let rel = format!("{}/.file", self.upload_id);
        let url = self.nc.dav_url(DavRoot::Uploads, &rel)?;

        let mut rb = self
            .nc
            .request(dav_method("MOVE"), url)
            .header("Destination", self.destination.to_string());

        if let Some(total) = self.total_length {
            rb = rb.header("OC-Total-Length", total.to_string());
        }
        if let Some(m) = self.mtime {
            rb = rb.header("X-OC-MTime", m.to_string());
        }

        let resp = send_dav(rb, "MOVE", &self.dest_path).await?;
        Ok(crate::files::upload_result(&resp))
    }

    pub async fn abort(self) -> Result<()> {
        let url = self.nc.dav_url(DavRoot::Uploads, &self.upload_id)?;
        let rb = self.nc.request(Method::DELETE, url);
        send_dav(rb, "DELETE", &self.upload_id).await?;
        Ok(())
    }
}

pub(crate) fn wants_chunking(len: u64) -> bool {
    len > CHUNKED_UPLOAD_THRESHOLD
}

/// Above this, [`Files::upload_auto`](crate::files::Files::upload_auto) chunks.
/// Set above [`MIN_CHUNK_SIZE`], so a chunked upload is always a full chunk plus
/// a remainder.
pub const CHUNKED_UPLOAD_THRESHOLD: u64 = 16 * 1024 * 1024;

/// A single undersized chunk is rejected by the server.
const _: () = assert!(CHUNKED_UPLOAD_THRESHOLD > MIN_CHUNK_SIZE);

/// Clamp a chunk size into the permitted range, without overflowing `usize` on
/// 32-bit targets.
pub(crate) fn clamp_chunk_size(requested: usize) -> usize {
    let ceiling = MAX_CHUNK_SIZE.min(usize::MAX as u64);
    ((requested as u64).clamp(MIN_CHUNK_SIZE, ceiling)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_names() {
        // the server sorts chunk names lexically
        assert_eq!(format!("{:05}", 1), "00001");
        assert_eq!(format!("{:05}", 42), "00042");
        assert_eq!(format!("{:05}", 10000), "10000");
        assert!(format!("{:05}", 2) < format!("{:05}", 10));
    }

    #[test]
    fn chunk_size_clamped() {
        assert_eq!(clamp_chunk_size(1024) as u64, MIN_CHUNK_SIZE);
        assert_eq!(clamp_chunk_size(0) as u64, MIN_CHUNK_SIZE);
        let ten_mib = (10 * 1024 * 1024) as usize;
        assert_eq!(clamp_chunk_size(ten_mib), ten_mib);
    }

    #[test]
    fn clamp_fits_usize() {
        // must saturate on 32-bit rather than wrap
        let clamped = clamp_chunk_size(usize::MAX) as u64;
        assert_eq!(clamped, MAX_CHUNK_SIZE.min(usize::MAX as u64));
    }

    #[test]
    fn threshold_above_min_chunk() {
        assert!(!wants_chunking(1024));
        assert!(!wants_chunking(CHUNKED_UPLOAD_THRESHOLD));
        assert!(wants_chunking(CHUNKED_UPLOAD_THRESHOLD + 1));
    }

    #[test]
    fn protocol_bounds() {
        assert_eq!(MIN_CHUNK_SIZE, 5 * 1024 * 1024);
        assert_eq!(MAX_CHUNK_SIZE, 5 * 1024 * 1024 * 1024);
    }
}
