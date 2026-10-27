use std::{cell::Cell, collections::BTreeSet, fmt, sync::Arc};

use agena_api::mcp_connections::{
    McpCapabilityPage, McpConnectionCatalog, McpConnectionDetail, McpConnectionScope,
    McpConnectionSummary, McpImportPreview,
};
use agena_tui_components::{ConfirmDialogState, Editor, EditorDialogState};
use portable_atomic::{AtomicU64, Ordering};
use serde_json::{Value, json};

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
pub(super) const REDACTED: &str = "[REDACTED]";
pub(super) const PENDING_RELOAD_HINT: &str =
    "R refresh · T test\nOpen Diagnostics to reload settings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpPane {
    Connections,
    Capabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CapabilityKind {
    Tools,
    Resources,
    Templates,
    Prompts,
}
impl CapabilityKind {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Resources => "resources",
            Self::Templates => "resource_templates",
            Self::Prompts => "prompts",
        }
    }
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Tools => "Tools",
            Self::Resources => "Resources",
            Self::Templates => "Resource templates",
            Self::Prompts => "Prompts",
        }
    }
    pub(super) fn next(self) -> Self {
        match self {
            Self::Tools => Self::Resources,
            Self::Resources => Self::Templates,
            Self::Templates => Self::Prompts,
            Self::Prompts => Self::Tools,
        }
    }
}

#[derive(Clone)]
pub(crate) struct McpWorkbench {
    pub(super) instance: u64,
    pub(super) catalog: Option<McpConnectionCatalog>,
    pub(super) detail: Option<McpConnectionDetail>,
    pub(super) selected: usize,
    pub(super) query: String,
    pub(super) scope_filter: Option<McpConnectionScope>,
    pub(super) target_scope: McpConnectionScope,
    pub(super) pane: McpPane,
    pub(super) compact: Cell<bool>,
    pub(super) catalog_request: u64,
    pub(super) detail_request: u64,
    pub(super) capability_request: u64,
    pub(super) operation_request: u64,
    pub(super) loading: bool,
    pub(super) detail_loading: bool,
    pub(super) capability_loading: bool,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
    pub(super) notice: Option<String>,
    pub(super) kind: CapabilityKind,
    pub(super) capabilities: Option<McpCapabilityPage>,
    pub(super) capability_index: usize,
    pub(super) cursor: Option<String>,
    pub(super) cursor_history: Vec<Option<String>>,
    pub(super) preview: Option<(String, Value)>,
    pub(super) scroll: u16,
    pub(super) draft: Option<McpDraft>,
    pub(super) map_editor: Option<MapEditor>,
    pub(super) import: Option<McpImport>,
    pub(super) authorization: Option<PendingAuthorization>,
    pub(super) editor: Option<EditorDialogState<EditAction>>,
    pub(super) editor_original: String,
    pub(super) confirm: Option<ConfirmDialogState<ConfirmAction>>,
}

