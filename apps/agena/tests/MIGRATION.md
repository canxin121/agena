# 真实运行测试迁移对照

本文件记录从局部业务测试到 `agena-testkit` 系统测试的替换关系。新的业务场景通过
普通 Rust `#[tokio::test]` 启动产品 `agena server`，用户操作走正式客户端，AI
的回复和工具调用走本地 HTTP / Responses / SSE，插件及运行时使用生产实现。
HTTP、WebSocket、stdio 对端只模拟外部服务；不在测试里直接执行被测插件的方法。

迁移按行为断言完成，不能仅凭“新测试也经过这个模块”删除旧回归。下面的对应关系
同时说明原本的权限类型、副作用、协议字段、版本、持久化及生命周期断言去向。
“退役”单列，表示去掉过时或重复的实现细节约束，不计作新增业务覆盖。

以本轮迁移开始时的工作区为基准，共移除 86 个旧测试：83 项迁移、3 项退役，另迁出一个 Web 聚合测试的
all-denied 分支。新增 58 个系统场景（57 项迁移场景，1 项本轮发现问题的回归），
同时增强已有的行为入口。内置插件局部测试从 109 项缩至 32 项；业务验证集中到
74 项内置插件目标和 27 项通用目标。这是完成迁移时的统计；后续测试库改进新增的
场景不回写这份迁移台账，当前执行入口和覆盖范围见同目录 `README.md`。

| 组 | 移除旧测试 | 新增系统场景 |
|---|---:|---:|
| Core 权限/技能/UI/commands（含 Provider 权限） | 21 | 10 |
| Provider 协议/文件/错误/计费，以及重复 manifest 断言 | 39（37 迁移、2 退役） | 28（含第二工作区文件回归） |
| 模型钩子、队列与 notifications | 16（15 迁移、1 退役） | 12 |
| Web | 3，另迁出一个分支 | 2 |
| MCP connections | 7 | 6 |

## 运行与后续新增规则

```sh
cargo --offline test-system
cargo --offline test-bundled

# 只运行指定组，仍由 Rust libtest 负责发现、过滤和报告。
cargo --offline test-bundled bundled::core::
cargo --offline test-bundled bundled::protocols::providers::
cargo --offline test-bundled bundled::notifications::
cargo --offline test-system model_hooks

# 保留的纯契约和必要的精确故障回归。
cargo test --offline -p agena-bundled-plugins --lib
cargo test --offline -p agena-testkit --lib
cargo test --offline -p agena-runtime-session --lib queue_tests
```

新插件业务测试放在 `bundled/`，跨插件、模型和执行生命周期放在 `scenarios/`。
优先使用 `call_tool`；有权限审批、问题或取消时用 `start_tool`；并发及多轮决策
使用明确匹配的模型脚本和门闩。必须等待精确 execution generation 完成。

失败类型用 `ToolResult::assert_failure_kind` 或 `failure()`，结构化落盘结果用
`data()`。不能靠任意报错代替权限断言，也不能仅靠文本“包含某词”证明实际副作用。
不新增 Fake Host、Fake Provider 来绕过产品执行路径。纯协议编码和无法从公开边界
安排的内部故障可以保留专门测试，但要写明它验证的独立边界。

## Core：权限、技能、命令和 Inline UI

新位置的共同前缀为 `apps/agena/tests/bundled/`。
旧 `crates/agena-bundled-plugins/src/permission_tests.rs` 已删除。

