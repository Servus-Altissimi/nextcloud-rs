//! Classifying a file as playable media.
//!
//! Not every file carries a MIME type, and some storage backends report
//! `application/octet-stream` for all of them, so the name is the fallback.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    /// Classify by MIME type where there is one, else by the extension of
    /// `name`, which may be a bare name or a path. `None` if not media.
    ///
    /// ```
    /// use nextcloud::files::MediaKind;
    ///
    /// assert_eq!(MediaKind::detect("clip.dat", Some("video/mp4")), Some(MediaKind::Video));
    /// // No usable MIME type: fall back to the extension.
    /// assert_eq!(MediaKind::detect("/Music/track.flac", None), Some(MediaKind::Audio));
    /// assert_eq!(MediaKind::detect("notes.txt", None), None);
    /// ```
    pub fn detect(name: &str, mime: Option<&str>) -> Option<Self> {
        // several backends send this for everything
        if let Some(mime) = mime.filter(|m| !m.starts_with("application/octet-stream")) {
            match mime.split('/').next().unwrap_or_default() {
                "image" => return Some(MediaKind::Image),
                "video" => return Some(MediaKind::Video),
                "audio" => return Some(MediaKind::Audio),
                _ => {}
            }
        }

        let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "heic" | "heif"
            | "tif" | "tiff" | "ico" => Some(MediaKind::Image),
            "mp4" | "webm" | "mkv" | "mov" | "m4v" | "ogv" | "avi" | "mpg" | "mpeg" | "wmv" => {
                Some(MediaKind::Video)
            }
            "mp3" | "flac" | "ogg" | "oga" | "wav" | "m4a" | "opus" | "aac" | "wma" | "aiff" => {
                Some(MediaKind::Audio)
            }
            _ => None,
        }
    }

    /// Whether playback runs against a clock, meaning the file benefits from
    /// ranged streaming rather than being fetched whole.
    pub fn is_timed(self) -> bool {
        matches!(self, MediaKind::Video | MediaKind::Audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_first() {
        assert_eq!(
            MediaKind::detect("x.txt", Some("image/png")),
            Some(MediaKind::Image)
        );
        assert_eq!(
            MediaKind::detect("x.png", Some("video/mp4")),
            Some(MediaKind::Video)
        );
    }

    #[test]
    fn generic_mime_uses_extension() {
        assert_eq!(
            MediaKind::detect("song.mp3", Some("application/octet-stream")),
            Some(MediaKind::Audio)
        );
        assert_eq!(
            MediaKind::detect("nameless", Some("application/octet-stream")),
            None
        );
    }

    #[test]
    fn extension_case() {
        assert_eq!(
            MediaKind::detect("PHOTO.JPEG", None),
            Some(MediaKind::Image)
        );
        assert_eq!(MediaKind::detect("Clip.MKV", None), Some(MediaKind::Video));
    }

    #[test]
    fn full_paths() {
        assert_eq!(
            MediaKind::detect("/Music/Album/01 track.flac", None),
            Some(MediaKind::Audio)
        );
    }

    #[test]
    fn unclassified() {
        assert_eq!(MediaKind::detect("notes.md", None), None);
        assert_eq!(MediaKind::detect("archive.zip", None), None);
        assert_eq!(MediaKind::detect("Makefile", None), None);
        assert_eq!(MediaKind::detect("", None), None);
    }

    #[test]
    fn dotfile_dir() {
        assert_eq!(MediaKind::detect(".config", None), None);
    }
}
