//! # HTTP 路由目录
//!
//! 本页列出当前挂载的每一条路由，按领域分组并标注 handler。运行
//! `cargo doc -p agena-api-server --no-deps --open` 浏览，或搜索 `route`。
//!
//! **生成文件，请勿手工编辑。** 修改路由后重新运行：
//!
//! ```sh
//! python3 scripts/agena/generate-route-docs.py
//! ```
//!
//! 路由的**目标形态**（含尚未落地的重命名）见 `docs/design/api-v1-routes.md`；
//! 本页描述的是**当前实际挂载**的路由，迁移完成前两者会存在差异。
//!
//! ## 约定
//!
//! - REST handler 位于 [`crate::rest`]：解析请求、调用 application 服务、
//!   返回 JSON 资源。
//! - `/api/v1/ws`、`/api/v1/changes/stream`、`/api/v1/notifications/stream`
//!   由 [`crate::ws`] 与 [`crate::sse`] 提供，出现在下表末尾。
//! - `.layer(DefaultBodyLimit::max(...))` 逐路由挂载：只有明确需要更大请求体
//!   的路由才放宽 axum 的 2 MiB 默认值。
//!
//! ## 当前路由
//!
//! ### `activities`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/activities` | `list_activities()` |
//! | POST | `/api/v1/activities/clear-finished` | `clear_finished_activities()` |
//! | GET | `/api/v1/activities/{activity_id}` | `get_activity()` |
//! | GET | `/api/v1/activities/{activity_id}/logs` | `get_activity_logs()` |
//! | POST | `/api/v1/activities/{activity_id}/stop` | `stop_activity()` |
//! | POST | `/api/v1/activities/{activity_id}/pause` | `pause_activity()` |
//! | POST | `/api/v1/activities/{activity_id}/resume` | `resume_activity()` |
//! | POST | `/api/v1/activities/{activity_id}/delete` | `delete_activity()` |
//! | POST | `/api/v1/activities/{activity_id}/dismiss` | `dismiss_activity()` |
//!
//! ### `auth`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/auth/providers` | `list_auth_providers()` |
//! | POST | `/api/v1/auth/providers/openai/browser/start` | `start_openai_browser_auth()` |
//! | POST | `/api/v1/auth/providers/openai/browser/finish` | `finish_openai_browser_auth()` |
//! | POST | `/api/v1/auth/providers/openai/device/start` | `start_openai_device_auth()` |
//! | POST | `/api/v1/auth/providers/openai/device/poll` | `poll_openai_device_auth()` |
//! | POST | `/api/v1/auth/providers/gitlab/browser/start` | `start_gitlab_browser_auth()` |
//! | POST | `/api/v1/auth/providers/gitlab/browser/finish` | `finish_gitlab_browser_auth()` |
//! | POST | `/api/v1/auth/providers/github-copilot/device/start` | `start_copilot_device_auth()` |
//! | POST | `/api/v1/auth/providers/github-copilot/device/poll` | `poll_copilot_device_auth()` |
//! | GET,DELETE | `/api/v1/auth/providers/{provider_id}` | `get_auth_provider()`, `delete_auth_provider()` |
//! | PUT | `/api/v1/auth/providers/{provider_id}/api-key` | `set_auth_provider_api_key()` |
//! | POST | `/api/v1/auth/providers/{provider_id}/refresh` | `refresh_auth_provider()` |
//!
//! ### `bot`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | POST | `/api/v1/bot/session` | `ensure_bot_session()` |
//!
//! ### `bots`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,POST | `/api/v1/bots` | `list_bots()`, `create_bot()` |
//! | GET,PUT | `/api/v1/bots/{id}` | `get_bot()`, `update_bot()` |
//! | POST | `/api/v1/bots/{id}/archive` | `archive_bot()` |
//! | POST | `/api/v1/bots/{id}/session` | `ensure_named_bot_session()` |
//! | POST | `/api/v1/bots/{id}/sessions` | `create_bot_conversation()` |
//!
//! ### `changes`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/changes/revisions` | `resource_revisions()` |
//! | GET | `/api/v1/changes/stream` | `sse()` |
//! | GET | `/api/v1/changes/stream` | `sse()` |
//!
//! ### `commands`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | POST | `/api/v1/commands` | `command()` |
//!
//! ### `config`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/config/resolved` | `get_resolved_config()` |
//!
//! ### `git`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/git/status` | `get_git_status()` |
//! | POST | `/api/v1/git/stage` | `stage_git_changes()` |
//! | POST | `/api/v1/git/commits` | `create_git_commit()` |
//! | POST | `/api/v1/git/pull-requests` | `create_git_pull_request()` |
//!
//! ### `health`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/health` | `health()` |
//!
//! ### `mcp-connections`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,POST | `/api/v1/mcp-connections` | `list_mcp_connections()`, `create_mcp_connection()` |
//! | POST | `/api/v1/mcp-connections/import/preview` | `preview_mcp_import()` |
//! | POST | `/api/v1/mcp-connections/import` | `import_mcp_connections()` |
//! | PUT | `/api/v1/mcp-connections/bridge/enabled` | `set_mcp_bridge_enabled()` |
//! | GET,PUT,DELETE | `/api/v1/mcp-connections/{id}` | `get_mcp_connection()`, `update_mcp_connection()`, `delete_mcp_connection()` |
//! | PUT | `/api/v1/mcp-connections/{id}/enabled` | `set_mcp_connection_enabled()` |
//! | POST | `/api/v1/mcp-connections/{id}/reconnect` | `reconnect_mcp_connection()` |
//! | POST | `/api/v1/mcp-connections/{id}/test` | `test_mcp_connection()` |
//! | GET | `/api/v1/mcp-connections/{id}/capabilities` | `mcp_connection_capabilities()` |
//! | POST | `/api/v1/mcp-connections/{id}/resource` | `read_mcp_connection_resource()` |
//! | POST | `/api/v1/mcp-connections/{id}/prompt` | `get_mcp_connection_prompt()` |
//! | PUT,DELETE | `/api/v1/mcp-connections/{id}/credential` | `set_mcp_connection_credential()`, `clear_mcp_connection_credential()` |
//! | POST | `/api/v1/mcp-connections/{id}/auth/start` | `start_mcp_connection_auth()` |
//! | POST | `/api/v1/mcp-connections/{id}/auth/complete` | `complete_mcp_connection_auth()` |
//! | DELETE | `/api/v1/mcp-connections/{id}/auth` | `clear_mcp_connection_auth()` |
//!
//! ### `memories`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/memories` | `list_memories()` |
//! | POST | `/api/v1/memories/batch-delete` | `delete_memories()` |
//! | GET | `/api/v1/memories/overview` | `get_memory_overview()` |
//! | POST | `/api/v1/memories/index` | `ensure_memory_index()` |
//! | GET,PUT,DELETE | `/api/v1/memories/{name}` | `get_memory()`, `save_memory()`, `delete_memory()` |
//!
//! ### `model-catalog`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/model-catalog` | `get_model_catalog()` |
//! | POST | `/api/v1/model-catalog/lookup` | `lookup_model_catalog()` |
//! | POST | `/api/v1/model-catalog/match` | `match_model_catalog()` |
//! | POST | `/api/v1/model-catalog/refresh` | `refresh_model_catalog()` |
//!
//! ### `notifications`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/notifications` | `list_notifications()` |
//! | POST | `/api/v1/notifications/{notification_id}/dismiss` | `dismiss_notification()` |
//! | POST | `/api/v1/notifications/{notification_id}/actions/{action_id}` | `resolve_notification_action()` |
//! | GET | `/api/v1/notifications/stream` | `sse()` |
//! | GET | `/api/v1/notifications/stream` | `sse()` |
//!
//! ### `operator`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/operator/tools` | `list_operator_tools()` |
//! | POST | `/api/v1/operator/tools/invoke` | `invoke_operator_tool()` |
//!
//! ### `ops`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/healthz` | `healthz()` |
//! | GET | `/readyz` | `readyz()` |
//! | GET | `/metrics` | `metrics()` |
//!
//! ### `permission-rules`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,POST | `/api/v1/permission-rules` | `list_permission_rules()`, `create_permission_rule()` |
//! | POST | `/api/v1/permission-rules/batch` | `batch_permission_rules()` |
//! | GET,PUT,DELETE | `/api/v1/permission-rules/{rule_id}` | `get_permission_rule()`, `replace_permission_rule()`, `delete_permission_rule()` |
//! | POST | `/api/v1/permission-rules/{rule_id}/revoke` | `revoke_permission_rule()` |
//!
//! ### `plugin-rpc`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | POST | `/plugin-rpc/{plugin_id}` | `plugin_rpc()` |
//!
//! ### `plugins`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/plugins` | `list_plugins()` |
//! | GET | `/api/v1/plugins/surface` | `get_plugin_surface_catalog()` |
//! | GET | `/api/v1/plugins/architecture` | `get_plugin_architecture_catalog()` |
//! | GET | `/api/v1/plugins/tools/changes` | `list_plugin_tool_registry_changes()` |
//! | POST | `/api/v1/plugins/tools/invoke` | `invoke_plugin_tool()` |
//! | GET | `/api/v1/plugins/{plugin_id}` | `get_plugin()` |
//! | GET,PUT | `/api/v1/plugins/{plugin_id}/settings` | `get_plugin_settings()`, `update_plugin_settings()` |
//! | POST | `/api/v1/plugins/{plugin_id}/commands/{command_id}/invoke` | `run_plugin_command()` |
//! | GET | `/api/v1/plugins/{plugin_id}/logs` | `list_plugin_logs()` |
//! | POST | `/api/v1/plugins/marketplace/search` | `search_marketplace_plugins()` |
//! | POST | `/api/v1/plugins/marketplace/sync` | `sync_marketplace_registry()` |
//! | GET | `/api/v1/plugins/marketplace/installed` | `list_marketplace_installed_plugins()` |
//! | GET | `/api/v1/plugins/marketplace/outdated` | `list_marketplace_outdated_plugins()` |
//! | POST | `/api/v1/plugins/marketplace/install` | `install_marketplace_plugin()` |
//! | POST | `/api/v1/plugins/marketplace/uninstall` | `uninstall_marketplace_plugin()` |
//! | POST | `/api/v1/plugins/marketplace/upgrade` | `upgrade_marketplace_plugins()` |
//!
//! ### `project`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | POST | `/api/v1/project/git/init` | `init_git_repository()` |
//!
//! ### `provider-studio`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/provider-studio/draft` | `get_provider_studio_draft()` |
//! | POST | `/api/v1/provider-studio/draft/models` | `list_provider_studio_draft_models()` |
//! | POST | `/api/v1/provider-studio/draft/model` | `get_provider_studio_model_draft()` |
//! | POST | `/api/v1/provider-studio/draft/model/catalog-template` | `apply_provider_studio_catalog_template()` |
//! | POST | `/api/v1/provider-studio/save` | `save_provider_studio_draft()` |
//! | POST | `/api/v1/provider-studio/save-adapter` | `save_provider_studio_adapter()` |
//! | POST | `/api/v1/provider-studio/save-model` | `save_provider_studio_model()` |
//! | POST | `/api/v1/provider-studio/delete-provider` | `delete_provider_studio_provider()` |
//! | POST | `/api/v1/provider-studio/delete-adapter` | `delete_provider_studio_adapter()` |
//! | POST | `/api/v1/provider-studio/delete-model` | `delete_provider_studio_model()` |
//! | POST | `/api/v1/provider-studio/auth/start` | `start_provider_studio_auth()` |
//! | POST | `/api/v1/provider-studio/auth/continue` | `continue_provider_studio_auth()` |
//!
//! ### `providers`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/providers` | `list_providers()` |
//! | POST | `/api/v1/providers/client-versions/refresh` | `refresh_provider_client_versions()` |
//! | GET | `/api/v1/providers/aws-profiles` | `list_aws_profile_names()` |
//! | POST | `/api/v1/providers/models` | `list_provider_adapter_models()` |
//! | GET,POST | `/api/v1/providers/{provider_id}/models` | `list_provider_models()`, `list_saved_provider_adapter_models()` |
//! | GET | `/api/v1/providers/{provider_id}/configured-models` | `list_configured_provider_adapter_models()` |
//! | GET | `/api/v1/providers/{provider_id}/configured-local-models` | `list_configured_provider_models()` |
//!
//! ### `queries`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | POST | `/api/v1/queries` | `query()` |
//!
//! ### `runtime`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/runtime` | `get_runtime_status()` |
//! | POST | `/api/v1/runtime/reload` | `reload_runtime()` |
//! | GET | `/api/v1/runtime/tasks` | `list_runtime_background_tasks()` |
//! | POST | `/api/v1/runtime/tasks/{task_id}/cancel` | `cancel_runtime_background_task()` |
//!
//! ### `sessions`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,PUT | `/api/v1/sessions/{session_id}/assistant` | `get_assistant()`, `put_assistant()` |
//! | POST | `/api/v1/sessions/{session_id}/assistant/reminders/{reminder_id}` | `control_assistant_reminder()` |
//! | POST | `/api/v1/sessions/{session_id}/assistant/memory` | `edit_assistant_memory()` |
//! | GET,POST | `/api/v1/sessions/{session_id}/integrations` | `list_integrations()`, `create_integration()` |
//! | GET | `/api/v1/sessions/{session_id}/integrations/{binding_id}` | `integration_detail()` |
//! | POST | `/api/v1/sessions/{session_id}/integrations/{binding_id}/control` | `control_integration()` |
//! | PUT | `/api/v1/sessions/{session_id}/integrations/{binding_id}/access` | `update_channel_access()` |
//! | GET | `/api/v1/sessions/{session_id}/platforms` | `platform_inventory()` |
//! | GET | `/api/v1/sessions/{session_id}/media` | `download_session_media_file()` |
//! | POST | `/api/v1/sessions/{session_id}/attachments` | `upload_session_file()` |
//! | GET,POST | `/api/v1/sessions` | `list_sessions()`, `create_session()` |
//! | GET | `/api/v1/sessions/overview` | `session_overview()` |
//! | GET,PUT,DELETE | `/api/v1/sessions/{session_id}` | `get_session()`, `replace_session()`, `delete_session()` |
//! | GET | `/api/v1/sessions/{session_id}/state` | `get_session_state()` |
//! | GET,DELETE | `/api/v1/sessions/{session_id}/executions/{execution_id}/attempts/{attempt_generation}` | `get_execution_attempt()`, `cancel_execution_attempt()` |
//! | GET | `/api/v1/sessions/{session_id}/prompts/preview` | `get_session_prompt_preview()` |
//! | GET | `/api/v1/sessions/{session_id}/file-changes` | `get_session_file_changes()` |
//! | GET | `/api/v1/sessions/{session_id}/plan` | `get_session_plan()` |
//! | GET | `/api/v1/sessions/{session_id}/cost` | `get_session_cost()` |
//! | PUT | `/api/v1/sessions/{session_id}/selection` | `replace_session_selection()` |
//! | PUT | `/api/v1/sessions/{session_id}/permission` | `replace_session_permission()` |
//! | GET | `/api/v1/sessions/{session_id}/parts` | `list_session_parts()` |
//! | GET | `/api/v1/sessions/{session_id}/content/{resource_id}` | `read_content()` |
//! | GET | `/api/v1/sessions/{session_id}/content/{resource_id}/text` | `read_content_text()` |
//! | GET | `/api/v1/sessions/{session_id}/content/{resource_id}/stream` | `stream_content()` |
//! | GET | `/api/v1/sessions/{session_id}/inline-ui/{reference}/live` | `read_inline_ui_snapshot()` |
//! | GET | `/api/v1/sessions/{session_id}/inline-ui/{reference}/stream` | `stream_inline_ui()` |
//! | GET | `/api/v1/sessions/{session_id}/inline-ui/{reference}` | `read_inline_ui_reference()` |
//! | GET | `/api/v1/sessions/{session_id}/parts/{part_id}/tool-sections/{section}` | `get_session_tool_detail()` |
//! | GET | `/api/v1/sessions/{session_id}/runs` | `read_session_runs()` |
//! | GET | `/api/v1/sessions/{session_id}/changes/stream` | `stream_session_changes()` |
//! | POST | `/api/v1/sessions/{session_id}/messages` | `submit_message()` |
//! | POST | `/api/v1/sessions/{session_id}/continue` | `continue_run()` |
//! | POST | `/api/v1/sessions/{session_id}/compact` | `compact_session()` |
//! | POST | `/api/v1/sessions/{session_id}/clear` | `clear_session()` |
//! | POST | `/api/v1/sessions/{session_id}/fork` | `fork_session()` |
//! | POST | `/api/v1/sessions/{session_id}/btw` | `ask_btw()` |
//! | GET,PUT,DELETE | `/api/v1/sessions/{session_id}/queue/{execution_id}` | `queued_message_for_edit()`, `edit_queued_message()`, `delete_queued_message()` |
//! | POST | `/api/v1/sessions/{session_id}/queue/{execution_id}/take` | `take_queued_message()` |
//! | POST | `/api/v1/sessions/{session_id}/queue/pause` | `set_message_queue_paused()` |
//! | POST | `/api/v1/sessions/{session_id}/queue/clear` | `clear_message_queue()` |
//! | POST | `/api/v1/sessions/{session_id}/cancel` | `cancel_run()` |
//! | POST | `/api/v1/sessions/{session_id}/permission-replies` | `reply_permission()` |
//! | POST | `/api/v1/sessions/{session_id}/user-input-replies` | `reply_user_input()` |
//! | POST | `/api/v1/sessions/{session_id}/interactive/{request_id}/present` | `mark_interactive_request_presented()` |
//! | POST | `/api/v1/sessions/{session_id}/rewind` | `rewind_session()` |
//! | GET | `/api/v1/sessions/{session_id}/export` | `export_session()` |
//! | POST | `/api/v1/sessions/import` | `import_session()` |
//! | GET | `/api/v1/sessions/tree/{root_id}` | `list_session_tree()` |
//!
//! ### `settings`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,PUT,PATCH,DELETE | `/api/v1/settings` | `get_settings()`, `set_settings()`, `patch_settings()`, `delete_settings()` |
//! | GET,PUT,DELETE | `/api/v1/settings/layers/{layer}` | `get_layer_settings()`, `set_layer_settings()`, `delete_layer_settings()` |
//! | GET | `/api/v1/settings/entries` | `list_settings()` |
//! | GET | `/api/v1/settings/sources` | `get_settings_sources()` |
//! | POST | `/api/v1/settings/validate` | `validate_settings()` |
//!
//! ### `skills`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,POST | `/api/v1/skills` | `list_skills()`, `create_skill()` |
//! | POST | `/api/v1/skills/validate` | `validate_skill()` |
//! | POST | `/api/v1/skills/install/preview` | `preview_skill_install()` |
//! | POST | `/api/v1/skills/install` | `install_skills()` |
//! | GET,PUT,DELETE | `/api/v1/skills/{id}` | `get_skill()`, `update_skill()`, `delete_skill()` |
//! | PUT | `/api/v1/skills/{id}/enabled` | `set_skill_enabled()` |
//! | GET,PUT,DELETE | `/api/v1/skills/{id}/resource` | `get_skill_resource()`, `write_skill_resource()`, `delete_skill_resource()` |
//!
//! ### `usage`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/usage` | `get_usage_stats()` |
//!
//! ### `vcs`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/vcs/diff/raw` | `get_vcs_diff_raw()` |
//!
//! ### `workspaces`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET,POST | `/api/v1/workspaces` | `list_workspaces()`, `create_workspace()` |
//! | POST | `/api/v1/workspaces/resolve` | `resolve_workspace()` |
//! | GET | `/api/v1/workspaces/session-stats` | `workspace_session_stats()` |
//! | GET,PUT,DELETE | `/api/v1/workspaces/{workspace_id}` | `get_workspace()`, `replace_workspace()`, `delete_workspace()` |
//! | GET,POST | `/api/v1/workspaces/{workspace_id}/files` | `list_workspace_files()`, `upload_workspace_file()` |
//! | GET | `/api/v1/workspaces/{workspace_id}/download` | `download_workspace_file()` |
//!
//! ### `ws`
//!
//! | Method | Path | Handlers |
//! |---|---|---|
//! | GET | `/api/v1/ws` | `ws()` |
//! | GET | `/api/v1/ws` | `ws()` |
//!
