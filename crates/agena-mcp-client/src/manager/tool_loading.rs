//! Track the catalogs and selections of imported on-demand MCP servers.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::*;

pub const MCP_TOOL_LOADER: &str = "load_tools";

#[derive(Clone)]
pub struct McpOnDemandConfig {
    pub source_name: String,
    pub description_limit: usize,
}

/// The effective workspace and permanent identity of the owning agent.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct McpOnDemandScope {
    pub session: i64,
    pub workspace: PathBuf,
}

pub(super) type SessionCatalogs = HashMap<(McpOnDemandScope, String), SessionCatalog>;

pub(super) struct SessionCatalog {
    started_at: u64,
    snapshot: Option<McpOnDemandCatalog>,
}

#[derive(Clone)]
pub struct McpOnDemandCatalog {
    observation: u64,
    pub offered: Arc<BTreeMap<String, ToolDescriptor>>,
    pub loaded: BTreeSet<String>,
}

pub struct McpOnDemandServer {
    pub id: String,
    pub source_name: String,
    pub catalog: Option<McpOnDemandCatalog>,
    namespace: String,
    description_limit: usize,
}

impl McpOnDemandServer {
    pub fn alias(&self, raw_name: &str) -> String {
        public_tool_name(&self.namespace, raw_name)
    }

    pub fn qualified_alias(&self, raw_name: &str) -> String {
        format!("agena.mcp.{}", public_tool_name(&self.id, raw_name))
    }

    pub fn loader(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: MCP_TOOL_LOADER.to_owned(),
            title: None,
            aliases: vec![self.alias(MCP_TOOL_LOADER)],
            description: Some(format!(
                "Load selected tools from MCP server {} into this session.",
                self.source_name
            )),
            before_help: None,
            after_help: Some(
                "Use the exact names from the on-demand catalog. To retrieve the catalog, call mcp.tools.search with on_demand_catalog set to true.".into(),
            ),
            input_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "tools": {
                        "description": "Exact tool names from the server's on-demand catalog",
                        "type": "array",
                        "minItems": 1,
                        "items": {"type": "string"}
                    }
                },
                "required": ["tools"],
                "additionalProperties": false
            })),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: Vec::new(),
            meta: None,
        }
    }

    pub fn summary(&self, description: &str) -> String {
        let line = description
            .trim()
            .split('\n')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if line.encode_utf16().count() <= self.description_limit {
            return line;
        }
        let mut end = 0;
        let mut remaining = self.description_limit.saturating_sub(3);
        for (index, character) in line.char_indices() {
            let Some(rest) = remaining.checked_sub(character.len_utf16()) else {
                break;
            };
            end = index + character.len_utf8();
            remaining = rest;
        }
        format!("{}...", &line[..end])
    }
}

pub struct McpOnDemandSelection {
    pub loaded: Vec<String>,
    pub already_loaded: Vec<String>,
    pub catalog: McpOnDemandCatalog,
}

impl McpConnectionManager {
    pub async fn has_on_demand_servers(&self) -> bool {
        self.inner
            .read()
            .await
            .specs
            .values()
            .any(|stored| stored.on_demand.is_some())
    }

    pub async fn on_demand_servers(
        &self,
        scope: Option<&McpOnDemandScope>,
    ) -> Vec<McpOnDemandServer> {
        let inner = self.inner.read().await;
        let mut names = BTreeMap::<&str, usize>::new();
        for config in inner
            .specs
            .values()
            .filter_map(|stored| stored.on_demand.as_ref())
        {
            *names.entry(&config.source_name).or_default() += 1;
        }
        let mut servers = Vec::new();
        for (id, stored) in &inner.specs {
            let Some(config) = &stored.on_demand else {
                continue;
            };
            let namespace = if names[config.source_name.as_str()] == 1 {
                config.source_name.clone()
            } else {
                // Distinct packages can install the same native server name.
                // Qualify both so a previously unique alias cannot retarget
                // from one package to another when a duplicate is installed.
                id.clone()
            };
            servers.push(McpOnDemandServer {
                id: id.clone(),
                source_name: config.source_name.clone(),
                catalog: scope
                    .and_then(|scope| inner.on_demand.get(&(scope.clone(), id.clone())))
                    .and_then(|catalog| catalog.snapshot.clone()),
                namespace,
                description_limit: config.description_limit,
            });
        }
        servers
    }

