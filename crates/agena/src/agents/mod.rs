//! Layered agent-profile registry.
//!
//! Built-ins are overlaid by user and project Markdown profiles, then by
//! profiles registered dynamically at runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::BTreeMap, fmt};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    Project,
    User,
    Default,
}

impl AsRef<str> for AgentScope {
    fn as_ref(&self) -> &str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Default => "default",
        }
    }
}

impl fmt::Display for AgentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSelectionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl AgentSelectionConfig {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontmatter {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "AgentSelectionConfig::is_empty")]
    pub defaults: AgentSelectionConfig,
    #[serde(default, skip_serializing_if = "AgentToolsConfig::is_empty")]
    pub tools: AgentToolsConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentToolsConfig {
    /// Exact model-facing tool names exposed to this profile. Empty means the
    /// profile inherits the runtime catalog without an additional allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

impl AgentToolsConfig {
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub name: String,
    pub frontmatter: AgentFrontmatter,
    pub prompt: String,
    pub source_path: Option<PathBuf>,
    pub scope: AgentScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "AgentSelectionConfig::is_empty")]
    pub defaults: AgentSelectionConfig,
    #[serde(default, skip_serializing_if = "AgentToolsConfig::is_empty")]
    pub tools: AgentToolsConfig,
    pub scope: AgentScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed agent: {0}")]
    Malformed(String),
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("unknown subagent profile: {0}")]
    UnknownProfile(String),
}

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug, Clone)]
pub struct SubagentRegistry {
    inner: Arc<RwLock<AgentRegistryInner>>,
    discovery: Arc<AgentDiscoveryRoots>,
}

#[derive(Debug, Default)]
struct AgentRegistryInner {
    by_name: BTreeMap<String, AgentProfile>,
    runtime_by_name: BTreeMap<String, AgentProfile>,
    runtime_disabled_names: std::collections::BTreeSet<String>,
}

#[derive(Debug, Default)]
struct AgentDiscoveryRoots {
    workspace_root: Option<PathBuf>,
    user_root: Option<PathBuf>,
}

impl Default for SubagentRegistry {
    fn default() -> Self {
        let registry = Self {
            inner: Arc::new(RwLock::new(AgentRegistryInner::default())),
            discovery: Arc::new(AgentDiscoveryRoots::default()),
        };
        registry.reload_disk();
        registry
    }
}

impl SubagentRegistry {
    pub fn discover(workspace_root: &Path, user_root: Option<&Path>) -> Self {
        let registry = Self {
            inner: Arc::new(RwLock::new(AgentRegistryInner::default())),
            discovery: Arc::new(AgentDiscoveryRoots {
                workspace_root: Some(workspace_root.to_path_buf()),
                user_root: user_root.map(Path::to_path_buf),
            }),
        };
        registry.reload_disk();
        registry
    }

    pub fn reload_disk(&self) {
        let mut inner = self.inner.write();
        inner.by_name.clear();
        for profile in default_profiles() {
            agents_upsert(&mut inner.by_name, profile, AgentScope::Default);
        }
        if let Some(root) = self.discovery.user_root.as_deref() {
            load_agent_directory(&mut inner.by_name, &root.join("agents"), AgentScope::User);
        }
        if let Some(root) = self.discovery.workspace_root.as_deref() {
            load_agent_directory(
                &mut inner.by_name,
                &root.join(".agena").join("agents"),
                AgentScope::Project,
            );
        }
        for profile in inner.runtime_by_name.values().cloned().collect::<Vec<_>>() {
            // Runtime registration is the highest-precedence layer. `scope`
            // remains provenance metadata; it must not demote a live profile
            // below a disk profile with the same name.
            inner.by_name.insert(profile.name.clone(), profile);
        }
        for name in inner.runtime_disabled_names.clone() {
            inner.by_name.remove(&name);
        }
    }

