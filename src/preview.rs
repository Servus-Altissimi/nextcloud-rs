//! Thumbnails, avatars, and public share links.
//!
//! Plain `index.php` routes returning image bytes, outside both OCS and WebDAV.
//! Used by the official clients, but undocumented, so likelier to shift between
//! major releases than the rest. URL building is separate from fetching, since
//! most callers only need the URL.
//!
//! ```no_run
//! # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
//! use nextcloud::PreviewOptions;
//!
//! let url = nc.previews().url_for_file_id(1234, PreviewOptions::square(256))?;
//! let bytes = nc.previews().fetch(url).await?;
//! # let _ = bytes;
//! # Ok(())
//! # }
//! ```

use bytes::Bytes;
use reqwest::Method;
use url::Url;

use crate::client::{Nextcloud, encode_segment};
use crate::error::Result;
use crate::files::send_dav;

/// How a preview should be rendered.
#[derive(Clone, Copy, Debug)]
pub struct PreviewOptions {
    pub width: u32,
    pub height: u32,
    /// Preserve aspect ratio instead of cropping to the exact box.
    pub keep_aspect: bool,
    /// Return a generic file-type icon when no preview can be generated,
    /// rather than an error.
    pub force_icon: bool,
    /// `cover` fills the box; `fill` letterboxes.
    pub mode: PreviewMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    Cover,
    Fill,
}

impl PreviewMode {
    fn as_str(self) -> &'static str {
        match self {
            PreviewMode::Cover => "cover",
            PreviewMode::Fill => "fill",
        }
    }
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            width: 256,
            height: 256,
            keep_aspect: true,
            force_icon: false,
            mode: PreviewMode::Cover,
        }
    }
}

impl PreviewOptions {
    pub fn square(size: u32) -> Self {
        Self {
            width: size,
            height: size,
            ..Default::default()
        }
    }

    pub fn sized(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..Default::default()
        }
    }

    pub fn keep_aspect(mut self, yes: bool) -> Self {
        self.keep_aspect = yes;
        self
    }

    pub fn force_icon(mut self, yes: bool) -> Self {
        self.force_icon = yes;
        self
    }

    pub fn mode(mut self, mode: PreviewMode) -> Self {
        self.mode = mode;
        self
    }

    /// The query parameters shared by the preview endpoints. `a=1` is the
    /// server's spelling of "keep aspect ratio".
    fn query(&self) -> Vec<(&'static str, String)> {
        vec![
            ("x", self.width.to_string()),
            ("y", self.height.to_string()),
            ("a", if self.keep_aspect { "1" } else { "0" }.to_string()),
            (
                "forceIcon",
                if self.force_icon { "1" } else { "0" }.to_string(),
            ),
            ("mode", self.mode.as_str().to_string()),
        ]
    }
}

pub struct Previews<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn previews(&self) -> Previews<'_> {
        Previews { nc: self }
    }
}