    pub async fn begin_on_demand_observation(
        &self,
        server: &str,
        scope: &McpOnDemandScope,
    ) -> McpResult<u64> {
        let mut inner = self.inner.write().await;
        if !inner
            .specs
            .get(server)
            .is_some_and(|spec| spec.on_demand.is_some())
        {
            return Err(McpError::ServerNotConnected(server.to_owned()));
        }
        let observation = self.next_tool_observation.fetch_add(1, Ordering::Relaxed);
        inner
            .on_demand
            .entry((scope.clone(), server.to_owned()))
            .or_insert(SessionCatalog {
                started_at: observation,
                snapshot: None,
            });
        Ok(observation)
    }

    /// Read the latest catalog from this authorized connection.
    pub async fn connected_tools(&self, name: &str) -> McpResult<Arc<Vec<ToolDescriptor>>> {
        let server = self.get(name).await?;
        Ok(server.events.tools.read().await.clone())
    }

    /// Publish an authorized list and validate the complete load request before
    /// adding any names. Older observations cannot overwrite newer catalogs.
    pub async fn update_on_demand_tools(
        &self,
        server: &str,
        scope: &McpOnDemandScope,
        observation: u64,
        tools: Arc<Vec<ToolDescriptor>>,
        requested: &[String],
    ) -> McpResult<McpOnDemandSelection> {
        let offered = TOOL_SNAPSHOTS
            .run(move || {
                let mut offered = BTreeMap::new();
                for tool in tools.iter().filter(|tool| {
                    tool.name != MCP_TOOL_LOADER && crate::apps::tool_visible_to(tool, "model")
                }) {
                    if offered.insert(tool.name.clone(), tool.clone()).is_some() {
                        return Err(McpError::Transport(format!(
                            "MCP tool list contains duplicate entries for {:?}",
                            tool.name
                        )));
                    }
                }
                Ok(offered)
            })
            .await
            .map_err(projection_error)??;
        let mut inner = self.inner.write().await;
        let entry = inner
            .on_demand
            .get_mut(&(scope.clone(), server.to_owned()))
            .filter(|entry| observation >= entry.started_at)
            .ok_or_else(|| McpError::ServerNotConnected(server.to_owned()))?;
        let catalog = entry.snapshot.get_or_insert_with(|| McpOnDemandCatalog {
            observation: 0,
            offered: Arc::default(),
            loaded: BTreeSet::new(),
        });
        let offered = Arc::new(offered);
        if observation >= catalog.observation {
            catalog.loaded.retain(|name| offered.contains_key(name));
            catalog.offered = Arc::clone(&offered);
            catalog.observation = observation;
        }

        let unknown = requested
            .iter()
            .filter(|name| !offered.contains_key(*name) || !catalog.offered.contains_key(*name))
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(McpError::Malformed(format!(
                "MCP server does not offer {}; use names from its on-demand tool list",
                unknown.join(", ")
            )));
        }
        let mut loaded = Vec::new();
        let mut already_loaded = Vec::new();
        let mut seen = BTreeSet::new();
        for name in requested.iter().filter(|name| seen.insert(*name)) {
            if catalog.loaded.insert(name.clone()) {
                loaded.push(name.clone());
            } else {
                already_loaded.push(name.clone());
            }
        }
        Ok(McpOnDemandSelection {
            loaded,
            already_loaded,
            catalog: catalog.clone(),
        })
    }

    pub async fn clear_on_demand_session(&self, session: i64) {
        self.inner
            .write()
            .await
            .on_demand
            .retain(|(scope, _), _| scope.session != session);
    }
}