| 旧测试 | 新测试及保留的实际行为 |
|---|---|
| `structural_directory_search_checks_children_and_preserves_denial` | `core_permissions::structural_search_checks_children_and_preview_respects_write_denial`：真实 AST 目录搜索独立检查子文件，拒绝类型正确，私有标记不泄漏。 |
| `ast_preview_only_needs_read_but_application_needs_write` | 同一新场景：只读权限下预览成功，携带有效 SHA 的应用被拒，原文件不变。 |
| `filesystem_mutation_requires_write_and_does_not_touch_denied_files`、`filesystem_reads_authorize_before_metadata_or_conversion`、`notebook_denial_precedes_file_access` | `core_permissions::filesystem_and_notebook_denials_precede_io_and_leave_no_side_effects`：拒绝先于不存在路径的读取、转换和编辑；写入连父目录都不能创建。 |
| `memory_index_permission_is_checked_before_creating_a_record`、`memory_mutation_honors_path_denial_before_creating_a_record`、`memory_authorization_uses_the_invoking_workspace_instead_of_initial_state_root` | `core_permissions::memory_record_and_index_denials_use_the_invoking_workspace`：真实第二工作区会话；分别拒绝记录与索引路径，确认记录、索引及其他 store 没有错误副作用。 |
| `skills_denied_create_authorizes_the_invoking_workspace_without_creating_files`、`skills_source_read_denial_precedes_folder_or_archive_access` | `core_permissions::skill_source_denials_precede_access_and_create_denial_uses_workspace`：真实第二工作区创建，以及 folder/archive 的 preview/install 读取拒绝。 |
| `skills_network_decline_is_preserved_before_downloading_any_archive` | `core_permissions::skill_downloads_wait_for_permission_and_preserve_user_declines`：正式审批 archive/GitHub 来源，验证重写后的授权目标、UserDeclined、零 HTTP 和无安装目录。 |
| `skills_install_previews_without_write_but_denied_install_creates_no_package` | `core_permissions::skill_install_preview_is_read_only_and_denied_install_preserves_source`：预览允许，使用正确 revision 的安装被拒，源文档和资源字节不变。 |
| `skills_crud_and_resource_contract_work_without_a_global_plugin_host` | `core::skills_manage_resources_revisions_and_availability`：真实 CRUD、启停、revision、资源；缺失 revision 的覆盖必须失败，原内容不变。 |
| `provider_requests_preserve_declines_without_sending_or_exposing_query_secrets` | `protocols::providers::boundaries::upload_and_network_denials_stop_before_io_and_hide_query_secrets`：实际 vendor tool → 审批拒绝，Peer 零请求，模型收到类型化拒绝，审批目标不含 query secret。 |

另外删除 `inline_ui.rs`、`inline_ui/errors.rs`、`commands.rs`、
`commands/discovery.rs` 的旧局部测试模块：

| 旧测试 | 新测试及保留的实际行为 |
|---|---|
| `default_core_is_complete_bounded_and_small_without_optional_references`、`topic_schema_does_not_advertise_unrequested_kinds` | `core_contracts::inline_ui_guide_is_complete_bounded_and_serves_only_requested_schemas`：真实 guide 的完整持久文本、24 KiB 上限、章节去重和数量/未知项拒绝；请求 steps 后通过 jsonschema 验证 steps 有效而 chart 无效。 |
| `prepare_update_and_inline_error_flow_needs_no_extra_validate_or_read` | `core_contracts::inline_ui_live_panel_repairs_errors_without_corrupting_the_published_head`：实际 prepare/update/read 与 HTTP/SSE live；revision、slider 状态、过期更新和错误修复、失败不污染 head。 |
| `independent_block_errors_are_bounded_with_explicit_truncation` | `core_contracts::inline_ui_invalid_blocks_return_bounded_safe_model_feedback`：12 个错误只向模型给出 8 个唯一、正确定位的修复字段；不泄漏正文或私有错误报告。私有的 truncated 字段不穿透正式投影去断言。 |
| `builtin_command_manifest_has_valid_unique_declarations`、`command_projection_reads_standard_yaml_and_keeps_legacy_names`、`disabled_packages_and_temporary_install_trees_are_not_advertised` | `core_contracts::command_catalog_and_discovery_preserve_public_authoring_contracts`：运行服务的 command catalog、参数和 client action；真实 CRLF/YAML/legacy/alias 发现；禁用和 staging 包不可列出或获取。 |

