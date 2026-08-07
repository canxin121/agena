# Agena 通知与状态显示系统全手册

> 分支: docs/notification-display-map
> 基线: 4eaebcfd (master)
> 范围: 运行时契约层、运行时产出、TUI 显示层、Web UI 显示层、HTTP/SSE API 端点
> 用途: 把所有「通知 / 状态 / 进度 / 模型 / 插件信息」的显示位置、显示内容、代码路径、调用的 API 梳理成一份可检索的参考文档。

---

## 1. 总体架构（三层）

| 层 | 位置 | 职责 |
|---|---|---|
| 运行时契约 | crates/agena-runtime-contracts、crates/agena-domain | 定义通知的数据结构（NoticePart / NoticeActivity），随 transcript 持久化并投影 |
| 运行时产出 | crates/agena-runtime-session | 在特定场景（如模型轮次预算耗尽）生成通知活动 |
| 显示端 | TUI（crates/agena-tui*）+ Web UI（packages/agena-web-ui） | 把通知 / 状态渲染到屏幕特定位置 |

核心结论：Agena 有两套并行的「瞬时通知」机制。

1. TUI 的 UiNotice（crates/agena-tui/src/notice.rs）：一条最近的通知显示在转录区底部 footer 行（composer 上方），默认 5 秒过期。
2. Web 的 errorMessage / localCommandNotice（ChatPage.vue）：显示在页面顶部的 inline .notice 横幅，无 fixed 定位；全仓库唯一真正的 fixed toast 在 Runtime 概览页 RuntimeOverviewPanel.vue（右上角）。

另外还有一套「持久化通知」：运行时把 NoticePart 作为一等消息 part 写入 transcript，TUI 与 Web 都把它渲染成一条可折叠的系统通知活动（例如 max_turns_exhausted）。

---

## 2. 运行时契约层

### 2.1 NoticePart（消息 part）

文件：crates/agena-runtime-contracts/src/message/part/notice.rs

字段：

- kind: String —— 机器可读通知类别，例如 max_turns_exhausted
- summary: String —— 短人类摘要（折叠标题）
- detail: Option<String> —— 展开时显示的可选详情

它是一等 transcript part（与工具调用同一条活动管线）：随消息持久化、投影给客户端、由 transcript 渲染。

### 2.2 NoticeActivity 与 ActivityPayload

文件：crates/agena-domain/src/activity.rs

- NoticeActivity（line 685）：与 NoticePart 同构的域层活动（kind/summary/detail），仅面向用户，绝不投影给模型。
- ActivityPayload::Notice(NoticeActivity)（line 364），活动枚举共 18 种：Resource / SkillReference / SkillExecution / TextArtifact / Reasoning / TextSegment / Operation / Interaction / Progress / Checklist / Search / FileChanges / NestedTask / Maintenance / Hook / Error / Custom / Notice。
- 活动载体 ActivityNode（line 289）：id / owner / actor / payload / state / position / revision_seq / lifecycle / provenance。

### 2.3 事件流与持久化

- 持久化：crates/agena-runtime-session/src/session/history/store/mod.rs line 1300 附近把 Notice 活动写入 agena_model_message_parts（SQLite），读回时还原。
- 消息 part 摘要投影：crates/agena-runtime-contracts/src/message/part/message_part.rs line 225（name=notice）、line 253（truncate_summary 取 notice.summary）。

---

## 3. 运行时产出（谁生成通知）

目前唯一的第一方产出点：模型轮次预算耗尽。

文件：crates/agena-runtime-session/src/session/manager/replies/replies_execution.rs

- line 506-520：model_turns_taken >= max_turns 时「软停」；先记录一条用户可见通知再返回，避免运行看起来像正常结束。
- record_model_turn_budget_notice（line 795-835）：
  - kind = max_turns_exhausted
  - summary = Model-turn budget exhausted; the run stopped.
  - detail = 说明达到 max_turns 上限、如何继续/调参（session.max_turns，0 表示不限）
  - 以 Role::Assistant + ExecutionStatus::Completed + MessageSource::System 写入会话（model_turn_id: None，不会触发新一轮模型调用）
  - 用户面专属：provider 投影跳过它，后续 normalize_prompt_messages 会丢弃无可见载荷的它

---

## 4. TUI 显示层（Rust / ratatui）

### 4.1 数据结构：UiNotice

文件：crates/agena-tui/src/notice.rs

- NoticeSeverity（line 8）：Success / Warning / Error / Info
- NoticeScope（line 16）：Global / Composer / Field / Form / Session / ToolCall / Provider / Plugin / Settings / BackgroundTask / Startup
- NoticeAction（line 32）：label + command（例如 Refresh -> session.refresh）
- UiNotice（line 38）：summary / detail / severity / scope / action / expires_at
- DEFAULT_NOTICE_DURATION = 5 秒（line 5）
- 构造：UiNotice::message(severity, summary)（默认 5 秒）、with_lifetime、from_failure（把 agena_failure::Failure 转成 UserProblem 摘要并附加恢复动作）、from_problem
- recovery_action（line 88）：RecoveryDirective -> 动作/命令映射：
  - Refresh -> session.refresh
  - Reauthenticate -> provider.authenticate
  - OpenSettings -> settings.open
  - RequestPermission -> permissions.open
  - Retry -> operation.retry
  - ChooseAlternative -> alternative.choose
  - RestartPlugin -> plugin.restart
  - RestartRuntime -> runtime.restart
  - None / AskUser -> 无动作
- display_summary()：只返回洗过的摘要，绝不含关联 ID 等机器信息
- is_expired_at(now)：过期判断

### 4.2 显示位置 ①：转录区底部 footer（通知横幅）

文件：crates/agena-tui-app/src/view/view_main.rs

