use agena_mcp_client::McpSkillResources;
use agena_plugin_host::sdk::{PluginErrorKind, host_api::AskUserRequest};
use serde_json::json;

use super::*;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput)]
#[input(trim("server", "uri"), non_empty("server", "uri"))]
#[serde(deny_unknown_fields)]
pub(super) struct McpResourceDirectoryInput {
    pub server: String,
    pub uri: String,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput)]
#[input(trim("server", "uri"), non_empty("server", "uri"))]
#[serde(deny_unknown_fields)]
pub(super) struct McpSkillReadInput {
    /// The server identifier or plugin:<installed plugin ID>:<server name>.
    server: String,
    /// The URI of the skill's SKILL.md.
    uri: String,
    /// A supporting file URI under the skill's directory on this server.
    #[serde(default)]
    file_uri: Option<String>,
    /// For skills with a file manifest, use the version returned by the SKILL.md read.
    #[serde(default)]
    version: Option<String>,
}

impl McpPlugin {
    pub(super) async fn resolve_skill_server(&self, selector: &str) -> SdkResult<String> {
        let Some((plugin, name)) = selector
            .strip_prefix("plugin:")
            .and_then(|value| value.split_once(':'))
        else {
            return Ok(selector.to_owned());
        };
        let name = agena_plugin_host::agent_plugin::agent_plugin_mcp_server_name(plugin, name);
        if !self.manager.is_plugin_server(&name).await {
            return Err(PluginError::invalid_params(
                "Unknown installed plugin MCP server",
            ));
        }
        Ok(name)
    }

    pub(super) async fn list_remote_skills(
        &self,
        input: &McpServerInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let server = self.resolve_skill_server(&input.server).await?;
        let manager = self.authorize_server(&server).await?;
        let page = manager
            .list_skills(&server, input.cursor.clone())
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let mut skills = Vec::with_capacity(page.skills.len());
        for skill in page.skills {
            let files = match &skill.resources {
                McpSkillResources::Files(files) => Some(files.len()),
                McpSkillResources::Dynamic(_) => None,
            };
            skills.push(json!({
                "uri": skill.uri,
                "name": skill.frontmatter["name"],
                "description": skill.frontmatter["description"],
                "file_count": files,
                "dynamic": files.is_none(),
            }));
        }
        let result = json!({
            "server": input.server,
            "skills": skills,
            "next_cursor": page.next_cursor,
        });
        Ok(ToolInvokeOutput::from_parts(
            format!("MCP {}/skills", input.server),
            format!("{} skills", skills.len()),
            result.to_string(),
            Some(result),
            Default::default(),
            Vec::new(),
        ))
    }

    pub(super) async fn read_remote_skill(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &McpSkillReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if context.is_global() || context.session_id <= 0 || context.call_id <= 0 {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "MCP skills require a project session with user interaction",
            ));
        }
        let uri = input.file_uri.as_deref().unwrap_or(&input.uri);
        if agena_mcp_client::apps::is_app_uri(uri) {
            return Err(PluginError::invalid_params(
                "Open this MCP App in its transcript card.",
            ));
        }
        let server = self.resolve_skill_server(&input.server).await?;
        let manager = self.authorize_server(&server).await?;
        let skill = manager
            .get_skill(&server, &input.uri)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let files = match &skill.resources {
            McpSkillResources::Files(files) => {
                Some(files.iter().map(|file| &file.uri).collect::<Vec<_>>())
            }
            McpSkillResources::Dynamic(_) => None,
        };
        let dynamic = files.is_none();
        let version = if dynamic {
            None
        } else {
            if input.file_uri.is_some() && input.version.is_none() {
                return Err(PluginError::invalid_params(
                    "A supporting file in a manifest requires the version from a verified SKILL.md read.",
                ));
            }
            Some(
                skill
                    .version(&server)
                    .map_err(|error| PluginError::invalid_params_error(&error))?,
            )
        };
        if input
            .version
            .as_ref()
            .is_some_and(|known| version.as_ref() != Some(known))
        {
            return Err(PluginError::invalid_params(
                "MCP skill version changed. Read and approve its current SKILL.md before reading its files.",
            ));
        }
        skill
            .expected_file(uri)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("MCP interaction host is not initialized"))?;
        let freshness = if dynamic {
            "\n\nThe server generates this content at read time. Approval applies to this read."
        } else {
            ""
        };
        let answer = agena_async::permission_wait::waiting(host.ask_user(AskUserRequest {
            title: "Read MCP skill content".into(),
            kind: "mcp_skill".into(),
            prompt: format!(
                "Allow content from MCP server '{}' to be read into this session?\n\nSkill {}\nFile {uri}{freshness}",
                input.server, input.uri,
            ),
            options: vec!["Allow once".into(), "Decline".into()],
            ..Default::default()
        })).await?;
        if answer.cancelled || answer.timed_out || answer.reply != "Allow once" {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "MCP skill read declined",
            ));
        }
        let result = manager
            .read_skill_file(&server, &skill, uri, Some(self))
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if result
            .contents
            .iter()
            .any(|item| apps::is_app_content(&item.uri, item.mime_type.as_deref()))
        {
            return Err(PluginError::invalid_params(
                "Open this MCP App in its transcript card.",
            ));
        }
        let body = read_resource_output(&input.server, uri, result)?;
        let manifest = json!({
            "server": input.server,
            "skill_uri": input.uri,
            "file_uri": uri,
            "base_uri": skill.base_uri(),
            "version": version,
            "files": files,
            "dynamic": dynamic,
        });
        let summary = if dynamic {
            "Dynamic MCP skill file"
        } else {
            "Verified MCP skill file"
        };
        let guidance = if dynamic {
            "Generated content. Use mcp.skills.read with the same server and skill URI. Each file read requires approval."
        } else {
            "Verified content. Use mcp.skills.read with the same server, skill URI, and version for supporting files."
        };
        let text = format!(
            "Skill content from MCP server '{}'. {guidance}\n{manifest}\n\n{}",
            input.server, body.output_text,
        );
        let mut payload = manifest;
        if let Some(content) = body.payload {
            payload["contents"] = content["contents"].clone();
        }
        Ok(ToolInvokeOutput::from_parts(
            format!("MCP {}/skill", input.server),
            summary,
            text,
            Some(payload),
            Default::default(),
            body.attachments,
        ))
    }
}
