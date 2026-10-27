//! Shared projections and isolated diagnostics for external MCP management.

use crate::{
    FileTokenStore, McpConnectionManager, McpError, McpResult, McpServerStatus, ServerSpec,
};
use agena_domain::mcp_connections::*;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub async fn project_catalog(
    catalog: &mut McpConnectionCatalog,
    manager: Option<&Arc<McpConnectionManager>>,
) {
    let Some(manager) = manager else {
        catalog.bridge_enabled = false;
        for item in &mut catalog.connections {
            if item.state == "disconnected" {
                item.state = "bridge_disabled".into();
            }
        }
        return;
    };
    let statuses = manager.statuses().await;
    for item in &mut catalog.connections {
        project_summary(item, statuses.iter().find(|s| s.name == item.name));
    }
}

pub async fn project_detail(
    detail: &mut McpConnectionDetail,
    manager: Option<&Arc<McpConnectionManager>>,
) {
    let Some(manager) = manager else {
        if detail.summary.state == "disconnected" {
            detail.summary.state = "bridge_disabled".into();
        }
        return;
    };
    let statuses = manager.statuses().await;
    let status = statuses.iter().find(|s| s.name == detail.summary.name);
    project_summary(&mut detail.summary, status);
    if detail.summary.effective
        && detail.summary.enabled
        && !matches!(
            detail.summary.state.as_str(),
            "pending_reload" | "bridge_disabled" | "invalid"
        )
        && let Some(status) = status
    {
        detail.protocol_version = status.protocol_version.clone();
        detail.server_info = status.server_info.clone();
        detail.capabilities = status.capabilities.clone();
        detail.oauth_health = status.oauth_health.as_ref().map(|h| json!({
                "credential_state":h.credential_state.as_str(),"expiry_state":h.expiry_state.map(|s| s.as_str()),"refresh_available":h.refresh_available
            }));
    }
}

fn project_summary(summary: &mut McpConnectionSummary, status: Option<&McpServerStatus>) {
    if !summary.effective
        || !summary.enabled
        || summary.state == "invalid"
        || summary.state == "bridge_disabled"
        || summary.state == "pending_reload"
    {
        return;
    }
    if let Some(status) = status {
        summary.tool_count = status.tool_count;
        summary.state = if status.connected {
            "connected"
        } else if status
            .oauth_health
            .as_ref()
            .is_some_and(|h| h.credential_state.as_str() == "missing")
        {
            "authentication_required"
        } else {
            "disconnected"
        }
        .into();
        summary.error = if status.legacy_credentials_present {
            Some("Stored credentials use the previous server-name binding. Enter the bearer token again or sign in to bind credentials to this endpoint.".into())
        } else {
            status
                .last_failure
                .as_ref()
                .or(status.last_refresh_failure.as_ref())
                .map(|f| f.user.fallback.clone())
        };
    }
}

pub async fn capabilities(
    manager: &McpConnectionManager,
    name: &str,
    kind: &str,
    cursor: Option<String>,
) -> McpResult<McpCapabilityPage> {
    let status = manager
        .statuses()
        .await
        .into_iter()
        .find(|s| s.name == name && s.connected)
        .ok_or_else(|| McpError::ServerNotConnected(name.into()))?;
    let capability = match kind {
        "tools" => "tools",
        "resources" | "resource_templates" => "resources",
        "prompts" => "prompts",
        _ => return Err(McpError::Malformed("Unknown capability category.".into())),
    };
    if status
        .capabilities
        .as_ref()
        .and_then(|v| v.get(capability))
        .is_none()
    {
        return Ok(McpCapabilityPage {
            kind: kind.into(),
            items: vec![],
            next_cursor: None,
        });
    }
    let (items, next_cursor) = match kind {
        "tools" => {
            let offset = cursor
                .as_deref()
                .unwrap_or("0")
                .parse::<usize>()
                .map_err(|_| McpError::Malformed("Invalid tool-page cursor.".into()))?;
            let tools: Vec<_> = manager
                .all_tools()
                .await
                .into_iter()
                .filter(|(server, _)| server == name)
                .map(|(_, tool)| tool)
                .collect();
            let next = (offset.saturating_add(100) < tools.len())
                .then(|| offset.saturating_add(100).to_string());
            (
                tools
                    .into_iter()
                    .skip(offset)
                    .take(100)
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?,
                next,
            )
        }
        "resources" => {
            let page = manager.list_resources(name, cursor).await?;
            (
                page.resources
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?,
                page.next_cursor,
            )
        }
        "resource_templates" => {
            let page = manager.list_resource_templates(name, cursor).await?;
            (
                page.resource_templates
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?,
                page.next_cursor,
            )
        }
        "prompts" => {
            let page = manager.list_prompts(name, cursor).await?;
            (
                page.prompts
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?,
                page.next_cursor,
            )
        }
        _ => unreachable!(),
    };
    Ok(McpCapabilityPage {
        kind: kind.into(),
        items,
        next_cursor,
    })
}

/// Probe an independent connection and close it without changing the active catalog.
pub async fn test_connection(
    name: &str,
    spec: ServerSpec,
    workspace: PathBuf,
    connect_ms: u64,
    request_ms: u64,
    token_store_enabled: bool,
) -> McpTestResult {
    let started = Instant::now();
    let mut probe = McpConnectionManager::new("agena-connection-test", env!("CARGO_PKG_VERSION"))
        .with_roots([workspace])
        .with_timeouts(
            Duration::from_millis(connect_ms),
            Duration::from_millis(request_ms),
        );
    if token_store_enabled {
        probe.set_token_store(Arc::new(FileTokenStore::new()));
    }
    let probe = Arc::new(probe);
    let result = probe.add_server(name, spec).await;
    let status = probe.statuses().await.into_iter().find(|s| s.name == name);
    let response = McpTestResult {
        ok: result.is_ok(),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        tool_count: status.as_ref().map_or(0, |s| s.tool_count),
        protocol_version: status.as_ref().and_then(|s| s.protocol_version.clone()),
        server_info: status.as_ref().and_then(|s| s.server_info.clone()),
        capabilities: status.as_ref().and_then(|s| s.capabilities.clone()),
        error: result.err().map(|e| e.user_message()),
    };
    probe.shutdown_all().await;
    response
}

pub fn prompt_arguments(
    arguments: serde_json::Map<String, Value>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    arguments
        .into_iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key, value.to_owned()))
                .ok_or_else(|| "MCP prompt arguments must be strings.".to_owned())
        })
        .collect()
}