// Drafts, import documents, and input buffers can contain credentials. Route
// diagnostics may describe the screen without rendering any of those values.
impl fmt::Debug for McpWorkbench {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpWorkbench")
            .field("instance", &self.instance)
            .field("loading", &self.loading)
            .field("busy", &self.busy)
            .field("editing", &self.draft.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormField {
    Name,
    Scope,
    Transport,
    Enabled,
    Command,
    Arguments,
    Cwd,
    Environment,
    Url,
    Headers,
    Auth,
    Token,
    AuthEnv,
    Scopes,
    ClientId,
    ClientSecret,
    ClientMetadataUrl,
    CustomHeaders,
    Include,
    Exclude,
    ConnectTimeout,
    RequestTimeout,
}

impl FormField {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Scope => "Scope",
            Self::Transport => "Transport",
            Self::Enabled => "Enabled",
            Self::Command => "Command",
            Self::Arguments => "Arguments",
            Self::Cwd => "Working directory",
            Self::Environment => "Environment",
            Self::Url => "URL",
            Self::Headers => "HTTP headers",
            Self::Auth => "Authentication",
            Self::Token => "Bearer token",
            Self::AuthEnv => "Token environment variable",
            Self::Scopes => "OAuth scopes",
            Self::CustomHeaders => "Authentication headers",
            Self::ClientId => "OAuth client ID",
            Self::ClientSecret => "OAuth client secret",
            Self::ClientMetadataUrl => "Client metadata URL",
            Self::Include => "Included tools",
            Self::Exclude => "Excluded tools",
            Self::ConnectTimeout => "Connection timeout (ms)",
            Self::RequestTimeout => "Request timeout (ms)",
        }
    }
    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Command => "/process/command",
            Self::Arguments => "/process/args",
            Self::Cwd => "/process/cwd",
            Self::Environment => "/process/env",
            Self::Url => "/endpoint/url",
            Self::Headers => "/endpoint/headers",
            Self::Token => "/auth/token",
            Self::AuthEnv => "/auth/env",
            Self::Scopes => "/auth/scopes",
            Self::ClientId => "/auth/client_id",
            Self::ClientSecret => "/auth/client_secret",
            Self::ClientMetadataUrl => "/auth/client_metadata_url",
            Self::CustomHeaders => "/auth/headers",
            Self::Include => "/tools/include",
            Self::Exclude => "/tools/exclude",
            Self::ConnectTimeout => "/timeouts/connect_ms",
            Self::RequestTimeout => "/timeouts/request_ms",
            Self::Enabled => "/enabled",
            _ => "",
        }
    }
    pub(super) fn hint(self) -> &'static str {
        match self {
            Self::Name => {
                "A stable connection name. Existing names are immutable; create or import another name to copy a connection."
            }
            Self::Scope => {
                "Global applies across projects. Workspace refers to the running Agena server's workspace shown above."
            }
            Self::Transport => {
                "Enter switches between a local stdio process and a streamable HTTP endpoint. Changing transport resets transport-specific fields."
            }
            Self::Enabled => {
                "Enter enables or disables this connection without deleting its configuration."
            }
            Self::Command => {
                "Executable on the Agena server. Arguments are supplied separately; this is not a shell command string."
            }
            Self::Arguments => {
                "A JSON array of strings. Spaces and empty arguments are preserved exactly, for example [\"-y\",\"@example/mcp\"]."
            }
            Self::Cwd => {
                "Optional working directory on the Agena server. Empty uses the server's default."
            }
            Self::Environment => {
                "Edit process environment keys individually. Values are masked and existing credentials remain unchanged unless explicitly replaced or removed."
            }
            Self::Url => {
                "The streamable HTTP MCP endpoint, including its MCP path. Sensitive URLs are hidden. Keep [REDACTED] to preserve a stored URL, or enter its complete replacement."
            }
            Self::Headers | Self::CustomHeaders => {
                "Edit header names and masked values. Existing values are preserved until explicitly replaced or removed."
            }
            Self::Auth => {
                "Enter cycles: None, inline bearer, environment variable, secret store, OAuth, custom headers. OAuth and stored credentials can be managed after saving."
            }
            Self::Token => {
                "Enter a replacement token in masked input. Leave the stored value unchanged by cancelling. Delete clears it explicitly."
            }
            Self::AuthEnv => {
                "Name of the environment variable available to the Agena server that contains the bearer token."
            }
            Self::Scopes => {
                "Requested OAuth scopes, one per line. Leave client registration fields empty for automatic registration when the service supports it."
            }
            Self::ClientId => {
                "Optional client ID supplied by a pre-registered OAuth application. Leave empty for automatic registration or a client metadata URL. Delete clears this field."
            }
            Self::ClientSecret => {
                "Optional secret for a pre-registered OAuth client. Input is masked; cancelling keeps the current value. Empty input or Delete removes it."
            }
            Self::ClientMetadataUrl => {
                "Optional HTTPS client metadata document URL when the service supports client metadata registration. Leave empty for another registration method. Delete clears this field."
            }
            Self::Include => {
                "Tool-name patterns, one per line. Empty allows all discovered tools. Patterns support *."
            }
            Self::Exclude => {
                "Tool-name patterns to exclude, one per line. Exclusion takes precedence over inclusion."
            }
            Self::ConnectTimeout | Self::RequestTimeout => {
                "Positive timeout in milliseconds. The server validates its supported bounds before saving."
            }
        }
    }
    pub(super) fn is_map(self) -> bool {
        matches!(
            self,
            Self::Environment | Self::Headers | Self::CustomHeaders
        )
    }
    pub(super) fn multiline(self) -> bool {
        matches!(
            self,
            Self::Arguments | Self::Scopes | Self::Include | Self::Exclude
        )
    }
}

