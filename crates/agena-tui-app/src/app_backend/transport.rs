//! Cloneable transport boundary between the TUI state machine and the owner
//! of session execution.
//!
//! The TUI is a pure HTTP client. [`TuiBackend`] contains only the public API
//! client plus the server workspace identity; it never owns a Runtime,
//! scheduler, provider client, session store, or execution lease.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_api::part::PartResource;
use agena_api::{
    commands::{
        Command, CommandResult, ForkSessionParams, RewindSessionParams, UpdateSessionParams,
    },
    pagination::PaginatedResponse,
    queries::{GetSessionParams, ListSessionsParams, Query, QueryResult},
    resource::{
        PermissionReply, ProviderSummaryResource, RunOptions, SessionExecutionResource,
        SessionOverviewResource, SessionResource, UserInputReply,
    },
};
use agena_client::{AgenaClient, SubscriptionEvent};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use tokio::sync::mpsc;

use super::{LiveEvent, SessionRefresh};

/// Messages per transcript page: one page is a user message plus the AI reply
/// that answers it, so opening a session and scrolling up always show both.
/// The server counts a page in messages, so consecutive runs of one role (a
/// multi-round reply, or a burst of user sends) are never split.
pub(crate) const SESSION_TRANSCRIPT_PAGE_SIZE: u64 = 2;
pub(crate) const OLDER_TRANSCRIPT_PAGE_SIZE: u64 = 2;
/// Parts loaded per message. A message renders only its newest
/// `COLLAPSED_ACTIVITY_VISIBLE_COUNT` activities; everything older stays behind
/// the fold control's explicit "load n"/"load all" request.
pub(crate) const TRANSCRIPT_PART_PAGE_SIZE: u64 =
    agena_tui_transcript::COLLAPSED_ACTIVITY_VISIBLE_COUNT as u64;

#[derive(Debug, Clone)]
pub(crate) struct SessionTranscriptPage {
    pub parts: Vec<PartResource>,
    pub folds: Vec<agena_tui_transcript::TranscriptFold>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateWithTranscriptPage {
    pub execution: SessionExecutionResource,
    pub page: SessionTranscriptPage,
    /// State and page were read across a mutation; retain their parts and retry.
    pub needs_reconciliation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionSubscriptionDispatch {
    Ignore,
    RefreshPluginRuntime,
    Emit { refresh_plugin_presentation: bool },
}

fn classify_session_subscription_event(
    event: &SubscriptionEvent,
    session_id: i64,
) -> SessionSubscriptionDispatch {
    match event {
        SubscriptionEvent::Content(_) => SessionSubscriptionDispatch::Ignore,
        SubscriptionEvent::SessionChanged(change) => {
            if change.session_id() != session_id {
                return SessionSubscriptionDispatch::Ignore;
            }
            let refresh_plugin_presentation = matches!(
                change,
                agena_api::live::SessionChangeResource::PartAdded { part, .. }
                    | agena_api::live::SessionChangeResource::PartUpdated { part, .. }
                    if part.is_message_boundary()
            );
            SessionSubscriptionDispatch::Emit {
                refresh_plugin_presentation,
            }
        }
        SubscriptionEvent::RuntimeSignal(signal) => match signal.session_id {
            None if signal.kind == "tool_registry_changed" => {
                SessionSubscriptionDispatch::RefreshPluginRuntime
            }
            None if signal.kind == "plugin"
                && signal
                    .payload
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("attention.changed") =>
            {
                // Attention can belong to any session. Refresh the shared
                // plugin queue and wake the TUI after the producer published it.
                SessionSubscriptionDispatch::Emit {
                    refresh_plugin_presentation: true,
                }
            }
            None => SessionSubscriptionDispatch::Ignore,
            Some(signal_session_id) if signal_session_id != session_id => {
                SessionSubscriptionDispatch::Ignore
            }
            Some(_) => SessionSubscriptionDispatch::Emit {
                refresh_plugin_presentation: signal.kind == "plugin",
            },
        },
        SubscriptionEvent::Lagged(_) => SessionSubscriptionDispatch::Emit {
            refresh_plugin_presentation: true,
        },
    }
}

#[cfg(test)]
mod notification_dispatch_tests {
    use super::*;

    #[test]
    fn retained_notification_history_does_not_realert_after_more_than_sixty_four_events() {
        let mut state = crate::app_types::TerminalIntegrationState::default();
        for index in 0..1024 {
            assert!(
                state.notify_consumed_once(&format!("agena.notifications:execution:{index}:1"))
            );
        }
        // A reloaded host replays its bounded recent history, including
        // identical summaries from many distinct completed executions.
        for index in 512..1024 {
            assert!(
                !state.notify_consumed_once(&format!("agena.notifications:execution:{index}:1"))
            );
        }
    }

    #[test]
    fn global_attention_invalidation_refreshes_the_plugin_queue_for_any_open_session() {
        let event = SubscriptionEvent::RuntimeSignal(agena_api::live::RuntimeSignalResource {
            kind: "plugin".into(),
            session_id: None,
            payload: serde_json::json!({"kind": "attention.changed"}),
        });
        assert_eq!(
            classify_session_subscription_event(&event, 7),
            SessionSubscriptionDispatch::Emit {
                refresh_plugin_presentation: true
            }
        );
    }

    #[test]
    fn unrelated_global_plugin_signals_do_not_refresh_the_session() {
        let event = SubscriptionEvent::RuntimeSignal(agena_api::live::RuntimeSignalResource {
            kind: "plugin".into(),
            session_id: None,
            payload: serde_json::json!({"kind": "unrelated"}),
        });
        assert_eq!(
            classify_session_subscription_event(&event, 7),
            SessionSubscriptionDispatch::Ignore
        );
    }
}

#[derive(Clone)]
pub struct TuiBackend {
    inner: Arc<RemoteBackend>,
    /// Server-owned workspace path used only as a resource identity.
    workspace_root: Arc<PathBuf>,
    instance_id: Arc<str>,
    /// Empty client-local confinement root for media that has not been
    /// fetched through the authenticated workspace API.
    media_workspace: Arc<tempfile::TempDir>,
}

struct RemoteBackend {
    client: AgenaClient,
    workspace_id: i64,
    providers: tokio::sync::RwLock<Vec<ProviderSummaryResource>>,
    models: tokio::sync::RwLock<HashMap<String, Vec<agena_domain::Model>>>,
    configured_provider_adapter_models: tokio::sync::RwLock<
        HashMap<String, Vec<agena_api::resource::ProviderAdapterModelsResource>>,
    >,
    provider_drafts: tokio::sync::RwLock<
        HashMap<String, agena_application::provider_studio::ProviderConfigDraft>,
    >,
    /// Cached configuration-source read model assembled over HTTP. Loaded at
    /// connect and refreshed before settings-studio rebuilds; settings reads
    /// are synchronous in the TUI event loop.
    config_sources: tokio::sync::RwLock<Option<agena_application::dto::ConfigJsonSources>>,
    /// Authoritative runtime projection used for status and capability caches.
    runtime_status: tokio::sync::RwLock<Option<agena_api::resource::RuntimeStatusResponse>>,
    /// Cached control projection for Agena's own HTTP MCP surface.
    mcp_server_control: tokio::sync::RwLock<Option<serde_json::Value>>,
    /// Cached plugin surface catalog (display contributions, theme palettes, slash
    /// commands) fetched from the server. Plugin reads are synchronous in the
    /// TUI event loop.
    plugin_catalog: tokio::sync::RwLock<Option<agena_plugin_host::PluginSurfaceCatalog>>,
    /// Cached plugin statuses fetched from the server.
    plugin_statuses: tokio::sync::RwLock<Vec<agena_plugin_host::status::PluginStatus>>,
    /// Cached plugin details and logs used by the synchronous workbench model.
    plugin_inspects: tokio::sync::RwLock<HashMap<String, agena_plugin_host::PluginInspect>>,
    plugin_logs: tokio::sync::RwLock<HashMap<String, Vec<agena_plugin_host::PluginLogRecord>>>,
    /// Plugin-adjacent presentation metadata returned with the UI catalog.
    permission_tools:
        tokio::sync::RwLock<Vec<agena_application::dto::PermissionToolCatalogResource>>,
    plugin_notifications: tokio::sync::RwLock<Vec<agena_plugin_host::HostNotification>>,
    plugin_notifications_baseline: std::sync::OnceLock<i64>,
    activity_kinds: tokio::sync::RwLock<Vec<agena_domain::ActivityKind>>,
    /// Server-owned workspace inventory used by synchronous pickers/search.
    workspace_files: tokio::sync::RwLock<Option<agena_application::dto::WorkspaceFileTreeResource>>,
    workspace_file_index: tokio::sync::RwLock<Vec<PathBuf>>,
    /// Non-ignore-aware, shallow directory pages for path browsers. Keeping
    /// this separate prevents ignored build/config paths from disappearing
    /// while the workspace-wide mention index remains ignore-aware. Writes
    /// are single in-memory inserts, so a synchronous lock lets the TUI read
    /// authoritative pages instead of treating transient async lock
    /// contention as an empty directory.
    workspace_directory_cache:
        std::sync::RwLock<HashMap<String, Vec<agena_application::dto::WorkspaceFileNode>>>,
    workspace_image_data_urls: tokio::sync::RwLock<HashMap<(i64, String, String), Arc<str>>>,
    /// Server-side credential environment metadata.
    aws_profiles: tokio::sync::RwLock<Vec<String>>,
    /// Cached model-catalog page fetched from the server. The settings studio
    /// reads model counts synchronously inside the TUI event loop.
    model_catalog: tokio::sync::RwLock<Option<agena_application::dto::ModelCatalogListResponse>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkspacePathMetadata {
    pub(crate) is_directory: bool,
    pub(crate) size: Option<u64>,
}

fn find_workspace_file_node<'a>(
    nodes: &'a [agena_application::dto::WorkspaceFileNode],
    relative: &Path,
) -> Option<&'a agena_application::dto::WorkspaceFileNode> {
    let relative = relative.to_string_lossy().replace('\\', "/");
    for node in nodes {
        if node.path == relative {
            return Some(node);
        }
        if let Some(found) = find_workspace_file_node(&node.children, Path::new(&relative)) {
            return Some(found);
        }
    }
    None
}

fn collect_workspace_file_paths(
    nodes: &[agena_application::dto::WorkspaceFileNode],
    output: &mut Vec<PathBuf>,
) {
    for node in nodes {
        if node.kind == agena_application::dto::WorkspaceFileKind::File {
            output.push(PathBuf::from(&node.path));
        }
        collect_workspace_file_paths(&node.children, output);
    }
}

type ImageReference = (String, String, String); // reference, MIME, immutable revision
const MAX_LOCALIZED_IMAGE_BYTES: usize = 64 * 1024 * 1024;

fn image_reference(object: &serde_json::Map<String, serde_json::Value>) -> Option<ImageReference> {
    let text = |key: &str| object.get(key).and_then(serde_json::Value::as_str);
    let artifact = object
        .get("artifact")
        .and_then(serde_json::Value::as_object);
    let mime = text("mime")
        .or_else(|| text("mime_type"))
        .or_else(|| text("media_type"))
        .or_else(|| {
            artifact
                .and_then(|a| a.get("mime"))
                .and_then(serde_json::Value::as_str)
        });
    if text("kind") != Some("image") && !mime.is_some_and(|mime| mime.starts_with("image/")) {
        return None;
    }
    let source = object.get("source").and_then(serde_json::Value::as_object);
    if source
        .and_then(|source| source.get("source"))
        .and_then(serde_json::Value::as_str)
        == Some("unavailable")
    {
        return None;
    }
    let reference = object
        .get("reference")
        .and_then(serde_json::Value::as_object);
    let value = source
        .and_then(|s| {
            s.get("path")
                .or_else(|| s.get("url"))
                .or_else(|| s.get("file_id"))
        })
        .or_else(|| artifact.and_then(|a| a.get("uri")))
        .or_else(|| reference.and_then(|r| r.get("uri").or_else(|| r.get("path"))))
        .or_else(|| object.get("uri").or_else(|| object.get("path")))?
        .as_str()?;
    // A provider's opaque upload id is not a server path. Only Agena's own
    // immutable references can be resolved by the authenticated media API.
    if source
        .and_then(|s| s.get("source"))
        .and_then(serde_json::Value::as_str)
        == Some("file_id")
        && !value.starts_with("agena:media:")
    {
        return None;
    }
    let hash = text("sha256")
        .or_else(|| {
            artifact
                .and_then(|a| a.get("sha256"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| {
            reference
                .and_then(|r| r.get("sha256"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or_default();
    server_image_reference(value, mime, hash)
}

fn server_image_reference(value: &str, mime: Option<&str>, hash: &str) -> Option<ImageReference> {
    let path = value.strip_prefix("file://").unwrap_or(value);
    if path.is_empty()
        || path.starts_with("data:")
        || path.starts_with("http:")
        || path.starts_with("https:")
        || path.starts_with("blob:")
    {
        return None;
    }
    Some((
        path.to_owned(),
        mime.filter(|m| m.starts_with("image/"))
            .map(str::to_owned)
            .unwrap_or_else(|| image_mime_from_path(path)),
        hash.to_owned(),
    ))
}

/// A Read result describes the original file in two payload objects, while
/// its actual attachment may already be a frozen, session-owned blob. Those
/// descriptions are not additional media resources. Require exact byte and
/// presentation identity before treating them as metadata; unrelated payload
/// images and legacy results without authoritative attachments still work.
fn read_output_has_bound_image_metadata(
    object: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    use serde_json::Value;

    let Some(payload) = object.get("payload").and_then(Value::as_object) else {
        return false;
    };
    let Some(description) = payload.get("attachment").and_then(Value::as_object) else {
        return false;
    };
    let Some(info) = payload.get("read_info").and_then(Value::as_object) else {
        return false;
    };
    let Some(path) = description.get("path").and_then(Value::as_str) else {
        return false;
    };
    if path.trim().is_empty()
        || info.get("path").and_then(Value::as_str) != Some(path)
        || !payload
            .get("loaded_paths")
            .and_then(Value::as_array)
            .is_some_and(|paths| paths.iter().any(|value| value.as_str() == Some(path)))
        || description.get("kind").and_then(Value::as_str) != Some("image")
        || !description
            .get("mime")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("image/"))
        || !description
            .get("size_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|size| size > 0)
    {
        return false;
    }
    let Some(hash) = info
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    else {
        return false;
    };
    // The localizer gives source/artifact/reference precedence over path.
    // Compare that effective identity too, so an overriding resource or an
    // explicitly different attachment hash is never suppressed as metadata.
    if description.get("sha256").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|value| !hash.eq_ignore_ascii_case(value))
    }) || ![description, info].iter().all(|metadata| {
        image_reference(metadata).is_some_and(|(reference, mime, revision)| {
            reference == path
                && Some(mime.as_str()) == description.get("mime").and_then(Value::as_str)
                && (revision.is_empty() || hash.eq_ignore_ascii_case(&revision))
        })
    }) {
        return false;
    }
    let Some(attachments) = object.get("attachments").and_then(Value::as_array) else {
        return false;
    };
    attachments.iter().filter_map(Value::as_object).any(|item| {
        if !item
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(|value| hash.eq_ignore_ascii_case(value))
            || !["kind", "mime", "size_bytes"].iter().all(|key| {
                description.get(*key) == info.get(*key) && description.get(*key) == item.get(*key)
            })
            || !["filename", "width", "height", "duration_ms", "page_count"]
                .iter()
                .all(|key| {
                    [description, info].iter().all(|metadata| {
                        metadata
                            .get(*key)
                            .filter(|value| !value.is_null())
                            .is_none_or(|value| item.get(*key) == Some(value))
                    })
                })
        {
            return false;
        }
        let Some(source) = item.get("source").and_then(Value::as_object) else {
            return false;
        };
        let text = |key: &str| source.get(key).and_then(Value::as_str).unwrap_or_default();
        match text("source") {
            "local_path" => !text("path").is_empty(),
            "url" => ["agena:media:", "file://", "https://", "http://"]
                .iter()
                .any(|prefix| text("url").starts_with(prefix)),
            "data_url" => {
                text("url").starts_with("data:")
                    && text("url")
                        .split_once(',')
                        .is_some_and(|(_, data)| !data.is_empty())
            }
            "base64" => !text("data").is_empty(),
            "file_id" => text("file_id").starts_with("agena:media:"),
            _ => false,
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageReferenceContext {
    Resource,
    ReadPayload,
    ReadMetadata,
}

impl ImageReferenceContext {
    fn child(self, key: &str, bound_read_metadata: bool) -> Self {
        if self == Self::ReadPayload && matches!(key, "read_info" | "attachment") {
            Self::ReadMetadata
        } else if bound_read_metadata && key == "payload" {
            Self::ReadPayload
        } else {
            Self::Resource
        }
    }
}

fn workspace_image_references(value: &serde_json::Value, output: &mut Vec<ImageReference>) {
    collect_workspace_image_references(value, output, ImageReferenceContext::Resource);
}

fn collect_workspace_image_references(
    value: &serde_json::Value,
    output: &mut Vec<ImageReference>,
    context: ImageReferenceContext,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_workspace_image_references(value, output, ImageReferenceContext::Resource);
            }
        }
        serde_json::Value::Object(object) => {
            if context != ImageReferenceContext::ReadMetadata
                && let Some(reference) = image_reference(object)
            {
                output.push(reference);
            }
            let bound_read_metadata = read_output_has_bound_image_metadata(object);
            for (key, value) in object {
                collect_workspace_image_references(
                    value,
                    output,
                    context.child(key, bound_read_metadata),
                );
            }
        }
        _ => {}
    }
}

fn strip_server_localization_metadata(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                strip_server_localization_metadata(value);
            }
        }
        serde_json::Value::Object(object) => {
            object.remove(agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD);
            for value in object.values_mut() {
                strip_server_localization_metadata(value);
            }
        }
        _ => {}
    }
}

fn replace_workspace_image_references(
    value: &mut serde_json::Value,
    images: &HashMap<ImageReference, Arc<str>>,
    remaining: &mut usize,
) {
    replace_workspace_image_references_in(
        value,
        images,
        remaining,
        ImageReferenceContext::Resource,
    );
}

fn replace_workspace_image_references_in(
    value: &mut serde_json::Value,
    images: &HashMap<ImageReference, Arc<str>>,
    remaining: &mut usize,
    context: ImageReferenceContext,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                replace_workspace_image_references_in(
                    value,
                    images,
                    remaining,
                    ImageReferenceContext::Resource,
                );
            }
        }
        serde_json::Value::Object(object) => {
            let bound_read_metadata = read_output_has_bound_image_metadata(object);
            let replacement = (context != ImageReferenceContext::ReadMetadata)
                .then(|| image_reference(object))
                .flatten()
                .and_then(|reference| images.get(&reference))
                .cloned();
            if let Some(url) = replacement.filter(|url| url.len() <= *remaining) {
                *remaining -= url.len();
                if object.contains_key("source") {
                    let original = object
                        .insert(
                            "source".into(),
                            serde_json::json!({"source":"data_url","url":url.as_ref()}),
                        )
                        .expect("attachment source exists");
                    object.insert(
                        agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD.into(),
                        original,
                    );
                } else if let Some(artifact) = object
                    .get_mut("artifact")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    artifact.insert("uri".into(), serde_json::json!(url.as_ref()));
                } else if let Some(reference) = object
                    .get_mut("reference")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    if reference.contains_key("uri") {
                        reference.insert("uri".into(), serde_json::json!(url.as_ref()));
                    } else {
                        *reference = serde_json::json!({"reference_type":"url","url":url.as_ref()})
                            .as_object()
                            .unwrap()
                            .clone();
                    }
                } else if object.contains_key("path") {
                    object.remove("path");
                    object.insert("data_url".into(), serde_json::json!(url.as_ref()));
                } else {
                    object.insert("uri".into(), serde_json::json!(url.as_ref()));
                }
            }
            for (key, value) in object.iter_mut() {
                if key != agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD {
                    replace_workspace_image_references_in(
                        value,
                        images,
                        remaining,
                        context.child(key, bound_read_metadata),
                    );
                }
            }
        }
        _ => {}
    }
}

