//! MCP Apps metadata and durable host bindings.

use std::collections::BTreeMap;

use agena_secret_store::{FileSecretStore, SecretStore};
use base64::Engine as _;
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::protocol::{ReadResourceResult, ToolDescriptor};
use crate::{HttpAuth, ServerSpec};

pub use agena_domain::mcp_connections::{APP_METADATA_KEY, APP_RESULT_METADATA_KEY};
pub const APP_MIME_TYPE: &str = "text/html;profile=mcp-app";
pub const APP_DOCUMENT_LIMIT: usize = 1024 * 1024;
pub const APP_MESSAGE_LIMIT: usize = 1024 * 1024;

/// An absent visibility declaration defaults to both audiences. An explicit
/// declaration must include the requested audience.
pub fn tool_visible_to(tool: &ToolDescriptor, audience: &str) -> bool {
    tool.meta
        .as_ref()
        .and_then(|meta| meta.pointer("/ui/visibility"))
        .is_none_or(|visibility| {
            visibility
                .as_array()
                .is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some(audience)))
        })
}

pub fn app_resource_uri(tool: &ToolDescriptor) -> Option<&str> {
    let meta = tool.meta.as_ref()?;
    let uri = meta
        .pointer("/ui/resourceUri")
        .or_else(|| meta.get("ui/resourceUri"))?
        .as_str()?;
    (tool_visible_to(tool, "app") && is_app_uri(uri)).then_some(uri)
}

pub fn is_app_uri(uri: &str) -> bool {
    uri.len() <= 2048
        && uri.starts_with("ui://")
        && !uri.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        && url::Url::parse(uri).is_ok_and(|url| {
            url.has_host() && url.username().is_empty() && url.password().is_none()
        })
}

pub fn has_app_mime_type(mime: &str) -> bool {
    let mut parts = mime.split(';').map(str::trim);
    if !parts
        .next()
        .is_some_and(|media| media.eq_ignore_ascii_case("text/html"))
    {
        return false;
    }
    let mut profiles = parts.filter_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("profile")
            .then_some(value.trim())
    });
    matches!(profiles.next(), Some("mcp-app" | "\"mcp-app\"")) && profiles.next().is_none()
}

pub fn app_html(uri: &str, result: ReadResourceResult) -> Option<String> {
    let mut contents = result
        .contents
        .into_iter()
        .filter(|content| content.uri == uri);
    let content = contents.next()?;
    if contents.next().is_some() || !content.mime_type.as_deref().is_some_and(has_app_mime_type) {
        return None;
    }
    let html = match (content.text, content.blob) {
        (Some(text), None) => text,
        (None, Some(base64)) if base64.len() <= APP_DOCUMENT_LIMIT.div_ceil(3) * 4 => {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(base64)
                    .ok()?,
            )
            .ok()?
        }
        _ => return None,
    };
    (!html.trim().is_empty() && html.len() <= APP_DOCUMENT_LIMIT).then_some(html)
}

/// The signature binds this reference to one original session tool call and
/// the configured transport. The browser never supplies this record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppBinding {
    pub server: String,
    pub tool: String,
    pub uri: String,
    signature: String,
}

impl McpAppBinding {
    pub fn revision(&self) -> &str {
        &self.signature
    }

    pub async fn issue(
        session: i64,
        call: i64,
        server: &str,
        tool: &ToolDescriptor,
        spec: &ServerSpec,
    ) -> Option<Self> {
        let uri = app_resource_uri(tool)?;
        if session <= 0 || call <= 0 {
            return None;
        }
        let mut binding = Self {
            server: server.to_owned(),
            tool: tool.name.clone(),
            uri: uri.to_owned(),
            signature: String::new(),
        };
        let key = signing_key().await?;
        binding.signature =
            hex::encode(hmac::sign(key, &binding.message(session, call, spec)).as_ref());
        Some(binding)
    }

    pub async fn verifies(&self, session: i64, call: i64, spec: &ServerSpec) -> bool {
        let Some(key) = signing_key().await else {
            return false;
        };
        session > 0
            && call > 0
            && is_app_uri(&self.uri)
            && hex::decode(&self.signature).is_ok_and(|signature| {
                hmac::verify(key, &self.message(session, call, spec), &signature).is_ok()
            })
    }

    fn message(&self, session: i64, call: i64, spec: &ServerSpec) -> Vec<u8> {
        json!([
            1,
            session,
            call,
            self.server,
            self.tool,
            self.uri,
            spec.app_identity()
        ])
        .to_string()
        .into_bytes()
    }
}

async fn signing_key() -> Option<&'static hmac::Key> {
    static KEY: OnceCell<hmac::Key> = OnceCell::const_new();
    KEY.get_or_try_init(|| async {
        tokio::task::spawn_blocking(|| {
            let mut candidate = [0; 32];
            SystemRandom::new().fill(&mut candidate).ok()?;
            let key = FileSecretStore::new("agena.mcp-apps")
                .get_or_insert_secret("binding-signature-v1", &hex::encode(candidate))
                .ok()?;
            let key: [u8; 32] = hex::decode(key).ok()?.try_into().ok()?;
            Some(hmac::Key::new(hmac::HMAC_SHA256, &key))
        })
        .await
        .ok()
        .flatten()
        .ok_or(())
    })
    .await
    .ok()
}

