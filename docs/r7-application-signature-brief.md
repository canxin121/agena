# R7 P0 — Application 方法签名冻结 brief(agena-application ↔ agena-tui-app 共同契约)

> 状态:**冻结**。本文是 R7 重构 P0 的唯一权威接口契约,供两个并行 worktree
> (一个改 `agena-application`、一个改 `agena-tui-app`)机械执行。
> 两个 worktree 都以本文为准,不得擅自改签名;发现分歧先回 master 更新本文再实施。
> 本文件只冻结**签名**;每个方法的具体实现按原 Backend 方法体下移(薄委托,
> 内部经 accessor 访问 runtime),不改变行为。

---

## 0. 目标与总规则

`agena-tui-backend` 将被删除。以下 Backend 方法携带共享逻辑,下移为
`agena-application` 的 `impl Application` 方法,经以下已有 accessor 访问 runtime:
`service()`、`provider_catalog()`、`runtime_config_settings()`、`runtime_control()`、
`session_execution_services()`、`session_store_facade()`、`runtime_activities()`、
`runtime_draft_authentication()`、`plugin_runtime()`、`session_query_service()`、
`workspace_root()`。

`agena-tui-app` 机械替换调用点:`backend.<method>()` → `application.<method>()`,
**只改接收者,不改参数**。

返回类型总规则:

- 原方法返回 `anyhow::Result<T>`(或已直接返回 `T`)→ 新返回
  `Result<T, agena_application::ApplicationError>`。
- 原方法已返回结构化错误 `std::result::Result<T, ProviderStudioSaveError>` →
  **保持结构化错误不变**(Provider Studio 保存/删除方法,见 §4.4/§4.5),
  但 `ProviderStudioSaveError`/`ProviderStudioSaveResult` 等类型路径改为
  `crate::provider_studio::…`(见 §2)。
- 原方法无 `Result`(直接返回 `T`)→ 保持不变(见 `activity_kind_catalog`、
  `resolved_model_default_modes`)。

错误上下文文本:原方法 `.context("…")` / `.with_context(|| format!("…"))` /
`anyhow!("…")` 的字符串**原样保留**,下移时包成
`ApplicationError::internal(format!("<上下文>: {e}"))`(或对直接 `anyhow!`
构造的消息,`ApplicationError::internal("<消息>")`)。每方法的上下文清单见 §4。

---

## 1. 类型路径速查(下移后统一使用)

| 原 tui-backend 简名 | 冻结类型路径(写进签名) |
|---|---|
| `SessionResource` | `agena_api::resource::SessionResource` |
| `SessionExecutionResource` | `agena_api::resource::SessionExecutionResource` |
| `RunOptions` | `agena_api::resource::RunOptions` |
| `ProviderAdapterModelsResource` | `agena_api::resource::ProviderAdapterModelsResource` |
| `ProviderAdapterModelsResponse` | `agena_api::resource::ProviderAdapterModelsResponse` |
| `ProviderModelResource` | `agena_api::resource::ProviderModelResource` |
| `ModelRef` | `agena_domain::ModelRef` |
| `JsonValue` / `JsonMap` | `serde_json::Value` / `serde_json::Map<String, Value>` |
| `ConfigSettingsEditResponse` | `agena_runtime::ConfigSettingsEditResponse` |
| `SessionPartView`(**新类型**) | `agena_storage::store::SessionPartView`(见 §3) |
| `ProviderConfigDraft` 等(§2) | `crate::provider_studio::…`(即 `agena_application::provider_studio::…`) |

> 说明:`agena-application` 的 `dto` 已 re-export `agena_api::resource::*`,但冻结签名
> 一律写**规范全路径** `agena_api::resource::…`,避免二义。

---

## 2. `agena_application::provider_studio` 模块 pub re-export 清单(从 backend_drafts/ 迁移)

新增模块 `crates/agena-application/src/provider_studio/mod.rs`,把
`crates/agena-tui-backend/src/backend_drafts/`(mod.rs + provider_draft_auth.rs +
provider_draft_config.rs + provider_draft_validation.rs)整体迁移进来,并 `pub use`
以下全部 pub 类型(供签名与 tui-app 引用):

