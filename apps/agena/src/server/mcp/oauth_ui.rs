//! Standalone browser UI. Assets and theme tokens live with the Web frontend,
//! but authorization remains a native form that also works without JavaScript.

use std::{collections::BTreeMap, sync::LazyLock};

use axum::http::{HeaderMap, header};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use url::Url;

use super::{OFFLINE_ACCESS_SCOPE, ValidatedAuthorizationRequest, html_escape};

const PAGE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/page.html"
));
const CONSENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/consent.html"
));
const ERROR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/error.html"
));
const THEME: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/styles/theme.css"
));
const CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/oauth.css"
));
const SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/oauth.js"
));
const LOGO: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/assets/agena.svg"
));
const TRANSLATIONS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/agena-web/src/standalone/oauth/translations.json"
));

const LOCALES: [(&str, &str); 7] = [
    ("zh-CN", "简体中文"),
    ("en-US", "English"),
    ("es-ES", "Español"),
    ("fr-FR", "Français"),
    ("hi-IN", "हिन्दी"),
    ("ar-SA", "العربية"),
    ("pt-BR", "Português"),
];

static MESSAGES: LazyLock<BTreeMap<String, BTreeMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(TRANSLATIONS).expect("embedded OAuth translations must be valid JSON")
});
static SCRIPT_HASH: LazyLock<String> =
    LazyLock::new(|| STANDARD.encode(Sha256::digest(SCRIPT.as_bytes())));
static CATALOG_JSON: LazyLock<String> = LazyLock::new(|| {
    // Even inert script elements are terminated by an HTML </script> sequence.
    serde_json::to_string(&*MESSAGES)
        .expect("embedded OAuth translations must serialize")
        .replace('<', "\\u003c")
});
static FAVICON: LazyLock<String> = LazyLock::new(|| STANDARD.encode(LOGO));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Locale(&'static str);

impl Locale {
    fn supported(value: &str) -> Option<Self> {
        let language = value.trim().split(['-', '_']).next()?;
        LOCALES.iter().find_map(|(locale, _)| {
            locale
                .split('-')
                .next()
                .filter(|base| base.eq_ignore_ascii_case(language))
                .map(|_| Self(locale))
        })
    }

    pub(super) fn for_request(headers: &HeaderMap, explicit: Option<&str>) -> Self {
        if let Some(locale) = explicit.and_then(Self::supported) {
            return locale;
        }
        // Respect the browser's preference ordering, including q weights.
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .into_iter()
            .flat_map(|value| value.split(','))
            .filter_map(|value| {
                let mut parts = value.trim().split(';');
                let locale = Self::supported(parts.next()?)?;
                let quality = parts
                    .find_map(|part| part.trim().strip_prefix("q="))
                    .map(|value| value.parse::<f32>().ok())
                    .unwrap_or(Some(1.0))?;
                (quality > 0.0 && quality <= 1.0).then_some((locale, quality))
            })
            .reduce(|best, next| if next.1 > best.1 { next } else { best })
            .map(|(locale, _)| locale)
            .unwrap_or(Self("zh-CN"))
    }

    fn message(self, key: &str) -> &'static str {
        MESSAGES[self.0]
            .get(key)
            .expect("embedded OAuth template messages must exist")
    }
}

#[derive(Clone, Copy)]
pub(super) enum PasswordError {
    Invalid,
    Unavailable,
    Locked,
}

impl PasswordError {
    fn key(self) -> &'static str {
        match self {
            Self::Invalid => "invalid_password",
            Self::Unavailable => "password_unavailable",
            Self::Locked => "locked",
        }
    }
}

pub(super) fn script_hash() -> &'static str {
    &SCRIPT_HASH
}

pub(super) fn wants_html(headers: &HeaderMap) -> bool {
    let header_is = |name: &str, expected: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
    };
    (header_is("sec-fetch-mode", "navigate") && header_is("sec-fetch-dest", "document"))
        || headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(',').any(|value| {
                    let mut parts = value.trim().split(';');
                    parts
                        .next()
                        .is_some_and(|mime| mime.eq_ignore_ascii_case("text/html"))
                        && parts.all(|part| {
                            part.trim()
                                .strip_prefix("q=")
                                .is_none_or(|q| q.parse::<f32>().is_ok_and(|q| q > 0.0))
                        })
                })
            })
}