#[derive(Clone)]
pub(super) struct McpDraft {
    pub(super) name: String,
    pub(super) scope: McpConnectionScope,
    pub(super) editing: Option<(String, String)>,
    pub(super) config: Value,
    pub(super) preserve_secrets: BTreeSet<String>,
    pub(super) selected: usize,
    pub(super) dirty: bool,
}
impl fmt::Debug for McpDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpDraft")
            .field("name", &self.name)
            .field("scope", &self.scope)
            .field("dirty", &self.dirty)
            .finish_non_exhaustive()
    }
}
impl McpDraft {
    pub(super) fn new(scope: McpConnectionScope) -> Self {
        Self {
            name: String::new(),
            scope,
            editing: None,
            config: default_config("stdio"),
            preserve_secrets: BTreeSet::new(),
            selected: 0,
            dirty: false,
        }
    }
    pub(super) fn from_detail(detail: &McpConnectionDetail) -> Self {
        Self {
            name: detail.summary.name.clone(),
            scope: detail.summary.scope,
            editing: Some((detail.summary.id.clone(), detail.summary.revision.clone())),
            config: detail.config.clone(),
            preserve_secrets: detail.secret_paths.iter().cloned().collect(),
            selected: 0,
            dirty: false,
        }
    }
    pub(super) fn transport(&self) -> &str {
        self.config
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or("stdio")
    }
    pub(super) fn auth(&self) -> &str {
        self.config
            .pointer("/auth/kind")
            .and_then(Value::as_str)
            .unwrap_or("none")
    }
    pub(super) fn enabled(&self) -> bool {
        self.config
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }
    pub(super) fn fields(&self) -> Vec<FormField> {
        use FormField::*;
        let mut fields = vec![Name, Scope, Transport, Enabled];
        if self.transport() == "http" {
            fields.extend([Url, Headers, Auth]);
            match self.auth() {
                "bearer" => fields.push(Token),
                "bearer_from_env" => fields.push(AuthEnv),
                "o_auth" | "oauth" => {
                    fields.extend([Scopes, ClientId, ClientSecret, ClientMetadataUrl])
                }
                "custom" => fields.push(CustomHeaders),
                _ => {}
            }
        } else {
            fields.extend([Command, Arguments, Cwd, Environment]);
        }
        fields.extend([Include, Exclude, ConnectTimeout, RequestTimeout]);
        fields
    }
    pub(super) fn field(&self) -> FormField {
        self.fields()
            .get(self.selected)
            .copied()
            .unwrap_or(FormField::Name)
    }
    pub(super) fn field_value(&self, field: FormField) -> String {
        match field {
            FormField::Name => self.name.clone(),
            FormField::Scope => scope_label(&self.scope).to_owned(),
            FormField::Transport => self.transport().to_owned(),
            FormField::Enabled => if self.enabled() { "Yes" } else { "No" }.to_owned(),
            FormField::Auth => self.auth().replace('_', " "),
            FormField::Token | FormField::ClientSecret => if self
                .config
                .pointer(field.path())
                .and_then(Value::as_str)
                .is_some_and(|v| !v.is_empty())
            {
                "Configured · hidden"
            } else {
                "Not set"
            }
            .to_owned(),
            FormField::Url
                if self
                    .config
                    .pointer(field.path())
                    .and_then(Value::as_str)
                    .is_some_and(secret_url) =>
            {
                "Configured · hidden".to_owned()
            }
            _ if field.is_map() => format!(
                "{} keys · values hidden",
                self.config
                    .pointer(field.path())
                    .and_then(Value::as_object)
                    .map_or(0, |v| v.len())
            ),
            _ => self
                .config
                .pointer(field.path())
                .map(display_value)
                .unwrap_or_default(),
        }
    }
    pub(super) fn set(&mut self, path: &str, value: Value) {
        self.preserve_secrets
            .retain(|p| p != path && !p.starts_with(&format!("{path}/")));
        put_pointer(&mut self.config, path, value);
        self.dirty = true;
    }
    pub(super) fn set_field(&mut self, field: FormField, value: Value) {
        let path = field.path();
        if value.as_str() == Some(REDACTED) && self.preserve_secrets.contains(path) {
            return;
        }
        if value.is_null()
            && matches!(
                field,
                FormField::ClientId | FormField::ClientSecret | FormField::ClientMetadataUrl
            )
        {
            self.remove(path);
        } else {
            self.set(path, value);
        }
    }
    pub(super) fn remove(&mut self, path: &str) {
        self.preserve_secrets
            .retain(|p| p != path && !p.starts_with(&format!("{path}/")));
        remove_pointer(&mut self.config, path);
        self.dirty = true;
    }
    pub(super) fn change_transport(&mut self) {
        let next = if self.transport() == "stdio" {
            "http"
        } else {
            "stdio"
        };
        let mut config = default_config(next);
        for key in ["enabled", "timeouts", "tools"] {
            if let Some(value) = self.config.get(key) {
                config[key] = value.clone();
            }
        }
        self.config = config;
        self.preserve_secrets.clear();
        self.dirty = true;
    }
    pub(super) fn change_auth(&mut self) {
        let kind = match self.auth() {
            "none" => "bearer",
            "bearer" => "bearer_from_env",
            "bearer_from_env" => "bearer_from_store",
            "bearer_from_store" => "o_auth",
            "o_auth" | "oauth" => "custom",
            _ => "none",
        };
        let value = match kind {
            "none" => Value::Null,
            "bearer" => json!({"kind":kind,"token":""}),
            "bearer_from_env" => json!({"kind":kind,"env":""}),
            "o_auth" => json!({"kind":kind,"scopes":[]}),
            "custom" => json!({"kind":kind,"headers":{}}),
            _ => json!({"kind":kind}),
        };
        self.set("/auth", value);
    }
    pub(super) fn redacted_config(&self) -> Value {
        let mut value = self.config.clone();
        for path in self.sensitive_paths() {
            if value.pointer(&path).is_some() {
                put_pointer(&mut value, &path, Value::String(REDACTED.to_owned()));
            }
        }
        value
    }
    fn sensitive_paths(&self) -> BTreeSet<String> {
        let mut paths = self.preserve_secrets.clone();
        for map in ["/process/env", "/endpoint/headers", "/auth/headers"] {
            if let Some(values) = self.config.pointer(map).and_then(Value::as_object) {
                paths.extend(
                    values
                        .keys()
                        .map(|key| format!("{map}/{}", pointer_segment(key))),
                );
            }
        }
        for path in ["/auth/token", "/auth/client_secret"] {
            if self
                .config
                .pointer(path)
                .is_some_and(|value| !value.is_null())
            {
                paths.insert(path.to_owned());
            }
        }
        if self
            .config
            .pointer("/endpoint/url")
            .and_then(Value::as_str)
            .is_some_and(secret_url)
        {
            paths.insert("/endpoint/url".to_owned());
        }
        paths
    }
    pub(super) fn replace_redacted_config(&mut self, mut value: Value) {
        for path in self.sensitive_paths() {
            if value.pointer(&path).and_then(Value::as_str) == Some(REDACTED)
                && let Some(original) = self.config.pointer(&path)
            {
                put_pointer(&mut value, &path, original.clone());
            }
        }
        self.preserve_secrets
            .retain(|path| value.pointer(path).and_then(Value::as_str) == Some(REDACTED));
        self.config = value;
        self.selected = 0;
        self.dirty = true;
    }
}