## Provider：协议、文件、计费和错误

新位置共同前缀为 `apps/agena/tests/bundled/protocols/providers/`。
旧 `provider_connection_tests.rs` 只保留 URL 编码契约；其 `more.rs`、`output.rs`
场景与夹具已移除。旧请求链不再直接实例化服务插件并注入 Host。

| 旧测试 | 新测试 |
|---|---|
| `provider_tools_follow_the_live_connection_and_model_override` | `catalog::provider_tools_follow_live_endpoint_changes_and_custom_headers`，以及三家 vendor 基础场景 |
| `chatgpt_image_tools_keep_direct_and_responses_models_separate` | `resources::image_and_embedding_models_reach_their_native_protocols` |
| `unified_provider_models_support_paging_filters_and_unambiguous_details` | `catalog` 的 model_catalog 场景及 `settings_provider` 分页场景，覆盖 adapter 歧义、描述搜索、metadata、限制、非法输入和 provider 切换后的 cursor |
| `model_discovery_has_one_public_tool_with_a_focused_schema`、`provider_plugin_contracts_remove_connection_settings_and_cloud_prefixes`、`model_tools_document_the_session_default_override_and_vendor_requirement` | `catalog::loaded_provider_manifests_expose_only_current_public_contracts`；验证运行时发布的 schema 语义，不固定总工具数或逐字说明文案 |
| `uploads_need_no_model_and_handles_stay_bound_to_the_provider` | `files::uploaded_file_handles_are_bound_to_the_selected_provider` |
| `resource_apis_use_provider_auth_and_the_correct_model_protocol` | `resources` 中真实平台、嵌套资源、inference 和 batch 请求 |
| `generated_files_and_audio_download_actual_bytes_without_a_model` | `files::downloads_and_generated_audio_persist_the_exact_bytes` |
| `generic_file_upload_accepts_binary_datasets_and_default_model_is_not_sent`、`native_multipart_repeats_arrays_and_names_consent_recordings` | `files::binary_and_native_multipart_uploads_keep_their_official_fields` |
| `realtime_uses_current_provider_waits_for_setup_and_saves_bounded_wav` | `realtime::realtime_waits_for_setup_and_persists_complete_audio_and_usage` |
| `knowledge_and_media_workflows_preserve_vendor_payloads_and_redact_all_outputs` | `resources::knowledge_inference_and_batches_preserve_vendor_fields` 和文件下载场景 |
| `resource_wait_retains_pending_id_when_polling_fails_without_resubmitting` | `usage::failed_poll_retains_pending_identity_without_resubmitting_paid_work` |
| `resource_uploads_and_requests_stop_at_permission_denial` | `boundaries::upload_and_network_denials_stop_before_io_and_hide_query_secrets` |
| `realtime_retains_partial_audio_and_does_not_report_failed_turns_as_completed` | `realtime` 中失败/超时语音场景，验证 partial WAV、未完成状态和真实 usage |
| `managed_session_events_decode_sse_and_keep_the_session_id`、`inference_stream_retains_sse_cursors_and_final_usage` | `usage::provider_sse_preserves_cursors_usage_and_managed_session_identity` |
| `prompt_voices_and_compaction_need_no_file_and_use_official_bodies`、`platform_nested_resources_support_real_checkpoint_ids_and_official_json` | `resources::platform_resources_and_prompt_voices_use_official_bodies` |
| `agent_artifacts_download_bytes_and_nested_resources_preserve_ids` | `resources::nested_agent_resources_and_artifacts_keep_all_identifiers` |
| `gemini_batch_embeddings_use_selected_model_and_native_endpoints` | `resources::gemini_batch_embeddings_keep_native_models_and_update_queries` |
| `file_search_resumable_upload_retains_metadata_and_checks_each_network_target` | `files::resumable_file_search_upload_keeps_metadata_and_operation_identity` |
| `file_search_rejects_cross_origin_upload_before_sending_bytes` | `files::cross_origin_upload_never_receives_the_document` |
| `async_usage_is_recorded_once_after_completion_even_across_tool_aliases` | `usage::background_completion_and_aliases_record_billable_usage_once` |
| `inference_without_usage_does_not_claim_later_usage`、`concurrent_response_polls_cannot_double_count_usage` | `usage::incomplete_usage_can_arrive_later_and_concurrent_polls_do_not_double_bill` |
| `claude_batch_results_keep_downloads_and_attribute_each_model_once`、`completed_gemini_batch_operation_attributes_inline_usage_once` | `usage::batch_results_attribute_each_model_once_and_keep_downloads` |
| `provider_response_text_survives_the_durable_payload_contract` | 三家 vendor 基础场景检查正式工具详情中的 `payload.text` |
| `blocked_provider_callbacks_do_not_reappear_in_durable_text` | `boundaries::blocked_provider_actions_never_reappear_as_authoritative_text` |
| `durable_provider_text_is_bounded_without_truncating_its_receipt` | `boundaries::unicode_provider_text_is_bounded_but_receipt_bytes_remain_complete` |
| `connection_refused_survives_rpc_without_url_or_header_credentials` | `boundaries::refused_vendor_connection_does_not_leak_query_or_credentials` |
| `stalled_response_reports_request_timeout_and_configured_limit` | `boundaries::request_timeouts_keep_read_and_mutation_failure_policies_distinct` |
| `unrecognized_and_malformed_responses_never_copy_body_content`、`count_tokens_non_json_404_is_not_partial_success` | `boundaries::malformed_and_unrecognized_http_errors_keep_typed_public_failures` |
| `get_connection_failure_keeps_read_failure_policy` | 真实 GET timeout、POST connection-refused，加保留的纯读写错误策略矩阵 |

