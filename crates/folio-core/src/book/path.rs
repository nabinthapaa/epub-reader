//! Href arithmetic inside an EPUB container.
//!
//! EPUB hrefs are relative, percent-encoded and may carry a fragment, so
//! resolving `../Text/ch3.xhtml#note-2` against the chapter that contains the
//! link is its own small problem. Kept pure and dependency-free so it can be
//! tested on its own.

/// Split `href` into its path and fragment parts.
pub fn split_fragment(href: &str) -> (&str, Option<&str>) {
    match href.split_once('#') {
        Some((path, frag)) if !frag.is_empty() => (path, Some(frag)),
        Some((path, _)) => (path, None),
        None => (href, None),
    }
}

/// True when the href points outside the book (and so belongs in a browser).
pub fn is_external(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:", "ftp://", "data:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Percent-decode a UTF-8 href. Invalid escapes are left as written.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Collapse `.` and `..` segments and drop any leading slash, so every path is
/// expressed the same way as the archive's own entry names.
pub fn normalize(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// Resolve `href` relative to the file it appears in.
pub fn resolve(base_file: &str, href: &str) -> String {
    if href.starts_with('/') {
        return normalize(href);
    }
    let dir = match base_file.rfind('/') {
        Some(idx) => &base_file[..idx],
        None => "",
    };
    if dir.is_empty() {
        normalize(href)
    } else {
        normalize(&format!("{dir}/{href}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_fragments() {
        assert_eq!(split_fragment("ch1.xhtml#n3"), ("ch1.xhtml", Some("n3")));
        assert_eq!(split_fragment("ch1.xhtml"), ("ch1.xhtml", None));
        assert_eq!(split_fragment("#top"), ("", Some("top")));
        assert_eq!(split_fragment("ch1.xhtml#"), ("ch1.xhtml", None));
    }

    #[test]
    fn detects_external_links() {
        assert!(is_external("https://example.org"));
        assert!(is_external("MAILTO:a@b.c"));
        assert!(!is_external("../Text/ch2.xhtml"));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("a%20b.xhtml"), "a b.xhtml");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize("OEBPS/./Text/ch1.xhtml"), "OEBPS/Text/ch1.xhtml");
        assert_eq!(normalize("OEBPS/Text/../img/a.png"), "OEBPS/img/a.png");
        assert_eq!(normalize("/EPUB/ch1.xhtml"), "EPUB/ch1.xhtml");
    }

    #[test]
    fn resolves_relative_to_the_containing_file() {
        assert_eq!(
            resolve("OEBPS/Text/ch1.xhtml", "../Images/fig.png"),
            "OEBPS/Images/fig.png"
        );
        assert_eq!(
            resolve("OEBPS/Text/ch1.xhtml", "ch2.xhtml"),
            "OEBPS/Text/ch2.xhtml"
        );
        assert_eq!(resolve("ch1.xhtml", "ch2.xhtml"), "ch2.xhtml");
        assert_eq!(
            resolve("OEBPS/Text/ch1.xhtml", "/EPUB/ch9.xhtml"),
            "EPUB/ch9.xhtml"
        );
    }
}