fn cache_localized_image(
    cache: &mut HashMap<(i64, String, String), Arc<str>>,
    key: (i64, String, String),
    value: Arc<str>,
) {
    // Mutable file paths without a content revision must be re-read. Never
    // reuse a session's authorization result for a different conversation.
    if key.2.is_empty() || value.len() > MAX_LOCALIZED_IMAGE_BYTES {
        return;
    }
    let used: usize = cache.values().map(|v| v.len()).sum();
    if used.saturating_add(value.len()) > MAX_LOCALIZED_IMAGE_BYTES {
        cache.clear();
    }
    cache.insert(key, value);
}

fn image_mime_from_path(path: &str) -> String {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        _ => "image/png",
    }
    .to_owned()
}

impl TuiBackend {
    pub(crate) async fn stream_content(
        &self,
        session_id: i64,
        id: agena_domain::ContentId,
        after: Option<agena_domain::ContentCursor>,
    ) -> Result<agena_client::ContentSubscription> {
        Ok(self
            .inner
            .client
            .stream_content(session_id, id, after, 64 * 1024)
            .await?)
    }
    /// Connect to and validate a server, then resolve the local
    /// workspace path into the server's public workspace identity.
    /// Pure in-memory transport fixture. Never handshakes with a real server.
    #[cfg(test)]
    pub(crate) fn isolated_test_backend(workspace_root: PathBuf) -> Self {
        Self {
            inner: Arc::new(RemoteBackend {
                client: AgenaClient::new("http://127.0.0.1:9").expect("test endpoint"),
                workspace_id: 1,
                providers: Default::default(),
                models: Default::default(),
                configured_provider_adapter_models: Default::default(),
                provider_drafts: Default::default(),
                config_sources: Default::default(),
                runtime_status: Default::default(),
                mcp_server_control: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                plugin_inspects: Default::default(),
                plugin_logs: Default::default(),
                permission_tools: Default::default(),
                plugin_notifications: Default::default(),
                plugin_notifications_baseline: Default::default(),
                activity_kinds: Default::default(),
                workspace_files: Default::default(),
                workspace_file_index: Default::default(),
                workspace_directory_cache: Default::default(),
                workspace_image_data_urls: Default::default(),
                aws_profiles: Default::default(),
                model_catalog: Default::default(),
            }),
            instance_id: format!("inline-ui-test-{}", workspace_root.display()).into(),
            workspace_root: Arc::new(workspace_root),
            media_workspace: Arc::new(tempfile::tempdir().expect("test media root")),
        }
    }

    /// HTTP transport fixture backed only by the endpoint supplied by its test.
    #[cfg(test)]
    pub(crate) fn isolated_test_backend_with_endpoint(
        workspace_root: PathBuf,
        endpoint: &str,
    ) -> Self {
        let mut backend = Self::isolated_test_backend(workspace_root);
        Arc::get_mut(&mut backend.inner)
            .expect("unshared test backend")
            .client = AgenaClient::new(endpoint).expect("test endpoint");
        backend
    }

    pub async fn connect_remote(
        server_url: impl AsRef<str>,
        workspace_root: PathBuf,
    ) -> Result<Self> {
        Self::connect_remote_authenticated(server_url, Some(workspace_root), None, None).await
    }