- draw()（line 50）：非 Main 路由渲染 render_transcript_footer_row（line 89）；Main 路由由 render_transcript_surface 用 layout_header_body_footer_surface 的 footer 承载（line 288-293）。
- transcript_footer_spec()（line 477）：若 self.notice 存在则优先显示通知摘要，样式按严重级取色；否则回退 transcript_footer_text()。
- notice_style()（line 569）：Success -> success_color，Warning -> warning_color，Error -> danger_color，Info -> info_color。
- transcript_footer_text()（line 501）无通知时的组合内容，用  |  分隔：
  1. 待发队列预览 self.queue.preview(28)
  2. 外部状态行输出 self.status_line.text()
  3. 插件 statusline 段 backend.plugin_statusline_segments()：跳过 agena.terminal.* 内部段（window-title/notification 信号）与 plan 段（有专属 chip）
  4. 插件 TUI 内容块 backend.plugin_tui_content_blocks() 中 location == composer_footer 的块（标题+正文）
- transcript_footer_height（line 557）：最多 2 行，footer 文本 wrap。

### 4.3 显示位置 ②：composer（输入框）边框四角状态 chip

文件：crates/agena-tui-app/src/view/view_main.rs + crates/agena-tui-components/src/surface.rs

render_composer()（line 361）把 chip 画在边框行，不占用独立行：

- 左上角 status chip = composer_status_parts().join('  |  ')：
  - current_session_status_parts()（app_status_context.rs line 214）：模型名（优先当前会话模型 -> 执行上下文 -> 解析默认模型）、thinking 模式、speed 模式、token 上下文百分比
  - 转录加载中、搜索摘要（当前/总数）、@ 文件提及 / / 命令候选、待用户输入数、被隐藏的交互对话框
- 右上角 chip = 历史搜索进度 或 待审批权限数（composer_history_search_part / composer_pending_approval_part，line 651-685）
- 左下角 chip = 后台活动数 ● N background（composer_background_activity_part，line 644）
- 右下角 chip = plan 进度（composer_plan_progress_part，line 689：取插件 statusline 段 plan:{session_id}）

chip 布局算法（surface.rs）：

- composer_status_placement（line 85）：居中 chip，左右各至少 1 个 ─，文本超宽截断加 …，chip_width = text_width + 2
- composer_status_placement_reserving（line 123）：居中但为右角 chip 预留空间
- composer_status_placement_left（line 175）：贴左角
- composer_corner_placement_left（line 221）：贴左下角
- composer_corner_placement_right（line 255）：贴右上/右下角
- layout_composer_surface（line 331）：整体布局

### 4.4 显示位置 ③：转录区头部（title / subtitle / right）

文件：crates/agena-tui-app/src/view/view_main.rs

- transcript_surface_title()（line 353）：#session_id · title
- subtitle：current_session_path_label()（app_navigation.rs line 376）——当前会话的路径标签
- right：transcript_surface_top_right()（line 346）= transcript_surface_top_right_parts(activity, mode)（line 1008）
  - activity = current_session_activity_indicator()（app_status_context.rs line 4）：Idle -> None；Running -> 旋转 spinner；AwaitingPermission / AwaitingUserInput / Blocked -> 对应文案
  - mode = main_surface_mode_label()（line 885）：例如 INSERT

### 4.5 显示位置 ④：外部状态行（status_line）

文件：crates/agena-tui/src/status_line.rs + crates/agena-tui/src/presentation_config.rs

- TuiStatusLineConfig（presentation_config.rs line 49）：command: Option<String> + refresh_interval_ms
- StatusLinePresentation（status_line.rs line 22）：command / refresh_interval / next_refresh_at / text / refresh_in_flight
- tick(now)（line 45）：到点且无 in-flight 时产生 StatusLineEffect::Refresh{command}
- apply_refresh(output)（line 58）：更新文本
- 输出最终渲染进 transcript footer（view_main.rs line 514）
- 配置入口：TuiConfig.status_line（presentation_config.rs line 22）

### 4.6 显示位置 ⑤：终端集成（窗口标题 / 铃响 / OSC 9;4 进度条）

文件：crates/agena-tui-app/src/app_terminal_integration.rs + crates/agena-bundled-plugins/src/plugins/provided/terminal.rs + crates/agena-tui-platform/src/terminal/integration.rs

#### 4.6.1 三套可配置开关（presentation_config.rs line 19-37）

- terminal_title / terminal_notifications / terminal_progress，取值 Auto / Enabled / Disabled（TerminalIntegrationMode，line 42-47）。Auto 遵循终端能力探测；Enabled/Disabled 覆盖探测结果。

#### 4.6.2 窗口/标签标题（OSC 0 / OSC 2）

- title_frames(family, title)（integration.rs line 65）：
  - 通用帧：\x1b]2;{title}\x07（OSC 2，标签/窗口标题）
  - iTerm2 / Apple Terminal 额外发 \x1b]0;{title}\x07（OSC 0，window + icon/tab title）；其他家族只发 OSC 2
  - 空标题回退产品名 agena；按显示宽度截断（MAX_TITLE_DISPLAY_WIDTH），按 UTF-8 边界回退避免切坏字符
- 标题文案 current_title_text()（app_terminal_integration.rs line 95）：会话标题 + 活动状态（working / permission / user-input / blocked）；无原生进度时状态放前面，有原生进度时状态放后面
- 变更检测 title_frames_if_changed()（line 123）：标题未变且无待发则不重发；sync_terminal_title()（line 146）写帧并记录 last_title

#### 4.6.3 注意力通知（BEL / OSC 9 / iTerm2 Dock 提醒）