impl Previews<'_> {
    /// Preview URL by [`file_id`](crate::files::FileEntry::file_id), which
    /// survives renames.
    pub fn url_for_file_id(&self, file_id: u64, opts: PreviewOptions) -> Result<Url> {
        let url = self.nc.url("index.php/core/preview")?;
        Ok(with_query(url, &[("fileId", &file_id.to_string())], opts))
    }

    pub fn url_for_path(&self, path: &str, opts: PreviewOptions) -> Result<Url> {
        let url = self.nc.url("index.php/core/preview.png")?;
        Ok(with_query(url, &[("file", path)], opts))
    }

    pub fn avatar_url(&self, user_id: &str, size: u32) -> Result<Url> {
        self.nc.url(&format!(
            "index.php/avatar/{}/{}",
            encode_segment(user_id),
            size
        ))
    }

    /// Preview URL for a file behind a public share token. Needs no credentials.
    pub fn public_preview_url(&self, share_token: &str, opts: PreviewOptions) -> Result<Url> {
        let url = self.nc.url(&format!(
            "index.php/apps/files_sharing/publicpreview/{}",
            encode_segment(share_token)
        ))?;
        Ok(with_query(url, &[], opts))
    }

    pub fn public_share_url(&self, share_token: &str) -> Result<Url> {
        self.nc.url(&format!("s/{}", encode_segment(share_token)))
    }

    pub fn public_download_url(&self, share_token: &str) -> Result<Url> {
        self.nc
            .url(&format!("s/{}/download", encode_segment(share_token)))
    }

    /// Fetch any of the URLs above, with credentials applied.
    ///
    /// `Ok(None)` means no preview exists: previews switched off instance-wide,
    /// no provider for the format, or generation has not run. `nc:has-preview`
    /// is an unreliable guard for this, since servers report `false` for files
    /// that preview fine.
    pub async fn fetch(&self, url: Url) -> Result<Option<Preview>> {
        let path = url.path().to_string();
        let rb = self.nc.request(Method::GET, url);

        match send_dav(rb, "GET", &path).await {
            Ok(resp) => {
                let bytes = resp.bytes().await?;
                Ok(Some(Preview {
                    content_type: sniff_image(&bytes),
                    bytes,
                }))
            }
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn with_query(mut url: Url, params: &[(&str, &str)], opts: PreviewOptions) -> Url {
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
        for (k, v) in opts.query() {
            q.append_pair(k, &v);
        }
    }
    url
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub bytes: Bytes,
    pub content_type: &'static str,
}

/// Identify an image from its leading bytes, since previews are not reliably
/// labelled. Anything unrecognised reads as JPEG, the server's default.
fn sniff_image(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Nextcloud {
        Nextcloud::builder("https://cloud.example.com")
            .basic_auth("alice", "pw")
            .build()
            .unwrap()
    }

    #[test]
    fn preview_by_id() {
        let url = client()
            .previews()
            .url_for_file_id(1234, PreviewOptions::square(256))
            .unwrap();

        assert_eq!(url.path(), "/index.php/core/preview");
        let q: Vec<_> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert!(q.contains(&("fileId".into(), "1234".into())));
        assert!(q.contains(&("x".into(), "256".into())));
        assert!(q.contains(&("y".into(), "256".into())));
        // a=1 keeps the aspect ratio
        assert!(q.contains(&("a".into(), "1".into())));
    }

    #[test]
    fn preview_options() {
        let opts = PreviewOptions::sized(64, 32)
            .keep_aspect(false)
            .force_icon(true)
            .mode(PreviewMode::Fill);
        let url = client().previews().url_for_file_id(1, opts).unwrap();
        let q: Vec<_> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        assert!(q.contains(&("x".into(), "64".into())));
        assert!(q.contains(&("y".into(), "32".into())));
        assert!(q.contains(&("a".into(), "0".into())));
        assert!(q.contains(&("forceIcon".into(), "1".into())));
        assert!(q.contains(&("mode".into(), "fill".into())));
    }

    #[test]
    fn preview_by_path_escaping() {
        let url = client()
            .previews()
            .url_for_path("/Music/a song #1.mp3", PreviewOptions::default())
            .unwrap();
        assert_eq!(url.path(), "/index.php/core/preview.png");
        let file = url
            .query_pairs()
            .find(|(k, _)| k == "file")
            .map(|(_, v)| v.to_string())
            .unwrap();
        assert_eq!(file, "/Music/a song #1.mp3");
    }

    #[test]
    fn avatar_url() {
        let url = client().previews().avatar_url("alice", 64).unwrap();
        assert_eq!(
            url.as_str(),
            "https://cloud.example.com/index.php/avatar/alice/64"
        );
    }

    #[test]
    fn avatar_url_escaping() {
        let nc = Nextcloud::builder("https://cloud.example.com")
            .build()
            .unwrap();
        let url = nc.previews().avatar_url("first last", 64).unwrap();
        assert!(url.as_str().ends_with("/index.php/avatar/first%20last/64"));
    }

    #[test]
    fn public_preview_url() {
        let p = client();
        let p = p.previews();
        assert_eq!(
            p.public_share_url("abc123").unwrap().as_str(),
            "https://cloud.example.com/s/abc123"
        );
        assert_eq!(
            p.public_download_url("abc123").unwrap().as_str(),
            "https://cloud.example.com/s/abc123/download"
        );
    }

    #[test]
    fn sniff() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(sniff_image(b"GIF89a"), "image/gif");
        assert_eq!(sniff_image(b"RIFF____WEBPVP8 "), "image/webp");
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(sniff_image(b""), "image/jpeg");
        assert_eq!(sniff_image(b"RIFF____WAVE"), "image/jpeg");
    }

    #[test]
    fn preview_subdir() {
        let nc = Nextcloud::builder("https://example.com/nextcloud")
            .build()
            .unwrap();
        assert_eq!(
            nc.previews().public_share_url("t").unwrap().as_str(),
            "https://example.com/nextcloud/s/t"
        );
    }
}