来自 `provider_draft_config.rs`:
- `ProviderConfigDraft`(含 `new_empty`、`normalize_shape`、`from_configured_editor`
  及全部 `pub fn` 方法)

来自 `provider_draft_auth.rs`:
- `ProviderDraftAuthKind`、`ProviderDraftAdapterRule`、`ProviderDraftSecretSourceKind`
- `ProviderOAuthTokensDraft`、`ProviderBrowserAuthSessionDraft`、`ProviderDeviceAuthSessionDraft`
- `ProviderDraftInteractiveLoginKind`、`OpenAiChatgptCredentialDraft`、
  `GithubCopilotCredentialDraft`、`GitlabCredentialDraft`、`ProviderCredentialDraftBundle`
- `ProviderDraftAuthMessage`、`ProviderDraftAuthField`、`ProviderDraftAuthError`、
  `ProviderDraftAuthActionResult`
- `ProviderStudioSaveResult`、`ProviderStudioSaveField`、
  `ProviderStudioSaveValidationError`、`ProviderStudioSaveError`
- `ProviderDraftAuthDetails`

来自 `provider_draft_validation.rs`(impl 块,迁移为模块内方法):
- `ProviderConfigDraft::validate_for_adapters_for_save`(pub(crate)→ 模块内可见)
- `ProviderConfigDraft::build_listing_request`(同上)

> 迁移后 `agena-tui-app` 从 `agena_application::provider_studio::…` 导入上述类型,
> 不再从 `agena_tui_backend::…` 导入。
> 注:`start_provider_draft_auth` / `continue_provider_draft_auth` 两个 Backend 方法
> **不在本批冻结范围**,本轮留在 tui-backend(其依赖的类型随模块迁移,类型导入切换即可)。

---

## 3. 新类型契约:`agena_storage::store::SessionPartView`

当前代码库中**不存在**该类型(已全仓 grep 确认),需由 agena-application worktree
在 `agena-storage` 新增(agena-storage 已是 agena-application 依赖)。

它是 `agena_storage::store::Part` 的投影视图,字段取原
`list_session_timeline` 中 `Part → SessionTimelineEntry` 映射所需的最小集
(参照 `crates/agena-tui-backend/src/backend_session.rs:93-106` 与
`crates/agena-tui-backend/src/lib.rs:94-107`):

```rust
#[derive(Debug, Clone)]
pub struct SessionPartView {
    pub part_id: i64,
    pub kind: String,
    pub role: PartRole,          // 保留 typed 枚举;tui-app 内 as_str().to_owned()
    pub state: PartState,        // 同上
    pub summary: Option<String>,
    pub content: serde_json::Value,
    pub rendered_markdown: Option<String>,
    pub parent_part_id: Option<i64>,
    pub run_id: Option<i64>,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
```

`list_session_timeline_parts` 只做:store load → `visibility.visible_to_user()` 过滤 →
`limit` tail 截断(原 `let skip = visible.len().saturating_sub(limit); …skip(skip)`)。
**`SessionTimelineEntry` 的字段映射逻辑留在 tui-app**(tui-app 拿到
`Vec<SessionPartView>` 后自行映射)。

---

## 4. 方法冻结对照表

> 每条格式:原位置(文件:行,当前精确签名)→ 冻结新 `Application` 签名 →
> 需保留的错误上下文文本清单。

### 4.1 `crates/agena-tui-backend/src/backend_session.rs`

**list_child_sessions**
- 原(backend_session.rs:17):
  `pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>>`
- 新:
  ```rust
  pub async fn list_child_sessions(
      &self,
      parent_id: i64,
  ) -> Result<Vec<agena_api::resource::SessionResource>, ApplicationError>
  ```
- 上下文:`"failed to list child sessions"`

**list_session_subtree**
- 原(backend_session.rs:40):
  `pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>>`