#[derive(Debug, Clone)]
pub(super) struct MapEditor {
    pub(super) field: FormField,
    pub(super) selected: usize,
}
#[derive(Clone)]
pub(super) struct McpImport {
    pub(super) scope: McpConnectionScope,
    pub(super) document: Arc<Value>,
    pub(super) preview: McpImportPreview,
    pub(super) selected: BTreeSet<String>,
    pub(super) cursor: usize,
    pub(super) replace: bool,
}
#[derive(Debug, Clone)]
pub(super) struct PendingAuthorization {
    pub(super) id: String,
    pub(super) flow_id: String,
    pub(super) authorization_url: String,
    pub(super) expires_at: String,
}
#[derive(Debug, Clone)]
pub(super) enum EditAction {
    Search,
    Field(FormField),
    MapKey(FormField),
    MapValue { field: FormField, key: String },
    RawConfig,
    Import,
    StoredToken { id: String },
    OAuthRedirect { id: String },
    OAuthCallback { id: String, flow_id: String },
    ResourceUri { id: String },
    PromptArguments { id: String, name: String },
}
impl EditAction {
    pub(super) fn secret(&self, text: &str) -> bool {
        matches!(
            self,
            Self::Field(FormField::Token | FormField::ClientSecret)
                | Self::MapValue { .. }
                | Self::StoredToken { .. }
                | Self::OAuthCallback { .. }
        ) || matches!(self, Self::Field(FormField::Url)) && secret_url(text)
    }
}
#[derive(Debug, Clone)]
pub(super) enum ConfirmAction {
    DiscardEditor,
    DiscardDraft,
    Delete { id: String, revision: String },
    RemoveMapKey { path: String },
    Import,
    DeleteCredential { id: String },
    Logout { id: String },
    CancelAuthorization { id: String, flow_id: String },
}

