//! Remote path arithmetic.
//!
//! Paths are `/`-separated and absolute from the user's files root. Normalised,
//! the root is `/`, every other path has a leading and no trailing slash, and
//! repeated separators collapse.

/// Normalise to a leading slash and no trailing slash. The root is `/`.
///
/// ```
/// use nextcloud::files::path;
/// assert_eq!(path::normalise("//a///b//"), "/a/b");
/// assert_eq!(path::normalise(""), "/");
/// ```
pub fn normalise(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 1);
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(segment);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// The containing directory, clamped at the root.
pub fn parent(path: &str) -> String {
    let normalised = normalise(path);
    match normalised.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => normalised[..index].to_string(),
    }
}

/// Append a child name to a directory. A `/` in the name is a separator.
pub fn join(dir: &str, name: &str) -> String {
    normalise(&format!("{dir}/{name}"))
}

pub fn name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// Cumulative segments, for breadcrumbs: `("b", "/a/b")`.
pub fn trail(path: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut accumulated = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        accumulated.push('/');
        accumulated.push_str(segment);
        out.push((segment.to_string(), accumulated.clone()));
    }
    out
}

/// Whether `path` is inside `dir`, at any depth.
pub fn contains(dir: &str, path: &str) -> bool {
    let dir = normalise(dir);
    let path = normalise(path);
    if dir == "/" {
        return path != "/";
    }
    // the separator check stops /ab claiming /abc
    path.starts_with(&dir) && path.as_bytes().get(dir.len()) == Some(&b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_twice() {
        for input in ["", "/", "a", "/a/", "//a//b//"] {
            let once = normalise(input);
            assert_eq!(normalise(&once), once, "input {input:?}");
        }
    }

    #[test]
    fn odd_names() {
        assert_eq!(normalise("/a b/c#1.mp3"), "/a b/c#1.mp3");
        assert_eq!(name("/a b/c#1.mp3"), "c#1.mp3");
        assert_eq!(parent("/a b/c#1.mp3"), "/a b");
    }

    #[test]
    fn join_slashes() {
        assert_eq!(join("/a/", "/b"), "/a/b");
        assert_eq!(join("", "b"), "/b");
        assert_eq!(join("/a", ""), "/a");
    }

    #[test]
    fn parent_at_root() {
        assert_eq!(parent("a"), "/");
        assert_eq!(parent("/a/"), "/");
    }

    #[test]
    fn contains_boundary() {
        assert!(contains("/photos", "/photos/a.jpg"));
        assert!(contains("/a", "/a/b/c"));
        assert!(contains("/", "/a"));
        assert!(!contains("/photos", "/photos2/a.jpg"));
        assert!(!contains("/ab", "/abc"));
        assert!(!contains("/photos", "/photos"));
        assert!(!contains("/a/b", "/a"));
        assert!(!contains("/", "/"));
    }

    #[test]
    fn trail_segments() {
        assert_eq!(
            trail("/a/b/c"),
            vec![
                ("a".to_string(), "/a".to_string()),
                ("b".to_string(), "/a/b".to_string()),
                ("c".to_string(), "/a/b/c".to_string()),
            ]
        );
        assert!(trail("/").is_empty());
        assert_eq!(trail("//a//b/").len(), 2);
    }
}