- 新:
  ```rust
  pub async fn list_session_subtree(
      &self,
      session_id: i64,
  ) -> Result<Vec<agena_api::resource::SessionResource>, ApplicationError>
  ```
- 上下文:循环内 per-child 动态消息(保留原格式):
  `format!("failed to list subtree children for session {parent_id}")`

**list_session_timeline → list_session_timeline_parts**(改名,见 §3)
- 原(backend_session.rs:71):
  `pub async fn list_session_timeline(&self, session_id: i64, limit: u64) -> Result<Vec<SessionTimelineEntry>>`
- 新(下移形态;`SessionTimelineEntry` 映射留在 tui-app):
  ```rust
  pub async fn list_session_timeline_parts(
      &self,
      session_id: i64,
      limit: u64,
  ) -> Result<Vec<agena_storage::store::SessionPartView>, ApplicationError>
  ```
- 上下文:原方法无 `.context()`;store load 错误经
  `ApplicationError::internal(<error>)` 上抛即可(实现 agent 可加
  `"failed to load session timeline parts"` 上下文,非强制)。

**set_session_permission**
- 原(backend_session.rs:488):
  `pub async fn set_session_permission(&self, session_id: i64, permission: agena_domain::PermissionConfig) -> Result<SessionExecutionResource>`
- 新:
  ```rust
  pub async fn set_session_permission(
      &self,
      session_id: i64,
      permission: agena_domain::PermissionConfig,
  ) -> Result<agena_api::resource::SessionExecutionResource, ApplicationError>
  ```
- 上下文:`format!("failed to set permission for session {session_id}")`;
  内部 `get_session_state` 的 `"failed to load session state"` 一并保留。

**rewind_session_to_turn**
- 原(backend_session.rs:644):
  `pub async fn rewind_session_to_turn(&self, session_id: i64, turn_id: agena_domain::TurnId) -> Result<SessionExecutionResource>`
- 新:
  ```rust
  pub async fn rewind_session_to_turn(
      &self,
      session_id: i64,
      turn_id: agena_domain::TurnId,
  ) -> Result<agena_api::resource::SessionExecutionResource, ApplicationError>
  ```
- 上下文:`"failed to rewind session to turn"`;内部 `get_session` 的
  `"failed to fetch session"` 与 `format!("session not found: {session_id}")` 一并保留。

### 4.2 `crates/agena-tui-backend/src/backend_workspace.rs`

**list_workspace_sessions**
- 原(backend_workspace.rs:29):
  `pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>>`
- 新:
  ```rust
  pub async fn list_workspace_sessions(
      &self,
      roots_only: bool,
  ) -> Result<Vec<agena_api::resource::SessionResource>, ApplicationError>
  ```
- 上下文:`"failed to list workspace sessions"`

**create_session**
- 原(backend_workspace.rs:75):
  `pub async fn create_session(&self, title: String, parent_id: Option<i64>) -> Result<SessionResource>`
- 新:
  ```rust
  pub async fn create_session(
      &self,
      title: String,
      parent_id: Option<i64>,
  ) -> Result<agena_api::resource::SessionResource, ApplicationError>
  ```
- 上下文:`"failed to resolve workspace for terminal UI"`、`"failed to create session"`

**rename_session**
- 原(backend_workspace.rs:96):
  `pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource>`
- 新:
  ```rust
  pub async fn rename_session(
      &self,
      session_id: i64,
      title: String,
  ) -> Result<agena_api::resource::SessionResource, ApplicationError>
  ```
- 上下文:`"failed to load session before rename"`、`format!("session not found: {session_id}")`、
  `"failed to assert session version before rename"`、`"failed to rename session"`

**set_config_setting**
- 原(backend_workspace.rs:184):
  `pub async fn set_config_setting(&self, path: &str, value: JsonValue) -> Result<agena_runtime::ConfigSettingsEditResponse>`
- 新:
  ```rust
  pub async fn set_config_setting(
      &self,
      path: &str,
      value: serde_json::Value,
  ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError>
  ```