    pub fn register_runtime(&self, mut profile: AgentProfile) -> AgentResult<()> {
        profile.name = profile.name.trim().to_string();
        if profile.name.is_empty() {
            return Err(AgentError::Malformed(
                "runtime agent name must not be empty".to_string(),
            ));
        }
        normalize_selection(&mut profile.frontmatter.defaults);
        normalize_allowed_tools(&mut profile.frontmatter.tools.allow);
        validate_profile_permission(&profile)?;
        let mut inner = self.inner.write();
        inner.runtime_disabled_names.remove(&profile.name);
        inner
            .runtime_by_name
            .insert(profile.name.clone(), profile.clone());
        inner.by_name.insert(profile.name.clone(), profile);
        Ok(())
    }

    pub fn disable_runtime(&self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut inner = self.inner.write();
        inner.runtime_by_name.remove(name);
        inner.runtime_disabled_names.insert(name.to_string());
        inner.by_name.remove(name);
    }

    pub fn remove_runtime(&self, name: &str) -> bool {
        let name = name.trim();
        let mut inner = self.inner.write();
        let removed_profile = inner.runtime_by_name.remove(name).is_some();
        let removed_disabled = inner.runtime_disabled_names.remove(name);
        let removed = removed_profile || removed_disabled;
        drop(inner);
        if removed {
            self.reload_disk();
        }
        removed
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().by_name.keys().cloned().collect()
    }

    pub fn list(&self) -> Vec<AgentProfile> {
        let inner = self.inner.read();
        let mut seen: BTreeMap<String, AgentProfile> = BTreeMap::new();
        for profile in inner.by_name.values() {
            if !profile.is_exposed() {
                continue;
            }
            seen.entry(profile.name.clone())
                .or_insert_with(|| profile.clone());
        }
        seen.into_values().collect()
    }

    pub fn list_descriptors(&self) -> Vec<AgentDescriptor> {
        let mut descriptors = self
            .list()
            .into_iter()
            .map(AgentDescriptor::from)
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    pub fn get(&self, name: &str) -> Option<AgentProfile> {
        self.inner.read().by_name.get(name).cloned()
    }

    pub fn require(&self, name: &str) -> AgentResult<AgentProfile> {
        self.get(name)
            .ok_or_else(|| AgentError::UnknownProfile(name.to_string()))
    }
}

fn load_agent_directory(
    by_name: &mut BTreeMap<String, AgentProfile>,
    directory: &Path,
    scope: AgentScope,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(
                target: "agena::agents",
                directory = %directory.display(),
                "failed to read agent profile directory: {error}"
            );
            return;
        }
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        })
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let Some(name) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            tracing::warn!(
                target: "agena::agents",
                path = %path.display(),
                "ignored agent profile with an invalid UTF-8 or empty file name"
            );
            continue;
        };
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(
                    target: "agena::agents",
                    path = %path.display(),
                    "failed to read agent profile: {error}"
                );
                continue;
            }
        };
        match AgentProfile::from_raw(raw.as_str(), name, scope) {
            Ok(mut profile) => {
                profile.source_path = Some(path);
                agents_upsert(by_name, profile, scope);
            }
            Err(error) => tracing::warn!(
                target: "agena::agents",
                path = %path.display(),
                "ignored malformed agent profile: {error}"
            ),
        }
    }
}

fn agents_upsert(
    by_name: &mut BTreeMap<String, AgentProfile>,
    profile: AgentProfile,
    scope: AgentScope,
) {
    let key = profile.name.clone();
    match by_name.get(&key) {
        Some(existing) => {
            if scope_priority(scope) >= scope_priority(existing.scope) {
                by_name.insert(key, profile);
            }
        }
        None => {
            by_name.insert(key, profile);
        }
    }
}

impl AgentProfile {
    pub fn is_exposed(&self) -> bool {
        self.name != "compaction"
    }