计费断言读取正式 `usage_stats` 和会话 cost summary 持久数据，按模型核对次数和
token；不以测试自行累计的数字证明产品去重。文件断言读取实际下载的字节、附件和
receipt；外部协议对端会检查请求方法、目标、header、body、顺序及未消费请求。

## Runtime：模型钩子和队列

旧 `session/manager/execution/model_hooks_tests.rs` 整文件及模块声明删除。
新文件为 `apps/agena/tests/scenarios/plugins/model_hooks.rs`。

以下五个测试保留原名并迁至真实 HTTP SDK 插件和本地模型协议：

| 测试 | 新运行边界 |
|---|---|
| `model_hooks_fire_at_the_actual_provider_boundary` | 实际 HTTP header、system/messages 内容、session/provider/adapter/execution 标识和钩子顺序 |
| `run_hooks_fire_once_even_when_an_execution_requires_two_model_rounds` | 实际 incomplete 响应触发第二轮；run hook 一次，模型 hook 两次 |
| `failed_model_request_is_retried_only_through_the_explicit_error_hook` | HTTP 400、显式 retry 与拒绝 retry 的负对照，新 request identity |
| `exact_attempt_waits_for_post_run_and_never_follows_a_retry` | HTTP run.post 门闩，Terminal/Settled、Superseded、新旧 generation 与未知代 404 |
| `per_run_system_reaches_provider_without_persisting_or_removing_hooks` | 会话 prompt、非空/空串 per-run 覆盖、继承恢复，正式 HTTP instructions 和持久层不变 |

从 `session/manager/commands/queue_tests.rs` 移除的四项：