- 逻辑不变:先 `plugin_config_setting_target(path)?` 判断是否 plugin 目标,是则委托
  `set_plugin_config_setting`,否则走私有助手 `set_config_setting_direct`(随迁,见 §5)。
- 上下文:`"failed to set config setting"`、`"failed to reload runtime after config change"`
  (二者来自 `set_config_setting_direct`)。

**delete_config_setting**
- 原(backend_workspace.rs:235):
  `pub async fn delete_config_setting(&self, path: &str) -> Result<agena_runtime::ConfigSettingsEditResponse>`
- 新:
  ```rust
  pub async fn delete_config_setting(
      &self,
      path: &str,
  ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError>
  ```
- 上下文:`"failed to delete config setting"`、`"failed to reload runtime after config change"`

**set_plugin_config_setting**(原 `pub(super)`,下移后 `pub`)
- 原(backend_workspace.rs:323):
  `pub(super) async fn set_plugin_config_setting(&self, plugin_id: &str, config_segments: &[String], value: JsonValue) -> Result<ConfigSettingsEditResponse>`
- 新:
  ```rust
  pub async fn set_plugin_config_setting(
      &self,
      plugin_id: &str,
      config_segments: &[String],
      value: serde_json::Value,
  ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError>
  ```
- 上下文:`"plugin config record must be an object"`(normalize 助手)、
  `"failed to set config setting"`、`"failed to reload runtime after config change"`

**delete_plugin_config_setting**(原 `pub(super)`,下移后 `pub`)
- 原(backend_workspace.rs:337):
  `pub(super) async fn delete_plugin_config_setting(&self, plugin_id: &str, config_segments: &[String]) -> Result<ConfigSettingsEditResponse>`
- 新:
  ```rust
  pub async fn delete_plugin_config_setting(
      &self,
      plugin_id: &str,
      config_segments: &[String],
  ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError>
  ```
- 上下文:同上(`"plugin config record must be an object"`、
  `"failed to set config setting"`、`"failed to reload runtime after config change"`)

### 4.3 `crates/agena-tui-backend/src/backend_plugins.rs`

**activity_kind_catalog**(无 Result,保持不变)
- 原(backend_plugins.rs:215):
  `pub fn activity_kind_catalog(&self) -> Vec<agena_domain::ActivityKind>`
- 新(经 `self.plugin_runtime()` 内联 `plugin_statuses()`/`plugin_inspect()`,不再经 Backend):
  ```rust
  pub fn activity_kind_catalog(&self) -> Vec<agena_domain::ActivityKind>
  ```
- 上下文:无(逻辑:builtin_activity_kinds() + 各已加载 plugin manifest 的
  activity_kinds 按 id 去重)。

**invoke_plugin_ui_tool**
- 原(backend_plugins.rs:500):
  `pub async fn invoke_plugin_ui_tool(&self, plugin_id: &str, tool_name: &str, input: serde_json::Value, session_id: Option<i64>) -> Result<agena_plugin_host::PluginUiToolInvokeResponse>`
- 新:
  ```rust
  pub async fn invoke_plugin_ui_tool(
      &self,
      plugin_id: &str,
      tool_name: &str,
      input: serde_json::Value,
      session_id: Option<i64>,
  ) -> Result<agena_plugin_host::PluginUiToolInvokeResponse, ApplicationError>
  ```
- 私有助手 `invoke_plugin_ui_tool_checked` 随迁(§5)。
- 上下文(原 `anyhow!` 消息 → `ApplicationError::internal(...)`):
  `"plugin tool invocation requires an active session"`、
  `format!("plugin tool not found: {plugin_id}/{tool_name}")`、
  `format!("plugin tool input must be an object, got {other}")`、
  `format!("invalid plugin tool input for {plugin_id}/{tool_name}: {error}")`;
  以及 `SessionToolExecutionError::Execution(error)` 的 `error` 文本。

### 4.4 `crates/agena-tui-backend/src/backend_provider/selection.rs`

