//! Model protocol dispatch. Discovery reads a catalog projection and bypasses
//! execution dispatch, invocation hooks, effect checks, and plugin rendering.

use super::*;
use crate::session::kernel::KernelToolApiCall;
use agena_domain::mcp_app_tools::CALL_MCP_APP_TOOL;
use agena_domain::{StructuredField, StructuredObject, StructuredValue, ToolInvocation};
use agena_runtime_tools::tool::{MCP_TOOL_LOADER, McpOnDemandServer, McpToolDescriptor};
use agena_tool_api::{
    CatalogPlugin, CatalogTool, ToolApiAction, ToolApiCatalog, ToolApiError, ToolApiResponse,
};
use serde_json::json;

pub(super) async fn catalog(executor: &ToolExecutor) -> ToolApiCatalog {
    projected_catalog(executor).await.0
}

struct McpToolRoute {
    name: String,
    qualified_name: String,
    server: String,
    raw_name: String,
}

impl McpToolRoute {
    fn invocation(self, input: StructuredObject) -> ToolInvocation {
        ToolInvocation::new(
            "agena.mcp.tools.call",
            StructuredObject {
                fields: vec![
                    StructuredField {
                        name: "server".into(),
                        value: StructuredValue::Text { value: self.server },
                    },
                    StructuredField {
                        name: "name".into(),
                        value: StructuredValue::Text {
                            value: self.raw_name,
                        },
                    },
                    StructuredField {
                        name: "arguments".into(),
                        value: StructuredValue::Object {
                            fields: input.fields,
                        },
                    },
                ],
            },
        )
    }
}