- NotificationMethod（integration.rs line 78）：Bell（0x07）/ Osc9（\x1b]9;{text}\x07）/ Osc9AndItermAttention（OSC 9 + \x1b]1337;RequestAttention=yes\x07）
- 按终端家族选择（notification_method，line 97）：iTerm2 -> Osc9AndItermAttention；Windows Terminal / WezTerm / Ghostty / foot / Warp -> Osc9；Dumb / LinuxConsole -> None；其余（含 Kitty、xterm 兼容、Unknown）-> Bell
- 载荷限制（line 110-123）：中和控制字符（sanitize_osc_text），硬上限 MAX_NOTIFICATION_TEXT_BYTES，按 UTF-8 边界截断
- 通知来源优先级（drain_terminal_notification，app_terminal_integration.rs line 230）：本地排队（权限/用户输入请求、flash 错误）优先；否则消费 agena.terminal.notify 插件段的一次性生命周期通知（run completed / blocked）
- 仅在 notifications_operational() 时发送（line 40）

#### 4.6.4 OSC 9;4 原生进度条

- ProgressState（integration.rs line 147）：Clear(0) / Working(3) / Awaiting(4) / Blocked(2)，帧 \x1b]9;4;{state}\x07（progress_frames line 173）
  - 0 = 移除/隐藏；3 = 不确定（终端自己跑脉冲动画）；4 = 暂停/等待（等权限或用户输入）；2 = 错误（运行被阻断）
- 状态映射 current_progress_state()（app_terminal_integration.rs line 182）：Idle -> Clear；Running -> Working；AwaitingPermission / AwaitingUserInput -> Awaiting；Blocked -> Blocked
- 仅在能力验证支持时发送（progress_operational line 166），因为不支持 OSC 9;4 的终端可能把 OSC 9;4;* 当作 OSC 9 通知
- 变更检测 progress_frames_if_changed()（line 195）；sync_terminal_progress()（line 207）写帧并记录 last_progress

#### 4.6.5 内建插件 agena.terminal（crates/agena-bundled-plugins/src/plugins/provided/terminal.rs）

- 插件 id agena.terminal（line 25）；保留 statusline 段 id：agena.terminal.title / agena.terminal.activity / agena.terminal.notify（line 29-33）
- 段优先级：notify = i32::MAX，activity = i32::MAX - 1（line 40-42），保证 notify 意图排在 activity 之上
- TerminalNotify：Done / Blocked（line 47）；TerminalActivity：Idle / Running / Blocked（line 62）
- 插件观察会话生命周期 hooks（run.pre / run.post / agent.stop 等）发布这些段；TUI 读取 backend.plugin_statusline_segments() 重建终端状态（app_terminal_integration.rs line 61-93 effective_terminal_activity）
- 权限/用户输入等待无法通过 hook 观察：TUI 用本地 pending-interactive 状态覆盖（line 64-69）

#### 4.6.6 每帧执行顺序（app_lifecycle.rs line 234-236）

1. sync_terminal_title(self, terminal)?
2. sync_terminal_progress(self, terminal)?
3. drain_terminal_notification(self, terminal)?

状态容器 app_types.rs TerminalIntegrationState（line 307）：last_title / pending_notifications / last_progress / consumed_notify；每帧排空最多一条本地通知（burst 合并）；notify_consumed_once 保证 agena.terminal.notify 段只触发一次（line 362）。

### 4.7 显示位置 ⑥：transcript 内的 Notice / Progress 活动

文件：crates/agena-tui-transcript/src/snapshot.rs + crates/agena-tui-transcript/src/renderer/transcript_render/message_render.rs

- snapshot.rs line 573：ActivityPayload::Notice ->（kind=notice，title=Notice，summary=notice.summary）
- message_render.rs line 418：Notice 展开时渲染 Notice 分节 + detail
- message_render.rs line 401：Progress 活动 -> current/total、current、total N 的纯文本详情
- 时间线摘要 i18n：timeline-summary-system-notice-appended = 系统 #{$message_id}：{$kind}（locales/zh-CN/main.ftl line 1290）

### 4.8 所有「写通知」的 API（TUI）

文件：crates/agena-tui-app/src/app_transcript_actions.rs

| 函数 | 行号 | 作用 |
|---|---|---|
| notify(severity, text) | 259 | 设置 self.notice = UiNotice::message(...)，并按严重级排队铃响 |
| queue_notice_notification(severity) | 267 | 仅 Error / Warning / Success 触发 NotificationMethod::Bell |
| notify_failure(failure, scope) | 279 | Failure -> 通知（按 failure.id 去重，seen_failure_ids 上限 512） |
| notify_ui_failure(error, scope) | 292 | UiFailure -> notify_failure |
| flash_error(notice) | 296 | 文本 -> Error；Failure -> Global 通知 |
| flash_warning(notice) | 307 | 文本 -> Warning；Failure -> Global 通知 |
| flash_success(text) | 318 | Success 通知 |
| flash_info(text) | 322 | Info 通知 |

调用点（301 处 flash_* / notify_*，分布示例）：