impl McpWorkbench {
    pub(super) fn new() -> Self {
        Self {
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            catalog: None,
            detail: None,
            selected: 0,
            query: String::new(),
            scope_filter: None,
            target_scope: McpConnectionScope::Workspace,
            pane: McpPane::Connections,
            compact: Cell::new(false),
            catalog_request: 0,
            detail_request: 0,
            capability_request: 0,
            operation_request: 0,
            loading: false,
            detail_loading: false,
            capability_loading: false,
            busy: false,
            error: None,
            notice: None,
            kind: CapabilityKind::Tools,
            capabilities: None,
            capability_index: 0,
            cursor: None,
            cursor_history: Vec::new(),
            preview: None,
            scroll: 0,
            draft: None,
            map_editor: None,
            import: None,
            authorization: None,
            editor: None,
            editor_original: String::new(),
            confirm: None,
        }
    }
    pub(super) fn visible(&self) -> Vec<&McpConnectionSummary> {
        let query = self.query.to_lowercase();
        self.catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .connections
                    .iter()
                    .filter(|c| {
                        self.scope_filter
                            .as_ref()
                            .is_none_or(|scope| scope == &c.scope)
                            && (query.is_empty()
                                || c.name.to_lowercase().contains(&query)
                                || c.transport.contains(&query)
                                || c.state.contains(&query))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn selected_connection(&self) -> Option<&McpConnectionSummary> {
        self.visible().get(self.selected).copied()
    }
    pub(super) fn reselect(&mut self, id: Option<&str>) {
        let visible = self.visible();
        self.selected = id
            .and_then(|id| visible.iter().position(|v| v.id == id))
            .unwrap_or(self.selected.min(visible.len().saturating_sub(1)));
    }
    pub(super) fn begin_operation(&mut self) -> u64 {
        self.operation_request = self.operation_request.wrapping_add(1);
        self.catalog_request = self.catalog_request.wrapping_add(1);
        self.detail_request = self.detail_request.wrapping_add(1);
        self.capability_request = self.capability_request.wrapping_add(1);
        self.loading = false;
        self.detail_loading = false;
        self.capability_loading = false;
        self.busy = true;
        self.error = None;
        self.operation_request
    }
    pub(super) fn open_editor(
        &mut self,
        title: &str,
        prompt: &str,
        multiline: bool,
        value: String,
        action: EditAction,
    ) {
        self.editor_original = value.clone();
        self.error = None;
        self.editor = Some(EditorDialogState::new(
            title.to_owned(),
            prompt.to_owned(),
            if multiline {
                "Ctrl+S apply · Esc cancel"
            } else {
                "Enter apply · Esc cancel"
            }
            .to_owned(),
            multiline,
            Editor::from_text(value),
            action,
        ));
    }
    pub(crate) fn accepts_text_input(&self) -> bool {
        !self.busy && self.confirm.is_none() && self.editor.is_some()
    }
    pub(crate) fn paste(&mut self, text: &str) {
        if self.accepts_text_input()
            && let Some(editor) = &mut self.editor
        {
            editor.input.insert_str(text);
        }
    }
    pub(crate) fn focus_list(&mut self, panel: usize) {
        if self.editor.is_none()
            && self.confirm.is_none()
            && self.draft.is_none()
            && self.import.is_none()
            && !self.compact.get()
        {
            self.pane = if panel == 0 {
                McpPane::Connections
            } else {
                McpPane::Capabilities
            };
        }
    }
    pub(super) fn map_keys(&self) -> Vec<String> {
        self.draft
            .as_ref()
            .zip(self.map_editor.as_ref())
            .and_then(|(draft, map)| {
                draft
                    .config
                    .pointer(map.field.path())
                    .and_then(Value::as_object)
            })
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }
}

pub(super) fn scope_label(scope: &McpConnectionScope) -> &'static str {
    match scope {
        McpConnectionScope::Global => "Global",
        McpConnectionScope::Workspace => "Server workspace",
    }
}
pub(super) fn next_scope(scope: &McpConnectionScope) -> McpConnectionScope {
    match scope {
        McpConnectionScope::Global => McpConnectionScope::Workspace,
        McpConnectionScope::Workspace => McpConnectionScope::Global,
    }
}
pub(super) fn pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
pub(super) fn display_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
fn secret_url(value: &str) -> bool {
    value == REDACTED
        || url::Url::parse(value).is_ok_and(|url| {
            url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
        })
}
pub(super) fn default_config(transport: &str) -> Value {
    let mut config = if transport == "http" {
        json!({"transport":"http","endpoint":{"url":"","headers":{}},"auth":null})
    } else {
        json!({"transport":"stdio","process":{"command":"","args":[],"env":{},"cwd":null}})
    };
    config["enabled"] = json!(true);
    config["tools"] = json!({"include":[],"exclude":[]});
    config["timeouts"] = json!({"connect_ms":30000,"request_ms":60000});
    config
}
fn put_pointer(root: &mut Value, path: &str, value: Value) {
    if let Some(target) = root.pointer_mut(path) {
        *target = value;
        return;
    }
    let segments: Vec<_> = path
        .trim_start_matches('/')
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mut target = root;
    for (index, segment) in segments.iter().enumerate() {
        if !target.is_object() {
            *target = json!({});
        }
        if index + 1 == segments.len() {
            target[segment] = value;
            return;
        }
        target = target
            .as_object_mut()
            .expect("object initialized")
            .entry(segment.clone())
            .or_insert_with(|| json!({}));
    }
}
fn remove_pointer(root: &mut Value, path: &str) {
    if let Some((parent, key)) = path.rsplit_once('/') {
        let parent = if parent.is_empty() {
            Some(root)
        } else {
            root.pointer_mut(parent)
        };
        if let Some(map) = parent.and_then(Value::as_object_mut) {
            map.remove(&key.replace("~1", "/").replace("~0", "~"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_round_trip_preserves_kept_values_and_removes_only_explicit_edits() {
        let mut draft = McpDraft::new(McpConnectionScope::Global);
        draft.config = json!({"transport":"http","endpoint":{"url":"https://example.test/mcp","headers":{"X/Token":"[REDACTED]","X-New":"new-secret"}},"auth":{"kind":"bearer","token":"[REDACTED]"}});
        draft.preserve_secrets = BTreeSet::from([
            "/endpoint/headers/X~1Token".to_owned(),
            "/auth/token".to_owned(),
        ]);
        let redacted = draft.redacted_config();
        assert!(!redacted.to_string().contains("new-secret"));
        draft.replace_redacted_config(redacted);
        assert_eq!(
            draft.config.pointer("/endpoint/headers/X-New"),
            Some(&json!("new-secret"))
        );
        assert_eq!(draft.preserve_secrets.len(), 2);
        draft.set("/auth/token", json!("replacement"));
        assert!(!draft.preserve_secrets.contains("/auth/token"));
        assert!(
            draft
                .preserve_secrets
                .contains("/endpoint/headers/X~1Token")
        );
        draft.remove("/endpoint/headers/X~1Token");
        assert!(draft.preserve_secrets.is_empty());
        assert!(draft.config.pointer("/endpoint/headers/X~1Token").is_none());
    }
    #[test]
    fn transport_switch_drops_credentials_but_keeps_policies_and_timeouts() {
        let mut draft = McpDraft::new(McpConnectionScope::Global);
        draft.set("/process/env/TOKEN", json!("hidden"));
        draft.set("/tools/exclude", json!(["write_*"]));
        draft.change_transport();
        assert_eq!(draft.transport(), "http");
        assert!(draft.config.get("process").is_none());
        assert_eq!(
            draft.config.pointer("/tools/exclude"),
            Some(&json!(["write_*"]))
        );
    }
    #[test]
    fn route_debug_does_not_expose_drafts_or_imported_credentials() {
        let mut state = McpWorkbench::new();
        let mut draft = McpDraft::new(McpConnectionScope::Global);
        draft.set("/process/env/TOKEN", json!("do-not-print"));
        state.draft = Some(draft);
        state.open_editor(
            "Token",
            "",
            false,
            "another-secret".to_owned(),
            EditAction::StoredToken { id: "x".to_owned() },
        );
        let output = format!("{state:?}");
        assert!(!output.contains("do-not-print"));
        assert!(!output.contains("another-secret"));
    }
    #[test]
    fn oauth_registration_fields_mask_and_round_trip_client_secrets() {
        let mut draft = McpDraft::new(McpConnectionScope::Global);
        draft.change_transport();
        draft.set("/auth",json!({"kind":"o_auth","scopes":["read"],"client_id":"registered-client","client_secret":"new-client-secret","client_metadata_url":"https://example.test/client.json"}));
        for field in [
            FormField::Scopes,
            FormField::ClientId,
            FormField::ClientSecret,
            FormField::ClientMetadataUrl,
        ] {
            assert!(draft.fields().contains(&field));
        }
        assert_eq!(draft.field_value(FormField::ClientId), "registered-client");
        assert!(
            draft
                .field_value(FormField::ClientSecret)
                .contains("hidden")
        );
        let mut value = draft.redacted_config();
        assert_eq!(value["auth"]["client_secret"], REDACTED);
        value["auth"]["scopes"] = json!(["read", "write"]);
        draft.replace_redacted_config(value);
        assert_eq!(draft.config["auth"]["client_secret"], "new-client-secret");
        draft.set_field(FormField::ClientSecret, Value::Null);
        assert!(draft.config["auth"].get("client_secret").is_none());
        assert_eq!(draft.config["auth"]["client_id"], "registered-client");
    }
    #[test]
    fn hidden_endpoint_and_oauth_secret_keep_preservation_until_explicitly_replaced() {
        let mut draft = McpDraft::new(McpConnectionScope::Workspace);
        draft.config = json!({"transport":"http","endpoint":{"url":REDACTED},"auth":{"kind":"o_auth","client_secret":REDACTED}});
        draft.preserve_secrets =
            BTreeSet::from(["/endpoint/url".to_owned(), "/auth/client_secret".to_owned()]);
        draft.set_field(FormField::Url, json!(REDACTED));
        draft.replace_redacted_config(draft.redacted_config());
        assert_eq!(draft.preserve_secrets.len(), 2);
        draft.set_field(
            FormField::Url,
            json!("https://example.test/mcp?key=new-endpoint-secret"),
        );
        assert!(!draft.preserve_secrets.contains("/endpoint/url"));
        assert!(draft.preserve_secrets.contains("/auth/client_secret"));
        assert!(
            !draft
                .field_value(FormField::Url)
                .contains("new-endpoint-secret")
        );
        assert!(
            !draft
                .redacted_config()
                .to_string()
                .contains("new-endpoint-secret")
        );
        assert!(
            EditAction::Field(FormField::Url)
                .secret("https://example.test/mcp?key=new-endpoint-secret")
        );
        assert!(!EditAction::Field(FormField::Url).secret("https://example.test/mcp"));
        draft.set_field(FormField::ClientSecret, json!("replacement-client-secret"));
        assert!(draft.preserve_secrets.is_empty());
        assert_eq!(draft.redacted_config()["auth"]["client_secret"], REDACTED);
    }
}