| 旧测试 | 新位置和行为 |
|---|---|
| `a_retry_is_not_lost_while_the_previous_generation_is_still_settling` | `plugins/model_hooks.rs` 同名；旧 post 门闩内 continue，旧代 CAS 不能停止新代，新代自动开始且仅一个模型请求 |
| `stopping_without_queued_input_allows_the_next_send_to_start` | `scenarios/queue.rs` 同名；session 和 execution 两条正式停止路径，随后消息真实到达模型 |
| `stopping_with_queued_input_keeps_it_paused_and_intact` | 同名；停止保留两条 Queued/Pending，暂停期间零额外模型请求，resume 后 FIFO 和上下文继承 |
| `stopping_an_empty_session_preserves_a_manual_pause` | 同名；空会话手动暂停、停止、发送，仍保持排队，显式 resume 才到模型 |

## Notifications：真实恢复与重载

新位置为 `apps/agena/tests/bundled/notifications.rs` 和
`notifications/recovery.rs`。旧 `notifications/attention/tests.rs` 从八项缩为
一项精确 lost-watch-event 故障回归。

| 旧测试 | 新覆盖 |
|---|---|
| `canonical_completions_and_failures_publish_without_a_connected_client` | recovery 同名；无 SSE 客户端，仅短 REST，检查 Completed/Failed/Cancelled 与 generation identity、唯一性 |
| `canonical_questions_and_permissions_enqueue_once_without_tool_display_events` | 主场景 Question 唯一性，recovery 的实际 fs.write Permission 唯一性 |
| `startup_recovers_pending_requests_without_replaying_unobserved_old_replies` | `canonical_pending_requests_recover_without_browser_state_and_keep_resolution`：先禁用通知完成旧 run 并生成实际 pending，再启用/重载，恢复 pending 而不回放旧 completion |
| `real_plugin_host_owns_attention_across_reload_disable_and_shutdown` | 主场景：A 运行中 reload 后启动 B；A 完成仍 running；disable/re-enable、真实 restart |
| `answered_requests_replace_the_pending_intent_and_persist_resolution` | recovery 正式 deny 后同 id resolved，文件不写入，重启后仍只有一条 resolved |
| `host_queue_does_not_resurrect_a_resolved_request_during_replay` | recovery 同名；三个真实 HTTP SDK tool 分别通知 pending/resolved/迟到 pending，公开 catalog 仍 resolved |

## Web：实际授权、网络副作用和输入边界

新位置为 `apps/agena/tests/bundled/protocols/web.rs`。

| 旧测试/分支 | 新覆盖及处理 |
|---|---|
| `non_public_fetch_targets_keep_their_policy_denial_classification` | `configured_search_uses_real_http`：实际私网 fetch 返回 PermissionDenied，完整 FailureCategory 也正确，HTTP handler 零请求；删除旧测试 |
| `denied_search_sources_keep_typed_denial_and_partial_sources_stay_visible` 的 all-denied 分支 | `configured_search_permission_decline_precedes_http`：真实配置的 SearXNG → network ask → deny，核对审批目标及前后零 HTTP，模型收到 UserDeclined；删除该分支 |
| `browser_open_requires_one_typed_target` | `browser_open_validates_typed_targets_before_browser_or_network_io`：无目标、空目标、两个目标分别为 InvalidInput；合法单目标进入权限分支，私网和未持有 preview 均被拒且无网络；删除旧测试 |
| `no_preview_grant_never_authorizes_a_private_request` | 真实 fetch 和 browser_open 的类型化拒绝及零 HTTP；删除旧测试 |

混合来源的聚合断言仍保留，改名 `mixed_search_sources_keep_success_and_report_denied_sources`：
成功来源不能被另一来源的拒绝抹掉，partial/unavailable 必须准确。

## MCP：连接管理、授权与真实 stdio

新位置为 `apps/agena/tests/bundled/protocols/mcp/connections.rs`；
`stdio_peer.py` 仅作为产品实际启动的本地 JSON-RPC 对端，Rust test 仍是唯一测试入口。
旧 `plugins/provided/mcp/connections/tests.rs` 从八项缩为一个精确 reload queue 故障回归。