async fn projected_catalog(executor: &ToolExecutor) -> (ToolApiCatalog, Vec<McpToolRoute>) {
    let tools = executor.available_execution_tools_async().await;
    let names = agena_runtime_tools::tool::execution_tool_names(&tools);
    let mut entries = tools
        .iter()
        .zip(names)
        .filter(|(tool, _)| tool.canonical_name() != CALL_MCP_APP_TOOL)
        .map(|(tool, name)| {
            let input_schema = tool.input_schema();
            CatalogTool {
                name,
                canonical_name: tool.canonical_name(),
                plugin_id: tool.plugin_full_name(),
                summary: tool.summary_text().unwrap_or_default().to_owned(),
                help: [
                    tool.before_help_text(),
                    tool.help_text(),
                    tool.after_help_text(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n"),
                tags: tool
                    .effective_tags()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                input_usage: agena_runtime_tools::tool::definition::schema_usage_text(
                    &input_schema,
                ),
                input_examples: agena_runtime_tools::tool::definition::schema_example_texts(
                    &input_schema,
                ),
                input_schema,
                output_schema: tool.output_schema(),
            }
        })
        .collect::<Vec<_>>();
    let routes = append_mcp_tools(executor, &mut entries).await;
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let mut plugins = executor
        .plugin_manager()
        .plugins()
        .iter()
        .filter(|plugin| executor.plugin_is_enabled(&plugin.key()))
        .map(|plugin| {
            let id = plugin.key().to_string();
            CatalogPlugin {
                tool_count: entries.iter().filter(|tool| tool.plugin_id == id).count(),
                id,
                version: plugin.manifest.version.clone(),
                summary: plugin.manifest.summary.clone().unwrap_or_default(),
                tags: plugin
                    .manifest
                    .tags
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    (
        ToolApiCatalog {
            tools: entries,
            plugins,
        },
        routes,
    )
}

async fn append_mcp_tools(
    executor: &ToolExecutor,
    entries: &mut Vec<CatalogTool>,
) -> Vec<McpToolRoute> {
    let servers = executor.on_demand_mcp_servers().await;
    if servers.is_empty() {
        return Vec::new();
    }
    let names = mcp_alias_counts(
        executor,
        &servers,
        entries.iter().map(|entry| entry.name.clone()),
    );
    let mut routes = Vec::new();
    for server in servers {
        let mut add = |tool: McpToolDescriptor| {
            if !executor.allows_mcp_tool_alias(&server.id, &tool.name) {
                return;
            }
            let route = McpToolRoute {
                name: visible_mcp_alias(&server, &tool.name, &names),
                qualified_name: server.qualified_alias(&tool.name),
                server: server.id.clone(),
                raw_name: tool.name,
            };
            let input_schema = tool
                .input_schema
                .unwrap_or_else(|| json!({"type": "object"}));
            entries.push(CatalogTool {
                name: route.name.clone(),
                canonical_name: route.qualified_name.clone(),
                plugin_id: "agena.mcp".into(),
                summary: tool.description.unwrap_or_default(),
                help: [tool.before_help, tool.after_help]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                tags: vec!["mcp".into()],
                input_usage: agena_runtime_tools::tool::definition::schema_usage_text(
                    &input_schema,
                ),
                input_examples: agena_runtime_tools::tool::definition::schema_example_texts(
                    &input_schema,
                ),
                input_schema,
                output_schema: tool.output_schema.unwrap_or_default(),
            });
            routes.push(route);
        };
        add(server.loader());
        if let Some(catalog) = &server.catalog {
            for tool in catalog
                .loaded
                .iter()
                .filter_map(|name| catalog.offered.get(name))
            {
                add(tool.clone());
            }
        }
    }
    routes
}

fn mcp_alias_counts(
    executor: &ToolExecutor,
    servers: &[McpOnDemandServer],
    registered_names: impl IntoIterator<Item = String>,
) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for name in registered_names {
        *counts.entry(name).or_default() += 1;
    }
    for server in servers {
        let loaded = server
            .catalog
            .iter()
            .flat_map(|catalog| catalog.loaded.iter().map(String::as_str));
        for name in std::iter::once(MCP_TOOL_LOADER).chain(loaded) {
            if executor.allows_mcp_tool_alias(&server.id, name) {
                *counts.entry(server.alias(name)).or_default() += 1;
            }
        }
    }
    counts
}

fn visible_mcp_alias(
    server: &McpOnDemandServer,
    raw_name: &str,
    counts: &HashMap<String, usize>,
) -> String {
    let alias = server.alias(raw_name);
    if counts.get(&alias).copied().unwrap_or_default() > 1 {
        server.qualified_alias(raw_name)
    } else {
        alias
    }
}

impl SessionManager {
    fn project_mcp_app_tools(
        &self,
        session: SessionId,
        executor: &ToolExecutor,
        catalog: &mut ToolApiCatalog,
    ) -> HashMap<String, uuid::Uuid> {
        let mut routes = HashMap::new();
        if !executor.mcp_tool_available(CALL_MCP_APP_TOOL) {
            return routes;
        }
        let mut used = catalog
            .tools
            .iter()
            .flat_map(|tool| [tool.name.clone(), tool.canonical_name.clone()])
            .collect::<std::collections::HashSet<_>>();
        for (part, app, tool) in self.mcp_app_views.tools(session.get()) {
            let alias = tool.alias(part);
            if !used.insert(alias.clone()) {
                continue;
            }
            let definition = &tool.definition;
            let name = definition.title.as_deref().unwrap_or(&definition.name);
            let input_schema = &definition.input_schema;
            catalog.tools.push(CatalogTool {
                name: alias.clone(),
                canonical_name: alias.clone(),
                plugin_id: "agena.mcp".into(),
                summary: format!("{name} (MCP App: {app})"),
                help: definition.description.clone().unwrap_or_default(),
                tags: vec!["mcp".into(), "execute".into(), "mutate".into()],
                input_usage: agena_runtime_tools::tool::definition::schema_usage_text(input_schema),
                input_examples: agena_runtime_tools::tool::definition::schema_example_texts(
                    input_schema,
                ),
                input_schema: input_schema.clone(),
                output_schema: definition.output_schema.clone().unwrap_or_default(),
            });
            routes.insert(alias, tool.id);
        }
        if !routes.is_empty() {
            catalog
                .tools
                .sort_by(|left, right| left.name.cmp(&right.name));
            if let Some(plugin) = catalog
                .plugins
                .iter_mut()
                .find(|plugin| plugin.id == "agena.mcp")
            {
                plugin.tool_count += routes.len();
            }
        }
        routes
    }

    pub(super) async fn append_on_demand_mcp_catalog(
        &self,
        executor: &ToolExecutor,
        control: &RunControl,
        system: &mut String,
    ) {
        if self.mcp_app_views.has_tools(control.session.get())
            && executor.mcp_tool_available(CALL_MCP_APP_TOOL)
        {
            system.push_str(
                "\n\nOpen MCP Apps in this session provide tools. Use `tools_list` with `plugin: \"mcp\"` to discover them and `tools_help` to read their inputs.",
            );
        }
        let permitted = |server: &McpOnDemandServer| {
            executor.allows_mcp_tool_alias(&server.id, MCP_TOOL_LOADER)
        };
        let mut servers = executor.on_demand_mcp_servers().await;
        if !servers.iter().any(permitted) {
            return;
        }
        let search_available = executor.mcp_tool_available("agena.mcp.tools.search");
        if search_available {
            // Prompt assembly has no approval slot. The same tool can retry
            // catalog discovery later under a normal interactive invocation.
            let refresh = self.invoke_imported_hook_tool(
                control.session.get(),
                "agena.mcp.tools.search",
                json!({"on_demand_catalog": true}),
            );
            tokio::select! {
                biased;
                _ = control.cancel.cancelled() => return,
                _ = tokio::time::timeout(Duration::from_secs(5), refresh) => {}
            }
            servers = executor.on_demand_mcp_servers().await;
        }
        let aliases = mcp_alias_counts(
            executor,
            &servers,
            agena_runtime_tools::tool::execution_tool_names(&executor.available_execution_tools()),
        );
        const CATALOG_BUDGET: usize = 16 * 1024;
        let start = system.len();
        for server in servers.into_iter().filter(permitted) {
            let name = visible_mcp_alias(&server, MCP_TOOL_LOADER, &aliases);
            let heading = format!(
                "\n\n### On-demand tools from MCP server {}\nUse `tools_help` for `{name}`, then load the exact tool names before calling them.\n",
                server.source_name,
            );
            if system.len() - start + heading.len() > CATALOG_BUDGET {
                return;
            }
            let Some(catalog) = &server.catalog else {
                if search_available {
                    system.push_str(&heading);
                    system.push_str("Use `mcp.tools.search` with `on_demand_catalog: true` to retrieve the catalog.");
                }
                continue;
            };
            let mut offered = catalog
                .offered
                .values()
                .filter(|tool| executor.allows_mcp_tool_alias(&server.id, &tool.name))
                .peekable();
            if offered.peek().is_none() {
                continue;
            }
            system.push_str(&heading);
            for tool in offered {
                let summary = server.summary(tool.description.as_deref().unwrap_or_default());
                let line = if summary.is_empty() {
                    format!("- {:?}\n", tool.name)
                } else {
                    format!("- {:?}: {summary}\n", tool.name)
                };
                if system.len() - start + line.len() > CATALOG_BUDGET {
                    if search_available {
                        system.push_str("Use `mcp.tools.search` with `on_demand_catalog: true` and `catalog_offset` to see additional names.");
                    }
                    return;
                }
                system.push_str(&line);
            }
        }
    }

    pub(super) async fn dispatch_tool_api(
        &self,
        protocol: KernelToolApiCall,
        executor: ToolExecutor,
        control: Arc<RunControl>,
        state: Arc<ExecutionState>,
        model: ModelRef,
    ) -> Result<(), AppError> {
        if control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let action = match protocol.error {
            Some(error) => Err(error),
            None => {
                let catalog = if protocol.call.function == agena_domain::ToolApiFunction::Call {
                    ToolApiCatalog::default()
                } else {
                    let mut catalog = catalog(&executor).await;
                    self.project_mcp_app_tools(control.session, &executor, &mut catalog);
                    catalog
                };
                catalog.dispatch(&protocol.call)
            }
        };
        let response = match action {
            Ok(ToolApiAction::Execute(request)) => {
                let (mut catalog, routes) = projected_catalog(&executor).await;
                let app_routes =
                    self.project_mcp_app_tools(control.session, &executor, &mut catalog);
                // The model may run a tool only after its complete live
                // contract entered the current context through tools_help.
                // A context that lost that help (compaction) must re-read it,
                // so the rejection teaches the contract instead of running a
                // tool whose input the model can only guess at.
                if let Some(gate) = self
                    .tool_help_gate(
                        &request.tool,
                        protocol.tool.session_id(),
                        protocol.tool.round_id(),
                        &catalog,
                    )
                    .await
                {
                    // The rejection itself carries the live contract, so the
                    // model leaves this result knowing how to call the tool.
                    let help = catalog
                        .dispatch(&agena_domain::ToolApiCall {
                            function: agena_domain::ToolApiFunction::Help,
                            arguments: agena_domain::StructuredObject::try_from(
                                json!({"tool": request.tool}),
                            )
                            .map_err(AppError::Internal)?,
                        })
                        .ok()
                        .and_then(|action| match action {
                            ToolApiAction::Respond(response) => Some(response),
                            ToolApiAction::Execute(_) => None,
                        });
                    let (response, failure) =
                        tool_help_required_response(&request.tool, &gate, help);
                    complete_protocol_response_with_failure(protocol.tool, response, failure)
                        .await?;
                    return Ok(());
                }
                // This is the only crossing into plugin execution. From here
                // on, the executor sees solely the target and its own input.
                let route = routes.into_iter().find(|route| {
                    route.name == request.tool || route.qualified_name == request.tool
                });
                let invocation = match (route, app_routes.get(&request.tool)) {
                    (Some(route), _) => route.invocation(request.input),
                    (None, Some(target)) => ToolInvocation::new(
                        CALL_MCP_APP_TOOL,
                        StructuredObject {
                            fields: vec![
                                StructuredField {
                                    name: "target".into(),
                                    value: StructuredValue::Text {
                                        value: target.to_string(),
                                    },
                                },
                                StructuredField {
                                    name: "arguments".into(),
                                    value: StructuredValue::Object {
                                        fields: request.input.fields,
                                    },
                                },
                            ],
                        },
                    ),
                    (None, None) => ToolInvocation::new(request.tool, request.input),
                };
                return self
                    .execute_tool(protocol.tool, invocation, executor, control, state, model)
                    .await;
            }
            Ok(ToolApiAction::Respond(response)) => Ok(response),
            Err(error) => Err(error),
        };
        complete_protocol_response(protocol.tool, response).await
    }

    /// Execution tools whose complete help is inside the context the model
    /// held when it produced this round's calls.
    ///
    /// Context, not transcript history, decides "the model knows this tool":
    /// the round's frozen input is exactly what the model could still see, so
    /// a compaction checkpoint that cut an earlier `tools_help` response also
    /// removes it here. Only a successful `tools_help` response inside that
    /// window registers a tool, and only for the names it actually rendered.
    pub(super) async fn helped_tools_in_context(
        &self,
        session: SessionId,
        round: agena_storage::kernel::RoundId,
    ) -> Vec<String> {
        let facts = match self.kernel().reader.prompt_of(round).await {
            Ok(facts) => facts,
            Err(error) => {
                tracing::warn!(
                    session = session.get(),
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "read the current prompt window for tool help state",
                        &error,
                    ),
                    "tool help gate cannot read the current context; treating no help as established"
                );
                return Vec::new();
            }
        };
        // A response records the compact and canonical spelling of every tool
        // it rendered; both resolve to one taught tool here. A name that has
        // since disappeared from the catalog cannot be executed anyway.
        let mut helped = std::collections::BTreeSet::<String>::new();
        for entry in &facts.entries {
            let Some(tool) = entry.tool.as_ref() else {
                continue;
            };
            if tool.phase != ToolPhase::Succeeded
                || tool.call.name != agena_domain::ToolApiFunction::Help.function_name()
            {
                continue;
            }
            let Some(result) = tool.result.as_ref() else {
                continue;
            };
            let Some(taught) = result
                .get("tool_api_response")
                .and_then(|response| response.get("helped_tools"))
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for name in taught.iter().filter_map(serde_json::Value::as_str) {
                helped.insert(name.to_owned());
            }
        }
        helped.into_iter().collect()
    }

    /// Why `tool` must not run yet, or `None` when its help is established or
    /// the gate does not apply.
    ///
    /// The gate only covers tools that exist in this session's catalog: an
    /// unknown name keeps its ordinary `tools_call` failure, and a tool that
    /// cannot be taught (no help response for it) keeps whatever error it
    /// produces on its own.
    async fn tool_help_gate(
        &self,
        tool: &str,
        session: SessionId,
        round: agena_storage::kernel::RoundId,
        catalog: &ToolApiCatalog,
    ) -> Option<ToolHelpGate> {
        let resolvable = catalog
            .tools
            .iter()
            .any(|entry| entry.name == tool || entry.canonical_name == tool);
        if !resolvable {
            return None;
        }
        let helped = self.helped_tools_in_context(session, round).await;
        if helped.iter().any(|name| name == tool) {
            return None;
        }
        Some(ToolHelpGate { helped })
    }
}

/// The state that blocked a tools_call because the model never read its help
/// in the current context.
pub(super) struct ToolHelpGate {
    pub helped: Vec<String>,
}

/// Reject a tools_call whose tool help is not in the current context, teaching
/// the live contract in the same result.
///
/// The teaching text lives in the result JSON (both the model-facing failure
/// and the Tool API response), not only in `ModelFeedback::text`: that field is
/// deliberately `#[serde(skip)]`, so a replayed result would otherwise lose
/// the very contract this rejection exists to deliver.
fn tool_help_required_response(
    tool: &str,
    gate: &ToolHelpGate,
    help: Option<ToolApiResponse>,
) -> (ToolApiResponse, agena_failure::Failure) {
    let established = if gate.helped.is_empty() {
        "No execution tool has an established help contract in the current context.".to_owned()
    } else {
        format!(
            "Help is currently established only for: {}.",
            gate.helped.join(", ")
        )
    };
    let reason = format!(
        "`{tool}` was not run because its help has not been read in the current context \
         (failure code `tool.help_required`). Call `tools_help` with tool=\"{tool}\" before the \
         first `tools_call` to it, and re-read it whenever compaction removed the earlier \
         answer. {established}"
    );
    let contract = help
        .as_ref()
        .map(|help| help.output_text.clone())
        .filter(|text| !text.trim().is_empty());
    let output_text = match &contract {
        Some(contract) => {
            format!("{reason}\n\nThe live contract returned by tools_help:\n{contract}")
        }
        None => reason.clone(),
    };
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("tool.help_required"),
        agena_failure::FailureCategory::InvalidInput,
        agena_failure::FailureResponsibility::Caller,
        agena_failure::RetryDirective::CorrectInput,
        agena_failure::RecoveryDirective::None,
        agena_failure::FailureImpact::RequestRejected,
        agena_failure::UserPresentation::validated(
            "tool.help_required",
            format!(
                "`{tool}` needs its help read before it can run; the contract was returned to the model."
            ),
        ),
    )
    .with_model_feedback(agena_failure::ModelFeedback::invalid_input().with_text(output_text.clone()));
    (
        ToolApiResponse {
            title: format!("Tool help required · {tool}"),
            summary: format!("`{tool}` was blocked until its help is read in the current context."),
            output_text,
            // This rejection teaches a contract without a tools_help call, so
            // the taught name counts exactly like a real help response.
            helped_tools: help.map(|help| help.helped_tools).unwrap_or_default(),
        },
        failure,
    )
}

async fn complete_protocol_response(
    tool: ToolHandle,
    response: Result<ToolApiResponse, ToolApiError>,
) -> Result<(), AppError> {
    match response {
        Ok(response) => complete_taught_response(tool, response, None).await,
        Err(error) => {
            let failure = error.failure();
            let result = ToolResult {
                phase: ToolPhase::Failed,
                value: json!({"failure": failure, "error": agena_failure::UserProblem::from(&failure)}),
            };
            persist(|| tool.complete(RequestId::new(), result.clone()))
                .await
                .map_err(kernel_error)?;
            Ok(())
        }
    }
}

/// Complete a tools_call rejected by the help gate: the response text is the
/// model's correction, and the structured failure keeps the transcript honest.
async fn complete_protocol_response_with_failure(
    tool: ToolHandle,
    response: ToolApiResponse,
    failure: agena_failure::Failure,
) -> Result<(), AppError> {
    complete_taught_response(tool, response, Some(failure)).await
}

async fn complete_taught_response(
    tool: ToolHandle,
    mut response: ToolApiResponse,
    failure: Option<agena_failure::Failure>,
) -> Result<(), AppError> {
    if response.output_text.len() > 64 * 1024 {
        let command = request_key("tool-api-response", &tool.id().get().to_string());
        let source = persist(|| tool.begin_text(command))
            .await
            .map_err(kernel_error)?
            .value;
        super::results::append_text_chunks(&source, &response.output_text)
            .await
            .map_err(kernel_error)?;
        response.output_text.clear();
    }
    let (phase, value) = match failure {
        Some(failure) => (
            ToolPhase::Failed,
            json!({
                "title": response.title,
                "summary": response.summary,
                "tool_api_response": response,
                "failure": failure,
                "error": agena_failure::UserProblem::from(&failure),
            }),
        ),
        None => (
            ToolPhase::Succeeded,
            json!({"title": response.title, "summary": response.summary, "tool_api_response": response}),
        ),
    };
    let result = ToolResult { phase, value };
    persist(|| tool.complete(RequestId::new(), result.clone()))
        .await
        .map_err(kernel_error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