    pub async fn connect_remote_authenticated(
        server_url: impl AsRef<str>,
        workspace_root: Option<PathBuf>,
        server_token: Option<&str>,
        server_password: Option<&str>,
    ) -> Result<Self> {
        let client =
            AgenaClient::connect_server(server_url.as_ref(), server_token, server_password)
                .await
                .context("server readiness/authentication handshake failed")?;
        let identity = client
            .server_identity()
            .await
            .context("failed to load persistent server identity")?;
        let runtime_status = client
            .runtime_status()
            .await
            .context("failed to load the server runtime status")?;
        let workspace_root = match workspace_root {
            Some(workspace_root) => workspace_root,
            None => PathBuf::from(runtime_status.workspace_root.as_str()),
        };
        let workspace = client
            .command(Command::ResolveWorkspace(
                agena_api::commands::ResolveWorkspaceParams {
                    path: workspace_root.to_string_lossy().into_owned(),
                    create_if_missing: true,
                },
            ))
            .await
            .context("failed to resolve the TUI workspace through the server")?;
        let CommandResult::Workspace(workspace) = workspace else {
            bail!("server returned the wrong result while resolving the workspace");
        };
        let workspace_root = PathBuf::from(workspace.path.as_str());
        let providers = match client.query(Query::ListProviders).await? {
            QueryResult::Providers(providers) => providers,
            _ => bail!("server returned the wrong provider-list result"),
        };
        let mut models = HashMap::new();
        let mut configured_provider_adapter_models = HashMap::new();
        for provider in &providers {
            let response = client
                .configured_provider_models(provider.provider_id.as_str())
                .await
                .with_context(|| {
                    format!(
                        "failed to load models for provider {} from the server",
                        provider.provider_id
                    )
                })?;
            let resources = response.models;
            let provider_models = resources
                .iter()
                .cloned()
                .map(provider_model_from_resource)
                .collect::<Result<Vec<_>>>()
                .with_context(|| {
                    format!(
                        "provider {} returned model metadata incompatible with this TUI",
                        provider.provider_id
                    )
                })?;
            models.insert(provider.provider_id.clone(), provider_models);
            let configured = client
                .configured_provider_adapter_models(provider.provider_id.as_str())
                .await
                .with_context(|| {
                    format!(
                        "failed to load configured routes for provider {} from the server",
                        provider.provider_id
                    )
                })?;
            configured_provider_adapter_models.insert(provider.provider_id.clone(), configured);
        }
        let backend = Self {
            inner: Arc::new(RemoteBackend {
                client,
                workspace_id: workspace.id,
                providers: tokio::sync::RwLock::new(providers),
                models: tokio::sync::RwLock::new(models),
                configured_provider_adapter_models: tokio::sync::RwLock::new(
                    configured_provider_adapter_models,
                ),
                provider_drafts: Default::default(),
                config_sources: Default::default(),
                runtime_status: tokio::sync::RwLock::new(Some(runtime_status)),
                mcp_server_control: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                plugin_inspects: Default::default(),
                plugin_logs: Default::default(),
                permission_tools: Default::default(),
                plugin_notifications: Default::default(),
                plugin_notifications_baseline: Default::default(),
                activity_kinds: Default::default(),
                workspace_files: Default::default(),
                workspace_file_index: Default::default(),
                workspace_directory_cache: Default::default(),
                workspace_image_data_urls: Default::default(),
                aws_profiles: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(workspace_root),
            instance_id: identity.instance_id.into(),
            media_workspace: Arc::new(
                tempfile::tempdir().context("failed to prepare the TUI media cache")?,
            ),
        };
        // Config/plugin snapshots are presentation metadata; a failure here
        // must not prevent the client from connecting and driving sessions,
        // but the diagnostic must remain observable.
        if let Err(error) = backend.refresh_config_sources().await {
            warn_initial_metadata_refresh("configuration sources", &error);
        }
        if let Err(error) = backend.refresh_provider_drafts().await {
            warn_initial_metadata_refresh("provider drafts", &error);
        }
        if let Err(error) = backend.refresh_plugin_runtime_snapshot().await {
            warn_initial_metadata_refresh("plugin runtime snapshot", &error);
        }
        if let Err(error) = backend.refresh_model_catalog_cache("", 0, 1).await {
            warn_initial_metadata_refresh("model catalog", &error);
        }
        // File mentions load the recursive index on first use. Connecting a
        // TUI must not crawl a home directory or trigger macOS privacy prompts.
        if let Err(error) = backend
            .refresh_workspace_directory(backend.workspace_root())
            .await
        {
            warn_initial_metadata_refresh("workspace directory", &error);
        }
        if let Err(error) = backend.refresh_aws_profiles().await {
            warn_initial_metadata_refresh("AWS profiles", &error);
        }
        if let Err(error) = backend.refresh_mcp_server_control().await {
            warn_initial_metadata_refresh("MCP server control", &error);
        }
        Ok(backend)
    }

    pub fn mode(&self) -> BackendMode {
        BackendMode::Remote
    }

    pub fn is_remote(&self) -> bool {
        true
    }

    /// Remote paths are resource identities; never canonicalize them against
    /// the client machine when selecting durable client state.
    pub(crate) fn persistence_scope(&self) -> String {
        format!("{}\0{}", self.instance_id, self.workspace_root.display())
    }

    pub fn workspace_root(&self) -> &Path {
        self.workspace_root.as_path()
    }

    pub(crate) fn workspace_id(&self) -> i64 {
        self.inner.workspace_id
    }

    pub(crate) async fn upload_workspace_attachment(
        &self,
        filename: &str,
        bytes: &[u8],
        mime: Option<&str>,
    ) -> Result<agena_application::dto::WorkspaceFileUploadResource> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let value = self
            .inner
            .client
            .upload_workspace_file(self.inner.workspace_id, filename, encoded.as_str(), mime)
            .await
            .context("failed to upload a client-local attachment into the server workspace")?;
        serde_json::from_value(value)
            .context("the server returned an undecodable workspace upload resource")
    }

    pub(crate) fn media_workspace_root(&self) -> &Path {
        self.media_workspace.path()
    }

    /// Access to the HTTP client for operations ported to REST.
    pub(crate) fn client(&self) -> &AgenaClient {
        &self.inner.client
    }

    /// Read Agena's live MCP server control projection through the remote
    /// HTTP server. The TUI does not own an MCP runtime; this call always
    /// reaches the server process connected during startup.
    pub(crate) async fn mcp_server_control(&self) -> Result<serde_json::Value> {
        self.refresh_mcp_server_control().await
    }

    pub(crate) fn cached_mcp_server_control(&self) -> Option<serde_json::Value> {
        self.inner
            .mcp_server_control
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) async fn refresh_mcp_server_control(&self) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .mcp_server_control()
            .await
            .context("failed to read the Agena MCP server control state")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn toggle_mcp_server(&self) -> Result<serde_json::Value> {
        let current = self.mcp_server_control().await?;
        let enabled = current
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let value = self
            .inner
            .client
            .update_mcp_server_control(!enabled, None)
            .await
            .context("failed to toggle the Agena MCP server")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn set_mcp_server_url(
        &self,
        server_url: Option<String>,
    ) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .set_mcp_server_url(server_url)
            .await
            .context("failed to update the Agena access URL")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    /// The cached configuration-source read model, if it has been loaded from
    /// the server. Synchronous because settings presentation is built inside
    /// the TUI event loop.
    pub(crate) fn config_sources(&self) -> Option<agena_application::dto::ConfigJsonSources> {
        self.inner
            .config_sources
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) fn runtime_status(&self) -> Option<agena_api::resource::RuntimeStatusResponse> {
        self.inner
            .runtime_status
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) async fn refresh_runtime_status_cache(
        &self,
    ) -> Result<agena_api::resource::RuntimeStatusResponse> {
        let status = self.inner.client.runtime_status().await?;
        *self.inner.runtime_status.write().await = Some(status.clone());
        Ok(status)
    }

    /// The resolved UI preferences projected from the server's effective
    /// configuration, for launching the terminal with the same
    /// theme/graphics/locale as an embedded runtime.
    pub fn tui_preferences(&self) -> crate::TuiPreferencesResource {
        super::config::ui_configuration(self)
    }

    /// Refresh the cached configuration-source read model from the server:
    /// the global config file, the workspace config file, and the resolved
    /// effective document. The resolved-config endpoint wraps the effective
    /// value and layer metadata in `{ config, meta }`.
    pub(crate) async fn refresh_config_sources(
        &self,
    ) -> Result<agena_application::dto::ConfigJsonSources> {
        use agena_application::dto::ConfigJsonSources;
        let client = &self.inner.client;
        let global = client.settings_layer_value("global", "").await?;
        let workspace = client.settings_layer_value("workspace", "").await?;
        let resolved = client.resolved_config().await?;
        let (effective, applied_layers) = config_and_layers_from_resolved_response(resolved)?;
        let config_path = global
            .get("config_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        let config_found = global
            .get("config_found")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let project_config_path = workspace
            .get("config_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        let project_config_found = workspace
            .get("config_found")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let sources = ConfigJsonSources {
            config_path,
            config_found,
            project_config_path,
            project_config_found,
            applied_layers,
            file: global
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            project_file: workspace
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            effective,
        };
        *self.inner.config_sources.write().await = Some(sources.clone());
        Ok(sources)
    }

    async fn refresh_after_config_edit(&self, path: &str) -> Result<()> {
        self.refresh_config_sources().await?;
        let path = path.trim();
        let root = path.split('.').next().unwrap_or_default();
        match root {
            "providers" => {
                self.refresh_provider_runtime_snapshot().await?;
                return Ok(());
            }
            "plugins" => self.refresh_plugin_runtime_snapshot().await?,
            _ => {}
        }
        self.refresh_runtime_status_cache().await?;
        Ok(())
    }

    /// The cached plugin surface catalog, if loaded from the server.
    pub(crate) fn plugin_catalog(&self) -> Option<agena_plugin_host::PluginSurfaceCatalog> {
        self.inner
            .plugin_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The cached plugin statuses, if loaded from the server.
    pub(crate) fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.inner
            .plugin_statuses
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Refresh the cached plugin snapshot from the server: statuses and the
    /// combined TUI/studio UI catalog.
    pub(crate) async fn refresh_plugin_runtime_snapshot(&self) -> Result<()> {
        self.refresh_plugin_presentation_snapshot().await?;
        let client = &self.inner.client;
        let statuses = plugin_statuses_from_response(client.plugin_statuses().await?)?;
        let mut inspects = self.inner.plugin_inspects.read().await.clone();
        let mut logs = self.inner.plugin_logs.read().await.clone();
        inspects.retain(|plugin_id, _| {
            statuses
                .iter()
                .any(|status| status.plugin_id.to_string() == *plugin_id)
        });
        logs.retain(|plugin_id, _| {
            statuses
                .iter()
                .any(|status| status.plugin_id.to_string() == *plugin_id)
        });
        let mut requests = tokio::task::JoinSet::new();
        for status in &statuses {
            let plugin_id = status.plugin_id.to_string();
            let client = client.clone();
            requests.spawn(async move {
                let (inspect, logs) = tokio::join!(
                    client.plugin_inspect(plugin_id.as_str()),
                    client.plugin_logs(plugin_id.as_str(), None, 200),
                );
                (plugin_id, inspect, logs)
            });
        }
        while let Some(result) = requests.join_next().await {
            let Ok((plugin_id, inspect, plugin_logs)) = result else {
                continue;
            };
            if let Ok(value) = inspect
                && let Some(value) = value.get("plugin").cloned()
                && let Ok(inspect) =
                    serde_json::from_value::<agena_plugin_host::PluginInspect>(value)
            {
                inspects.insert(plugin_id.clone(), inspect);
            }
            if let Ok(value) = plugin_logs
                && let Some(value) = value.get("logs").cloned()
                && let Ok(records) =
                    serde_json::from_value::<Vec<agena_plugin_host::PluginLogRecord>>(value)
            {
                logs.insert(plugin_id, records);
            }
        }
        *self.inner.plugin_statuses.write().await = statuses;
        *self.inner.plugin_inspects.write().await = inspects;
        *self.inner.plugin_logs.write().await = logs;
        Ok(())
    }

    /// Refresh only frame-consumed plugin presentation metadata. This is
    /// intentionally separate from workbench status/inspect/log reads so a
    /// run lifecycle signal costs one HTTP request instead of one per plugin.
    pub(crate) async fn refresh_plugin_presentation_snapshot(&self) -> Result<()> {
        let catalog_response = self.inner.client.plugin_surface_catalog().await?;
        let catalog = catalog_response
            .get("catalog")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let catalog = serde_json::from_value::<agena_plugin_host::PluginSurfaceCatalog>(catalog)
            .context("the server returned an undecodable plugin surface catalog")?;
        let permission_tools = catalog_response
            .get("permission_tools")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned an undecodable permission tool catalog")?
            .unwrap_or_default();
        let notifications = catalog_response
            .get("notifications")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable plugin notifications")?
            .unwrap_or_default();
        let activity_kinds = catalog_response
            .get("activity_kinds")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable activity kinds")?
            .unwrap_or_else(agena_domain::builtin_activity_kinds);
        if let Some(server_time) = catalog_response
            .get("notifications_at_ms")
            .and_then(serde_json::Value::as_i64)
        {
            let _ = self.inner.plugin_notifications_baseline.set(server_time);
        }
        *self.inner.plugin_catalog.write().await = Some(catalog);
        *self.inner.permission_tools.write().await = permission_tools;
        *self.inner.plugin_notifications.write().await = notifications;
        *self.inner.activity_kinds.write().await = activity_kinds;
        Ok(())
    }

    pub(crate) fn permission_tools(
        &self,
    ) -> Vec<agena_application::dto::PermissionToolCatalogResource> {
        self.inner
            .permission_tools
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn plugin_notifications(&self) -> Vec<agena_plugin_host::HostNotification> {
        self.inner
            .plugin_notifications
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn plugin_notifications_baseline_ms(&self) -> i64 {
        self.inner
            .plugin_notifications_baseline
            .get()
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn activity_kinds(&self) -> Vec<agena_domain::ActivityKind> {
        self.inner
            .activity_kinds
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .filter(|kinds| !kinds.is_empty())
            .unwrap_or_else(agena_domain::builtin_activity_kinds)
    }

    pub(crate) async fn refresh_workspace_file_tree(&self) -> Result<()> {
        let value = self
            .inner
            .client
            .workspace_file_tree(self.inner.workspace_id, None, 64, 50_000, true)
            .await?;
        let tree: agena_application::dto::WorkspaceFileTreeResource = serde_json::from_value(value)
            .context("the server returned an undecodable workspace file tree")?;
        let mut index = Vec::new();
        collect_workspace_file_paths(&tree.entries, &mut index);
        index.sort();
        *self.inner.workspace_files.write().await = Some(tree);
        *self.inner.workspace_file_index.write().await = index;
        Ok(())
    }

    pub(crate) fn workspace_file_index(&self) -> Vec<PathBuf> {
        self.inner
            .workspace_file_index
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) async fn refresh_workspace_directory(&self, path: &Path) -> Result<()> {
        let relative = self.workspace_relative_path(path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            bail!("workspace directory path is outside the active server workspace");
        }
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        let value = self
            .inner
            .client
            .workspace_file_tree(
                self.inner.workspace_id,
                (!relative_text.is_empty()).then_some(relative_text.as_str()),
                0,
                5_000,
                false,
            )
            .await?;
        let tree: agena_application::dto::WorkspaceFileTreeResource = serde_json::from_value(value)
            .context("the server returned an undecodable workspace directory")?;
        self.inner
            .workspace_directory_cache
            .write()
            .map_err(|_| anyhow::anyhow!("workspace directory cache lock poisoned"))?
            .insert(tree.path, tree.entries);
        Ok(())
    }

    pub(crate) fn workspace_directory_entries(
        &self,
        path: &Path,
    ) -> Vec<agena_application::dto::WorkspaceFileNode> {
        let relative = self.workspace_relative_path(path);
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if let Some(entries) = self
            .inner
            .workspace_directory_cache
            .read()
            .ok()
            .and_then(|guard| guard.get(relative_text.as_str()).cloned())
        {
            return entries;
        }
        let tree = self
            .inner
            .workspace_files
            .try_read()
            .ok()
            .and_then(|guard| guard.clone());
        let Some(tree) = tree else {
            return Vec::new();
        };
        if relative.as_os_str().is_empty() {
            return tree.entries;
        }
        find_workspace_file_node(&tree.entries, relative.as_path())
            .filter(|node| node.kind == agena_application::dto::WorkspaceFileKind::Directory)
            .map(|node| node.children.clone())
            .unwrap_or_default()
    }

    pub(crate) fn workspace_path_metadata(&self, path: &Path) -> Option<WorkspacePathMetadata> {
        let relative = self.workspace_relative_path(path);
        if relative.as_os_str().is_empty() {
            return Some(WorkspacePathMetadata {
                is_directory: true,
                size: None,
            });
        }
        let node = self
            .inner
            .workspace_files
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
            .and_then(|tree| find_workspace_file_node(&tree.entries, relative.as_path()).cloned())
            .or_else(|| {
                self.inner
                    .workspace_directory_cache
                    .read()
                    .ok()
                    .and_then(|guard| {
                        guard.values().find_map(|entries| {
                            find_workspace_file_node(entries, relative.as_path()).cloned()
                        })
                    })
            })?;
        match node.kind {
            agena_application::dto::WorkspaceFileKind::Directory => Some(WorkspacePathMetadata {
                is_directory: true,
                size: None,
            }),
            agena_application::dto::WorkspaceFileKind::File => Some(WorkspacePathMetadata {
                is_directory: false,
                size: node.size,
            }),
            agena_application::dto::WorkspaceFileKind::Symlink
            | agena_application::dto::WorkspaceFileKind::Other => None,
        }
    }

    fn workspace_relative_path(&self, path: &Path) -> PathBuf {
        let relative = if path.is_absolute() {
            path.strip_prefix(self.workspace_root())
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.to_path_buf())
        } else {
            path.to_path_buf()
        };
        // Cache keys come back from the server without trailing separators.
        // Rebuild from components so equivalent client inputs such as
        // `target`, `target/`, and `./target` address the same page.
        relative.components().collect()
    }

    pub(crate) async fn refresh_aws_profiles(&self) -> Result<()> {
        let value = self.inner.client.aws_profile_names().await?;
        let profiles = value
            .get("profiles")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable AWS profile names")?
            .unwrap_or_default();
        *self.inner.aws_profiles.write().await = profiles;
        Ok(())
    }

    pub(crate) fn aws_profiles(&self) -> Vec<String> {
        self.inner
            .aws_profiles
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) async fn download_workspace_file(&self, path: &str) -> Result<(String, Vec<u8>)> {
        self.inner
            .client
            .download_workspace_file(self.inner.workspace_id, path)
            .await
            .context("failed to download the server workspace file")
    }

    async fn download_server_image(
        &self,
        session_id: i64,
        path: &str,
    ) -> Result<(String, Vec<u8>)> {
        // The session endpoint resolves workspace-relative files, managed
        // uploads, generated images, and immutable media under the owning
        // session's authorization. Never resolve history against the active
        // workspace or fall back to an unrelated same-named file.
        self.inner
            .client
            .download_session_media_file(session_id, path)
            .await
            .context("failed to download server-managed session media")
    }

    async fn localize_workspace_images(&self, execution: &mut SessionExecutionResource) {
        self.localize_workspace_image_parts(execution.session.id, &mut execution.parts)
            .await;
    }

    async fn localize_workspace_image_parts(&self, session_id: i64, parts: &mut [PartResource]) {
        let mut references = Vec::new();
        for part in parts.iter_mut() {
            strip_server_localization_metadata(&mut part.content);
            workspace_image_references(&part.content, &mut references);
            if let Some(presentation) = &part.presentation {
                for block in &presentation.blocks {
                    if let agena_domain::ViewBlock::Media { artifact, .. } = block
                        && artifact.mime.starts_with("image/")
                        && let Some(reference) = server_image_reference(
                            &artifact.uri,
                            Some(&artifact.mime),
                            artifact.sha256.as_deref().unwrap_or_default(),
                        )
                    {
                        references.push(reference);
                    }
                }
            }
        }
        let localized = self
            .load_workspace_image_references(session_id, references)
            .await;
        let mut presentation_bytes = MAX_LOCALIZED_IMAGE_BYTES;
        for part in parts.iter_mut() {
            replace_workspace_image_references(
                &mut part.content,
                &localized,
                &mut presentation_bytes,
            );
            if let Some(presentation) = &mut part.presentation {
                for block in &mut presentation.blocks {
                    if let agena_domain::ViewBlock::Media { artifact, .. } = block
                        && artifact.mime.starts_with("image/")
                        && let Some(reference) = server_image_reference(
                            &artifact.uri,
                            Some(&artifact.mime),
                            artifact.sha256.as_deref().unwrap_or_default(),
                        )
                        && let Some(url) = localized.get(&reference)
                        && url.len() <= presentation_bytes
                    {
                        presentation_bytes -= url.len();
                        artifact.uri = url.to_string();
                    }
                }
            }
        }
    }

    async fn localize_tool_detail_images(
        &self,
        session_id: i64,
        detail: &mut agena_api::live::ToolDetailResource,
    ) {
        // Detail values are fresh server facts, including conditional-cache
        // hits. A server-supplied client marker is never localization proof.
        strip_server_localization_metadata(&mut detail.value);
        if !matches!(
            detail.section,
            agena_api::live::ToolDetailSection::Output
                | agena_api::live::ToolDetailSection::Presentation
        ) {
            return;
        }
        let mut references = Vec::new();
        workspace_image_references(&detail.value, &mut references);
        let localized = self
            .load_workspace_image_references(session_id, references)
            .await;
        let mut budget = MAX_LOCALIZED_IMAGE_BYTES;
        replace_workspace_image_references(&mut detail.value, &localized, &mut budget);
    }

    async fn load_workspace_image_references(
        &self,
        session_id: i64,
        mut references: Vec<ImageReference>,
    ) -> HashMap<ImageReference, Arc<str>> {
        use sha2::Digest as _;
        const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
        references.sort();
        references.dedup();
        let mut localized = HashMap::<ImageReference, Arc<str>>::new();
        let mut batch_bytes = 0usize;
        for (path, mime, revision) in references {
            // Different revisions of the same path can coexist in history.
            // Keep their lookup identities intact throughout the whole batch.
            let reference = (path.clone(), mime.clone(), revision.clone());
            let key = (session_id, path.clone(), revision);
            let cached = self
                .inner
                .workspace_image_data_urls
                .read()
                .await
                .get(&key)
                .cloned();
            let value = if let Some(value) = cached {
                value
            } else {
                if self
                    .workspace_path_metadata(Path::new(&path))
                    .and_then(|m| m.size)
                    .is_some_and(|size| size > MAX_IMAGE_BYTES)
                {
                    continue;
                }
                let Ok((_, bytes)) = self.download_server_image(session_id, &path).await else {
                    continue;
                };
                if bytes.len() as u64 > MAX_IMAGE_BYTES {
                    continue;
                }
                if !key.2.is_empty() {
                    // Some authorized workspace endpoints serve mutable files.
                    // Never label those new bytes with an older content hash.
                    let actual: String = sha2::Sha256::digest(&bytes)
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    if !actual.eq_ignore_ascii_case(&key.2) {
                        continue;
                    }
                }
                let value: Arc<str> = format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
                .into();
                cache_localized_image(
                    &mut *self.inner.workspace_image_data_urls.write().await,
                    key,
                    Arc::clone(&value),
                );
                value
            };
            if batch_bytes.saturating_add(value.len()) > MAX_LOCALIZED_IMAGE_BYTES {
                break;
            }
            batch_bytes += value.len();
            localized.insert(reference, value);
        }
        localized
    }

    async fn localized_execution(
        &self,
        mut execution: SessionExecutionResource,
    ) -> SessionExecutionResource {
        self.localize_workspace_images(&mut execution).await;
        execution
    }

    pub(crate) fn plugin_inspect(
        &self,
        plugin_id: &str,
    ) -> Option<agena_plugin_host::PluginInspect> {
        self.inner
            .plugin_inspects
            .try_read()
            .ok()
            .and_then(|guard| guard.get(plugin_id.trim()).cloned())
    }

    pub(crate) fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord> {
        let mut records = self
            .inner
            .plugin_logs
            .try_read()
            .ok()
            .and_then(|guard| guard.get(plugin_id.trim()).cloned())
            .unwrap_or_default();
        if let Some(after_seq) = after_seq {
            records.retain(|record| record.seq > after_seq);
        }
        if records.len() > limit {
            records.drain(..records.len() - limit);
        }
        records
    }

    pub async fn invoke_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> std::result::Result<
        agena_plugin_host::PluginToolInvokeResponse,
        agena_application::ApplicationError,
    > {
        let response = if plugin_id == "agena.plan"
            && tool_name == "get"
            && let Some(session_id) = session_id
        {
            self.client().session_plan(session_id, false).await
        } else {
            self.client()
                .invoke_plugin_tool(plugin_id, tool_name, input, session_id)
                .await
        }
        .map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "failed to invoke plugin tool `{tool_name}` through the server: {}",
                error.operator_diagnostic()
            ))
        })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal_error_with_context(
                format!("plugin tool `{tool_name}` returned a response this TUI cannot decode"),
                &error,
            )
        })?;
        if tool_name != "get"
            && let Err(error) = self.refresh_plugin_presentation_snapshot().await
        {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                "plugin tool invocation succeeded, but refreshing the TUI plugin presentation snapshot failed"
            );
        }
        Ok(response)
    }