pub(super) fn consent(
    request: &ValidatedAuthorizationRequest,
    locale: Locale,
    error: Option<PasswordError>,
) -> String {
    let callback = Url::parse(&request.redirect_uri).expect("redirect URI was validated");
    let origin = callback.origin().ascii_serialization();
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(&origin);
    let mut hidden_fields = String::new();
    for (name, value) in [
        ("response_type", "code"),
        ("client_id", request.client_id.as_str()),
        ("redirect_uri", request.redirect_uri.as_str()),
        ("state", request.state.as_str()),
        ("code_challenge", request.code_challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", request.resource.as_str()),
        ("scope", request.scope.as_str()),
    ] {
        hidden_fields.push_str(&format!(
            "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
            html_escape(value)
        ));
    }
    let offline_permission = if request
        .scope
        .split_whitespace()
        .any(|scope| scope == OFFLINE_ACCESS_SCOPE)
    {
        template(
            "<li><span class=\"permission-icon\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\"><path d=\"M20 7v5h-5M4 17v-5h5m-3.2-5a7 7 0 0 1 11.5-1L20 9M4 15l2.7 3A7 7 0 0 0 18.2 17\"/></svg></span><div><p class=\"permission-title\" data-i18n=\"offline_title\">{{OFFLINE_TITLE}}</p><p class=\"permission-description\" data-i18n=\"offline_description\">{{OFFLINE_DESCRIPTION}}</p></div></li>",
            locale,
            &[],
        )
    } else {
        String::new()
    };
    let error_html = error
        .map(|error| {
            format!(
                "<div class=\"form-error\" id=\"password-error\" role=\"alert\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M12 7v6m0 4h.01\"/></svg><span data-i18n=\"{}\">{}</span></div>",
                error.key(),
                html_escape(locale.message(error.key()))
            )
        })
        .unwrap_or_default();
    let body = template(
        CONSENT,
        locale,
        &[
            (
                "CLIENT_NAME",
                html_escape(
                    request
                        .client_name
                        .as_deref()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or("MCP client"),
                ),
            ),
            ("CALLBACK_HOST", html_escape(host)),
            ("CALLBACK_ORIGIN", html_escape(&origin)),
            ("RESOURCE", html_escape(&request.resource)),
            ("SCOPES", html_escape(&request.scope)),
            ("OFFLINE_PERMISSION", offline_permission),
            ("HIDDEN_FIELDS", hidden_fields),
            ("ERROR", error_html),
            (
                "ERROR_DESCRIPTION",
                if error.is_some() {
                    " password-error"
                } else {
                    ""
                }
                .to_owned(),
            ),
            (
                "INVALID",
                if matches!(error, Some(PasswordError::Invalid)) {
                    "aria-invalid=\"true\""
                } else {
                    ""
                }
                .to_owned(),
            ),
        ],
    );
    page(locale, body, locale.message("title"))
}

pub(super) fn error(locale: Locale, code: &str, description: &str) -> String {
    let body = template(
        ERROR,
        locale,
        &[
            ("CODE", html_escape(code)),
            ("DESCRIPTION", html_escape(description)),
        ],
    );
    page(locale, body, locale.message("error_title"))
}

fn page(locale: Locale, body: String, title: &str) -> String {
    let options = LOCALES
        .iter()
        .map(|(code, label)| {
            format!(
                "<option value=\"{code}\" lang=\"{code}\"{}>{label}</option>",
                if *code == locale.0 { " selected" } else { "" }
            )
        })
        .collect();
    template(
        PAGE,
        locale,
        &[
            ("LANG", locale.0.to_owned()),
            (
                "DIRECTION",
                if locale.0 == "ar-SA" { "rtl" } else { "ltr" }.to_owned(),
            ),
            ("TITLE", html_escape(title)),
            ("FAVICON", FAVICON.clone()),
            ("THEME_CSS", THEME.to_owned()),
            ("PAGE_CSS", CSS.to_owned()),
            ("TRANSLATIONS", CATALOG_JSON.clone()),
            ("SCRIPT", SCRIPT.to_owned()),
            ("LOGO", LOGO.to_owned()),
            ("LANGUAGE_OPTIONS", options),
            ("THEME_LABEL", html_escape(locale.message("dark_theme"))),
            ("BODY", body),
        ],
    )
}

/// Interpolate only markers from the static template. Never rescan inserted
/// values: a registered client name may itself contain template syntax.
fn template(source: &str, locale: Locale, values: &[(&str, String)]) -> String {
    let mut rendered = String::with_capacity(source.len());
    let mut remaining = source;
    while let Some(start) = remaining.find("{{") {
        rendered.push_str(&remaining[..start]);
        let marker = &remaining[start + 2..];
        let end = marker
            .find("}}")
            .expect("embedded OAuth template marker must close");
        let key = &marker[..end];
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            rendered.push_str(value);
        } else {
            rendered.push_str(&html_escape(locale.message(&key.to_ascii_lowercase())));
        }
        remaining = &marker[end + 2..];
    }
    rendered.push_str(remaining);
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ValidatedAuthorizationRequest {
        ValidatedAuthorizationRequest {
            client_id: "client-1".into(),
            client_name: Some("Example client".into()),
            redirect_uri: "https://client.example.com/callback".into(),
            state: "<state>&\"'".into(),
            code_challenge: "pkce-challenge".into(),
            issuer: "https://agena.example.com".into(),
            resource: "https://agena.example.com/mcp".into(),
            scope: "agena:tools offline_access".into(),
        }
    }

    #[test]
    fn every_locale_has_the_same_messages_and_renders_both_pages() {
        let keys: Vec<_> = MESSAGES["en-US"].keys().collect();
        assert_eq!(MESSAGES.len(), LOCALES.len());
        for (locale, _) in LOCALES {
            assert_eq!(
                MESSAGES[locale].keys().collect::<Vec<_>>(),
                keys,
                "{locale}"
            );
            assert!(MESSAGES[locale].values().all(|value| !value.is_empty()));
            for html in [
                consent(&request(), Locale(locale), Some(PasswordError::Invalid)),
                error(Locale(locale), "invalid_client", "client is not registered"),
            ] {
                assert!(!html.contains("{{"), "{locale}");
                assert!(html.contains(&format!("lang=\"{locale}\"")));
                assert!(html.contains(if locale == "ar-SA" {
                    "dir=\"rtl\""
                } else {
                    "dir=\"ltr\""
                }));
            }
        }
    }

    #[test]
    fn untrusted_fields_cannot_inject_html_or_expand_template_markers() {
        let mut request = request();
        request.client_name =
            Some("<script>alert(1)</script>\"' {{HIDDEN_FIELDS}} {{SCRIPT}}".into());
        let html = consent(&request, Locale("en-US"), None);
        assert!(html.contains(
            "&lt;script&gt;alert(1)&lt;/script&gt;&quot;&#39; {{HIDDEN_FIELDS}} {{SCRIPT}}"
        ));
        assert!(html.contains("name=\"state\" value=\"&lt;state&gt;&amp;&quot;&#39;\""));
        assert_eq!(html.matches("name=\"client_id\"").count(), 1);
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(!html.contains("<script>alert(1)"));
        let html = error(
            Locale("en-US"),
            "<bad>",
            "</script><img src=x onerror=alert(1)>",
        );
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;bad&gt;"));
    }

    #[test]
    fn only_requested_permissions_are_present_and_password_is_never_prefilled() {
        let mut request = request();
        request.scope = "agena:tools".into();
        let html = consent(&request, Locale("en-US"), Some(PasswordError::Invalid));
        assert!(!html.contains("data-i18n=\"offline_title\""));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("password-hint password-error"));
        let input = html
            .split("id=\"password\"")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap();
        assert!(!input.contains("value="));
        assert!(html.contains("value=\"deny\" formnovalidate"));
        assert!(html.find("value=\"allow\"").unwrap() < html.find("value=\"deny\"").unwrap());
    }

    #[test]
    fn script_hash_covers_the_exact_inline_script() {
        let html = consent(&request(), Locale("en-US"), None);
        let script = html
            .split("<script>")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        assert_eq!(
            STANDARD.encode(Sha256::digest(script.as_bytes())),
            script_hash()
        );
        assert!(!CATALOG_JSON.contains('<'));
    }

    #[test]
    fn locale_selection_respects_quality_explicit_language_and_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_LANGUAGE,
            "de-DE,fr-FR;q=0.6,en;q=0.9,ar;q=0".parse().unwrap(),
        );
        assert_eq!(Locale::for_request(&headers, None), Locale("en-US"));
        assert_eq!(Locale::for_request(&headers, Some("ar")), Locale("ar-SA"));
        assert_eq!(
            Locale::for_request(&headers, Some("unknown")),
            Locale("en-US")
        );
        assert_eq!(
            Locale::for_request(&HeaderMap::new(), None),
            Locale("zh-CN")
        );
    }

    #[test]
    fn browser_errors_are_negotiated_without_changing_protocol_json() {
        let mut headers = HeaderMap::new();
        assert!(!wants_html(&headers));
        headers.insert(
            header::ACCEPT,
            "application/json, text/html;q=0".parse().unwrap(),
        );
        assert!(!wants_html(&headers));
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml,*/*;q=0.8".parse().unwrap(),
        );
        assert!(wants_html(&headers));
    }
}