**provider_config_draft**
- 原(selection.rs:21):
  `pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft>`
- 新:
  ```rust
  pub fn provider_config_draft(
      &self,
      provider_id: Option<&str>,
  ) -> Result<crate::provider_studio::ProviderConfigDraft, ApplicationError>
  ```
- 上下文:`format!("provider not found: {provider_id}")`

**save_provider_draft**(结构化错误,不转 ApplicationError)
- 原(selection.rs:353):
  ```rust
  pub async fn save_provider_draft(
      &self,
      draft: ProviderConfigDraft,
      adapter_model_lists: &[ProviderAdapterModelsResource],
      selected_adapter_ids: &[String],
      selected_model_keys: &std::collections::BTreeSet<String>,
  ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>
  ```
- 新:
  ```rust
  pub async fn save_provider_draft(
      &self,
      draft: crate::provider_studio::ProviderConfigDraft,
      adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
      selected_adapter_ids: &[String],
      selected_model_keys: &std::collections::BTreeSet<String>,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

**save_provider_adapter_matches**(结构化错误)
- 原(selection.rs:563):
  `pub async fn save_provider_adapter_matches(&self, draft: ProviderConfigDraft, adapter_models: ProviderAdapterModelsResource) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>`
- 新:
  ```rust
  pub async fn save_provider_adapter_matches(
      &self,
      draft: crate::provider_studio::ProviderConfigDraft,
      adapter_models: agena_api::resource::ProviderAdapterModelsResource,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

**list_draft_provider_adapter_models**
- 原(selection.rs:321):
  `pub async fn list_draft_provider_adapter_models(&self, draft: &ProviderConfigDraft, adapter_ids: &[String]) -> Result<ProviderAdapterModelsResponse>`
- 新:
  ```rust
  pub async fn list_draft_provider_adapter_models(
      &self,
      draft: &crate::provider_studio::ProviderConfigDraft,
      adapter_ids: &[String],
  ) -> Result<agena_api::resource::ProviderAdapterModelsResponse, ApplicationError>
  ```

**list_saved_provider_adapter_models**
- 原(selection.rs:338):
  `pub async fn list_saved_provider_adapter_models(&self, provider_id: &str, adapter_ids: &[String]) -> Result<ProviderAdapterModelsResponse>`
- 新:
  ```rust
  pub async fn list_saved_provider_adapter_models(
      &self,
      provider_id: &str,
      adapter_ids: &[String],
  ) -> Result<agena_api::resource::ProviderAdapterModelsResponse, ApplicationError>
  ```

**provider_model_draft_value**
- 原(selection.rs:642):
  `pub fn provider_model_draft_value(&self, draft: &ProviderConfigDraft, adapter_id: &str, model_id: &str, provider_model: Option<&ProviderModelResource>) -> Result<JsonValue>`
- 新:
  ```rust
  pub fn provider_model_draft_value(
      &self,
      draft: &crate::provider_studio::ProviderConfigDraft,
      adapter_id: &str,
      model_id: &str,
      provider_model: Option<&agena_api::resource::ProviderModelResource>,
  ) -> Result<serde_json::Value, ApplicationError>
  ```
- 上下文:`"failed to read configured provider model"`

**set_provider_default_selection**
- 原(selection.rs:735):
  `pub async fn set_provider_default_selection(&self, provider_id: &str, selection: JsonValue) -> Result<agena_runtime::ConfigSettingsEditResponse>`
- 新:
  ```rust
  pub async fn set_provider_default_selection(
      &self,
      provider_id: &str,
      selection: serde_json::Value,
  ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError>
  ```
- 上下文:`"provider id is required"`、`"failed to set provider default selection"`、
  `"failed to reload runtime after provider default selection change"`

**resolved_model_for_run_options**
- 原(selection.rs:181):
  `pub fn resolved_model_for_run_options(&self, request: &RunOptions) -> Result<ModelRef>`
- 新:
  ```rust
  pub fn resolved_model_for_run_options(
      &self,
      request: &agena_api::resource::RunOptions,
  ) -> Result<agena_domain::ModelRef, ApplicationError>
  ```
- 上下文:`"run option contains an invalid model reference"`、`"no providers configured"`

**resolved_model_default_modes**(无 Result,保持不变)
- 原(selection.rs:206):
  `pub fn resolved_model_default_modes(&self, request: &RunOptions) -> (Option<String>, Option<String>)`
- 新:
  ```rust
  pub fn resolved_model_default_modes(
      &self,
      request: &agena_api::resource::RunOptions,
  ) -> (Option<String>, Option<String>)
  ```
- 上下文:无(内部吞错返回 `(None, None)`)。

### 4.5 `crates/agena-tui-backend/src/backend_provider/settings.rs`

**save_provider_model_value**(结构化错误)
- 原(settings.rs:42):
  `pub async fn save_provider_model_value(&self, draft: ProviderConfigDraft, adapter_id: &str, model_id: &str, model_value: JsonValue) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>`
- 新:
  ```rust
  pub async fn save_provider_model_value(
      &self,
      draft: crate::provider_studio::ProviderConfigDraft,
      adapter_id: &str,
      model_id: &str,
      model_value: serde_json::Value,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

**delete_provider_model**(结构化错误)
- 原(settings.rs:156):
  `pub async fn delete_provider_model(&self, draft: ProviderConfigDraft, adapter_id: &str, model_id: &str) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>`
- 新:
  ```rust
  pub async fn delete_provider_model(
      &self,
      draft: crate::provider_studio::ProviderConfigDraft,
      adapter_id: &str,
      model_id: &str,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

**delete_provider**(结构化错误)
- 原(settings.rs:241):
  `pub async fn delete_provider(&self, provider_id: &str) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>`
- 新:
  ```rust
  pub async fn delete_provider(
      &self,
      provider_id: &str,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

**delete_provider_adapter**(结构化错误)
- 原(settings.rs:312):
  `pub async fn delete_provider_adapter(&self, draft: ProviderConfigDraft, adapter_id: &str) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError>`
- 新:
  ```rust
  pub async fn delete_provider_adapter(
      &self,
      draft: crate::provider_studio::ProviderConfigDraft,
      adapter_id: &str,
  ) -> std::result::Result<
      crate::provider_studio::ProviderStudioSaveResult,
      crate::provider_studio::ProviderStudioSaveError,
  >
  ```

---

## 5. 随迁私有助手(agena-application 内实现,不改变调用面)

下列原 Backend 私有/`pub(super)` 助手随对应方法一并下移到 agena-application(模块内私有,
非冻结签名,列出以便 worktree 实现 agent 知晓完整范围):

会话/工作区(来自 backend_session.rs / backend_workspace.rs / backend_plugins.rs):
- `get_session`、`list_sessions_query`、`current_workspace_id`、
  `resolve_workspace_resource`、`resolve_session_root`

配置(来自 backend_config.rs,随 §4.2 四个 config 方法):
- `set_config_setting_direct`(Backend pub(super) 方法)
- `plugin_config_setting_target(path: &str) -> Result<Option<(String, Vec<String>)>>`
- `plugin_record_for_config_edit(sources: &ConfigJsonSources, plugin_id: &str) -> JsonValue`
- `normalize_plugin_record_for_config_edit(record: &mut JsonValue) -> Result<&mut JsonValue>`
- `set_nested_json_value(root: &mut JsonValue, segments: &[String], value: JsonValue)`
- `remove_nested_json_value(root: &mut JsonValue, segments: &[String]) -> bool`
- `default_static_plugin_record() -> JsonValue`

插件(来自 backend_plugins.rs):
- `invoke_plugin_ui_tool_checked`(私有 async 助手)
- `activity_kind_catalog` 需要的 `plugin_statuses` / `plugin_inspect` 内联为直接经
  `self.plugin_runtime()`

Provider Studio(来自 backend_catalog.rs / backend_auth.rs / backend_events.rs /
selection.rs / settings.rs,随 §4.4/§4.5):
- 路径助手:`quoted_settings_segment`、`provider_settings_path`、
  `provider_adapter_settings_path`、`provider_model_settings_path`
- 补丁/校验助手:`build_provider_patch_value_for_save`、
  `build_provider_auth_patch_value_for_save`、
  `apply_provider_auth_required_adapter_defaults_to_json_adapters`、
  `apply_provider_auth_required_adapter_defaults_to_json_value`、
  `merge_provider_model_adapter_patch_for_save`、
  `resolve_provider_defaults_from_value_for_save`、`required_provider_save_field`、
  `provider_value_contains_model`、`provider_defaults_point_to`、
  `provider_defaults_adapter`、`provider_model_selection_contains`
- 目录/模型助手:`catalog_lookup_id_for_provider_model`、
  `preferred_catalog_model_for_provider_model`、`provider_model_json_for_model_id`、
  `provider_model_overlay_for_model_id`、`provider_model_overlay_to_json`、
  `ensure_provider_model_entry`
- 泛用:`optional_non_empty`(原 backend_events.rs)、`required_trimmed`(原 backend_auth.rs)
- Backend pub(super) 方法:`read_file_provider_settings`、`patch_provider_settings`、
  `patch_provider_settings_root`、`set_provider_settings`、`provider_adapter_models_response`、
  `configured_provider_adapter_ids`、`effective_provider_draft_adapter_ids`
- 私有函数:`build_provider_adapter_matches_patch`、`preserve_existing_model_execution_policy`、
  `apply_provider_adapter_selection`(selection.rs 内)、`rename_provider_references`、
  `clear_default_selection_for_removed_adapter`(settings.rs 内)

---

## 6. tui-app 调用点改动要点(第二个 worktree)

1. `backend.<method>()` → `application.<method>()`;参数一律不变。
2. 类型导入切换:`ProviderConfigDraft`、`ProviderStudioSaveResult`、
   `ProviderStudioSaveError`、`ProviderStudioSaveField`、
   `ProviderStudioSaveValidationError` 等改从 `agena_application::provider_studio::` 导入。
3. `list_session_timeline` → `application.list_session_timeline_parts(session_id, limit)`,
   返回 `Vec<agena_storage::store::SessionPartView>`;tui-app 把原
   `backend_session.rs:93-106` 的 `SessionTimelineEntry` 字段映射逻辑搬进 tui-app
   (`role.as_str().to_owned()` / `state.as_str().to_owned()`,字段一一对应 §3)。
4. 错误类型由 `anyhow::Result` 变 `Result<…, ApplicationError>`;调用点 `?` 可直接解
   (`ApplicationError: std::error::Error`)。若调用点原本链式 `.context(...)`
   或在 anyhow 上下文里传播,需相应调整(ApplicationError 无 `.context`);
   若只需展示,`ApplicationError` 的 `Display`/`failure.user` 可用。
5. `set_plugin_config_setting` / `delete_plugin_config_setting` 由 tui-backend 的
   `pub(super)` 变 Application 的 `pub`,tui-app 可直接调用。
6. Provider Studio 保存/删除方法的错误类型不变(`ProviderStudioSaveError`),
   仅导入路径切换。

---

## 7. Cargo 依赖调整(agena-application worktree)

`crates/agena-application/Cargo.toml` 增加(唯一新增依赖):

```toml
anyhow = { workspace = true }
```

其余已具备:`agena-api`(resource 类型)、`agena-plugin-host`
(`PluginUiToolInvokeResponse`)、`agena-storage`(`SessionPartView`)、`agena-domain`
(`ModelRef`/`PermissionConfig`/`TurnId`/`get_json_path`)、`agena-runtime`
(`ConfigSettingsEditResponse` 等)、`agena-provider`、`serde_json`。
`agena-tui-backend` 删除前,其 `Cargo.toml` 依赖(`anyhow`、`ignore`、`imagesize`、
`mime_guess` 等)中仅真正随迁的部分在后续步骤处理,本轮不动。
