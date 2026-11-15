//! Image-source metadata shared by plugin manifests and host presentation APIs.

/// Maximum UTF-8 size of one icon source, including a data URI's header.
pub const MAX_PLUGIN_ICON_SOURCE_BYTES: usize = 256 * 1024;

/// Return a supported image source without fetching it. A rejected source only
/// removes the optional presentation icon; it does not disable the plugin.
///
/// HTTP(S) sources and same-origin absolute paths are loaded by the browser.
/// Data URIs support base64 raster/SVG images and URI-encoded SVG. Rendering
/// hosts must use an image element rather than inserting SVG as document HTML.
pub fn normalize_plugin_icon(source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty()
        || source.len() > MAX_PLUGIN_ICON_SOURCE_BYTES
        || source
            .bytes()
            .any(|byte| byte <= 0x20 || byte == 0x7f || byte == b'\\')
    {
        return None;
    }
    if source.starts_with('/') {
        return (!source.starts_with("//")).then(|| source.to_owned());
    }
    let (scheme, _) = source.split_once(':')?;
    if scheme.eq_ignore_ascii_case("data") {
        return valid_image_data_uri(&source[5..]).then(|| source.to_owned());
    }
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    if !source[scheme.len() + 1..].starts_with("//") {
        return None;
    }
    let url = url::Url::parse(source).ok()?;
    (url.host_str().is_some() && url.username().is_empty() && url.password().is_none())
        .then(|| source.to_owned())
}

fn valid_image_data_uri(value: &str) -> bool {
    let Some((header, payload)) = value.split_once(',') else {
        return false;
    };
    if payload.is_empty() {
        return false;
    }
    let header = header.to_ascii_lowercase();
    let mut fields = header.split(';');
    let mime = fields.next().unwrap_or_default();
    if !matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/svg+xml"
            | "image/x-icon"
            | "image/vnd.microsoft.icon"
    ) {
        return false;
    }
    let mut charset = false;
    let mut base64 = false;
    for field in fields {
        // `base64` is the final encoding marker in a data URI.
        if base64 {
            return false;
        }
        match field {
            "charset=utf-8" | "charset=utf8" | "utf8" if !charset => charset = true,
            "base64" => base64 = true,
            _ => return false,
        }
    }
    if base64 {
        valid_base64(payload)
    } else {
        mime == "image/svg+xml" && valid_percent_encoding(payload)
    }
}

fn valid_base64(value: &str) -> bool {
    let body = value.trim_end_matches('=');
    let padding = value.len() - body.len();
    !body.is_empty()
        && padding <= 2
        && body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
        && if padding == 0 {
            value.len() % 4 != 1
        } else {
            value.len() % 4 == 0
        }
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes
                .get(index + 1..index + 3)
                .is_none_or(|digits| !digits.iter().all(u8::is_ascii_hexdigit))
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_browser_image_sources_without_fetching_them() {
        for source in [
            "https://example.com/icon.svg",
            "HTTP://localhost:3210/icons/plugin.png",
            "/plugin-assets/demo.board/r1/icon.svg",
            "data:image/png;base64,iVBORw0KGgo=",
            "data:image/jpeg;base64,/9j/",
            "data:image/gif;base64,R0lGODlh",
            "data:image/webp;base64,UklGRg==",
            "data:image/avif;base64,AAAA",
            "data:image/x-icon;base64,AAAB",
            "data:image/vnd.microsoft.icon;base64,AAAB",
            "data:image/svg+xml;base64,PHN2Zy8+",
            "data:image/png;base64,AA",
            "data:image/png;base64,AAA",
            "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E",
            "DATA:IMAGE/SVG+XML;CHARSET=UTF-8,%3Csvg%2F%3E",
            "data:image/svg+xml;utf8;base64,PHN2Zy8+",
        ] {
            assert_eq!(
                normalize_plugin_icon(source).as_deref(),
                Some(source),
                "{source}"
            );
        }
        assert_eq!(
            normalize_plugin_icon("  https://example.com/icon.png  ").as_deref(),
            Some("https://example.com/icon.png")
        );
    }

    #[test]
    fn rejects_non_image_schemes_ambiguous_urls_and_malformed_data() {
        for source in [
            "",
            "javascript:alert(1)",
            "file:///tmp/icon.png",
            "blob:https://example.com/icon",
            "ftp://example.com/icon.png",
            "//example.com/icon.png",
            "/\\example.com/icon.png",
            "icons/plugin.png",
            "https:example.com/icon.png",
            "https:/example.com/icon.png",
            "https://",
            "https://user:password@example.com/icon.png",
            "https://example.com/icon\n.png",
            "https://example.com/icon name.png",
            "data:text/html;base64,PHN2Zy8+",
            "data:image/png,AAAA",
            "data:image/png;base64,",
            "data:image/png;base64,A",
            "data:image/png;base64,AA=",
            "data:image/png;base64,AAA===",
            "data:image/png;base64,AA=A",
            "data:image/png;base64,AA-_",
            "data:image/png;unknown=1;base64,AAAA",
            "data:image/png;base64;charset=utf-8,AAAA",
            "data:image/svg+xml;charset=utf-8;charset=utf-8,%3Csvg%2F%3E",
            "data:image/svg+xml,%ZZ",
            "data:image/svg+xml,%3",
        ] {
            assert_eq!(normalize_plugin_icon(source), None, "{source}");
        }
        assert!(
            normalize_plugin_icon(&format!("/{}", "a".repeat(MAX_PLUGIN_ICON_SOURCE_BYTES)))
                .is_none()
        );
    }
}
