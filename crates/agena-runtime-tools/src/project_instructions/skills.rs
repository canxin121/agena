use std::{
    collections::BTreeSet,
    io::Read,
    path::Path,
    sync::Arc,
};

use agena_domain::{AccessKind, PermissionDecision};
use agena_plugin_host::{
    AgentPluginFormat, AgentPluginSkill,
    agent_plugin::{ClaudeAgentMemoryScope, ClaudeAgentOptions},
    agent_plugin_data_path,
    sdk::PluginKey,
};
use agena_skills::{
    MAX_DOCUMENT_BYTES, SkillInvocation, SkillManager, SkillResourceBase, SkillSummary,
};

use crate::tool::ToolExecutor;

const MAX_PRELOADED_BYTES: usize = 128 * 1024;
const MAX_CLAUDE_AGENT_MEMORY_LINES: usize = 200;
const MAX_CLAUDE_AGENT_MEMORY_BYTES: usize = 25 * 1024;

impl ToolExecutor {
    pub async fn deepseek_file_skills(&self, names: &[String]) -> Vec<AgentPluginSkill> {
        if names.is_empty() || self.is_global() {
            return Vec::new();
        }
        let requested = names.iter().cloned().collect::<BTreeSet<_>>();
        match super::INSTRUCTION_READERS
            .run({
                let executor = self.clone();
                move || executor.read_deepseek_file_skills(&requested)
            })
            .await
        {
            Ok(skills) => skills,
            Err(error) => {
                tracing::warn!(%error, "DeepSeek file skill loader failed");
                Vec::new()
            }
        }
    }

    fn read_deepseek_file_skills(&self, requested: &BTreeSet<String>) -> Vec<AgentPluginSkill> {
        let permissions = self.sandbox_permission_snapshot();
        let allowed = |path: &Path| {
            let actual = crate::canonicalize_mutation_path(path);
            matches!(
                permissions.authorize_path_access(AccessKind::Read, &actual),
                PermissionDecision::Allow
            )
        };
        let mut skills = Vec::new();
        for plugin in self.plugin_manager().plugins() {
            if self.ensure_not_cancelled().is_err() {
                break;
            }
            let key = plugin.key();
            if !self.plugin_is_enabled(&key) {
                continue;
            }
            let Some(package) = plugin
                .agent_plugin()
                .filter(|package| package.format == AgentPluginFormat::DeepSeekHarness)
            else {
                continue;
            };
            let plugin_id = key.to_string();
            let version = &plugin.manifest.version;
            for dir in &package.skill_dirs {
                if self.ensure_not_cancelled().is_err() {
                    break;
                }
                let Some(dir_name) = dir.file_name().and_then(std::ffi::OsStr::to_str) else {
                    continue;
                };
                if !requested.contains(dir_name) {
                    continue;
                }
                if !dir.starts_with(&package.root)
                    || dir.canonicalize().ok().as_deref() != Some(dir.as_path())
                    || !allowed(dir)
                {
                    continue;
                }
                let skill_path = dir.join("SKILL.md");
                if !allowed(&skill_path) {
                    continue;
                }
                let Some(document) = read_regular_skill_document(&skill_path) else {
                    continue;
                };
                let Ok(parsed) = agena_skills::parse_document(&document, Some(dir_name)) else {
                    continue;
                };
                if !parsed.diagnostics.is_empty() || parsed.body.trim().is_empty() {
                    continue;
                }
                let Ok(frontmatter) = agena_skills::parse_document_frontmatter(&document) else {
                    continue;
                };
                let Some(invocation) = parse_deepseek_invocation(&frontmatter) else {
                    continue;
                };
                skills.push(AgentPluginSkill {
                    plugin_id: plugin_id.clone(),
                    version: version.clone(),
                    name: parsed.metadata.name,
                    description: parsed.metadata.description,
                    instructions: parsed.body,
                    aliases: parsed.metadata.aliases,
                    invocation: Some(invocation),
                    resource_base: Some(SkillResourceBase::Directory { path: dir.clone() }),
                });
            }
        }
        skills
    }

