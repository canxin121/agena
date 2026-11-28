use std::borrow::Cow;

use base64::Engine as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use super::*;

pub(super) const EXTENSION_ID: &str = "io.modelcontextprotocol/skills";
const MAX_FILES: usize = 512;
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSkill {
    pub uri: String,
    pub frontmatter: serde_json::Map<String, Value>,
    pub resources: McpSkillResources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpSkillResources {
    Files(Vec<McpSkillFile>),
    Dynamic(McpDynamicSkill),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpDynamicSkill {
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSkillFile {
    pub uri: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSkillPage {
    pub skills: Vec<McpSkill>,
    pub next_cursor: Option<String>,
}

impl McpSkill {
    pub fn base_uri(&self) -> &str {
        self.uri.strip_suffix("SKILL.md").unwrap_or_default()
    }

    fn validate(&self) -> McpResult<()> {
        let invalid = || McpError::Malformed("invalid MCP skill manifest".into());
        let Some(name) = self.frontmatter.get("name").and_then(Value::as_str) else {
            return Err(invalid());
        };
        let description = self.frontmatter.get("description").and_then(Value::as_str);
        validate_resource_uri(&self.uri)?;
        let directory = self.base_uri().trim_end_matches('/');
        let directory_name = directory.rsplit('/').next().unwrap_or_default();
        if !self.base_uri().ends_with('/')
            || agena_skills::validate_name(name).is_err()
            || !percent_encoding::percent_decode_str(directory_name)
                .decode_utf8()
                .is_ok_and(|segment| segment == name)
            || !description
                .is_some_and(|value| !value.trim().is_empty() && value.chars().count() <= 1024)
        {
            return Err(invalid());
        }
        let McpSkillResources::Files(files) = &self.resources else {
            return Ok(());
        };
        if files.len() > MAX_FILES {
            return Err(McpError::Malformed(
                "MCP skill has more than 512 files".into(),
            ));
        }
        let mut uris = BTreeSet::new();
        let mut total = 0_u64;
        for file in files {
            validate_resource_uri(&file.uri)?;
            let valid_sha256 = file.digest.strip_prefix("sha256:").is_some_and(|hex| {
                hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            });
            if !file.uri.starts_with(self.base_uri()) || !valid_sha256 || !uris.insert(&file.uri) {
                return Err(invalid());
            }
            total = total.saturating_add(file.size);
        }
        if total > MAX_BYTES {
            return Err(McpError::Malformed("MCP skill files exceed 16 MiB".into()));
        }
        if !uris.contains(&self.uri) {
            return Err(invalid());
        }
        Ok(())
    }

    /// Identifies the exact set of files advertised by this server for this skill.
    pub fn version(&self, server: &str) -> McpResult<String> {
        let McpSkillResources::Files(files) = &self.resources else {
            return Err(McpError::Malformed(
                "MCP skill publishes dynamic content without file digests".into(),
            ));
        };
        let mut sorted = files.iter().collect::<Vec<_>>();
        sorted.sort_unstable_by(|a, b| a.uri.cmp(&b.uri));
        let manifest = serde_json::to_vec(&(server, &self.uri, sorted))?;
        Ok(hex::encode(Sha256::digest(manifest)))
    }

    /// Return the manifest entry, or `None` for a dynamic file inside this skill.
    pub fn expected_file(&self, uri: &str) -> McpResult<Option<&McpSkillFile>> {
        validate_resource_uri(uri)?;
        match &self.resources {
            McpSkillResources::Files(files) => files
                .iter()
                .find(|file| file.uri == uri)
                .map(Some)
                .ok_or_else(|| {
                    McpError::Malformed(
                        "requested file is absent from the MCP skill manifest".into(),
                    )
                }),
            McpSkillResources::Dynamic(_)
                if self.base_uri().ends_with('/') && uri.starts_with(self.base_uri()) =>
            {
                Ok(None)
            }
            McpSkillResources::Dynamic(_) => Err(McpError::Malformed(
                "requested file is outside the dynamic MCP skill directory".into(),
            )),
        }
    }
}

fn validate_resource_uri(uri: &str) -> McpResult<()> {
    let invalid = || McpError::Malformed("invalid MCP skill resource URI".into());
    let parsed = Url::parse(uri).map_err(|_| invalid())?;
    let Some((_, path)) = uri.split_once(':') else {
        return Err(invalid());
    };
    if parsed.cannot_be_a_base()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || path
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        || path.trim_start_matches('/').split('/').any(|segment| {
            let segment = percent_encoding::percent_decode_str(segment).collect::<Vec<_>>();
            matches!(segment.as_slice(), b"" | b"." | b"..")
                || segment
                    .iter()
                    .any(|&byte| matches!(byte, b'/' | b'\\') || byte.is_ascii_control())
        })
    {
        return Err(invalid());
    }
    Ok(())
}

fn skill_file_bytes<'a>(
    uri: &str,
    expected: Option<&McpSkillFile>,
    result: &'a ReadResourceResult,
) -> McpResult<Cow<'a, [u8]>> {
    let invalid = || {
        McpError::Malformed(
            "MCP skill read must return one bounded resource at the requested URI".into(),
        )
    };
    let limit = expected.map_or(MAX_BYTES, |file| file.size.min(MAX_BYTES));
    let [content] = result.contents.as_slice() else {
        return Err(invalid());
    };
    if content.uri != uri {
        return Err(invalid());
    }
    let bytes = match (&content.text, &content.blob) {
        (Some(text), None) => Cow::Borrowed(text.as_bytes()),
        (None, Some(blob)) if blob.len() as u64 <= 4 * limit.div_ceil(3) => Cow::Owned(
            base64::engine::general_purpose::STANDARD
                .decode(blob)
                .map_err(|_| invalid())?,
        ),
        _ => return Err(invalid()),
    };
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    if let Some(file) = expected
        && (bytes.len() as u64 != file.size
            || file.digest != format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
    {
        return Err(McpError::Malformed(
            "MCP skill file does not match its listed size and digest".into(),
        ));
    }
    Ok(bytes)
}

async fn extension_request<T>(
    server: &ConnectedServer,
    method: &str,
    params: Value,
    directory: bool,
) -> McpResult<T>
where
    T: DeserializeOwned + Send + 'static,
{
    require_capability(server, "resources")?;
    let supported = server.peer.peer_info().is_some_and(|info| {
        info.capabilities
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get(EXTENSION_ID))
            .is_some_and(|settings| {
                !directory || settings.get("directoryRead").and_then(Value::as_bool) == Some(true)
            })
    });
    if !supported {
        return Err(McpError::CapabilityUnavailable {
            server: server.name.clone(),
            capability: if directory {
                "MCP Skills directoryRead"
            } else {
                EXTENSION_ID
            }
            .into(),
        });
    }
    let request = ClientRequest::CustomRequest(CustomRequest::new(method, Some(params)));
    let response = request_with_elicitation(server, request, None, method).await?;
    let payload = match response {
        ServerResult::CustomResult(result) => result.0,
        ServerResult::ListResourcesResult(result) if directory => serde_json::to_value(result)?,
        _ => return Err(ServiceError::UnexpectedResponse.into()),
    };
    if payload
        .get("resultType")
        .is_some_and(|kind| kind != "complete")
    {
        return Err(McpError::Malformed(
            "MCP Skills response did not contain a complete result".into(),
        ));
    }
    let redactions = server.redactions.clone();
    project_payload(move || {
        let payload = redactions.payload(payload)?;
        serde_json::from_value(payload).map_err(Into::into)
    })
    .await?
}

impl McpConnectionManager {
    /// Read one page of the negotiated MCP Skills catalog without fetching skill bodies.
    pub async fn list_skills(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<McpSkillPage> {
        let server = self.get(server).await?;
        let params = cursor.as_ref().map_or_else(
            || serde_json::json!({}),
            |cursor| serde_json::json!({"cursor": cursor}),
        );
        let page: McpSkillPage = extension_request(&server, "skills/list", params, false).await?;
        if page.next_cursor.is_some() && page.next_cursor == cursor {
            return Err(McpError::Malformed(
                "MCP skill listing repeated its cursor".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for skill in &page.skills {
            skill.validate()?;
            if !seen.insert(&skill.uri) {
                return Err(McpError::Malformed(
                    "MCP skill listing repeated a skill URI".into(),
                ));
            }
        }
        Ok(page)
    }

    /// Resolve a skill by its SKILL.md URI, including entries omitted from the catalog.
    pub async fn get_skill(&self, server: &str, uri: &str) -> McpResult<McpSkill> {
        #[derive(Deserialize)]
        struct SkillResult {
            skill: McpSkill,
        }

        validate_resource_uri(uri)?;
        let server = self.get(server).await?;
        let params = serde_json::json!({"uri": uri});
        let result: SkillResult = extension_request(&server, "skills/get", params, false).await?;
        if result.skill.uri != uri {
            return Err(McpError::Malformed(
                "MCP server returned a different skill URI".into(),
            ));
        }
        result.skill.validate()?;
        Ok(result.skill)
    }

    /// Read a file of this skill, verifying its digest and size when supplied.
    pub async fn read_skill_file(
        &self,
        server: &str,
        skill: &McpSkill,
        uri: &str,
        handler: Option<&dyn McpElicitationHandler>,
    ) -> McpResult<ReadResourceResult> {
        let file = skill.expected_file(uri)?.cloned();
        let frontmatter = (uri == skill.uri).then(|| skill.frontmatter.clone());
        let mut result = self
            .read_resource_with_elicitation(server, uri, handler)
            .await?;
        let uri = uri.to_owned();
        project_payload(move || {
            let bytes = skill_file_bytes(&uri, file.as_ref(), &result)?;
            if let Some(expected) = frontmatter {
                let invalid = || {
                    McpError::Malformed(
                        "MCP skill document does not match its advertised frontmatter".into(),
                    )
                };
                let document = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                let actual =
                    agena_skills::parse_document_frontmatter(document).map_err(|_| invalid())?;
                if actual != expected {
                    return Err(invalid());
                }
                if let Cow::Owned(bytes) = bytes {
                    let content = &mut result.contents[0];
                    content.text = Some(String::from_utf8(bytes).map_err(|_| invalid())?);
                    content.blob = None;
                }
            }
            Ok(result)
        })
        .await?
    }

    pub async fn list_resource_directory(
        &self,
        server: &str,
        uri: &str,
        cursor: Option<String>,
    ) -> McpResult<ListResourcesResult> {
        validate_resource_uri(uri)?;
        let server = self.get(server).await?;
        let mut params = serde_json::json!({"uri": uri});
        if let Some(cursor) = &cursor {
            params["cursor"] = serde_json::json!(cursor);
        }
        let page: ListResourcesResult =
            extension_request(&server, "resources/directory/read", params, true).await?;
        if page.next_cursor.is_some() && page.next_cursor == cursor {
            return Err(McpError::Malformed(
                "MCP directory listing repeated its cursor".into(),
            ));
        }
        let prefix = format!("{uri}/");
        let mut seen = BTreeSet::new();
        for item in &page.resources {
            validate_resource_uri(&item.uri)?;
            if !item
                .uri
                .strip_prefix(&prefix)
                .is_some_and(|child| !child.contains('/'))
                || !seen.insert(&item.uri)
            {
                return Err(McpError::Malformed(
                    "MCP directory listing returned an invalid child".into(),
                ));
            }
        }
        Ok(page)
    }
}