    /// Build the Provider Studio draft from the authenticated server snapshot.
    /// Draft reads stay synchronous because provider-workbench navigation runs
    /// in the terminal event loop; the cache is refreshed at connect and after
    /// every provider mutation.
    pub fn provider_config_draft(
        &self,
        provider_id: Option<&str>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderConfigDraft,
        agena_application::ApplicationError,
    > {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = agena_application::provider_studio::ProviderConfigDraft::new_empty();
            draft.normalize_shape();
            return Ok(draft);
        };
        self.inner
            .provider_drafts
            .try_read()
            .ok()
            .and_then(|guard| guard.get(provider_id).cloned())
            .ok_or_else(|| {
                agena_application::ApplicationError::internal(format!(
                    "provider draft is not loaded: {provider_id}"
                ))
            })
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        let value = self
            .client()
            .provider_studio_operation(
                "draft/models",
                serde_json::json!({
                    "draft": draft,
                    "adapter_ids": adapter_ids,
                }),
            )
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to list provider draft models through the server: {error}"
                ))
            })?;
        serde_json::from_value(value).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned undecodable provider draft models: {error}"
            ))
        })
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        self.provider_studio_auth_operation("auth/start", draft)
            .await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        self.provider_studio_auth_operation("auth/continue", draft)
            .await
    }

    async fn provider_studio_auth_operation(
        &self,
        operation: &str,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        let value = self
            .client()
            .provider_studio_operation(operation, serde_json::json!({ "draft": draft }))
            .await
            .map_err(agena_application::provider_studio::ProviderDraftAuthError::other)?;
        serde_json::from_value(value).map_err(|error| {
            agena_application::provider_studio::ProviderDraftAuthError::other(format!(
                "failed to decode Provider Studio authentication response: {}",
                agena_failure::diagnostic::format_error_chain(&error),
            ))
        })
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        self.client()
            .list_saved_provider_adapter_models(
                provider_id,
                agena_api::resource::SavedProviderAdapterModelsRequest {
                    adapter_ids: adapter_ids.to_vec(),
                },
            )
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to list saved provider adapter models through the server: {error}"
                ))
            })
    }

    pub fn provider_model_draft_value(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&agena_api::resource::ProviderModelResource>,
    ) -> std::result::Result<serde_json::Value, agena_application::ApplicationError> {
        let adapter_id = adapter_id.trim();
        let model_id = model_id.trim();
        if adapter_id.is_empty() || model_id.is_empty() {
            return Err(agena_application::ApplicationError::internal(
                "adapter id and model id are required",
            ));
        }
        if let Some(provider_id) = draft.source_provider_id.as_deref()
            && let Some(value) = self.config_sources().and_then(|sources| {
                sources
                    .file
                    .get("providers")?
                    .get(provider_id)?
                    .get("adapters")?
                    .get(adapter_id)?
                    .get("models")?
                    .get(model_id)
                    .cloned()
            })
            && !value.is_null()
        {
            return Ok(value);
        }
        agena_application::provider_studio::provider_model_draft_value_from_resource(
            model_id,
            provider_model,
        )
        .map_err(|error| {
            agena_application::ApplicationError::internal_error_with_context(
                "failed to serialize provider model draft settings",
                &error,
            )
        })
    }

    pub async fn save_provider_draft(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
        model_config_values: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "save",
            serde_json::json!({
                "draft": draft,
                "adapter_model_lists": adapter_model_lists,
                "selected_adapter_ids": selected_adapter_ids,
                "selected_model_keys": selected_model_keys,
                "model_config_values": model_config_values,
            }),
        )
        .await
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_models: agena_api::resource::ProviderAdapterModelsResource,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "save-adapter",
            serde_json::json!({
                "draft": draft,
                "adapter_models": adapter_models,
            }),
        )
        .await
    }

    pub async fn save_provider_model_value(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        model_value: serde_json::Value,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "save-model",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
                "model_id": model_id,
                "model_value": model_value,
            }),
        )
        .await
    }

    pub async fn delete_provider_model(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "delete-model",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
                "model_id": model_id,
            }),
        )
        .await
    }

    pub async fn delete_provider(
        &self,
        provider_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "delete-provider",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn delete_provider_adapter(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "delete-adapter",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
            }),
        )
        .await
    }

    async fn provider_studio_save_operation(
        &self,
        operation: &str,
        body: serde_json::Value,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let value = self
            .client()
            .provider_studio_operation(operation, body)
            .await
            .map_err(provider_studio_transport_error)?;
        let result: agena_application::provider_studio::ProviderStudioSaveResult =
            serde_json::from_value(value).map_err(|error| {
                provider_studio_transport_error(format_args!(
                    "failed to decode Provider Studio save response: {}",
                    agena_failure::diagnostic::format_error_chain(&error),
                ))
            })?;
        self.refresh_config_sources()
            .await
            .map_err(provider_studio_transport_error)?;
        self.refresh_provider_runtime_snapshot()
            .await
            .map_err(provider_studio_transport_error)?;
        Ok(result)
    }

    async fn refresh_provider_drafts(&self) -> Result<()> {
        let providers = self.inner.providers.read().await.clone();
        let mut drafts = self.inner.provider_drafts.read().await.clone();
        drafts.retain(|provider_id, _| {
            providers
                .iter()
                .any(|provider| provider.provider_id == *provider_id)
        });
        let mut failures = Vec::new();
        for provider in providers {
            let provider_id = provider.provider_id;
            let response = self
                .client()
                .provider_studio_draft(Some(provider_id.as_str()))
                .await;
            match response {
                Ok(value) => match serde_json::from_value(value) {
                    Ok(draft) => {
                        drafts.insert(provider_id, draft);
                    }
                    Err(error) => failures.push(format!(
                        "{provider_id}: the server returned an undecodable draft: {error}"
                    )),
                },
                Err(error) => failures.push(format!("{provider_id}: {error}")),
            }
        }
        *self.inner.provider_drafts.write().await = drafts;
        if failures.is_empty() {
            Ok(())
        } else {
            tracing::warn!(
                failed_provider_count = failures.len(),
                diagnostics = ?failures,
                "some Provider Studio drafts could not be refreshed"
            );
            bail!(
                "failed to refresh {} Provider Studio draft(s): {}",
                failures.len(),
                failures.join("; ")
            )
        }
    }

    pub(crate) async fn refresh_provider_runtime_snapshot(&self) -> Result<()> {
        let providers = match self.client().query(Query::ListProviders).await? {
            QueryResult::Providers(providers) => providers,
            _ => bail!("server returned the wrong provider-list result"),
        };
        let mut models = HashMap::new();
        let mut configured_adapter_models = HashMap::new();
        let mut drafts = self
            .inner
            .provider_drafts
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        drafts.retain(|provider_id, _| {
            providers
                .iter()
                .any(|provider| provider.provider_id == *provider_id)
        });
        for provider in &providers {
            let response = self
                .client()
                .configured_provider_models(provider.provider_id.as_str())
                .await?;
            let resources = response.models;
            let provider_models = resources
                .iter()
                .cloned()
                .map(provider_model_from_resource)
                .collect::<Result<Vec<_>>>()?;
            models.insert(provider.provider_id.clone(), provider_models);
            configured_adapter_models.insert(
                provider.provider_id.clone(),
                self.client()
                    .configured_provider_adapter_models(provider.provider_id.as_str())
                    .await?,
            );

            if let Ok(value) = self
                .client()
                .provider_studio_draft(Some(provider.provider_id.as_str()))
                .await
                && let Ok(draft) = serde_json::from_value(value)
            {
                drafts.insert(provider.provider_id.clone(), draft);
            }
        }
        *self.inner.providers.write().await = providers;
        *self.inner.models.write().await = models;
        *self.inner.configured_provider_adapter_models.write().await = configured_adapter_models;
        *self.inner.provider_drafts.write().await = drafts;
        self.refresh_runtime_status_cache().await?;
        Ok(())
    }

    /// Set a GLOBAL config file setting through the server, reloading the
    /// runtime when the edit requires it. The server validates the edit
    /// against the full composed configuration and owns the file write.
    pub async fn set_config_setting(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .set_settings_layer_value("global", path.trim(), value, false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to set global config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the setting was saved but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Delete a GLOBAL config file setting through the server, reloading the
    /// runtime when the edit requires it.
    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .delete_settings_layer_value("global", path.trim(), false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to delete global config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the setting was deleted but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Set a WORKSPACE-scoped config file setting through the server,
    /// reloading the runtime when the edit requires it.
    pub async fn set_workspace_config_setting(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .set_settings_layer_value("workspace", path.trim(), value, false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to set workspace config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the workspace setting was saved but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Delete a WORKSPACE-scoped config file setting through the server,
    /// reloading the runtime when the edit requires it.
    pub async fn delete_workspace_config_setting(
        &self,
        path: &str,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .delete_settings_layer_value("workspace", path.trim(), false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to delete workspace config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the workspace setting was deleted but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Set a session's selected permission policy through the server.
    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> std::result::Result<SessionExecutionResource, agena_application::ApplicationError> {
        let resource: agena_api::resource::PermissionConfigResource =
            serde_json::from_value(serde_json::to_value(permission).map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to encode session permission: {error}"
                ))
            })?)
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to encode session permission: {error}"
                ))
            })?;
        self.client()
            .set_session_permission(session_id, resource)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to update session permission through the server: {error}"
                ))
            })
    }

    pub async fn session_overview(
        &self,
        workspace_id: Option<i64>,
        recent_limit: u64,
    ) -> Result<SessionOverviewResource> {
        Ok(self
            .client()
            .session_overview(workspace_id.or(Some(self.inner.workspace_id)), recent_limit)
            .await?)
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        let page = self
            .list_sessions(ListSessionsParams {
                workspace_id: Some(self.inner.workspace_id),
                roots: roots_only,
                exclude_subagents: true,
                limit: Some(200),
                ..Default::default()
            })
            .await?;
        Ok(page.items)
    }

    pub async fn list_workspace_sessions_page(
        &self,
        roots_only: bool,
        exclude_subagents: bool,
        search: Option<&str>,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<SessionResource>> {
        self.list_sessions(ListSessionsParams {
            workspace_id: Some(self.inner.workspace_id),
            roots: roots_only,
            exclude_subagents,
            search: search.map(str::to_owned),
            cursor,
            limit: Some(limit),
            ..Default::default()
        })
        .await
    }

    pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>> {
        Ok(self
            .list_sessions(ListSessionsParams {
                workspace_id: Some(self.inner.workspace_id),
                parent_id: Some(parent_id),
                limit: Some(200),
                ..Default::default()
            })
            .await?
            .items)
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        let mut cursor = self.get_session(session_id).await?;
        while let Some(parent_id) = cursor.parent_id {
            cursor = self.get_session(parent_id).await?;
        }
        let result = self
            .client()
            .command(Command::ListSessionTree(
                agena_api::commands::ListSessionTreeParams { root_id: cursor.id },
            ))
            .await?;
        let CommandResult::SessionTree(items) = result else {
            bail!("server returned the wrong session-tree result");
        };
        Ok(items)
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        let result = self
            .client()
            .command(Command::UpdateSession(UpdateSessionParams {
                session_id,
                title,
                expected_version: None,
            }))
            .await?;
        let CommandResult::Session(session) = result else {
            bail!("server returned the wrong session-update result");
        };
        Ok(session)
    }

    pub async fn set_session_additional_system_prompt(
        &self,
        session_id: i64,
        value: Option<String>,
    ) -> Result<SessionResource> {
        Ok(self
            .client()
            .set_session_additional_system_prompt(session_id, value)
            .await?)
    }

    pub async fn set_session_favorite(
        &self,
        session_id: i64,
        favorite: bool,
    ) -> Result<SessionResource> {
        Ok(self
            .client()
            .set_session_favorite(session_id, favorite)
            .await?)
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource> {
        Ok(self
            .client()
            .create_session_with_auto_title(self.inner.workspace_id, title, parent_id, true)
            .await?)
    }

    pub async fn get_session_state(&self, session_id: i64) -> Result<SessionExecutionResource> {
        Ok(self
            .get_session_state_with_transcript_page(session_id)
            .await?
            .execution)
    }

    pub(crate) async fn get_session_state_with_transcript_page(
        &self,
        session_id: i64,
    ) -> Result<SessionStateWithTranscriptPage> {
        self.transcript_page_with_execution(session_id, None).await
    }

    async fn transcript_page_with_execution(
        &self,
        session_id: i64,
        mut initial: Option<SessionExecutionResource>,
    ) -> Result<SessionStateWithTranscriptPage> {
        // `/state` is an execution shell. Load only the newest bounded
        // run windows separately. Disclosure and role grouping are local.
        let mut attempts = 0;
        let mut retained_parts = std::collections::BTreeMap::new();
        let (mut execution, page_resource, needs_reconciliation) = loop {
            let (execution, page) = tokio::try_join!(
                async {
                    if let Some(execution) = initial.take() {
                        Ok(execution)
                    } else {
                        self.client()
                            .get_session_state_validated(session_id, attempts > 0)
                            .await
                    }
                },
                self.client().session_run_window(
                    session_id,
                    SESSION_TRANSCRIPT_PAGE_SIZE,
                    TRANSCRIPT_PART_PAGE_SIZE,
                    None
                ),
            )?;
            for part in execution.parts.iter().chain(&page.parts) {
                let mut part = part.clone();
                // Merge logic reads local preview provenance, so discard any
                // server claim before the first merge, not only before render.
                strip_server_localization_metadata(&mut part.content);
                if let Some(previous) = retained_parts.get(&part.part_id) {
                    crate::transcript_state::merge_part_snapshot(previous, &mut part);
                }
                retained_parts.insert(part.part_id, part);
            }
            if execution.session.version == page.version {
                break (execution, page, false);
            }
            attempts += 1;
            if attempts >= 3 {
                // Streaming can advance on every read. The returned parts
                // remain useful by identity and revision; show them now and
                // request another bounded refresh for the state/page boundary.
                break (execution, page, true);
            }
        };
        // Queued input is outside the conversational run window. Read the
        // backend queue head explicitly for the footer, without paging past
        // the active reply or loading the whole queue's documents.
        if let Some(head) = execution.queued_messages.first()
            && !retained_parts.contains_key(&head.message_id)
        {
            let queued = self
                .client()
                .session_parts_by_ids(session_id, &[head.message_id])
                .await?;
            for part in queued.parts {
                retained_parts.insert(part.part_id, part);
            }
        }
        let folds = agena_tui_transcript::folds_from_run_window(&page_resource);
        let mut page = SessionTranscriptPage {
            parts: page_resource.parts,
            folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        // The shell can carry pending owners outside the paged tail; command
        // responses can also contain useful committed parts. Preserve both.
        execution.parts = retained_parts.into_values().collect();
        self.localize_workspace_images(&mut execution).await;
        page.parts = execution.parts.clone();
        Ok(SessionStateWithTranscriptPage {
            execution,
            page,
            needs_reconciliation,
        })
    }

    pub(crate) async fn list_session_transcript_page(
        &self,
        session_id: i64,
        limit: u64,
        cursor: &str,
    ) -> Result<SessionTranscriptPage> {
        let page_resource = self
            .client()
            .session_run_window(session_id, limit, TRANSCRIPT_PART_PAGE_SIZE, Some(cursor))
            .await?;
        let folds = agena_tui_transcript::folds_from_run_window(&page_resource);
        let mut page = SessionTranscriptPage {
            parts: page_resource.parts,
            folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        self.localize_workspace_image_parts(session_id, &mut page.parts)
            .await;
        Ok(page)
    }

    pub(crate) async fn list_session_transcript_fold_page(
        &self,
        session_id: i64,
        run_ids: &[i64],
        limit: u64,
        cursor: &str,
    ) -> Result<SessionTranscriptPage> {
        let page_resource = self
            .client()
            .session_run_parts_window(session_id, run_ids, limit, Some(cursor))
            .await?;
        let folds = agena_tui_transcript::folds_from_run_window(&page_resource);
        let mut page = SessionTranscriptPage {
            parts: page_resource.parts,
            folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        self.localize_workspace_image_parts(session_id, &mut page.parts)
            .await;
        Ok(page)
    }

    pub async fn list_session_parts(
        &self,
        session_id: i64,
        limit: u64,
        cursor: Option<&str>,
    ) -> Result<agena_api::live::SessionPartsResource> {
        let mut page = self
            .client()
            .session_parts_page(session_id, limit, cursor)
            .await?;
        for part in &mut page.parts {
            strip_server_localization_metadata(&mut part.content);
        }
        Ok(page)
    }

    pub(crate) async fn get_tool_detail(
        &self,
        session_id: i64,
        part_id: i64,
        section: agena_api::live::ToolDetailSection,
    ) -> Result<agena_api::live::ToolDetailResource> {
        let mut detail = self
            .client()
            .session_tool_detail(session_id, part_id, section)
            .await?;
        anyhow::ensure!(
            detail.part_id == part_id && detail.section == section,
            "the server returned a different tool detail section"
        );
        self.localize_tool_detail_images(session_id, &mut detail)
            .await;
        Ok(detail)
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        force: bool,
    ) -> Result<SessionRefresh> {
        let mut execution = self
            .client()
            .get_session_state_validated(session_id, force)
            .await?;
        for part in &mut execution.parts {
            strip_server_localization_metadata(&mut part.content);
        }
        if !force && after_seq.is_some() && execution.latest_event_seq == after_seq {
            return Ok(SessionRefresh {
                execution_only: Some(execution),
                reconciled_parts: None,
                reconciliation_error: None,
                latest_event_seq: after_seq,
                event_count: 0,
                snapshot: None,
            });
        }
        let snapshot = self
            .transcript_page_with_execution(session_id, Some(execution))
            .await?;
        let latest_event_seq = snapshot.execution.latest_event_seq;
        let event_count = after_seq
            .zip(latest_event_seq)
            .map(|(after, current)| current.saturating_sub(after).clamp(0, 256) as usize)
            .unwrap_or(0);
        Ok(SessionRefresh {
            execution_only: None,
            reconciled_parts: None,
            reconciliation_error: None,
            latest_event_seq,
            event_count,
            snapshot: Some(snapshot),
        })
    }

    pub async fn submit_document(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .submit_message(agena_api::commands::SubmitRunParams {
                session_id,
                options,
                document,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn continue_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self.client().continue_run(session_id, options).await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self.client().compact_session(session_id, options).await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .update_session_selection(session_id, options)
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: Option<agena_domain::ExecutionId>,
    ) -> Result<agena_domain::CancellationOutcome> {
        Ok(self.client().cancel_run(session_id, execution_id).await?)
    }

    pub async fn rewind_session_to_message(
        &self,
        session_id: i64,
        at_message_id: i64,
    ) -> Result<SessionExecutionResource> {
        let result = self
            .client()
            .command(Command::RewindSession(RewindSessionParams {
                session_id,
                at_message_id,
                expected_version: None,
            }))
            .await?;
        let execution = execution_result(result, "session rewind")?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn fork_session(
        &self,
        session_id: i64,
        title: Option<String>,
        conversation_mode: Option<agena_domain::ConversationMode>,
    ) -> Result<SessionExecutionResource> {
        let result = self
            .client()
            .command(Command::ForkSession(ForkSessionParams {
                conversation_mode,
                session_id,
                at_message_id: None,
                title,
            }))
            .await?;
        let execution = execution_result(result, "session fork")?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn reply_permission(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: PermissionReply,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .reply_permission(agena_api::commands::ReplyPermissionParams {
                session_id,
                options,
                reply,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn reply_user_input(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: UserInputReply,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .reply_user_input(agena_api::commands::ReplyUserInputParams {
                session_id,
                options,
                reply,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .mark_interactive_request_presented(session_id, request_id.as_str())
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderSummaryResource>> {
        Ok(self.provider_summaries())
    }

    pub(crate) fn provider_summaries(&self) -> Vec<ProviderSummaryResource> {
        self.inner
            .providers
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn list_local_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>> {
        let models = self.inner.models.try_read().map_err(|error| {
            anyhow::anyhow!("provider model cache is busy while reading {provider_id}: {error}")
        })?;
        Ok(models.get(provider_id.trim()).cloned().unwrap_or_default())
    }

    pub(crate) fn configured_provider_adapter_models(
        &self,
        provider_id: &str,
    ) -> Vec<agena_api::resource::ProviderAdapterModelsResource> {
        self.inner
            .configured_provider_adapter_models
            .try_read()
            .ok()
            .and_then(|guard| guard.get(provider_id.trim()).cloned())
            .unwrap_or_default()
    }

    /// The cached model-catalog page, if one has been loaded from the server.
    /// Synchronous because the settings studio reads model counts inside the
    /// TUI event loop. An empty response is returned when nothing is cached
    /// yet, so sync consumers can render without blocking on HTTP.
    pub(crate) fn model_catalog(&self) -> agena_application::dto::ModelCatalogListResponse {
        self.inner
            .model_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(agena_application::dto::ModelCatalogListResponse::empty)
    }

    /// Fetch a model-catalog page from the server and cache it for
    /// synchronous readers.
    pub(crate) async fn refresh_model_catalog_cache(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<()> {
        let value = self
            .inner
            .client
            .model_catalog(query, None, offset, limit)
            .await
            .context("failed to load the model catalog from the server")?;
        let response = serde_json::from_value(value)
            .context("the server returned an undecodable model catalog")?;
        *self.inner.model_catalog.write().await = Some(response);
        Ok(())
    }

    pub(crate) async fn match_model_catalog_models(
        &self,
        model_ids: &[String],
    ) -> Result<Vec<agena_application::dto::CatalogModelMatchResource>> {
        let response = self
            .inner
            .client
            .model_catalog_matches(model_ids)
            .await
            .context("failed to match provider models against the model catalog")?;
        serde_json::from_value(response.get("items").cloned().unwrap_or_default())
            .context("the server returned undecodable model-catalog matches")
    }

    pub(crate) fn configured_model(
        &self,
        model: &agena_domain::ModelRef,
    ) -> Option<agena_domain::Model> {
        let models = match self.list_local_provider_models(model.provider_id.as_ref()) {
            Ok(models) => models,
            Err(error) => {
                tracing::warn!(
                    provider_id = %model.provider_id,
                    model_id = %model.model_id,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "read the TUI provider model cache",
                        error.as_ref(),
                    ),
                    "configured model lookup could not read the current cache"
                );
                return None;
            }
        };
        models.into_iter().find(|candidate| {
            candidate.id == model.model_id
                && model
                    .adapter_id
                    .as_ref()
                    .is_none_or(|adapter_id| candidate.adapter_id.as_ref() == Some(adapter_id))
        })
    }

    pub fn resolved_model_for_run_options(
        &self,
        request: &RunOptions,
    ) -> std::result::Result<agena_domain::ModelRef, agena_application::ApplicationError> {
        if let Some(model) = request.model.as_ref() {
            return match model.adapter_id.as_deref() {
                Some(adapter_id) => agena_domain::ModelRef::try_new_with_adapter(
                    model.provider_id.as_str(),
                    adapter_id,
                    model.model_id.as_str(),
                ),
                None => agena_domain::ModelRef::try_new(
                    model.provider_id.as_str(),
                    model.model_id.as_str(),
                ),
            }
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "run option contains an invalid model reference: {error}"
                ))
            });
        }
        if let Some(selection) = self.global_default_selection()
            && let (Some(provider_id), Some(model_id)) =
                (selection.provider.as_deref(), selection.model.as_deref())
        {
            return match selection.adapter.as_deref() {
                Some(adapter_id) => {
                    agena_domain::ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id)
                }
                None => agena_domain::ModelRef::try_new(provider_id, model_id),
            }
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "global default model reference is invalid: {error}"
                ))
            });
        }
        Err(agena_application::ApplicationError::bad_request(
            "model is required; select a model or configure the global default model before running",
        ))
    }

    pub fn resolved_model_default_modes(
        &self,
        request: &RunOptions,
    ) -> (Option<String>, Option<String>) {
        let Ok(model_ref) = self.resolved_model_for_run_options(request) else {
            return (None, None);
        };
        let Some(model) = self.configured_model(&model_ref) else {
            return (None, None);
        };
        let configured_modes = if request.model.is_none() {
            self.global_default_selection().filter(|selection| {
                selection.provider.as_deref() == Some(model_ref.provider_id.as_ref())
                    && selection.model.as_deref() == Some(model_ref.model_id.as_ref())
                    && selection.adapter.as_deref()
                        == model_ref
                            .adapter_id
                            .as_ref()
                            .map(|adapter| adapter.as_ref())
            })
        } else {
            None
        };
        let thinking = model
            .thinking_modes
            .iter()
            .find(|mode| mode.is_default)
            .or_else(|| model.thinking_modes.first())
            .and_then(|mode| mode.selector().map(|selector| selector.into_owned()));
        let speed = model
            .speed_modes
            .iter()
            .find(|(_, mode)| mode.is_default)
            .map(|(name, _)| name.clone());
        (
            configured_modes
                .as_ref()
                .and_then(|selection| selection.thinking_mode.clone())
                .or(thinking),
            configured_modes
                .as_ref()
                .and_then(|selection| selection.speed_mode.clone())
                .or(speed),
        )
    }

    fn global_default_selection(&self) -> Option<agena_domain::ModelSelectionConfig> {
        if let Some(selection) = self
            .runtime_status()
            .and_then(|status| status.default_selection)
        {
            return Some(agena_domain::ModelSelectionConfig {
                provider: selection.provider,
                adapter: selection.adapter,
                model: selection.model,
                thinking_mode: selection.thinking_mode,
                speed_mode: selection.speed_mode,
                verbosity: selection.verbosity,
                parallel_tool_calls: None,
            });
        }
        self.config_sources().and_then(|sources| {
            sources
                .effective
                .get("providers")
                .and_then(|providers| providers.get("default_selection"))
                .and_then(|value| {
                    serde_json::from_value::<agena_domain::ModelSelectionConfig>(value.clone()).ok()
                })
        })
    }

    /// Start a session-scoped invalidation stream. Establishing the remote SSE
    /// stream happens inside the spawned task so the synchronous TUI event
    /// handler never blocks. Any event causes snapshot convergence; lag and
    /// transport closure are also surfaced as forced refreshes.
    pub fn subscribe_session_events(&self, session_id: i64) -> mpsc::Receiver<LiveEvent> {
        let backend = self.clone();
        let client = self.client().clone();
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            loop {
                // Subscribe before reading the snapshot. Global scope is
                // required for session-less runtime signals such as dynamic
                // tool-registry changes; session mutations are filtered below.
                let mut subscription = match tokio::select! {
                    _ = tx.closed() => return,
                    result = client.stream_changes(agena_api::Scope::Global) => result,
                } {
                    Ok(subscription) => subscription,
                    Err(error) => {
                        tracing::warn!(
                            session_id,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to establish the TUI session event subscription",
                                &error,
                            ),
                            "TUI session event subscription failed; scheduling convergence retry"
                        );
                        if tx
                            .send(LiveEvent {
                                part_update: None,
                                session_deleted: false,
                                snapshot: None,
                                force_refresh: true,
                                ..crate::LiveEvent::default()
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        tokio::select! {
                            _ = tx.closed() => return,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                        continue;
                    }
                };
                let snapshot = match tokio::select! {
                    _ = tx.closed() => return,
                    result = backend.get_session_state_with_transcript_page(session_id) => result,
                } {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        if error
                            .downcast_ref::<agena_client::ClientError>()
                            .and_then(|error| error.problem())
                            .is_some_and(|problem| {
                                problem.category == agena_failure::FailureCategory::NotFound
                            })
                        {
                            let _ = tx
                                .send(LiveEvent {
                                    part_update: None,
                                    session_deleted: true,
                                    snapshot: None,
                                    force_refresh: false,
                                    ..crate::LiveEvent::default()
                                })
                                .await;
                            return;
                        }
                        tracing::warn!(
                            session_id,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to load the TUI session snapshot after subscribing",
                                error.as_ref(),
                            ),
                            "TUI session snapshot load failed; scheduling convergence retry"
                        );
                        drop(subscription);
                        if tx
                            .send(LiveEvent {
                                part_update: None,
                                session_deleted: false,
                                snapshot: None,
                                force_refresh: true,
                                ..crate::LiveEvent::default()
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        tokio::select! {
                            _ = tx.closed() => return,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                        continue;
                    }
                };
                if tx
                    .send(LiveEvent {
                        part_update: None,
                        session_deleted: false,
                        snapshot: Some(snapshot),
                        force_refresh: false,
                        ..crate::LiveEvent::default()
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                while let Some(item) = tokio::select! {
                    _ = tx.closed() => None,
                    item = subscription.recv() => item,
                } {
                    if let Ok(SubscriptionEvent::RuntimeSignal(signal)) = &item
                        && signal.session_id == Some(session_id)
                    {
                        let incremental = if signal.kind == "activity" {
                            signal
                                .payload
                                .get("activity")
                                .cloned()
                                .and_then(|value| serde_json::from_value(value).ok())
                                .map(|activity| LiveEvent {
                                    activity_update: Some((
                                        signal
                                            .payload
                                            .get("ts_ms")
                                            .and_then(serde_json::Value::as_i64)
                                            .unwrap_or(0),
                                        signal
                                            .payload
                                            .get("reason")
                                            .and_then(serde_json::Value::as_str)
                                            == Some("dismissed"),
                                        activity,
                                    )),
                                    ..LiveEvent::default()
                                })
                        } else if signal.kind == "plugin"
                            && signal
                                .payload
                                .get("kind")
                                .and_then(serde_json::Value::as_str)
                                == Some("plan.changed")
                        {
                            Some(LiveEvent {
                                plan_changed: true,
                                ..LiveEvent::default()
                            })
                        } else {
                            None
                        };
                        if let Some(live) = incremental {
                            if tx.send(live).await.is_err() {
                                return;
                            }
                            continue;
                        }
                    }
                    let reconnect = item
                        .as_ref()
                        .map_or(true, |event| matches!(event, SubscriptionEvent::Lagged(_)));
                    if matches!(&item, Ok(SubscriptionEvent::SessionChanged(agena_api::live::SessionChangeResource::SessionDeleted { session_id: deleted_id, .. })) if *deleted_id == session_id)
                    {
                        let _ = tx
                            .send(LiveEvent {
                                part_update: None,
                                session_deleted: true,
                                snapshot: None,
                                force_refresh: false,
                                ..crate::LiveEvent::default()
                            })
                            .await;
                        return;
                    }
                    if let Ok(SubscriptionEvent::SessionChanged(
                        agena_api::live::SessionChangeResource::PartUpdated {
                            session_id: origin,
                            part,
                        },
                    )) = &item
                        && (!part.is_message_boundary() || *origin != session_id)
                    {
                        let mut part = *part.clone();
                        // Even a forwarded event for another session must
                        // never carry server-asserted client provenance.
                        strip_server_localization_metadata(&mut part.content);
                        if *origin == session_id {
                            // A completed tool arrives as an incremental part,
                            // without a snapshot reload. Resolve its server
                            // attachments through the same authenticated API
                            // used when opening history.
                            tokio::select! {
                                _ = tx.closed() => return,
                                _ = backend.localize_workspace_image_parts(
                                    session_id,
                                    std::slice::from_mut(&mut part),
                                ) => {},
                            }
                        }
                        if tx
                            .send(LiveEvent {
                                part_update: Some((*origin, part)),
                                session_deleted: false,
                                snapshot: None,
                                force_refresh: false,
                                ..crate::LiveEvent::default()
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                    let (force_refresh, refresh_plugin_presentation) = match item.as_ref() {
                        Err(error) => {
                            tracing::warn!(
                                session_id,
                                diagnostic = %error.operator_diagnostic(),
                                "TUI session event stream yielded a transport failure; reconnecting"
                            );
                            (true, true)
                        }
                        Ok(event) => match classify_session_subscription_event(event, session_id) {
                            SessionSubscriptionDispatch::Ignore => continue,
                            SessionSubscriptionDispatch::RefreshPluginRuntime => {
                                if let Err(error) = backend.refresh_plugin_runtime_snapshot().await
                                {
                                    tracing::warn!(
                                        diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                                        "failed to refresh the TUI plugin runtime snapshot after a runtime signal"
                                    );
                                }
                                continue;
                            }
                            SessionSubscriptionDispatch::Emit {
                                refresh_plugin_presentation,
                            } => (
                                matches!(
                                    event,
                                    SubscriptionEvent::Lagged(_)
                                        | SubscriptionEvent::SessionChanged(
                                            agena_api::live::SessionChangeResource::PartRemoved { .. }
                                        )
                                ),
                                refresh_plugin_presentation,
                            ),
                        },
                    };
                    if refresh_plugin_presentation
                        && let Err(error) = backend.refresh_plugin_presentation_snapshot().await
                    {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                            "failed to refresh the TUI plugin presentation snapshot after a session event"
                        );
                    }
                    if tx
                        .send(LiveEvent {
                            part_update: None,
                            session_deleted: false,
                            snapshot: None,
                            force_refresh,
                            ..LiveEvent::default()
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if reconnect {
                        break;
                    }
                }
                // A closed/lagged stream is not replayable. Invalidate,
                // then establish a fresh subscribe-before-snapshot pair.
                if tx
                    .send(LiveEvent {
                        part_update: None,
                        session_deleted: false,
                        snapshot: None,
                        force_refresh: true,
                        ..crate::LiveEvent::default()
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    _ = tx.closed() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                }
            }
        });
        rx
    }

    pub(super) async fn list_sessions(
        &self,
        params: ListSessionsParams,
    ) -> Result<PaginatedResponse<SessionResource>> {
        let result = self
            .client()
            .query_revalidated(Query::ListSessions(params))
            .await?;
        let QueryResult::Sessions(page) = result else {
            bail!("server returned the wrong session-list result");
        };
        Ok(page)
    }

    async fn get_session(&self, session_id: i64) -> Result<SessionResource> {
        let result = self
            .client()
            .query(Query::GetSession(GetSessionParams { session_id }))
            .await?;
        let QueryResult::Session(session) = result else {
            bail!("server returned the wrong session result");
        };
        Ok(session)
    }
}

fn warn_initial_metadata_refresh(operation: &str, error: &anyhow::Error) {
    tracing::warn!(
        operation,
        diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
        "optional TUI startup metadata refresh failed"
    );
}

fn execution_result(result: CommandResult, operation: &str) -> Result<SessionExecutionResource> {
    let CommandResult::Execution(execution) = result else {
        bail!("server returned the wrong result for {operation}");
    };
    Ok(execution)
}

fn provider_model_from_resource(
    model: agena_api::resource::ProviderModelResource,
) -> Result<agena_domain::Model> {
    let metadata = model.metadata;
    let value = serde_json::json!({
        "provider_id": model.provider_id,
        "adapter_id": model.adapter_id,
        "id": model.id,
        "catalog_model_id": model.catalog_model_id,
        "display_name": model.display_name,
        "native_compaction": model.native_compaction,
        // Capabilities are deliberately omitted here. The TUI model picker
        // needs route identity, labels, modes, and limits; the public API uses
        // a tri-state capability contract that must not be coerced back into
        // the runtime's internal boolean model.
        "metadata": {
            "lifecycle": metadata.lifecycle,
            "limits": {
                "context_window_tokens": metadata.context_window_tokens,
                "max_input_tokens": metadata.max_input_tokens,
                "max_output_tokens": metadata.max_output_tokens,
            },
            "description": metadata.description,
            "knowledge_cutoff": metadata.knowledge_cutoff,
            "release_date": metadata.release_date,
            "last_updated": metadata.last_updated,
            "open_weights": metadata.open_weights,
            "supports_parallel_tool_calls": metadata.supports_parallel_tool_calls,
            "supports_verbosity": metadata.supports_verbosity,
            "default_verbosity": metadata.default_verbosity,
            "verbosity_levels": metadata.verbosity_levels,
            "provider_default_verbosity": metadata.provider_default_verbosity,
            "default_temperature": metadata.default_temperature,
            "default_top_p": metadata.default_top_p,
            "default_top_k": metadata.default_top_k,
            "assistant_reasoning_interleaved": metadata.assistant_reasoning_interleaved,
            "assistant_reasoning_field": metadata.assistant_reasoning_field,
            "output_modalities": metadata.output_modalities,
            "pricing": metadata.pricing,
        },
        "thinking_modes": model.thinking_modes,
        "speed_modes": model.speed_modes,
    });
    serde_json::from_value(value).map_err(anyhow::Error::from)
}

fn plugin_statuses_from_response(
    value: serde_json::Value,
) -> Result<Vec<agena_plugin_host::status::PluginStatus>> {
    serde_json::from_value(
        value
            .get("items")
            .cloned()
            .context("the plugin-status response has no `items` field")?,
    )
    .context("the server returned undecodable plugin statuses")
}

fn config_and_layers_from_resolved_response(
    value: serde_json::Value,
) -> Result<(serde_json::Value, Vec<String>)> {
    let effective = value
        .get("config")
        .cloned()
        .context("the resolved-config response has no `config` field")?;
    if !effective.is_object() {
        bail!("the resolved-config response contains a non-object `config` field");
    }
    let applied_layers = value
        .pointer("/meta/applied_layers")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            layer
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    Ok((effective, applied_layers))
}

fn provider_studio_transport_error(
    error: impl std::fmt::Display,
) -> agena_application::provider_studio::ProviderStudioSaveError {
    let diagnostic = error.to_string();
    tracing::error!(diagnostic = %diagnostic, "Provider Studio transport failed");
    let mut presentation = agena_failure::UserPresentation::validated_with_context(
        "tui.provider_studio_transport_failed",
        diagnostic.as_str(),
    );
    if presentation.fallback == "The request is invalid. Review the input and try again." {
        presentation = agena_failure::UserPresentation::new(
            "tui.provider_studio_transport_failed",
            "The Provider Studio transport diagnostic contained no safely displayable details; review the diagnostic log for the recorded failure id.",
        );
    }
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("tui.provider_studio_transport_failed"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterRefresh,
        agena_failure::RecoveryDirective::OpenSettings,
        agena_failure::FailureImpact::RequestRejected,
        presentation,
    );
    agena_application::provider_studio::ProviderStudioSaveError::Other(failure.into())
}

#[cfg(test)]
mod attachment_localization_tests {
    use super::*;

    fn bound_read_output(hash: &str, size: usize) -> serde_json::Value {
        serde_json::json!({
            "payload": {
                "read_info": {
                    "path":"ai-drawings/image.png", "kind":"image", "mime":"image/png",
                    "size_bytes":size, "sha256":hash, "width":20, "height":40,
                    "delivery":"model_input"
                },
                "attachment": {
                    "path":"ai-drawings/image.png", "kind":"image", "mime":"image/png",
                    "filename":"image.png", "size_bytes":size, "width":20, "height":40
                },
                "loaded_paths":["ai-drawings/image.png"],
                "metadata":{"delivery":"model_input"}
            },
            "truncated":false,
            "attachments":[{
                "kind":"image", "mime":"image/png", "filename":"image.png",
                "sha256":hash, "size_bytes":size, "width":20, "height":40,
                "source":{"source":"url","url":"agena:media:v1:fixture"}
            }]
        })
    }

    #[test]
    fn bound_read_metadata_is_not_a_second_resource_or_rewritten_technical_fact() {
        let hash = "a".repeat(64);
        for wrapped in [false, true] {
            let raw = bound_read_output(&hash, 100);
            let payload = raw["payload"].clone();
            let mut value = if wrapped {
                serde_json::json!({"raw_output":raw})
            } else {
                raw
            };
            let mut references = Vec::new();
            workspace_image_references(&value, &mut references);
            assert_eq!(
                references,
                vec![(
                    "agena:media:v1:fixture".into(),
                    "image/png".into(),
                    hash.clone()
                )]
            );
            // Even a pre-existing path cache must not replace original
            // read_info/attachment metadata with presentation data URLs.
            let images = HashMap::from([
                (
                    references[0].clone(),
                    Arc::<str>::from("data:image/png;base64,AAAA"),
                ),
                (
                    (
                        "ai-drawings/image.png".into(),
                        "image/png".into(),
                        hash.clone(),
                    ),
                    Arc::<str>::from("data:image/png;base64,BBBB"),
                ),
                (
                    (
                        "ai-drawings/image.png".into(),
                        "image/png".into(),
                        String::new(),
                    ),
                    Arc::<str>::from("data:image/png;base64,CCCC"),
                ),
            ]);
            let mut budget = MAX_LOCALIZED_IMAGE_BYTES;
            replace_workspace_image_references(&mut value, &images, &mut budget);
            let raw = if wrapped {
                &value["raw_output"]
            } else {
                &value
            };
            assert_eq!(raw["payload"], payload);
            assert_eq!(raw["attachments"][0]["source"]["source"], "data_url");
            assert_eq!(
                raw["attachments"][0]["source"]["url"],
                "data:image/png;base64,AAAA"
            );
        }
    }

    #[test]
    fn read_metadata_requires_matching_effective_path_mime_and_optional_hash() {
        let hash = "a".repeat(64);
        let different_hash = "b".repeat(64);
        let original = bound_read_output(&hash, 100);
        let mut matching = original.clone();
        matching["payload"]["attachment"]["sha256"] = serde_json::json!(hash);
        assert!(read_output_has_bound_image_metadata(
            matching.as_object().unwrap()
        ));

        for (key, value) in [
            ("sha256", serde_json::json!(different_hash)),
            (
                "artifact",
                serde_json::json!({"uri":"ai-drawings/image.png","sha256":different_hash}),
            ),
            (
                "reference",
                serde_json::json!({"path":"ai-drawings/image.png","sha256":different_hash}),
            ),
        ] {
            let mut raw = original.clone();
            raw["payload"]["attachment"][key] = value;
            assert!(
                !read_output_has_bound_image_metadata(raw.as_object().unwrap()),
                "{key}"
            );
            let mut references = Vec::new();
            workspace_image_references(&raw, &mut references);
            assert!(
                references
                    .iter()
                    .any(|item| item.0 == "ai-drawings/image.png" && item.2 == different_hash),
                "{key}"
            );
        }
        for metadata in ["attachment", "read_info"] {
            for (key, value) in [
                (
                    "source",
                    serde_json::json!({"source":"local_path","path":"different.png"}),
                ),
                (
                    "artifact",
                    serde_json::json!({"uri":"file://different.png","mime":"image/png"}),
                ),
                ("reference", serde_json::json!({"path":"different.png"})),
            ] {
                let mut raw = original.clone();
                raw["payload"][metadata][key] = value;
                assert!(
                    !read_output_has_bound_image_metadata(raw.as_object().unwrap()),
                    "{metadata}.{key}"
                );
                let mut references = Vec::new();
                workspace_image_references(&raw, &mut references);
                assert!(
                    references.iter().any(|item| item.0 == "different.png"),
                    "{metadata}.{key}"
                );
            }
        }
    }

    #[test]
    fn unrelated_read_payload_images_and_legacy_fallbacks_remain_resources() {
        let original = bound_read_output(&"a".repeat(64), 100);
        let mut extra = original.clone();
        extra["payload"]["extra_image"] =
            serde_json::json!({"mime":"image/png","path":"extra.png"});
        extra["payload"]["read_info"]["thumbnail"] =
            serde_json::json!({"mime":"image/png","path":"thumbnail.png"});
        let mut references = Vec::new();
        workspace_image_references(&extra, &mut references);
        assert_eq!(references.len(), 3);
        assert!(references.iter().any(|item| item.0 == "extra.png"));
        assert!(references.iter().any(|item| item.0 == "thumbnail.png"));
        assert!(
            !references
                .iter()
                .any(|item| item.0 == "ai-drawings/image.png")
        );

        for (pointer, different) in [
            ("/attachments", serde_json::json!([])),
            ("/attachments/0/sha256", serde_json::json!("b".repeat(64))),
            ("/attachments/0/size_bytes", serde_json::json!(101)),
            ("/attachments/0/width", serde_json::json!(21)),
            ("/attachments/0/filename", serde_json::json!("other.png")),
            (
                "/payload/read_info/path",
                serde_json::json!("other/image.png"),
            ),
            ("/payload/read_info/mime", serde_json::json!("image/jpeg")),
            ("/payload/read_info/sha256", serde_json::Value::Null),
            ("/payload/loaded_paths", serde_json::json!([])),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = different;
            let mut references = Vec::new();
            workspace_image_references(&value, &mut references);
            assert!(
                references
                    .iter()
                    .any(|item| item.0 == "ai-drawings/image.png"),
                "{pointer}"
            );
        }
        let mut legacy = original;
        legacy.as_object_mut().unwrap().remove("attachments");
        let mut references = Vec::new();
        workspace_image_references(&legacy, &mut references);
        assert_eq!(references.len(), 2);
    }

    #[tokio::test]
    async fn read_output_localization_only_downloads_the_owned_media_reference() {
        use agena_api::live::{ToolDetailResource, ToolDetailSection};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::{Duration, timeout};

        const IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAKUlEQVR4nO3MoQEAAAwCIP9/envCJoFKLrmmCIVCoXA2LH9CoVAo3A0fsaQ6G+7F7JIAAAAASUVORK5CYII=";
        const HASH: &str = "cfdb6b08410a8a0be495f5b1c59bd4fc2575d1d0ff7c65dcd67d539bc2a1af50";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(IMAGE)
            .unwrap();
        let raw = bound_read_output(HASH, bytes.len());
        let original_payload = raw["payload"].clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let count = timeout(Duration::from_secs(3), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() < 16 * 1024);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /api/v1/sessions/49/media?"));
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = url::Url::parse(&format!("http://fixture{target}")).unwrap();
            assert_eq!(
                url.query_pairs().find(|(key, _)| key == "path").unwrap().1,
                "agena:media:v1:fixture"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer attachment-test-token\r\n")
            );
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
            drop(socket);
            assert!(
                timeout(Duration::from_millis(300), listener.accept())
                    .await
                    .is_err(),
                "Read metadata must not trigger another filesystem media request"
            );
        });
        let mut backend =
            TuiBackend::isolated_test_backend(PathBuf::from("/server/current-workspace"));
        Arc::get_mut(&mut backend.inner).unwrap().client = AgenaClient::new(endpoint)
            .unwrap()
            .with_bearer_token("attachment-test-token")
            .unwrap();
        let mut detail = ToolDetailResource {
            part_id: 1245,
            revision: 1,
            updated_at_ms: 1,
            section: ToolDetailSection::Output,
            part_state: Some("completed".into()),
            value: raw,
        };
        timeout(
            Duration::from_secs(3),
            backend.localize_tool_detail_images(49, &mut detail),
        )
        .await
        .unwrap();
        assert_eq!(detail.value["payload"], original_payload);
        assert_eq!(
            detail.value["attachments"][0]["source"]["source"],
            "data_url"
        );
        server.await.unwrap();
    }

    #[test]
    fn finds_and_replaces_immutable_generated_and_local_images() {
        let mut value = serde_json::json!([
            {"kind":"image","mime":"image/png","source":{"source":"local_path","path":"/upload/a.png"},"sha256":"a"},
            {"type":"media","artifact":{"uri":"file:///generated/b.png","mime":"image/png","sha256":"b"}},
            {"activity_type":"resource","kind":"image","media_type":"image/png","reference":{"reference_type":"artifact","uri":"agena:media:v1:abcd","sha256":"c"}},
            {"kind":"image","source":{"source":"unavailable","reason":"missing"}}
        ]);
        let mut refs = Vec::new();
        workspace_image_references(&value, &mut refs);
        refs.sort();
        refs.dedup();
        assert_eq!(refs.len(), 3);
        let images = refs
            .iter()
            .map(|reference| {
                (
                    reference.clone(),
                    Arc::<str>::from("data:image/png;base64,AAAA"),
                )
            })
            .collect();
        let mut budget = MAX_LOCALIZED_IMAGE_BYTES;
        replace_workspace_image_references(&mut value, &images, &mut budget);
        assert_eq!(value[0]["source"]["source"], "data_url");
        assert!(
            value[1]["artifact"]["uri"]
                .as_str()
                .unwrap()
                .starts_with("data:")
        );
        assert!(
            value[2]["reference"]["uri"]
                .as_str()
                .unwrap()
                .starts_with("data:")
        );
        assert_eq!(value[3]["source"]["source"], "unavailable");
    }
    #[test]
    fn repeated_presentation_copies_share_one_byte_budget() {
        let item = serde_json::json!({"kind":"image","mime":"image/png","source":{"source":"local_path","path":"/a.png"}});
        let mut value = serde_json::json!([item.clone(), item]);
        let url: Arc<str> = Arc::from("data:image/png;base64,AAAA");
        let mut budget = url.len();
        let images = HashMap::from([(("/a.png".into(), "image/png".into(), String::new()), url)]);
        replace_workspace_image_references(&mut value, &images, &mut budget);
        assert_eq!(budget, 0);
        assert_eq!(value[0]["source"]["source"], "data_url");
        assert_eq!(value[1]["source"]["source"], "local_path");
    }

    #[test]
    fn cache_is_revision_and_session_scoped_and_skips_mutable_paths() {
        let mut cache = HashMap::new();
        cache_localized_image(
            &mut cache,
            (43, "/image.png".into(), "hash-a".into()),
            Arc::from("first"),
        );
        assert!(!cache.contains_key(&(44, "/image.png".into(), "hash-a".into())));
        assert!(!cache.contains_key(&(43, "/image.png".into(), "hash-b".into())));
        cache_localized_image(
            &mut cache,
            (43, "/mutable.png".into(), String::new()),
            Arc::from("uncacheable"),
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn one_localization_batch_keeps_revisions_and_server_markers_separate() {
        use agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD;
        let mut value = serde_json::json!([
            {"kind":"image","mime":"image/png","sha256":"old",
                "source":{"source":"local_path","path":"/same.png"},
                (LOCALIZED_ATTACHMENT_SOURCE_FIELD):{"source":"local_path","path":"/forged.png"}},
            {"kind":"image","mime":"image/png","sha256":"new",
                "source":{"source":"local_path","path":"/same.png"}},
            {"kind":"image","mime":"image/png","sha256":"unavailable",
                "source":{"source":"local_path","path":"/same.png"},
                (LOCALIZED_ATTACHMENT_SOURCE_FIELD):{"source":"local_path","path":"/forged.png"}}
        ]);
        strip_server_localization_metadata(&mut value);
        let images = HashMap::from([
            (
                ("/same.png".into(), "image/png".into(), "old".into()),
                Arc::<str>::from("data:image/png;base64,OLD"),
            ),
            (
                ("/same.png".into(), "image/png".into(), "new".into()),
                Arc::<str>::from("data:image/png;base64,NEW"),
            ),
        ]);
        let mut budget = MAX_LOCALIZED_IMAGE_BYTES;
        replace_workspace_image_references(&mut value, &images, &mut budget);
        assert_eq!(value[0]["source"]["url"], "data:image/png;base64,OLD");
        assert_eq!(value[1]["source"]["url"], "data:image/png;base64,NEW");
        assert_eq!(
            value[0][LOCALIZED_ATTACHMENT_SOURCE_FIELD]["path"],
            "/same.png"
        );
        assert_eq!(
            value[1][LOCALIZED_ATTACHMENT_SOURCE_FIELD]["path"],
            "/same.png"
        );
        assert_eq!(value[2]["source"]["source"], "local_path");
        assert!(value[2].get(LOCALIZED_ATTACHMENT_SOURCE_FIELD).is_none());
    }

    #[test]
    fn file_refs_and_owned_file_ids_localize_without_treating_provider_ids_as_paths() {
        let mut value = serde_json::json!([
            {"mime":"image/png","path":"/upload/image.png","name":"image.png"},
            {"kind":"image","source":{"source":"file_id","file_id":"agena:media:v1:owned"}},
            {"kind":"image","source":{"source":"file_id","file_id":"provider-upload-id"}},
            {"kind":"image","source":{"source":"url","url":"https://images.example.test/image.png"}}
        ]);
        let mut references = Vec::new();
        workspace_image_references(&value, &mut references);
        assert_eq!(references.len(), 2);
        let images = references
            .into_iter()
            .map(|reference| (reference, Arc::<str>::from("data:image/png;base64,AAAA")))
            .collect();
        let mut budget = MAX_LOCALIZED_IMAGE_BYTES;
        replace_workspace_image_references(&mut value, &images, &mut budget);
        assert_eq!(value[0]["data_url"], "data:image/png;base64,AAAA");
        assert!(value[0].get("path").is_none());
        assert_eq!(value[1]["source"]["source"], "data_url");
        assert_eq!(value[2]["source"]["file_id"], "provider-upload-id");
        assert_eq!(value[3]["source"]["source"], "url");
    }

    #[tokio::test]
    async fn output_details_keep_image_previews_across_folds_cache_revisions_and_failed_reads() {
        use agena_api::live::{ToolDetailResource, ToolDetailSection};
        use agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD;
        use agena_tui_transcript::{
            TranscriptActivitySection, TranscriptContentId, TranscriptNodeKey,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::{Duration, timeout};

        const IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAKUlEQVR4nO3MoQEAAAwCIP9/envCJoFKLrmmCIVCoXA2LH9CoVAo3A0fsaQ6G+7F7JIAAAAASUVORK5CYII=";
        const SECOND_IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAK0lEQVR4nO3OsQ0AAAzCMP5/un2CDQ9ZrSS56wYEAoHAVbC8BwQCgcBd8AGxpDobPseZLgAAAABJRU5ErkJggg==";
        const FIRST_HASH: &str = "cfdb6b08410a8a0be495f5b1c59bd4fc2575d1d0ff7c65dcd67d539bc2a1af50";
        const SECOND_HASH: &str =
            "bf74b363920168d5adc6dee64d568ee704949f4355ff4c74a1ea8470d6e4af3b";
        const PATH: &str = "/server/managed/generated.png";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(IMAGE.split_once(',').unwrap().1)
            .unwrap();
        let second_bytes = base64::engine::general_purpose::STANDARD
            .decode(SECOND_IMAGE)
            .unwrap();
        let output = |hash: &str, size: usize| {
            serde_json::json!({
                "payload":{"text":"Finished image"}, "truncated":false,
                "attachments":[{
                    "kind":"image", "mime":"image/png", "filename":"generated.png",
                    "sha256":hash, "size_bytes":size,
                    "source":{"source":"local_path","path":PATH},
                    (LOCALIZED_ATTACHMENT_SOURCE_FIELD):{"source":"local_path","path":"/forged.png"}
                }]
            })
        };
        let first_raw = output(FIRST_HASH, bytes.len());
        let second_raw = output(SECOND_HASH, second_bytes.len());
        let resource = |revision: i64, value: serde_json::Value| ToolDetailResource {
            part_id: 1129,
            revision,
            updated_at_ms: revision,
            section: ToolDetailSection::Output,
            part_state: Some("completed".into()),
            value,
        };
        let first_body = serde_json::to_vec(&resource(1, first_raw.clone())).unwrap();
        let second_body = serde_json::to_vec(&resource(2, second_raw.clone())).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server_bytes = bytes.clone();
        let server = tokio::spawn(async move {
            // An unexpected duplicate media read fails this ordered protocol.
            // 304 bodies are cached by AgenaClient; image caches remain scoped
            // to session + exact path + content revision.
            let replies = vec![
                (
                    "/api/v1/sessions/46/media?",
                    "200 OK",
                    "image/png",
                    "",
                    server_bytes.clone(),
                    None,
                ),
                (
                    "/api/v1/sessions/46/parts/1129/tool-sections/output ",
                    "200 OK",
                    "application/json",
                    "ETag: \"output-1\"\r\nX-Resource-Revision: image-fixture:1\r\n",
                    first_body,
                    None,
                ),
                (
                    "/api/v1/sessions/46/parts/1129/tool-sections/output ",
                    "304 Not Modified",
                    "application/json",
                    "ETag: \"output-1\"\r\nX-Resource-Revision: image-fixture:1\r\n",
                    Vec::new(),
                    Some("output-1"),
                ),
                (
                    "/api/v1/sessions/46/parts/1129/tool-sections/output ",
                    "200 OK",
                    "application/json",
                    "ETag: \"output-2\"\r\nX-Resource-Revision: image-fixture:2\r\n",
                    second_body.clone(),
                    Some("output-1"),
                ),
                (
                    "/api/v1/sessions/46/media?",
                    "200 OK",
                    "image/png",
                    "",
                    second_bytes,
                    None,
                ),
                (
                    "/api/v1/sessions/47/parts/1129/tool-sections/output ",
                    "200 OK",
                    "application/json",
                    "ETag: \"output-2\"\r\nX-Resource-Revision: image-fixture:2\r\n",
                    second_body,
                    None,
                ),
                (
                    "/api/v1/sessions/47/media?",
                    "403 Forbidden",
                    "application/json",
                    "",
                    b"{}".to_vec(),
                    None,
                ),
                (
                    "/api/v1/sessions/46/parts/1129/tool-sections/output ",
                    "409 Conflict",
                    "application/json",
                    "",
                    b"{}".to_vec(),
                    Some("output-2"),
                ),
                (
                    "/api/v1/sessions/46/parts/1129/tool-sections/output ",
                    "304 Not Modified",
                    "application/json",
                    "ETag: \"output-2\"\r\nX-Resource-Revision: image-fixture:2\r\n",
                    Vec::new(),
                    Some("output-2"),
                ),
                (
                    "/api/v1/sessions/46/media?",
                    "503 Service Unavailable",
                    "application/json",
                    "",
                    b"{}".to_vec(),
                    None,
                ),
            ];
            for (expected, status, content_type, headers, body, etag) in replies {
                let (mut socket, _) = timeout(Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let count = timeout(Duration::from_secs(10), socket.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {expected}")), "{request}");
                let headers_in = request.to_ascii_lowercase();
                assert!(headers_in.contains("authorization: bearer attachment-test-token\r\n"));
                if let Some(etag) = etag {
                    assert!(headers_in.contains(&format!("if-none-match: \"{etag}\"\r\n")));
                }
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let mut backend = TuiBackend::isolated_test_backend(PathBuf::from("/server/workspace"));
        Arc::get_mut(&mut backend.inner).unwrap().client = AgenaClient::new(endpoint)
            .unwrap()
            .with_bearer_token("attachment-test-token")
            .unwrap();
        let mut initial_output = first_raw;
        initial_output["payload"] = serde_json::Value::Null;
        let mut parts = [PartResource {
            part_id: 1129,
            run_id: Some(1129),
            message_id: Some(1128),
            message_boundary: true,
            kind: "tool_call".into(),
            role: "assistant".into(),
            state: "completed".into(),
            run_state: Some("completed".into()),
            revision: 1,
            updated_at_ms: 1,
            content: serde_json::json!({"name":"image_generation","call_id":557,"state":"completed",
                "lifecycle":agena_domain::TimeRange::default(),"output":initial_output}),
            ..Default::default()
        }];
        backend.localize_workspace_image_parts(46, &mut parts).await;
        let original = serde_json::json!({"source":"local_path","path":PATH});
        assert_eq!(
            parts[0].content["output"]["attachments"][0][LOCALIZED_ATTACHMENT_SOURCE_FIELD],
            original
        );
        let mut state = crate::TranscriptState::default();
        state.reset(46, "Image session".into());
        state.set_math_render_context(agena_tui_media::MathRenderContext::new(
            None,
            &std::env::temp_dir(),
        ));
        state.merge_parts(parts.to_vec());
        let assert_image = |state: &mut crate::TranscriptState| {
            let rendered = state.rendered(100);
            assert!(rendered.lines.iter().any(|line| line.text.contains('▀')));
            assert_eq!(
                rendered
                    .lines
                    .iter()
                    .filter(|line| line.text.contains('🖼'))
                    .count(),
                1
            );
            assert!(
                !rendered
                    .lines
                    .iter()
                    .any(|line| line.text.contains("iVBORw0KGgo"))
            );
        };
        assert_image(&mut state);
        assert!(!state.tool_detail_is_current(1129, ToolDetailSection::Output));
        let activity = state
            .rendered(100)
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::StoredPart(1129),
                        ..
                    }
                )
            })
            .unwrap()
            .key
            .clone();
        let section_key = |section| TranscriptNodeKey::ActivitySection {
            entry_id: activity.entry_id(),
            content_id: TranscriptContentId::StoredPart(1129),
            section,
        };
        let details = section_key(TranscriptActivitySection::TechnicalDetails);
        let output_key = section_key(TranscriptActivitySection::Output);
        state.node_expansions.insert(details.clone(), true);
        state.node_expansions.insert(output_key.clone(), true);
        state.invalidate_render();
        let requested = state
            .begin_tool_detail_load(1129, ToolDetailSection::Output)
            .unwrap();
        let first = backend
            .get_tool_detail(46, 1129, ToolDetailSection::Output)
            .await
            .unwrap();
        assert!(state.accept_tool_detail_load(1129, ToolDetailSection::Output, requested));
        assert!(state.finish_tool_detail_load(first.clone()));
        assert_image(&mut state);
        assert!(
            state
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains(PATH))
        );
        assert!(
            !state
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains(LOCALIZED_ATTACHMENT_SOURCE_FIELD))
        );
        for key in [&output_key, &details, &activity] {
            state.node_expansions.insert(key.clone(), false);
            state.invalidate_render();
            if key == &activity {
                assert!(
                    !state
                        .rendered(100)
                        .lines
                        .iter()
                        .any(|line| line.text.contains('▀'))
                );
            } else {
                assert_image(&mut state);
            }
            state.node_expansions.insert(key.clone(), true);
            state.invalidate_render();
            assert_image(&mut state);
            assert!(
                state
                    .begin_tool_detail_load(1129, ToolDetailSection::Output)
                    .is_none()
            );
        }

        let cached = backend
            .get_tool_detail(46, 1129, ToolDetailSection::Output)
            .await
            .unwrap();
        assert_eq!(cached.value, first.value);
        assert!(state.finish_tool_detail_load(cached));
        assert_image(&mut state);
        state.parts[0].revision = 2;
        state.parts[0].updated_at_ms = 2;
        let revised = backend
            .get_tool_detail(46, 1129, ToolDetailSection::Output)
            .await
            .unwrap();
        assert_eq!(revised.value["attachments"][0]["sha256"], SECOND_HASH);
        assert_ne!(revised.value["attachments"][0]["source"]["url"], IMAGE);
        assert!(state.finish_tool_detail_load(revised));
        assert!(!state.finish_tool_detail_load(first));
        assert_eq!(
            state.parts[0].content["output"]["attachments"][0]["sha256"],
            SECOND_HASH
        );
        assert_image(&mut state);

        let foreign = backend
            .get_tool_detail(47, 1129, ToolDetailSection::Output)
            .await
            .unwrap();
        assert_eq!(
            foreign.value["attachments"][0]["source"]["source"],
            "local_path"
        );
        assert!(
            foreign.value["attachments"][0]
                .get(LOCALIZED_ATTACHMENT_SOURCE_FIELD)
                .is_none()
        );
        let before = state.parts[0].content.clone();
        assert!(
            backend
                .get_tool_detail(46, 1129, ToolDetailSection::Output)
                .await
                .is_err()
        );
        assert_eq!(state.parts[0].content, before);
        assert_image(&mut state);

        backend
            .inner
            .workspace_image_data_urls
            .write()
            .await
            .clear();
        let failed_media = backend
            .get_tool_detail(46, 1129, ToolDetailSection::Output)
            .await
            .unwrap();
        assert_eq!(
            failed_media.value["attachments"][0]["source"]["source"],
            "local_path"
        );
        assert!(state.finish_tool_detail_load(failed_media));
        assert_eq!(state.parts[0].content, before);
        assert_image(&mut state);
        timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn mismatched_download_hash_cannot_create_a_preview_or_poison_the_cache() {
        use agena_api::live::{ToolDetailResource, ToolDetailSection};
        use agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::{Duration, timeout};

        let bytes = base64::engine::general_purpose::STANDARD.decode(
            "iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAK0lEQVR4nO3OsQ0AAAzCMP5/un2CDQ9ZrSS56wYEAoHAVbC8BwQCgcBd8AGxpDobPseZLgAAAABJRU5ErkJggg=="
        ).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = timeout(Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let count = timeout(Duration::from_secs(10), socket.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with("GET /api/v1/sessions/46/media?"));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer attachment-test-token\r\n")
                );
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&bytes).await.unwrap();
            }
        });
        let mut backend = TuiBackend::isolated_test_backend(PathBuf::from("/server/workspace"));
        Arc::get_mut(&mut backend.inner).unwrap().client = AgenaClient::new(endpoint)
            .unwrap()
            .with_bearer_token("attachment-test-token")
            .unwrap();
        let mut detail = ToolDetailResource {
            part_id: 1129,
            revision: 1,
            updated_at_ms: 1,
            section: ToolDetailSection::Output,
            part_state: Some("completed".into()),
            value: serde_json::json!({"attachments":[{
                "kind":"image","mime":"image/png","size_bytes":98,
                "sha256":"cfdb6b08410a8a0be495f5b1c59bd4fc2575d1d0ff7c65dcd67d539bc2a1af50",
                "source":{"source":"local_path","path":"/server/changed.png"}
            }],"truncated":false}),
        };
        backend.localize_tool_detail_images(46, &mut detail).await;
        assert_eq!(
            detail.value["attachments"][0]["source"]["source"],
            "local_path"
        );
        assert!(
            detail.value["attachments"][0]
                .get(LOCALIZED_ATTACHMENT_SOURCE_FIELD)
                .is_none()
        );
        assert!(
            backend
                .inner
                .workspace_image_data_urls
                .read()
                .await
                .is_empty()
        );
        // A hashless mutable path can still be displayed, but must be read
        // again next time and never be admitted to the revision cache.
        detail.value["attachments"][0]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        backend.localize_tool_detail_images(46, &mut detail).await;
        assert_eq!(
            detail.value["attachments"][0]["source"]["source"],
            "data_url"
        );
        assert!(
            backend
                .inner
                .workspace_image_data_urls
                .read()
                .await
                .is_empty()
        );
        timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn generated_images_use_authenticated_media_and_keep_session_authorization() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::{Duration, timeout};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            // Session 46 owns the image. Session 47 must make its own request
            // and remain denied; no current-workspace fallback is permitted.
            for expected in ["/api/v1/sessions/46/media?", "/api/v1/sessions/47/media?"] {
                let (mut socket, _) = timeout(Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let count = timeout(Duration::from_secs(10), socket.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {expected}")), "{request}");
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer attachment-test-token\r\n")
                );
                let allowed = expected.contains("/sessions/46/");
                let (status, content_type, body) = if allowed {
                    ("200 OK", "image/png", "image-bytes")
                } else {
                    ("403 Forbidden", "application/json", "{}")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let mut backend = TuiBackend::isolated_test_backend(PathBuf::from("/server/workspace"));
        Arc::get_mut(&mut backend.inner).unwrap().client = AgenaClient::new(endpoint)
            .unwrap()
            .with_bearer_token("attachment-test-token")
            .unwrap();
        let part = || {
            PartResource {
            kind: "tool_call".into(),
            content: serde_json::json!({"output":{"attachments":[{
                "kind":"image","mime":"image/png","filename":"generated.png","sha256":"2c8648d103e3dd7ad87660da0f126a1443b6d21ac1bd3ec000c5e24e2373a90c",
                "source":{"source":"local_path","path":"/server/managed/generated.png"}
            }]}}),
            presentation: Some(
                serde_json::from_value(serde_json::json!({
                    "title":"Generated image","summary":"","blocks":[{
                        "type":"media","artifact":{
                            "uri":"file:///server/managed/generated.png","mime":"image/png",
                            "name":"generated.png","sha256":"2c8648d103e3dd7ad87660da0f126a1443b6d21ac1bd3ec000c5e24e2373a90c"
                        }
                    }]
                }))
                .unwrap(),
            ),
            ..Default::default()
        }
        };
        for session_id in [46, 46, 47] {
            let mut parts = [part()];
            timeout(
                Duration::from_secs(10),
                backend.localize_workspace_image_parts(session_id, &mut parts),
            )
            .await
            .unwrap();
            let source = &parts[0].content["output"]["attachments"][0]["source"];
            let agena_domain::ViewBlock::Media { artifact, .. } =
                &parts[0].presentation.as_ref().unwrap().blocks[0]
            else {
                panic!("expected a media block");
            };
            if session_id == 46 {
                assert_eq!(source["source"], "data_url");
                assert_eq!(source["url"], artifact.uri);
                assert!(artifact.uri.starts_with("data:image/png;base64,"));
            } else {
                assert_eq!(source["source"], "local_path");
                assert_eq!(artifact.uri, "file:///server/managed/generated.png");
            }
        }
        timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
    }
}