    pub fn from_raw(raw: &str, default_name: &str, scope: AgentScope) -> AgentResult<Self> {
        if default_name.trim().is_empty() {
            return Err(AgentError::Malformed(
                "agent file name (stem) must not be empty".into(),
            ));
        }
        let (mut frontmatter, prompt) = parse_frontmatter(raw)?;
        normalize_selection(&mut frontmatter.defaults);
        normalize_allowed_tools(&mut frontmatter.tools.allow);
        let profile = Self {
            name: default_name.trim().to_string(),
            frontmatter,
            prompt,
            source_path: None,
            scope,
        };
        validate_profile_permission(&profile)?;
        Ok(profile)
    }
}

impl From<AgentProfile> for AgentDescriptor {
    fn from(profile: AgentProfile) -> Self {
        Self {
            name: profile.name,
            description: profile.frontmatter.description,
            permission: profile.frontmatter.permission,
            defaults: profile.frontmatter.defaults,
            tools: profile.frontmatter.tools,
            scope: profile.scope,
            source_path: profile.source_path,
        }
    }
}

fn normalize_allowed_tools(tools: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    *tools = std::mem::take(tools)
        .into_iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty() && seen.insert(tool.clone()))
        .collect();
}

fn normalize_selection(selection: &mut AgentSelectionConfig) {
    for value in [
        &mut selection.provider,
        &mut selection.adapter,
        &mut selection.model,
        &mut selection.thinking_mode,
        &mut selection.speed_mode,
        &mut selection.verbosity,
    ] {
        *value = value
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
}

fn validate_profile_permission(profile: &AgentProfile) -> AgentResult<()> {
    crate::agent::Agent::new(
        profile.name.clone(),
        crate::permission::PermissionPolicy::allow_all(),
        crate::permission::ToolPermissionPolicy::allow_all(),
    )
    .try_apply_permission_config(&profile.frontmatter.permission)
    .map(|_| ())
    .map_err(|error| {
        AgentError::Malformed(format!(
            "agent '{}' has invalid permissions: {error}",
            profile.name
        ))
    })
}

pub(crate) fn allowed_tools(profile: &AgentProfile) -> Vec<String> {
    match profile.name.as_str() {
        "compaction" => vec!["__agena_compaction_no_tools__".to_string()],
        _ => profile.frontmatter.tools.allow.clone(),
    }
}

fn parse_frontmatter(raw: &str) -> AgentResult<(AgentFrontmatter, String)> {
    let normalized = raw.replace("\r\n", "\n");
    let Some(stripped) = normalized.strip_prefix("---\n") else {
        return Ok((AgentFrontmatter::default(), normalized.trim().to_string()));
    };
    let Some(end) = stripped.find("\n---") else {
        return Err(AgentError::Malformed(
            "frontmatter missing closing '---'".into(),
        ));
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter: AgentFrontmatter = if yaml.trim().is_empty() {
        AgentFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml)?
    };
    Ok((frontmatter, body))
}

fn scope_priority(scope: AgentScope) -> u8 {
    match scope {
        AgentScope::Default => 0,
        AgentScope::User => 1,
        AgentScope::Project => 2,
    }
}

fn default_permission(
    workspace_write: crate::permission::PermissionMode,
    external: crate::permission::PermissionMode,
    tags: &[(&str, crate::permission::PermissionMode)],
) -> crate::agent::AgentPermissionConfig {
    crate::agent::PermissionConfig {
        path: Some(crate::agent::PathPermissionConfig {
            workspace: Some(crate::agent::PathAccessModes {
                read: Some(crate::permission::PermissionMode::Allow),
                write: Some(workspace_write),
            }),
            external: Some(crate::agent::PathAccessModes {
                read: Some(external),
                write: Some(external),
            }),
            ..Default::default()
        }),
        tools: Some(crate::agent::ToolPermissionConfig {
            tags: tags
                .iter()
                .map(|(name, mode)| ((*name).to_string(), *mode))
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn default_profiles() -> Vec<AgentProfile> {
    vec![
        compaction_profile(),
        default_profile(
            "build",
            "Primary coding agent for normal end-to-end implementation work.",
            default_permission(
                crate::permission::PermissionMode::Ask,
                crate::permission::PermissionMode::Ask,
                &[
                    ("interactive", crate::permission::PermissionMode::Allow),
                    ("planning", crate::permission::PermissionMode::Allow),
                    ("task", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("goal", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are the primary engineering agent. Own the task end to end, choose tools pragmatically, delegate when it helps, preserve surrounding behavior, and avoid reverting unrelated work.",
        ),
        default_profile(
            "general",
            "General-purpose delegated agent for broad research and mixed tasks.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("interactive", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a general-purpose delegated agent. Investigate broadly, combine code reading with focused web research when useful, and return evidence-backed conclusions without making workspace edits unless explicitly allowed.",
        ),
        default_profile(
            "explore",
            "Read-only codebase explorer for fast repo analysis.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a focused read-only engineering explorer. Gather evidence quickly, inspect code paths, summarize findings concisely, and do not make edits.",
        ),
        default_profile(
            "scout",
            "Read-only external research agent for docs, APIs, and dependency behavior.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a read-only research agent for external documentation, APIs, and dependency behavior. Prefer direct evidence from docs, source, or fetched pages, separate verified facts from inference, and do not modify the user's workspace.",
        ),
        default_profile(
            "implement",
            "Editing agent for making targeted code changes.",
            default_permission(
                crate::permission::PermissionMode::Ask,
                crate::permission::PermissionMode::Ask,
                &[],
            ),
            "You are a pragmatic implementation agent. Make the requested code changes, preserve surrounding behavior, adapt to concurrent edits, and avoid reverting unrelated work.",
        ),
        default_profile(
            "verify",
            "Validation agent for targeted testing and regression checks.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a verification agent. Run focused checks, inspect outputs critically, look for regressions, and report the remaining risks plainly.",
        ),
        default_profile(
            "planner",
            "Planning agent for read-only decomposition and execution strategy.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("planning", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("interactive", crate::permission::PermissionMode::Allow),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a planning agent. Break work into concrete steps, surface assumptions and blockers, and keep the output actionable. Prefer read-only investigation unless the user explicitly asks to execute.",
        ),
        default_profile(
            "reviewer",
            "Code review agent focused on bugs, risks, and missing tests.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("read_only", crate::permission::PermissionMode::Allow),
                    ("filesystem_read", crate::permission::PermissionMode::Allow),
                    ("network", crate::permission::PermissionMode::Allow),
                    ("internet", crate::permission::PermissionMode::Allow),
                    ("discovery", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a strict code review agent. Prioritize correctness issues, behavioral regressions, and test gaps. Findings come first; summaries are secondary.",
        ),
    ]
}

fn compaction_profile() -> AgentProfile {
    let deny = crate::permission::PermissionMode::Deny;
    AgentProfile {
        name: "compaction".to_string(),
        frontmatter: AgentFrontmatter {
            description: "Agent used only for conversation compaction.".to_string(),
            permission: crate::agent::PermissionConfig {
                path: Some(crate::agent::PathPermissionConfig {
                    workspace: Some(crate::agent::PathAccessModes {
                        read: Some(deny),
                        write: Some(deny),
                    }),
                    external: Some(crate::agent::PathAccessModes {
                        read: Some(deny),
                        write: Some(deny),
                    }),
                    ..Default::default()
                }),
                network: Some(crate::agent::NetworkPermissionConfig {
                    internet: Some(deny),
                    private: Some(deny),
                    loopback: Some(deny),
                    ..Default::default()
                }),
                tools: Some(crate::agent::ToolPermissionConfig::default()),
            },
            defaults: AgentSelectionConfig::default(),
            tools: AgentToolsConfig::default(),
        },
        prompt: "You are Agena's conversation compaction agent. Summarize only the transcript and context provided by the user message. Preserve the user's current objective, explicit constraints, decisions already made, important files or commands, tool results, pending work, blockers, and open questions. Do not call tools, do not invent facts, and do not mention the act of compaction. Return a concise Markdown summary with stable section headings.".to_string(),
        source_path: None,
        scope: AgentScope::Default,
    }
}

fn default_profile(
    name: &str,
    description: &str,
    permission: crate::agent::AgentPermissionConfig,
    prompt: &str,
) -> AgentProfile {
    AgentProfile {
        name: name.to_string(),
        frontmatter: AgentFrontmatter {
            description: description.to_string(),
            permission,
            defaults: AgentSelectionConfig::default(),
            tools: AgentToolsConfig {
                allow: default_profile_tools(name),
            },
        },
        prompt: format!(
            "{prompt} When the user asks what tools are available, whether a tool exists, or how to inspect tool usage, do not answer from memory. Inspect the live tools gateway first. `tools_list`, `tools_search`, `tools_tags`, and `tools_help` are gateway functions; only catalog targets are passed to `tools_call`. Before every `tools_call`, call `tools_help` for that exact target, then make one `tools_call`; its help preflight is consumed by that call. The names `tools_help` and `tools_call` are top-level gateway functions, not values for the `tool` argument of `tools_call`; pass the dotted catalog target name returned by the catalog, such as `web.search`."
        ),
        source_path: None,
        scope: AgentScope::Default,
    }
}

fn default_profile_tools(name: &str) -> Vec<String> {
    let tools: &[&str] = match name {
        "explore" => &[
            "agena.fs.read",
            "agena.fs.glob",
            "agena.fs.grep",
            "agena.code.search_ast",
            "agena.code.syntax_tree",
            "agena.lsp.servers",
            "agena.lsp.definition",
            "agena.lsp.references",
            "agena.lsp.hover",
            "agena.lsp.diagnostics",
            "agena.shell.run",
            "agena.shell.list",
            "agena.shell.logs",
            "agena.web.search",
            "agena.web.fetch",
            "agena.session.get",
        ],
        "scout" => &[
            "agena.fs.read",
            "agena.fs.glob",
            "agena.fs.grep",
            "agena.web.search",
            "agena.web.fetch",
            "agena.mcp.resources.list",
            "agena.mcp.resources.read",
            "agena.mcp.prompts.list",
            "agena.mcp.prompts.get",
            "agena.session.get",
        ],
        "verify" | "reviewer" => &[
            "agena.fs.read",
            "agena.fs.glob",
            "agena.fs.grep",
            "agena.code.search_ast",
            "agena.code.syntax_tree",
            "agena.lsp.servers",
            "agena.lsp.definition",
            "agena.lsp.references",
            "agena.lsp.hover",
            "agena.lsp.diagnostics",
            "agena.shell.run",
            "agena.shell.list",
            "agena.shell.logs",
            "agena.web.search",
            "agena.web.fetch",
            "agena.session.get",
        ],
        "planner" => &[
            "agena.fs.read",
            "agena.fs.glob",
            "agena.fs.grep",
            "agena.code.search_ast",
            "agena.code.syntax_tree",
            "agena.lsp.servers",
            "agena.lsp.definition",
            "agena.lsp.references",
            "agena.lsp.hover",
            "agena.lsp.diagnostics",
            "agena.plan.get",
            "agena.plan.set",
            "agena.plan.update",
            "agena.plan.clear",
            "agena.interaction.ask",
            "agena.session.get",
            "agena.web.search",
            "agena.web.fetch",
        ],
        // Primary, general and implementation profiles intentionally inherit
        // the full live catalog; execution permissions remain the authority.
        _ => &[],
    };
    tools.iter().map(|tool| (*tool).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_is_immediately_usable() {
        let registry = SubagentRegistry::default();
        assert!(registry.get("build").is_some());
        assert!(registry.get("explore").is_some());
        assert!(registry.get("compaction").is_some());
    }

    #[test]
    fn project_agent_file_overrides_user_and_default_profiles() {
        let root = std::env::temp_dir().join(format!(
            "agena-agent-registry-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let workspace = root.join("workspace");
        let user = root.join("user");
        std::fs::create_dir_all(workspace.join(".agena/agents")).expect("project agents dir");
        std::fs::create_dir_all(user.join("agents")).expect("user agents dir");
        std::fs::write(
            user.join("agents/explore.md"),
            "---\ndescription: user explorer\n---\nuser prompt",
        )
        .expect("user profile");
        std::fs::write(
            workspace.join(".agena/agents/explore.md"),
            "---\ndescription: project explorer\ntools:\n  allow:\n    - agena.fs.read\n---\nproject prompt",
        )
        .expect("project profile");

        let registry = SubagentRegistry::discover(&workspace, Some(&user));
        let profile = registry.require("explore").expect("explore profile");
        assert_eq!(profile.scope, AgentScope::Project);
        assert_eq!(profile.frontmatter.description, "project explorer");
        assert_eq!(profile.prompt, "project prompt");
        assert_eq!(profile.frontmatter.tools.allow, vec!["agena.fs.read"]);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_overrides_survive_reload_and_removal_restores_lower_layer() {
        let registry = SubagentRegistry::default();
        let mut runtime = registry.require("explore").expect("default explore");
        runtime.prompt = "runtime prompt".to_string();
        runtime.scope = AgentScope::Default;
        registry.register_runtime(runtime).expect("runtime profile");

        registry.reload_disk();
        assert_eq!(
            registry.require("explore").expect("runtime explore").prompt,
            "runtime prompt"
        );
        assert!(registry.remove_runtime("explore"));
        assert_ne!(
            registry
                .require("explore")
                .expect("default restored")
                .prompt,
            "runtime prompt"
        );
    }

    #[test]
    fn runtime_disable_masks_and_removal_restores_lower_profile() {
        let registry = SubagentRegistry::default();
        assert!(registry.get("explore").is_some());

        registry.disable_runtime("explore");
        assert!(registry.get("explore").is_none());

        assert!(registry.remove_runtime("explore"));
        assert!(registry.get("explore").is_some());
    }

    #[test]
    fn malformed_profile_permissions_are_rejected_before_registration() {
        let raw = "---\npermission:\n  path:\n    rules:\n      '<unknown>/secret': allow\n---\nunsafe prompt";
        assert!(AgentProfile::from_raw(raw, "unsafe", AgentScope::Project).is_err());

        let registry = SubagentRegistry::default();
        let mut profile = registry.require("explore").expect("default explore");
        profile.frontmatter.permission.path = Some(crate::agent::PathPermissionConfig {
            rules: indexmap::IndexMap::from([(
                "<unknown>/secret".to_string(),
                crate::agent::PathAccessRuleConfig::Shorthand("allow".to_string()),
            )]),
            ..Default::default()
        });
        assert!(registry.register_runtime(profile).is_err());
        assert_eq!(
            registry.require("explore").expect("default remains").scope,
            AgentScope::Default
        );
    }

    #[test]
    fn read_only_profile_permissions_use_real_tool_tags() {
        let profile = SubagentRegistry::default()
            .require("explore")
            .expect("explore profile");
        let mut permission = crate::agent::PermissionConfig::global_default();
        permission.merge_from(profile.frontmatter.permission);
        let agent = crate::agent::Agent::new(
            "explore",
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&permission)
        .expect("valid explore permission");

        assert!(matches!(
            agent.authorize_tool(
                "agena.web.search",
                None,
                &[
                    crate::plugin::sdk::ToolTag::ReadOnly,
                    crate::plugin::sdk::ToolTag::Internet,
                ],
            ),
            crate::permission::PermissionDecision::Allow
        ));
        assert!(matches!(
            agent.authorize_tool(
                "agena.shell.run",
                Some("cargo check"),
                &[crate::plugin::sdk::ToolTag::Shell],
            ),
            crate::permission::PermissionDecision::Ask { .. }
        ));
    }
}