| 旧测试 | 新覆盖 |
|---|---|
| `reads_require_both_config_paths_and_return_redacted_values_in_every_output_channel` | 同名真实场景；list/get 获取实际 opaque id，检查 URL/header/token 脱敏；分别拒绝全局和工作区配置读权限，文件不变 |
| `denied_config_reads_fail_before_parsing_files_and_preserve_policy_error` | 同名真实场景；模型请求已建立后用门闩暂停工具调用，再写入无效配置，释放真实工具调用，必须先返回 PermissionDenied；恢复源文件 |
| `denied_process_authorization_leaves_config_unchanged_and_does_not_queue_reload`、`denied_config_write_never_requests_execution_authority_or_reload` | `denied_writes_and_declined_process_approval_leave_connections_unchanged`：写拒绝在先；随后实际进程审批拒绝，配置逐字不变、进程未启动、catalog 无新增 |
| `accepted_mutation_authorizes_server_workspace_then_queues_reload_after_persisting` | `accepted_mutation_authorizes_then_persists_reloads_and_runs_the_real_stdio_peer`：审批前不保存或启动；允许后保存、精确 reload receipt、新 generation、实际 initialize/search/call，argv 不被 shell 展开、cwd/env 正确，最后删除 |
| `flat_update_input_keeps_only_requested_secrets_and_omitted_preservation_clears_removed_fields` | `flat_update_keeps_only_requested_secrets_and_omitted_preservation_clears_removed_fields`：两次真实 revision 更新、按 JSON pointer 保留指定密钥、移除其他值，禁用记录不启动进程 |
| `deleting_a_workspace_override_requires_authorization_before_reactivating_global_process` | `deleting_a_workspace_override_requires_approval_before_reactivating_global_process`：全局记录被工作区禁用覆盖，删除时实际审批拒绝；覆盖层、配置和进程状态不变 |

配置读取的返回值检查整个模型 envelope。对模型自己提交密钥的 create/update，
检查输出时只排除正常的 `result.invocation.input` 调用参数回显，仍检查完整输出、
model/error/failure 和 hydrated raw_output；不把同一调用的输入误当作插件额外泄漏。

## 明确退役的实现断言

| 旧测试 | 原因 |
|---|---|
| `browser_subscription_state_is_not_an_attention_recovery_source` | 向生产已退役的 `webPush.active` 私有字段写入数据再检查无影响；当前公开 canonical 恢复及无 browser/SSE 场景已有覆盖 |
| `real_discovery_catalog_has_exactly_four_inline_ui_tools` | 固定工具总数与清单；prepare/read/update/guide 已有真实行为场景，新插件登记由 inventory guard 检查 |
| `bundled_plugins_publish_their_own_localized_hook_introductions` | 固定若干语言逐字文案；保留跨全部产品 locale 的 summary/help 完整性测试 |

## 真实运行暴露并修复的缺口

| 缺口 | 修复与验证位置 |
|---|---|
| HTTP 插件的 `host.notify` 仍落到默认 HostUnavailable | SDK 将 notify 接入已有公共 callback 转发；仍通过已认证的 plugin owner/scope。真实 HTTP probe 的 pending/resolved/迟到 pending 场景验证队列单调性 |
| Provider 相对文件被解析到插件配置目录 | 统一先用当前 callback workspace 解析显式路径，再授权和 I/O；files 新增 `relative_provider_files_use_the_invoking_workspace_and_preserve_absolute_paths`，第二工作区与两个含同名 decoy 的目录对照，含阻塞媒体输入、相对下载及绝对路径 |
| 已持久化的 Provider 工具用量未进入统计 | usage/cost 查询读取 canonical tool metadata，与 round usage 分别累加；真实异步、并发、别名、batch、SSE/语音场景检查按模型/会话的实际账单，不把工具账单回写到上下文 token 状态 |
| HTTP fixture 不能完成 WebSocket 升级 | 开启 Hyper upgrades，并跟踪升级 socket 的释放；testkit 自测验证真实帧交换和仍连接时的关闭，Realtime peer 的协议循环由 case-owned worker 管理 |