fn public_tool_name(server: &str, tool: &str) -> String {
    const MAX: usize = 64;
    const HASH_LENGTH: usize = 12;
    let joined = format!("mcp__{server}__{tool}");
    let normalized = joined
        .encode_utf16()
        .map(|character| {
            u8::try_from(character)
                .ok()
                .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                .map_or('_', char::from)
        })
        .collect::<String>();
    if joined == normalized && normalized.len() <= MAX {
        return normalized;
    }
    let mut hash = Sha256::new();
    hash.update(server.as_bytes());
    hash.update([0]);
    hash.update(tool.as_bytes());
    let hash = hex::encode(&hash.finalize()[..HASH_LENGTH / 2]);
    let prefix = &normalized[..normalized.len().min(MAX - HASH_LENGTH - 1)];
    format!("{prefix}_{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SERVER: &str = "plugin-a";

    async fn register(manager: &McpConnectionManager, id: &str) {
        manager
            .register_scoped_plugin_server_with_loading(
                id,
                ServerSpec::Http {
                    header_helper: None,
                    url: Url::parse("http://127.0.0.1:1/mcp").unwrap(),
                    transport: Default::default(),
                    headers: HashMap::new(),
                    auth: None,
                    tool_policy: Default::default(),
                },
                ScopedPluginTimeouts::default(),
                None,
                Some(McpOnDemandConfig {
                    source_name: "same".into(),
                    description_limit: 160,
                }),
            )
            .await
            .unwrap();
    }

    fn tools(names: &[&str]) -> Arc<Vec<ToolDescriptor>> {
        Arc::new(
            names
                .iter()
                .map(|name| serde_json::from_value(json!({"name":name})).unwrap())
                .collect(),
        )
    }

    #[test]
    fn public_names_match_deepseek_normalization() {
        assert_eq!(
            public_tool_name("server", "plain-tool"),
            "mcp__server__plain-tool"
        );
        assert_eq!(
            public_tool_name("foo.bar", "thing/☀"),
            "mcp__foo_bar__thing___f2521cc2021f"
        );
        assert_eq!(public_tool_name(&"long".repeat(20), "tool").len(), 64);
    }

    #[tokio::test]
    async fn cleared_or_newer_catalogs_fence_in_flight_observations() {
        let manager = McpConnectionManager::default();
        register(&manager, SERVER).await;
        assert_eq!(
            manager.on_demand_servers(None).await[0].alias(MCP_TOOL_LOADER),
            "mcp__same__load_tools"
        );
        register(&manager, "plugin-z").await;
        let servers = manager.on_demand_servers(None).await;
        assert_eq!(
            servers[0].alias(MCP_TOOL_LOADER),
            "mcp__plugin-a__load_tools"
        );
        assert_ne!(
            servers[0].alias(MCP_TOOL_LOADER),
            servers[1].alias(MCP_TOOL_LOADER)
        );
        let scope = McpOnDemandScope {
            session: 1,
            workspace: "/workspace".into(),
        };
        let first = manager
            .begin_on_demand_observation(SERVER, &scope)
            .await
            .unwrap();
        let observed = manager
            .begin_on_demand_observation(SERVER, &scope)
            .await
            .unwrap();
        let loaded = manager
            .update_on_demand_tools(
                SERVER,
                &scope,
                observed,
                tools(&["fresh", MCP_TOOL_LOADER]),
                &["fresh".into()],
            )
            .await
            .unwrap();
        assert_eq!(loaded.loaded, ["fresh"]);
        assert!(!loaded.catalog.offered.contains_key(MCP_TOOL_LOADER));
        let late = manager
            .update_on_demand_tools(SERVER, &scope, first, tools(&["stale"]), &["stale".into()])
            .await;
        assert!(late.is_err());
        assert!(
            manager.on_demand_servers(Some(&scope)).await[0]
                .catalog
                .as_ref()
                .unwrap()
                .loaded
                .contains("fresh")
        );

        manager.clear_on_demand_session(scope.session).await;
        let next = manager
            .begin_on_demand_observation(SERVER, &scope)
            .await
            .unwrap();
        assert!(
            manager
                .update_on_demand_tools(
                    SERVER,
                    &scope,
                    observed,
                    tools(&["fresh"]),
                    &["fresh".into()],
                )
                .await
                .is_err()
        );
        let cleared = manager
            .update_on_demand_tools(SERVER, &scope, next, tools(&["fresh"]), &[])
            .await
            .unwrap();
        assert!(cleared.catalog.loaded.is_empty());
    }
}