impl ServerSpec {
    // This private identity is input to an HMAC and is never returned or
    // logged. Ordered headers/environment make it stable after reload.
    fn app_identity(&self) -> Value {
        match self {
            Self::Stdio {
                command,
                args,
                env,
                cwd,
                resources,
                ..
            } => json!([
                "stdio",
                command,
                args,
                env.iter().collect::<BTreeMap<_, _>>(),
                cwd,
                resources,
            ]),
            Self::Http {
                url, headers, auth, ..
            } => {
                let auth = match auth {
                    None => Value::Null,
                    Some(HttpAuth::Bearer(token)) => json!(["bearer", token]),
                    Some(HttpAuth::BearerFromEnv(name)) => {
                        json!(["environment", name, std::env::var(name).ok()])
                    }
                    Some(HttpAuth::BearerFromStore) => json!(["store"]),
                    Some(HttpAuth::OAuth {
                        scopes,
                        registration,
                    }) => json!([
                        "oauth",
                        crate::oauth_credential_binding(scopes, registration)
                    ]),
                    Some(HttpAuth::Custom(headers)) => {
                        json!(["custom", headers.iter().collect::<BTreeMap<_, _>>()])
                    }
                };
                json!([
                    "http",
                    url.as_str(),
                    headers.iter().collect::<BTreeMap<_, _>>(),
                    auth
                ])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::McpToolPolicy;

    fn descriptor(meta: Value) -> ToolDescriptor {
        serde_json::from_value(json!({"name":"render", "_meta":meta})).unwrap()
    }

    #[test]
    fn app_tool_metadata_keeps_visibility_scopes_independent() {
        let both = descriptor(json!({"ui":{"resourceUri":"ui://mcp/app"}}));
        assert_eq!(app_resource_uri(&both), Some("ui://mcp/app"));
        assert!(tool_visible_to(&both, "model"));

        let app_only =
            descriptor(json!({"ui":{"resourceUri":"ui://mcp/app", "visibility":["app"]}}));
        assert!(app_resource_uri(&app_only).is_some());
        assert!(!tool_visible_to(&app_only, "model"));
        let model_only =
            descriptor(json!({"ui":{"resourceUri":"ui://mcp/app", "visibility":["model"]}}));
        assert!(app_resource_uri(&model_only).is_none());
        assert!(tool_visible_to(&model_only, "model"));

        let legacy = descriptor(json!({"ui/resourceUri":"ui://mcp/app"}));
        assert_eq!(app_resource_uri(&legacy), Some("ui://mcp/app"));
        let invalid = descriptor(json!({"ui":{"resourceUri":"https://other/app"}}));
        assert!(app_resource_uri(&invalid).is_none());
    }

    #[test]
    fn html_requires_the_exact_declared_uri_and_profile() {
        let resource = |uri, mime, text| json!({"uri":uri,"mimeType":mime,"text":text});
        let document = |contents| serde_json::from_value(json!({"contents":contents})).unwrap();
        let uri = "ui://mcp/app";
        let exact = resource(uri, APP_MIME_TYPE, "<p>hello</p>");
        assert_eq!(
            app_html(uri, document(vec![exact.clone()])).as_deref(),
            Some("<p>hello</p>")
        );
        assert!(app_html(uri, document(vec![exact.clone(), exact])).is_none());
        assert!(app_html(uri, document(vec![resource(uri, "text/html", "hello")])).is_none());
        assert!(
            app_html(
                uri,
                document(vec![resource("ui://mcp/other", APP_MIME_TYPE, "hello")])
            )
            .is_none()
        );
    }

    #[test]
    fn signature_covers_the_session_tool_call_and_transport() {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &[1; 32]);
        let endpoint = |url: &str| ServerSpec::Http {
            url: url.parse().unwrap(),
            headers: Default::default(),
            auth: None,
            tool_policy: McpToolPolicy::default(),
        };
        let binding = McpAppBinding {
            server: "mcp".into(),
            tool: "render".into(),
            uri: "ui://mcp/app".into(),
            signature: String::new(),
        };
        let server = endpoint("https://mcp.test/server");
        let signed = hmac::sign(&key, &binding.message(92, 3, &server));
        assert!(hmac::verify(&key, &binding.message(92, 3, &server), signed.as_ref()).is_ok());
        assert!(hmac::verify(&key, &binding.message(93, 3, &server), signed.as_ref()).is_err());
        assert!(hmac::verify(&key, &binding.message(92, 4, &server), signed.as_ref()).is_err());
        assert!(
            hmac::verify(
                &key,
                &binding.message(92, 3, &endpoint("https://elsewhere.test/server")),
                signed.as_ref()
            )
            .is_err()
        );
    }
}