    pub async fn preload_claude_agent_skills(
        &self,
        plugin: &PluginKey,
        skills: &[String],
    ) -> String {
        if skills.is_empty() || self.is_global() || !self.plugin_is_enabled(plugin) {
            return String::new();
        }
        let plugin = plugin.clone();
        let skills = skills.to_vec();
        match super::INSTRUCTION_READERS
            .run({
                let executor = self.clone();
                move || executor.read_claude_agent_skills(&plugin, &skills)
            })
            .await
        {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(%error, "Claude role skill loader failed");
                String::new()
            }
        }
    }

    fn read_claude_agent_skills(&self, plugin: &PluginKey, skills: &[String]) -> String {
        let active = self
            .plugin_manager()
            .plugins()
            .iter()
            .filter(|entry| self.plugin_is_enabled(&entry.key()))
            .collect::<Vec<_>>();
        let packages = active
            .iter()
            .filter_map(|entry| Some((entry.key(), entry.agent_plugin()?)))
            .collect::<Vec<_>>();
        let command_settings = active
            .iter()
            .find(|entry| entry.key().to_string() == "agena.commands")
            .map(|entry| entry.configured_plugin().settings());
        let command_setting = |name: &str| command_settings.as_ref()?.get(name).cloned();
        let permissions = self.sandbox_permission_snapshot();
        let workspace = self.workspace_root();
        let library = SkillManager::new(
            workspace.to_path_buf(),
            agena_storage_layout::StorageLayout::current()
                .root()
                .join("skills"),
        )
        .with_exact_skill_dirs(
            packages
                .iter()
                .flat_map(|(_, package)| package.skill_dirs.iter().cloned())
                .collect(),
        )
        .with_additional_roots(
            command_setting("additional_roots")
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
        )
        .with_disabled_names(
            command_setting("disabled")
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
        )
        .with_read_filter(Arc::new(move |path| {
            let actual = crate::canonicalize_mutation_path(path);
            matches!(
                permissions.authorize_path_access(AccessKind::Read, &actual),
                PermissionDecision::Allow
            )
        }));
        let Ok(catalog) = library.list() else {
            return String::new();
        };
        let owner = |skill: &SkillSummary| {
            let dir = Path::new(&skill.path).parent()?;
            packages
                .iter()
                .find(|(_, package)| package.skill_dirs.iter().any(|path| path == dir))
        };
        let mut text = String::new();
        let mut seen = std::collections::BTreeSet::new();
        for reference in skills {
            if self.ensure_not_cancelled().is_err() {
                break;
            }
            let (namespace, name) = reference
                .split_once(':')
                .map_or((None, reference.as_str()), |(namespace, name)| {
                    (Some(namespace), name)
                });
            let available = || {
                catalog
                    .skills
                    .iter()
                    .filter(|skill| skill.enabled && skill.valid && skill.name == name)
            };
            let selected = match namespace {
                Some(namespace) => {
                    let mut matches = available().filter(|skill| {
                        owner(skill).is_some_and(|(key, package)| {
                            key.to_string() == namespace || package.name == namespace
                        })
                    });
                    matches.next().filter(|_| matches.next().is_none())
                }
                None => available()
                    .find(|skill| owner(skill).is_some_and(|(key, _)| key == plugin))
                    .or_else(|| available().find(|skill| !skill.shadowed)),
            };
            let Some(skill) = selected else {
                tracing::warn!(%plugin, skill = %reference, "Claude role skill is unavailable");
                continue;
            };
            if !seen.insert(&skill.id) {
                continue;
            }
            let Ok(detail) = library.get(&skill.id) else {
                continue;
            };
            let Ok(parsed) = agena_skills::parse_document(&detail.document, None) else {
                continue;
            };
            let dir = Path::new(&skill.path).parent().unwrap_or(workspace);
            let mut body = parsed
                .body
                .replace("${CLAUDE_SKILL_DIR}", &dir.to_string_lossy());
            if let Some((_, package)) = owner(skill) {
                body = body.replace("${CLAUDE_PLUGIN_ROOT}", &package.root.to_string_lossy());
                match package.expand_claude_prompt_options(&body) {
                    Ok(expanded) => body = expanded,
                    Err(error) => {
                        tracing::warn!(%plugin, skill = %reference, %error, "could not render Claude role skill");
                        continue;
                    }
                }
            }
            let section = format!("\n\n# Preloaded skill {reference}\n\n{body}");
            if text.len() + section.len() > MAX_PRELOADED_BYTES {
                tracing::warn!(%plugin, skill = %reference, "Claude role skill exceeds the preload budget");
                continue;
            }
            text.push_str(&section);
        }
        text
    }

    pub async fn preload_claude_agent_memory(
        &self,
        plugin: &PluginKey,
        agent: &ClaudeAgentOptions,
    ) -> String {
        let Some(scope) = agent.memory else {
            return String::new();
        };
        if self.is_global() || !self.plugin_is_enabled(plugin) {
            return String::new();
        }
        let plugin = plugin.clone();
        let role_name = agent.role_name.clone();
        match super::INSTRUCTION_READERS
            .run({
                let executor = self.clone();
                move || executor.read_claude_agent_memory(&plugin, &role_name, scope)
            })
            .await
        {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(%error, "Claude role memory loader failed");
                String::new()
            }
        }
    }

    fn read_claude_agent_memory(
        &self,
        plugin: &PluginKey,
        role_name: &str,
        scope: ClaudeAgentMemoryScope,
    ) -> String {
        let Some(slug) = claude_agent_memory_slug(role_name) else {
            return String::new();
        };
        let workspace = self.workspace_root();
        let (scope_label, dir) = match scope {
            ClaudeAgentMemoryScope::Project => (
                "project",
                workspace.join(".claude").join("agent-memory").join(&slug),
            ),
            ClaudeAgentMemoryScope::Local => (
                "local",
                workspace
                    .join(".claude")
                    .join("agent-memory-local")
                    .join(&slug),
            ),
            ClaudeAgentMemoryScope::User => (
                "user",
                agent_plugin_data_path(workspace, &plugin.to_string())
                    .join("agent-memory")
                    .join(&slug),
            ),
        };
        let memory_file = dir.join("MEMORY.md");
        let permissions = self.sandbox_permission_snapshot();
        let allowed = |path: &Path| {
            let actual = crate::canonicalize_mutation_path(path);
            matches!(
                permissions.authorize_path_access(AccessKind::Read, &actual),
                PermissionDecision::Allow
            )
        };
        if !allowed(&dir) || !allowed(&memory_file) {
            return String::new();
        }
        let mut section = format!(
            "\n\n# Agent memory ({scope_label})\n\nPersistent memory directory: {}\nRecord durable cross-session learnings for this role in {}.",
            dir.display(),
            memory_file.display()
        );
        if let Some(document) = read_regular_skill_document(&memory_file) {
            let excerpt = truncate_claude_agent_memory(&document);
            if !excerpt.trim().is_empty() {
                section.push_str("\n\n## MEMORY.md\n\n");
                section.push_str(&excerpt);
            }
        }
        section
    }
}