- app_skill_studio.rs：技能刷新/删除/只读警告
- app_provider_runtime/*：模型页、目录刷新、认证、保存
- plugin_workbench/*：配置读写、导航、输入
- app_permissions/*：规则操作、覆盖层
- app_session_events/handlers.rs、dispatch.rs、requests.rs：用户输入回复、运行取消、权限/用户输入请求、provider studio 认证
- app_lifecycle.rs line 247：草稿存储错误
- app_command_actions.rs line 205：命令失败（Session scope）
- app_input.rs：退出确认警告
- app_plan_viewer.rs：plan 查看器错误

### 4.9 生命周期与过期

- 字段定义：app_types.rs line 406 notice: Option<UiNotice>
- 过期清理：app_lifecycle.rs line 307-313 每 tick 检查 is_expired_at，过期置 None
- UI 刷新节拍：UI_TICK_MS = 100ms（app_types.rs line 63）
- 通知去重：notify_failure 按 failure.id 去重，超过 512 清空重建

### 4.10 后台活动面板（Background Activities）

文件：crates/agena-tui/src/activities.rs（展示层）+ crates/agena-tui-app/src/app_activities.rs（应用适配）+ crates/agena-tui-app/src/view/view_overlays/view_activities.rs（渲染入口）

#### 4.10.1 打开方式

- 命令：`activities`（别名 background / tasks，commands.rs line 352-358）
- open_activities_panel()（app_activities.rs line 40）：Route::Activities(state)，立即 refresh_activities_panel()

#### 4.10.2 数据模型（crates/agena-domain/src/background_activity.rs）

- BackgroundActivityKind（line 20）：Shell（shell.run 长期进程）/ Task（tasks.create|run 委派子任务）/ Runtime（marketplace sync、catalog refresh、runtime reload 等维护任务）/ Browser（web.browser_* 交互会话）
- BackgroundActivityStatus（line 56）：pending / running / succeeded / failed / cancelled / stopped；is_active() = pending|running（line 70）
- BackgroundActivity（line 92）：id（前缀 proc_ / task_ / rtask_ / browser_）、kind、status、title、description、command、workdir、session_id、parent_session_id、created_at_ms、started_at_ms、finished_at_ms、exit_code、message、failure、last_seq、has_more、dropped_lines、cancellable、dismissible

#### 4.10.3 面板展示（crates/agena-tui/src/activities.rs）

- 布局：左列表 58% + 右详情 42%（render_activities_panel line 306-328）
- 列表：分组 Active / Finished；每行 kind 图标（⚙ ◈ ↻ ◉ •）+ 状态着色（running/pending accent，succeeded success，failed danger，cancelled/stopped warning）+ 时长（running_seconds，line 71）+ 命令
- 标题：Background Activities + filter 后缀；底栏显示 N active · M finished 与按键提示（↑↓ select、↵ detail、s stop、d dismiss、x clear、r refresh、q close，line 343-354）
- 筛选：kind（shell/task/runtime/browser）、status（running/pending/failed/succeeded）、show_finished（cycle_kind_filter line 209、cycle_status_filter line 222）
- 详情窗：日志尾（ActivitiesLogTail line 237），stderr 前缀 e>（app_activities.rs line 93-95）
- 操作：stop / dismiss / clear finished / refresh（ActivitiesControl line 23、ActivitiesEffect line 39）

#### 4.10.4 后台活动计数（composer 左下角 chip）

- composer_background_activity_part()（view_main.rs line 644）：background_activity_summary 计数 > 0 时显示 ● N background
- 数据刷新：refresh_background_activity_summary_if_due()（app_activities.rs line 17，10 秒间隔，active_only 列表请求）
- 状态字段：app_types.rs line 491 background_activity_summary: Option<(usize, Instant)>；AppMessage::BackgroundActivitySummaryLoaded（line 505）

#### 4.10.5 键盘映射（crates/agena-tui/src/keymap/activities.rs）

s stop / d dismiss / x clear finished / f toggle finished / k cycle kind / t cycle status（line 16-21）；KeyAction 定义 keymap/mod.rs line 217-222；handle_activities_key（app_activities.rs line 221）

---

## 5. Web UI 显示层（Vue 3 + TS）

### 5.1 Chat 页顶部通知横幅（瞬时提示）

文件：packages/agena-web-ui/src/agena/pages/ChatPage.vue line 580-581

- <div v-if=errorMessage class=notice>{{ errorMessage }}</div>
- <div v-else-if=localCommandNotice class=notice>{{ localCommandNotice }}</div>
- 位置：page-header 之下、ChatPageContent 之上（line 569-584），inline 流内，无 fixed 定位
- 状态定义：useChatPageState.ts line 56（errorMessage）、line 60（localCommandNotice）
- CSS：.notice（style.css line 676-682）——警告色圆角横幅（padding 12px 14px，background var(--warning-soft)，无 position）

写入方（大量）：

- useChatSessionActions.ts：errorMessage（创建/重命名/删除/排队/取消/恢复等操作失败，几十处）；localCommandNotice（Created session #... / Renamed session... / Deleted session... / Queued message... / Cleared N queued message(s)... / Cancellation requested... / Restored canonical turn...）
- useChatSessionLifecycle.ts：errorMessage、localCommandNotice（Prepared /... from runtime inspector. / Select a session before using subtree view.）
- useChatConversationRuntime.ts line 234：刷新失败 errorMessage = userErrorMessage(err)
- useChatPageUiState.ts：localCommandNotice（Forgot memory ... 等操作结果）
- ChatPage.vue line 84-138：附件上限、Skill 附加提示

错误文本来源：lib/api.ts line 167-174 userErrorMessage —— 只透出 ApiError.message（服务端 problem.user.fallback），internal/data_corruption 类加 Reference: {problem.id}；其余回退固定文案。

### 5.2 唯一的 fixed toast：RuntimeOverviewPanel.vue

文件：packages/agena-web-ui/src/agena/pages/RuntimeOverviewPanel.vue

- 状态：toasts ref、pushToast(kind, message, ttlMs=4000)、removeToast（line 53-89）；error 用 7000ms
- 模板：<teleport to=body> .toast-stack（line 641-648），每条 .toast.toast-{kind} + toast-close
- CSS：position fixed; top 16px; right 16px; z-index 1000（line 651-715）；toast-info / toast-success / toast-error 边框色
- 用途：后台任务、重载、模型目录刷新等操作反馈，不用于 Chat 页

### 5.3 Chat composer（输入框）周边

文件：packages/agena-web-ui/src/agena/pages/ChatComposerPanel.vue line 127-255

- Prompt 文本域（line 135-147）：placeholder Ask agena to inspect the repo...
- 隐藏文件输入（line 148-156）：#composer-file-input（任意文件）、#composer-image-input（image/*）
- 附件 chip（line 157-171）：name + kind · size，Remove 按钮（发送中/读取中禁用）
- Skill chip（line 172-189）：badge Skill + name + 描述 + source · contentHash 前 12 位
- 待发队列（line 190-209）：Pending Messages + N queued 徽章；每项 composerQueuePreview(item)、N file(s)、N Skill(s)；Edit First / Clear Queue
- Slash 命令菜单（line 210-242）：#composer-slash-candidates，item.slash 徽章 + 标题 + 描述 + category + usage；空态 No slash commands matched.
- 操作按钮（line 243-254）：Attach File / Attach Image / Attach Skill；主按钮文案随状态：Sending... / Reading files... / Send Prompt
- 注意：composer 本身不显示所选模型名；模型名在 ChatActiveSessionPanel 的 executionFacts 与 ChatRunOptionsPanel 下拉框

数据模型：

- chatAttachmentModel.ts：MAX_COMPOSER_ATTACHMENT_BYTES=50MB、MAX_COMPOSER_ATTACHMENTS=8、MAX_COMPOSER_ATTACHMENT_TOTAL_BYTES=64MB（line 3-5）；formatComposerAttachmentSize（line 77）
- chatSkillModel.ts：SKILL_PICKER_PAGE_SIZE=12、MAX_COMPOSER_SKILLS=8（line 3-4）；ComposerSkillDraft 含 contentHash/source
- chatQueueModel.ts：ComposerQueueItem（line 4-10）、createComposerQueueItem（line 12-25）、composerQueuePreview(item, maxLength=80)（line 27-35）
- useChatCommandState.ts：slashQuery（line 61-68）、buildSlashSuggestions(items, composer, limit=10)（line 96-115）；来源 createChatCommandCatalog + runtime skills/commands + plugin studio commands（line 137-180）

### 5.4 Active Session / Run Options（模型、进度、用量）

ChatActiveSessionPanel.vue：

- executionFacts（line 39-41）：agent=... · access=... · task=... · model=provider/adapter/id · think=... · speed=...
- 生成：useChatDerivedState.ts line 161-207 buildExecutionFacts —— 优先执行上下文，回退 run-options 模型栈（formatSessionExecutionModelLabel，agenaApi.ts line 1164）
- workflow=... , execution=...phase（line 49-51）
- contextUsageLabel（line 116）：context N% used 或 context Nk used（useChatDerivedState.ts line 96-105）
- goal / ancestors / siblings / children / automation / session_usage（line 53-158）

ChatRunOptionsPanel.vue（模型选择）：

- Provider / Adapter / Model / Thinking / Speed / Verbosity / Parallel / Temperature / MaxOutput / System 下拉（line 41-114+）
- v-model 双向同步到 useChatPageState.ts line 43-45 等 refs
- 选项来源：providerApi.ts listProviders（GET /api/v1/providers）、listProviderModels（GET /api/v1/providers/{id}/models）、listProviderAdapterModels（POST /api/v1/providers/models）

### 5.5 Messages / Timeline 面板

ChatMessagesPanel.vue line 52-121：

- 消息头部：role / Inspect / Rewind Here（user）/ formatMessageTime
- usage 行：usage={{ messageUsageFacts(message).join(' · ') }}（line 77-81）
- 渲染块按 kind 分派：operation_outcome（Blocked/Not run，line 85-95）、diff（line 96-99）、input_activity（Skill/Attachment/Input，line 100-107）、terminal（line 108-112）、markdown（line 113-116）

渲染模型 chatRenderModel.ts：

- RenderBlock kind：markdown | terminal | diff | input_activity | operation_outcome（line 11-19）
- transcriptMessages（line 161-194）：turns -> MessageResource[]（user turn:{id}:input + assistant reply:{id}）
- canonicalPart（line 60-125）、canonicalMessage（line 127-154）
- partBlocks（line 256-378）：policy_denied / user_declined / capability_unavailable / tool_unavailable -> operation_outcome；skill_reference / attachment -> input_activity；apply_patch -> diff；operationRenderBlocks（line 380-478）：text/markdown、log（terminal）、diff、command（$ cmd + stdout/stderr + cwd ... · exit N）、file_changes、checklist、json、table
- messageUsageFacts（line 502-524）：提取 in/out/reasoning/cost
- markdown.ts：renderMarkdown（line 91-199，自研 escape 后渲染）、renderDiff（line 45-63）、renderTerminal（line 65-67）

ChatTimelinePanel.vue line 38-67：

- 每条事件：event.kind 加粗 + summaryFor(event)（取 payload.summary/command/message，缺省 kind）+ message_id/part_id + Jump to Message + Inspect Activity + seq/session/时间

### 5.6 Usage 面板

ChatUsagePanel.vue line 20-75：

- 头部 sessionUsageHeadline：N requests · M visible tokens · $X（useChatDerivedState.ts line 92-95）
- 汇总 sessionUsageSummaryFacts
- 指标网格：Provider Requests / Total Cost / Input Tokens / Output Tokens / Reasoning Tokens / Cache Read / Cache Write
- 按模型明细 sessionUsageModelLines：provider/model + chatUsageBreakdownFacts

数据来源：/api/v1/usage + session state 的 usage（current_tokens / projected_tokens / model_context_window_tokens）

### 5.7 Sidebar / App 外壳

ChatSidebarPanel.vue：Workspace 路径 + Resolve or Create / Create Only；Workspaces 列表（session_count）；Sessions 搜索 + 视图模式（all / roots / subtree）+ 新建会话。

App.vue：

- booting 屏（Starting runtime，全屏居中，style.css .boot-screen 72-77）
- Backend unavailable 屏（health.error 或默认文案 + Retry 按钮）
- 登录页 LoginPage（auth.needsLogin；LoginPage.vue line 41 也有 .notice 错误横幅）
- 侧边栏品牌区：Gen {generation} · mode {activeModeLabel}（App.vue line 266）
- 侧边栏 meta：Workspace Root（health.data.workspaceRoot）、Config（health.data.configPath）（line 294-299）
- 全局命令面板 commandPalette（line 307-340）：搜索 commands/pages/skills/runtime actions；Ctrl/Cmd+Shift+P 打开
- 数据：fetchStudioHealth（GET /api/v1/health）、fetchRuntimeStatus（GET /api/v1/runtime）、auth 刷新

### 5.8 其他 .notice 使用点

- LoginPage.vue line 41：auth.lastError
- SectionPageShell.vue line 38-39：actionError / actionMessage（Runtime 各 section 页）
- UsagePage.vue line 109、ActivitiesPage.vue line 212：各自页面错误

### 5.9 实时数据流（SSE）

文件：packages/agena-web-ui/src/agena/lib/agenaApi.ts + sse.ts + useChatConversationRuntime.ts + chatPageModel.ts

- streamSessionEvents(sessionId, options)（agenaApi.ts line 2226-2367）：SSE GET /api/v1/sessions/{id}/events/stream；事件名 session_event / descendant_session_event / lagged / error；after_seq + poll_interval_ms 参数；自动重连（250ms/1s）
- 消费端 useChatConversationRuntime.syncEventStream（line 124-176）：onEvent -> applyChatSessionEvent -> applySessionEvent（chatPageModel.ts line 524-595）
- applySessionEvent 处理的事件种类（chatPageModel.ts line 534-594）：
  - message_part_checkpointed / transcript_part_upserted -> upsertLiveMessagePart（line 144-227）
  - command_begin / command_output_delta / command_end -> applyLiveCommandEvent（line 305-444）生成/更新 Running shell command 操作块（__live_command_seq/bytes/prefixes 记录流式 stdout/stderr；结束时 Command exited with code N.）
  - user_message_appended / assistant_message_finished -> 整段刷新（line 560-579）
  - execution_started / execution_finished / run_started / run_completed / run_aborted -> patchSessionStateFromEvent（line 469-522）更新 active_execution
  - 默认：追加 timeline 事件并请求刷新（line 586-593）
- 轮询兜底：ensurePolling 每 1800ms refreshConversation（workflow_state=blocked 或有 active_execution 时启用）
- 整段刷新 refreshConversation（line 178-246）：getSessionState + listSessionTimeline（limit=100）+ 叠加 liveCommandEvents
- streamNotifications（agenaApi.ts line 2369-2512）：SSE GET /api/v1/events/stream；事件名 notification；EventNotification kind：event / lagged / resumed / subscription_closed；scope_kind/since_seq_global/workspace_id/session_id/kinds 参数
- streamPluginToolRegistryChanges（line 2514-2539）：包装 streamNotifications，kinds=['plugin_tool_registry_changed']
- sse.ts：normalizeSseBuffer + parseSseEventBlock（event/id/data 字段解析）

### 5.10 Background Activities 页面（Web）

文件：packages/agena-web-ui/src/agena/pages/ActivitiesPage.vue

- 入口：侧边栏 Activities 导航（App.vue line 280-282，/activities 路由）
- 头部：页面标题 + N active · M finished 摘要 + Refresh / Clear Finished 按钮（line 190-210）
- 错误横幅：.notice（line 212，error ref，写入方 userErrorMessage）
- 筛选区：Kind（All/Shell/Task/Runtime/Browser）、Status（All/Running/Pending/Succeeded/Failed/Cancelled/Stopped）、Active only 复选框（line 214-237）
- 列表（line 239-288）：每行 kind 图标（⚙ ◈ ↻ ◉ •）+ title + description + command + status badge（statusClass 着色）+ 时长（durationLabel，<60s 显示 Ns，否则 Nm Ns）+ Stop/Dismiss 按钮
- 详情展开（line 277-284）：message、exit code、日志尾（pre.log-tail，limit 300 行）
- 轮询：每 4 秒 loadActivities + loadLogs（line 176-182）
- API：fetchActivities（GET /api/v1/activities?kinds&statuses&session_id&active_only，agenaApi.ts line 1312-1327）、fetchActivityLogs（GET /activities/{id}/logs?since_seq&limit&wait_ms，line 1329-1341）、stopActivity（POST /stop，line 1343）、dismissActivity（POST /dismiss，line 1349）、clearFinishedActivities（POST /clear-finished，line 1355）

### 5.11 Runtime 概览页（Web，RuntimeOverviewPanel.vue）

文件：packages/agena-web-ui/src/agena/pages/RuntimeOverviewPanel.vue + useRuntimeDerivedState.ts + runtimePageModel.ts

- Operator Cards（line 332-337）：Generation / Tool Registry / Providers / Plugins / Agent / MCP Servers / LSP Servers / Skills（buildOperatorCards，runtimePageModel.ts line 39-51）
- Runtime Snapshot（line 340-349）：Generation / Loaded At / Workspace Root / Config Path / Config Found / Auth Store / Tool Registry Generation / Tool Registry Last Event / Providers / Session Runtime / Automation / Scheduled Jobs（buildRuntimeSnapshotFacts line 53-72）
- Runtime Tasks（line 351-371）：Reload（enabled/interval）、Session GC、Watch Paths
- Recent Automation（line 374-396）：ScheduledJobResource 列表（kind/id/session/status/triggered/next/expression、last_run.failure.user.fallback）
- Background Tasks（line 398-432）：RuntimeBackgroundTask 列表（title/id/origin/started/finished/message + status badge + Cancel；taskFailureMessage 取 failure.user.fallback，internal 类加 Reference）
- Session Cache（line 457-465）：Entries / Total Bytes / Max Bytes / Hits / Misses / Inserts / Evictions / TTL / Max Sessions（buildSessionCacheFacts line 75-87）
- Model Catalog（line 468-497）：Last Source / Last Refresh / last_failure.user.fallback + Refreshing badge + Refresh Catalog 按钮
- Catalog Entries（line 499-637）：总数/显示数 + 搜索 + 分页
- toast 反馈：pushToast（line 84-90）在任务成功/失败/取消时触发（line 254-282），默认 4s、error 7s
- 数据来源：GET /api/v1/runtime（RuntimeStatus 类型见 agenaApi.ts line 59-122；background_tasks 字段 line 91；automation line 92）

---

## 6. HTTP/SSE API 端点（crates/agena-api-server/src/lib.rs）

| 端点 | 方法 | Handler | 用途 |
|---|---|---|---|
| /healthz /readyz /metrics | GET | rest::healthz/readyz/metrics | 存活/就绪/指标 |
| /api/v1/health | GET | rest::health | 健康与运行时元数据（Web App 启动） |
| /api/v1/runtime | GET | rest::get_runtime_status | 运行时状态（providers/skills/ui catalog） |
| /api/v1/usage | GET | rest::get_usage_stats | 用量统计（Usage 页） |
| /api/v1/runtime/reload | POST | rest::reload_runtime | 重载运行时（toast 反馈） |
| /api/v1/runtime/tasks | GET | list_runtime_background_tasks | 后台任务列表 |
| /api/v1/runtime/tasks/{id}/cancel | POST | cancel_runtime_background_task | 取消后台任务 |
| /api/v1/settings* | GET/PUT/PATCH/DELETE | rest::get/set/patch/delete_settings | 配置（Settings 页） |
| /api/v1/model-catalog | GET | get_model_catalog | 模型目录 |
| /api/v1/model-catalog/lookup | POST | lookup_model_catalog | 模型查找 |
| /api/v1/model-catalog/refresh | POST | refresh_model_catalog | 刷新目录（toast 反馈） |
| /api/v1/plugins | GET | list_plugins | 插件列表 |
| /api/v1/plugins/ui | GET | get_plugin_ui_catalog | 插件 UI 目录（statusline/命令） |
| /api/v1/plugins/tools/changes | GET | list_plugin_tool_registry_changes | 插件工具注册变更 |
| /api/v1/plugins/ui/invoke-tool | POST | invoke_plugin_ui_tool | 调用插件 UI 工具 |
| /api/v1/plugins/{id}/ui/actions/{action_id} | POST | run_plugin_ui_action | 运行插件 UI 动作 |
| /api/v1/sessions | GET/POST | list_sessions/create_session | 会话列表/创建 |
| /api/v1/sessions/{id} | GET/PUT/DELETE | get/replace/delete_session | 会话 CRUD |
| /api/v1/sessions/{id}/state | GET | get_session_state | 会话执行状态（Chat 主数据） |
| /api/v1/sessions/{id}/events | GET | list_session_events | 会话时间线（REST 预载） |
| /api/v1/sessions/{id}/events/stream | GET(SSE) | stream_session_events | 会话实时事件流 |
| /api/v1/sessions/{id}/messages | POST | submit_message | 提交消息 |
| /api/v1/sessions/{id}/continue | POST | continue_run | 继续运行 |
| /api/v1/sessions/{id}/compact | POST | compact_session | 压缩会话 |
| /api/v1/sessions/{id}/fork | POST | fork_session | 派生会话 |
| /api/v1/sessions/{id}/cancel | POST | cancel_run | 取消运行 |
| /api/v1/sessions/{id}/permission-replies | POST | reply_permission | 权限回复 |
| /api/v1/sessions/{id}/user-input-replies | POST | reply_user_input | 用户输入回复 |
| /api/v1/sessions/{id}/rewind | POST | rewind_session | 回退 |
| /api/v1/sessions/{id}/export | GET | export_session | 导出 |
| /api/v1/sessions/import | POST | import_session | 导入 |
| /api/v1/sessions/tree/{root_id} | GET | list_session_tree | 会话树 |
| /api/v1/events | GET | list_events | 全局事件列表 |
| /api/v1/events/stream | GET(SSE) | sse::handler | 全局通知流（notification 事件） |
| /api/v1/activities | GET | list_activities | 后台活动列表（kinds/statuses/session_id/active_only 过滤） |
| /api/v1/activities/{id}/logs | GET | get_activity_logs | 活动日志尾（since_seq/limit/wait_ms） |
| /api/v1/activities/{id}/stop | POST | stop_activity | 停止后台活动 |
| /api/v1/activities/{id}/dismiss | POST | dismiss_activity | 忽略后台活动 |
| /api/v1/activities/clear-finished | POST | clear_finished_activities | 清空已完成活动 |
| /api/v1/permission-rules* | GET/POST/PUT/DELETE | rest::permission_rules | 权限规则 |
| /api/v1/memories* | GET/PUT/DELETE | rest::memories | 记忆 |
| /api/v1/git* / vcs* | GET/POST | rest::git/vcs | Git 状态/提交/PR |
| /api/v1/auth/providers* | GET/POST/PUT/DELETE | rest::auth | 认证 |
| /api/v1/workspaces* | GET/POST/PUT/DELETE | rest::workspaces | 工作区 |
| /api/v1/ws | GET(WS) | ws::handler | WebSocket（feature ws） |
| /plugin-rpc/{plugin_id} | POST | rest::plugin_rpc | 插件 RPC |

---

## 7. 汇总表：显示位置 -> 内容 -> 代码路径 -> API/事件

| 显示位置 | 内容 | 代码路径 | API / 数据来源 |
|---|---|---|---|
| TUI 转录 footer 行（composer 上方） | 最近一条通知（5s 过期） | view_main.rs 477-493 -> app_transcript_actions.rs 259-324 | UiNotice；失败来自 agena_failure |
| TUI composer 左上角 chip | 模型 / thinking / speed / token % | view_main.rs 584-641 -> app_status_context.rs 214-292 | current_session_model_ref、session_status.rs |
| TUI composer 右上角 chip | 历史搜索进度 / 待审批数 | view_main.rs 651-685 | prompt_history_search、pending_interactive_counts |
| TUI composer 左下角 chip | 后台活动数 | view_main.rs 644-648 | background_activity_summary |
| TUI composer 右下角 chip | plan 进度 | view_main.rs 689-698 | 插件 statusline plan:{session_id} |
| TUI 转录头部 right | spinner / awaiting / blocked | view_main.rs 346-351 -> app_status_context.rs 4-16 | current_session_activity() |
| TUI 转录 footer（非通知时） | 队列预览 / status_line / 插件段 / 插件块 | view_main.rs 501-555 | plugin_statusline_segments、plugin_tui_content_blocks |
| TUI 窗口标题 / 铃响 / OSC 9;4 | 会话活动状态 | app_terminal_integration.rs | agena.terminal.* 插件段 |
| TUI transcript 内 Notice 活动 | 系统 #id：kind + 详情 | snapshot.rs 573、message_render.rs 418 | ActivityPayload::Notice |
| TUI transcript 内 Progress 活动 | current/total | message_render.rs 401 | ActivityPayload::Progress |
| TUI 后台活动面板（activities 命令） | Shell/Task/Runtime/Browser 列表 + 详情日志 + stop/dismiss/clear | activities.rs 306-410、app_activities.rs 40-390 | list_activities / activity_logs / stop / dismiss / clear_finished |
| TUI composer 左下角 chip | ● N background 后台活动计数 | view_main.rs 644-648、app_activities.rs 17-38 | list_activities(active_only) 10s 轮询 |
| TUI 窗口标题 OSC 0/2 + 铃响/OSC 9 + OSC 9;4 进度条 | 会话活动状态 → 终端 chrome | app_terminal_integration.rs 95-249、integration.rs 65-175 | agena.terminal.* 插件段 + 本地 pending-interactive |
| Web ChatPage 顶部横幅 | 错误 / 操作结果 .notice | ChatPage.vue 580-581 | errorMessage / localCommandNotice |
| Web RuntimeOverview 右上角 | toast（info/success/error，4s） | RuntimeOverviewPanel.vue 53-89,641-715 | 本地状态 |
| Web Background Activities 页 | 活动列表 + 日志 + 筛选 + Stop/Dismiss/Clear | ActivitiesPage.vue 190-288 | fetchActivities / fetchActivityLogs / stop / dismiss / clear（4s 轮询） |
| Web Runtime 概览页 | Operator Cards / Snapshot / Tasks / Automation / Background Tasks / Session Cache / Model Catalog | RuntimeOverviewPanel.vue 330-637 | GET /api/v1/runtime |
| Web composer | 附件 / Skill / 队列 / slash / 发送状态 | ChatComposerPanel.vue 127-255 | 本地状态 + chatQueueModel |
| Web ActiveSession | agent/model/access/task/context%/workflow | ChatActiveSessionPanel.vue 39-131 | getSessionState（/state） |
| Web RunOptions | Provider/Adapter/Model/模式下拉 | ChatRunOptionsPanel.vue 41-114 | listProviders / listProviderModels |
| Web Messages/Timeline | 消息块 + 事件流 | ChatMessagesPanel.vue、ChatTimelinePanel.vue | getSessionState、listSessionTimeline、SSE streamSessionEvents |
| Web Usage | 请求/成本/token/按模型 | ChatUsagePanel.vue | /api/v1/usage + session state usage |
| Web App 侧边栏 | generation / mode / workspace / config | App.vue 260-300 | fetchStudioHealth、fetchRuntimeStatus |

---

## 8. 关键事件 / 配置词汇表

### 8.1 SSE 事件名（会话流）

- session_event、descendant_session_event、lagged、error
- 内部 DomainEventRecord.kind 示例：message_part_checkpointed、transcript_part_upserted、command_begin、command_output_delta、command_end、user_message_appended、assistant_message_finished、execution_started、execution_finished、run_started、run_completed、run_aborted、system_notice_appended、session_goal_updated、tool_call_issued、tool_call_completed、plugin_event

### 8.2 SSE 事件名（全局通知流）

- notification（EventNotification.kind：event / lagged / resumed / subscription_closed）

### 8.3 TUI 配置项（presentation_config.rs）

- TuiConfig.status_line.command + refresh_interval_ms
- TuiConfig.terminal_title / terminal_notifications / terminal_progress（Auto/Enabled/Disabled）
- TuiConfig.transcript.activity_default_expanded

### 8.4 内建插件保留 statusline 段

- agena.terminal.title / agena.terminal.activity / agena.terminal.notify（agena.terminal 插件）
- plan:{session_id}（workflow 插件，priority 120，workflow_plan.rs line 565-597）

### 8.5 后台活动模型（crates/agena-domain/src/background_activity.rs）

- Kind：shell / task / runtime / browser（line 20-48）
- Status：pending / running / succeeded / failed / cancelled / stopped（line 56-88）
- id 前缀：proc_（shell）/ task_（委派）/ rtask_（runtime）/ browser_（line 94）
- RuntimeBackgroundTask（Web，agenaApi.ts line 136-148）：kind = model_catalog_refresh / runtime_reload / marketplace_registry_sync / marketplace_plugin_install / marketplace_plugin_uninstall / marketplace_plugin_upgrade；origin = system / user；status = running / succeeded / failed / cancelled

### 8.6 Web 常量

- MAX_COMPOSER_ATTACHMENT_BYTES=50MB、MAX_COMPOSER_ATTACHMENTS=8、MAX_COMPOSER_ATTACHMENT_TOTAL_BYTES=64MB
- MAX_COMPOSER_SKILLS=8、SKILL_PICKER_PAGE_SIZE=12
- 轮询间隔：Chat 1800ms；Activities 4000ms；后台活动计数 10s；SSE 重连 250ms/1s；toast 默认 4s（error 7s）

---

## 9. 备注与后续建议

1. Web 端 streamNotifications 目前只被 streamPluginToolRegistryChanges 复用；没有独立的全局通知 UI 消费它——若要做全局通知中心，这是现成接入点。
2. TUI 的 UiNotice 是「最近一条」模型；若需要多条通知队列，需要在 app_types 增加通知列表并在 footer/overlay 渲染。
3. NoticePart 目前只有 max_turns_exhausted 一个 kind；插件可通过活动管线新增自定义通知（CustomActivity）或扩展 Notice 类别。