迁移时也纠正了测试自身的假设：transport hook 的 adapter 标识与 configured
provider 标识分开断言；真实 Gemini adapter 使用 `gemini`；引用正文经 content
API 读取；错误文本不泄漏的断言不混入模型自己提交的 invocation input。

## 保留的聚焦回归

保留的依据是具体不可替代的验证目标，不是文件旧或小：

| 范围 | 保留边界 |
|---|---|
| Provider 16 项 | URL 单段编码、OS ErrorKind 映射、错误净化/凭据剥离/Unicode 限制/分类、读写策略身份、route/media/SSE 截断游标等纯契约；没有 Fake Host 请求链 |
| capability manifest 2 项 | 注册表与目录一致，包括无工具插件；所有产品 locale 的 hook summary/help 翻译完整性 |
| notifications 1 项 | 明确丢弃已提交 terminal watch event，再恢复并核对稳定 identity；普通 restart 不能等价安排这个故障 |
| MCP 1 项 | 配置已保存后精确注入 reload 请求排队失败，必须返回 saved + warning 并保留配置；公共服务没有可确定安排此窗口而仍让工具正常返回的入口 |
| queue 4 项 | 精确 supervisor retirement 竞态；owning workspace/server default/effective cwd 三者不同的图片归属；persisted workspace 权限和缺失 session fail-closed；无可用 provider 时的 prompt projection |
| agents 8 项 | 子代理的 stored budget、timeout、panic/unwind、废弃 settlement registration、manual compaction、精确排队 settlement 和 launch hook 拒绝；现有真实 agent lifecycle 场景没有完整替代这些断言 |
| SQLite / client / testkit / SDK | 事务、claim generation、内容传输完整性、协议兼容、测试框架自身的负对照和资源清理契约 |
| Web 安全边界 | 确定性 DNS/IP 分类、managed-preview lease/当前 callback/逐次重授权、CDP 错误隔离和 CrawlError identity；没有实际等价覆盖前不删除 |
| 安装、终端、跨平台专项 | 服务管理器、PTY、安装脚本、终端布局、浏览器渲染等不是 Rust server HTTP 场景的同一验证目标 |

内置插件目录之外的低层测试没有被成批删掉。后续要迁移这些保留场景，应先构造
能验证同一个故障或权限边界的真实场景，再删除原回归和不再需要的夹具。

## 本轮验证结果

| 目标 | 结果 |
|---|---|
| `agena --test bundled_plugins` | 全部 74 项取得通过结果：全量运行中非 Provider 的 43 项通过；修正 Provider fixture 后，完整复验 Provider 31 项，全部通过 |
| `agena --test system` | 完整运行 27 项通过 |
| `agena-testkit --lib` | 30 项通过，含真实 WebSocket 帧交换和仍连接时的关闭回归 |
| `agena-plugin-sdk --lib --features http,stdio` | 66 项通过 |
| `agena-bundled-plugins --lib` | 保留的 32 项契约/精确故障回归通过 |
| `agena-runtime-session --lib` | 88 项通过 |
| `agena-storage-sqlite --test tool_metadata` | 1 项通过，覆盖 canonical metadata、分页、会话所有权、终态、投影副本和 fork 继承 |

上述命令均以 `--offline` 运行。系统测试只使用本地脚本模型和协议对端；内置插件
及通用系统两个目标没有 `#[ignore]` 用例。最后一轮修正仅涉及 Provider fixture、
文档和未使用的测试 helper，没有更改已经通过其他场景验证的产品行为。

另已对工作区中 112 个修改或新增的 Rust 文件执行格式检查，对两个 Python 协议
fixture 执行语法解析，并通过 `git diff --check`。