fn claude_agent_memory_slug(name: &str) -> Option<String> {
    let slug: String = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('.').to_owned();
    (!slug.is_empty() && slug != "." && slug != ".." && slug.len() <= 128).then_some(slug)
}

fn truncate_claude_agent_memory(document: &str) -> String {
    let mut out = String::new();
    let mut truncated = false;
    for (index, line) in document.lines().enumerate() {
        if index >= MAX_CLAUDE_AGENT_MEMORY_LINES {
            truncated = true;
            break;
        }
        let extra = usize::from(!out.is_empty()) + line.len();
        if out.len() + extra > MAX_CLAUDE_AGENT_MEMORY_BYTES {
            truncated = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if truncated {
        out.push_str("\n\n[MEMORY.md truncated to 200 lines / 25 KiB]");
    }
    out
}

fn read_regular_skill_document(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_DOCUMENT_BYTES as u64
    {
        return None;
    }
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW);
    }
    let file = open.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn parse_deepseek_invocation(
    frontmatter: &serde_json::Map<String, serde_json::Value>,
) -> Option<SkillInvocation> {
    if frontmatter.contains_key("userInvocable")
        || frontmatter.contains_key("disableModelInvocation")
    {
        return None;
    }
    let user_invocable = match frontmatter.get("user-invocable") {
        None => true,
        Some(serde_json::Value::Bool(value)) => *value,
        Some(_) => return None,
    };
    let disable_model = match frontmatter.get("disable-model-invocation") {
        None => false,
        Some(serde_json::Value::Bool(value)) => *value,
        Some(_) => return None,
    };
    Some(SkillInvocation {
        model_invocable: !disable_model,
        user_invocable,
    })
}
