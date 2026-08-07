# Agena 通知与显示系统彻底重构计划（Unified Notification & Display Architecture）

> 分支: docs/notification-display-map
> 基线: 4eaebcfd (master)
> 配套文档: docs/notification-and-status-display.md（现状全量盘点）
> 本文目标: 基于「谁发出通知/显示、用户如何与之交互」的完整分析，设计一套整体统一、高复用、插件无侵入的 Rust 架构，并给出可执行的分阶段迁移路径。

---

## ✅ 实施状态（Phase 0–7 全部落地）

> 分支 `feat/notification-unified`（worktree `.agena/worktrees/notification-unified`），基于 master `889cb3e8`。

| Phase | 内容 | 状态 |
|---|---|---|
| 0 | 独立 worktree + 分支 + 源码基线确认 | ✅ |
| 1 | `agena-notification` 领域 crate（Notification/Kind/Surface/Control/ActionTarget + 纯逻辑） | ✅ |
| 2 | `agena-runtime-notifications` 聚合器 + InMemory store + application 层接线 | ✅ |
| 3 | `agena-api` / `agena-api-server`：notifications REST + `/notifications/stream` SSE | ✅ |
| 4 | Web 迁移：`useNotifications` + toaster/banner，删除 errorMessage/localCommandNotice 直写 | ✅ |
| 5 | TUI 迁移：NotificationStore 替代 `Option<UiNotice>`，flash_* 走统一 sink | ✅ |
| 6 | 插件契约收敛：`PluginDisplayContribution` 声明式 + `host.notify` | ✅ |
| 7 | 删除旧通道（UiNotice / errorMessage 直写 / `tui.content_blocks` location 通道）；全量回归 | ✅ |

落地要点：

- 统一通知模型：`agena_notification::model::Notification`（Kind 16 / Surface 16 / Control 3 / ActionTarget 4）。
- 聚合入口：application 层 `spawn_notification_aggregator` 订阅 runtime 事件流，把 `message_part_checkpointed`（NoticePart）与 `background_activity_changed`（BackgroundActivity）投影进 `InMemoryNotificationStore`。
- 消费端：Web `useNotifications`（REST + SSE `/api/v1/notifications/stream`）；TUI 通过 `Application.notifications()` 读同一 store。
- 插件：manifest 声明 `ui.display: Vec<PluginDisplayContribution>`（无 location/color）；运行时一次性通知走 `host.notify(PluginNotifyRequest)`（severity + actions）。
- 已删除：`ui_statusline_contribute` / `statusline_segments` 旧命令式桥接层；插件动态显示状态统一走 `host.display_contribute` / `display_remove`（`PluginDisplayContribution`，无 location/color，host 决定放置）。

---

## 0. 摘要（目标与原则）

本次重构把 Agena 里所有「通知 / 状态 / 进度 / 提示 / 后台活动 / 插件贡献的显示」收拢为**一个领域模型（Notification）+ 一套服务 API（REST/SSE + trait）+ 每个前端只有一层薄渲染适配**。

核心原则：

1. **单一事实源**：任何显示意图都以 `Notification` 领域对象进入统一总线；TUI / Web / 终端 chrome 只消费同一个模型。
2. **声明式插件，禁止命令式侵入**：插件只能通过 manifest 声明贡献、或调用统一 host trait 发送「通知意图」；不得直接操作 statusline / toast / 页面 DOM。
3. **前后端对称**：TUI 与 Web 消费同一组 API（REST 查询 + SSE 推送），不再各自实现一套 emit/render。
4. **领域层纯逻辑，适配层薄**：severity/scope/action/过期/去重/排序全部在 domain crate 内可单测；TUI/Web 只做渲染。
5. **类型安全**：kind / severity / scope / action 用 Rust enum + serde 契约，杜绝 Stringly-typed 散落。
6. **可追溯、可持久化**：通知有 id、时间戳、来源、因果链；可查询历史、可恢复、可订阅。

---

## 1. 现状盘点：谁发出、谁显示、谁交互

### 1.1 发出者（Producer）清单

| 发出者 | 例子 | 现状通道 | 代码位置 |
|---|---|---|---|
| 运行时核心 | max_turns_exhausted、执行状态事件 | NoticePart / RuntimeEvent / ActivityPayload | crates/agena-runtime-session、agena-runtime-contracts、agena-domain |
| 会话/应用层 | 权限请求、用户输入请求、execution 状态 | PermissionRequest / UserInputRequest / SessionExecutionResource | agena-runtime-session、agena-application |
| 后台任务 | shell 进程、委派任务、marketplace sync、browser 会话 | BackgroundActivity | agena-domain/src/background_activity.rs |
| TUI 应用层 | flash_*、notify、状态行、搜索反馈 | UiNotice / StatusLine / composer chips | agena-tui-app/src/app_transcript_actions.rs、view_main.rs |
| 终端集成 | 窗口标题、铃响、OSC 9;4 进度 | OSC 帧 + agena.terminal 插件段 | agena-tui-app/app_terminal_integration.rs、agena-bundled-plugins/terminal.rs |
| Web 前端本地 | 操作成功/失败、队列反馈、命令结果 | errorMessage / localCommandNotice / toast | packages/agena-web-ui/src/agena/pages/* |
| 插件（命令式） | ui_statusline_contribute、tui content blocks、studio 命令/控件/视图 | HostStatuslineSegment / PluginTuiContentBlock / PluginStudio* | agena-plugin-sdk/src/host_api.rs、manifest.rs |

### 1.2 显示者（Surface / Renderer）清单

| Surface | 承载内容 | 代码位置 |
|---|---|---|
| TUI 转录 footer 行 | UiNotice 横幅 / 队列预览 / 插件段 | view_main.rs transcript_footer_spec / transcript_footer_text |
| TUI composer 四角 chip | 模型 / think / speed / token% / 历史搜索 / 待审批 / 后台计数 / plan 进度 | view_main.rs composer_chip_texts |
| TUI 转录头部 right | spinner / awaiting / blocked | view_main.rs transcript_surface_top_right |
| TUI 后台活动面板 | Shell/Task/Runtime/Browser 列表 + 日志 | agena-tui/src/activities.rs |
| TUI 覆盖层对话框 | 权限 / 用户输入 / 选择器 | view_overlays/overlay_core.rs |
| 终端 chrome | OSC 0/2 标题、OSC 9 通知、OSC 9;4 进度 | agena-tui-platform/src/terminal/integration.rs |
| Web ChatPage 顶部 | errorMessage / localCommandNotice 横幅 | ChatPage.vue 580-581 |
| Web RuntimeOverview | toast（右上 fixed）+ 后台任务 + 自动化 + 目录状态 | RuntimeOverviewPanel.vue |
| Web Activities 页 | 后台活动列表 + 日志 + 操作 | ActivitiesPage.vue |
| Web Messages/Timeline | 消息块 + 活动流 | ChatMessagesPanel.vue、ChatTimelinePanel.vue |

### 1.3 用户交互（Interaction）清单

| 交互 | 现状实现 | 归属通知类型 |
|---|---|---|
| 查看/读取通知摘要 | TUI footer 横幅；Web .notice 横幅 | notice.summary |
| 展开详情 | TUI 活动折叠；Web details/message inspector | notice.detail |
| 执行恢复动作 | UiNotice.recovery_action -> 命令；Web userErrorMessage 无动作 | action（Refresh / Sign in / Open settings / Permissions / Retry / …） |
| 停止/忽略/清空后台活动 | TUI s/d/x；Web Stop/Dismiss/Clear Finished | activity action |
| 审批/拒绝权限 | TUI permission overlay；Web ChatPendingPermissionsPanel | interaction action |
| 回复用户输入 | TUI user_input overlay；Web ChatPendingUserInputPanel | interaction action |
| 跳转消息 / 检查活动 | Web Jump to Message / Inspect Activity；TUI 折叠导航 | navigation action |
| 复制/导出 | TUI copy/export；Web 复制 usage | utility action |
| 取消运行 / 继续 | TUI / Web cancel & continue | run action |
| 命令面板 / 模型切换 | TUI / Web command palette、model chooser | command action |
| 关闭 toast | Web RuntimeOverview toast-close | dismiss action |

### 1.4 现状痛点（为什么必须重构）

1. **通道碎片化**：UiNotice、errorMessage/localCommandNotice、toast、terminal notification 是四套互不相通的实现，同一语义（错误/成功/警告）在不同端重复编码。
2. **插件侵入式显示**：插件可以直接贡献 statusline 段、TUI 内容块、studio 命令/控件/视图，还通过 `agena.terminal.notify` 触发终端通知——渲染细节泄漏进插件，宿主无法统一管控、去重、排序、权限。
3. **无统一 API**：emit 端各自为政（TUI 直接写 self.notice；Web 直接写 ref）；没有统一 REST/SSE 通知接口，Web 只能靠轮询 + 三条 SSE 通道（session events、global notifications、plugin registry）。
4. **无历史与持久化**：UiNotice 只有「最近一条」，过期即丢；无法查询「刚才发生了什么」。
5. **重复代码**：severity 配色、scope 定位、action 映射在 TUI/Web 重复；插件 manifest 三套 UI 贡献结构互不共享。
6. **无订阅抽象**：TUI 走轮询（100ms tick、250ms refresh、10s 后台计数），Web 走 SSE + 1.8s 轮询兜底；同一运行时没有单一推送通道。
7. **测试困难**：显示逻辑与 UI 框架耦合，难以做纯逻辑单测。

---

## 2. 目标架构总览

```
                    ┌────────────────────────────────────────────┐
                    │            agena-notification              │
                    │   (domain: Notification + trait + 纯逻辑)   │
                    └────────────────────────────────────────────┘
                          ▲                    │
         emit / subscribe │                    │ 聚合/持久化/去重/过期
                          │                    ▼
        ┌─────────────────┴────────────────────────────┐
        │         agena-runtime-notifications          │
        │   (RuntimeEvent/Activity/Plugin -> Notification)
        └─────────────────┬────────────────────────────┘
                          │ NotificationService trait
        ┌─────────────────┴────────────────────────────┐
        │        agena-api-server (REST + SSE)          │
        │   /api/v1/notifications*  /notifications/stream │
        └──────────┬─────────────────────┬─────────────┘
                   │                     │
        ┌──────────▼───────┐   ┌─────────▼──────────┐
        │  TUI adapter     │   │  Web adapter       │
        │  (renderer+reducer│   │  (Vue store/组件)  │
        └──────────────────┘   └────────────────────┘
                   │                     │
        ┌──────────▼───────┐   ┌─────────▼──────────┐
        │ terminal chrome  │   │ toast/banner/页面   │
        └──────────────────┘   └────────────────────┘
```

### 2.1 统一领域模型（crates/agena-notification）

```rust
//! 所有通知/显示意图的唯一领域对象。

/// 严重级别（替代 TUI NoticeSeverity 与 Web toast kind 的重复定义）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity { Info, Success, Warning, Error }

/// 作用域（替代 TUI NoticeScope 与 Web scope_kind 的重复定义）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationScope {
    Global,
    Session(i64),
    Workspace(i64),
    ToolCall(String),
    Provider(String),
    Plugin(String),
    BackgroundTask(String),
}

/// 机器可读类别（枚举 + 自定义）。全系统收敛为 16 种，见文档附录 A。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationKind {
    Notice { code: String },
    Progress { current: Option<u64>, total: Option<u64> },
    Status { state: NotificationState },
    ModelStatus { model: String, thinking: Option<String>, speed: Option<String> },
    PlanProgress { current: u64, total: u64 },
    RunState { state: RunNotificationState },
    CommandExecution { command: String, stream: Option<String>, exit_code: Option<i32> },
    ToolCall { call_id: String, name: String },
    BackgroundActivity { activity_id: String },
    PermissionRequest { request_id: String },
    UserInputRequest { request_id: String },
    HistorySearch { query: String, current: u64, total: u64 },
    TerminalTitle { title: String },
    TerminalNotify { text: String },
    UsageUpdate { current_tokens: u64, projected_tokens: Option<u64>, context_window: Option<u32> },
    Custom(CustomNotification),
}

/// 物理渲染位置（Surface）：宿主根据 kind/scope 决定，前端只按 surface 渲染。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSurface {
    Banner,           // 顶部横幅（Web .notice / TUI 顶栏）
    Toast,            // 浮动 toast（Web 右上 / TUI 覆盖层）
    ComposerChip,     // 输入区四角 chip（状态/搜索/后台/plan）
    ComposerFooter,   // 输入框上方 footer 行（TUI）
    StatusLine,       // 状态行（TUI 底部 / Web 面板状态段）
    TerminalTitle,    // 终端窗口标题（OSC 0/2）
    TerminalProgress, // 终端进度（OSC 9;4 / 任务栏）
    TerminalBell,     // 终端铃响 / 系统通知
    ActivitiesPanel,  // 后台活动面板
    HistorySearch,    // 历史搜索浮动条
    PermissionDialog, // 权限请求对话框
    InputPrompt,      // 用户输入请求对话框
    Settings,         // 设置面板
    PlanPanel,        // 计划/进度面板
    BackgroundTask,   // 后台任务面板
    Log,              // 仅记录（活动日志流）
}

/// 状态类通知的取值
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationState {
    Idle,
    Running,
    Awaiting,
    Blocked,
    Finished,
    Failed,
    Cancelled,
}

/// Run 状态类通知的取值（plan / workflow 执行）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationState {
    Queued,
    Running,
    Paused,
    AwaitingInput,
    Blocked,
    Finished,
    Failed,
    Cancelled,
}

/// 插件自定义通知载荷（kind = Custom）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CustomNotification {
    pub plugin_id: String,
    pub code: String,
    pub data: Option<serde_json::Value>,
}

/// 通知自身的操作（对通知对象的操作；前端本地处理，不经服务端）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationControl {
    Dismiss,
    Copy,
    Pin,
}

/// 通知上渲染的按钮：外部动作入口（不属于通知领域，点击后转交各自系统执行）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum ActionTarget {
    /// 失败恢复指令（agena_failure::RecoveryDirective；运行时翻译为命令）
    Recovery(RecoveryDirective),
    /// 通用命令（应用命令注册表 / 插件命令）
    Command { command: String, input: Option<serde_json::Value> },
    /// 前端路由（含 open_url）
    Navigate { route: String },
    /// 复制文本
    Copy { text: String },
}

/// 用户可执行的单一动作（入口）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
    pub target: ActionTarget,
}

/// 一条完整通知
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Notification {
    pub id: NotificationId,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub scope: NotificationScope,
    pub surface: NotificationSurface,  // 物理渲染位置（宿主根据 kind/scope 分配）
    pub source: NotificationSource,       // runtime | app | plugin | background | frontend
    pub summary: String,
    pub detail: Option<String>,
    pub control: NotificationControl,  // 通知自身允许的控制操作（Dismiss/Copy/Pin）
    pub actions: Vec<NotificationAction>,  // 外部动作入口（点击后转交各自系统）
    pub priority: i32,
    pub dedup_key: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub dismissed: bool,
}

/// 通知服务端口（应用层唯一入口）
#[async_trait]
pub trait NotificationService: Send + Sync {
    /// 发出通知（聚合/去重/持久化/推送订阅者）
    async fn emit(&self, request: EmitNotificationRequest) -> Result<Notification, NotificationError>;
    /// 按过滤器查询（分页）
    async fn list(&self, filter: NotificationFilter) -> Result<Vec<Notification>, NotificationError>;
    /// 忽略/关闭
    async fn dismiss(&self, id: NotificationId, reason: Option<String>) -> Result<(), NotificationError>;
    /// 执行入口动作（转交对应系统：命令注册表 / 路由 / 剪贴板）
    async fn resolve_action(&self, id: NotificationId, action_id: String) -> Result<(), NotificationError>;
    /// 订阅推送（SSE 后端）
    fn subscribe(&self, filter: NotificationFilter) -> NotificationSubscription;
}
```

### 2.2 新增/调整 crate 布局

| crate | 职责 | 说明 |
|---|---|---|
| **agena-notification**（新） | 领域模型 + trait + 纯逻辑（severity 排序、scope 定位、去重、过期、action 注册表接口） | 零运行时依赖、可单测 |
| **agena-runtime-notifications**（新） | 聚合器：把 RuntimeEvent / ActivityPayload / BackgroundActivity / 插件贡献转成 Notification；内置内存存储 + 可选 SQLite 历史 | 实现 NotificationService |
| **agena-api** | 增加 NotificationResource / filter / action 资源类型 | 扩展现有 resource 层 |
| **agena-api-server** | REST `/api/v1/notifications*` + SSE `/api/v1/notifications/stream` | 复用 event-stream 基建 |
| **agena-tui-backend** | 暴露 NotificationService 端口给 TUI | 现有 Backend 增加方法 |
| **agena-tui-app** | 替换 UiNotice/flash_* 为 Notification adapter（renderer + reducer + action dispatch） | 移除自研 emit |
| **agena-web-ui** | 统一 consume `/api/v1/notifications` + SSE；toast/banner 渲染器 | 移除 errorMessage/localCommandNotice 各自写入 |
| **agena-plugin-host / agena-plugin-sdk** | 插件贡献收敛为声明式 + 统一 notify trait | 见 §3 |

### 2.3 统一 API 契约（外部）

```
GET    /api/v1/notifications?scope=&severity=&kind=&limit=&cursor=
POST   /api/v1/notifications                      // 仅内部服务间用，前端一般只读
POST   /api/v1/notifications/{id}/dismiss
POST   /api/v1/notifications/{id}/actions/{action_id}
GET    /api/v1/notifications/stream               // SSE: notification 事件
```

SSE 事件统一为 `notification`，载荷即 `NotificationResource`；保留 `lagged / resumed / subscription_closed` 控制事件。

---

## 3. 插件契约收敛（重点）

### 3.1 现状三套插件 UI 通道

1. **statusline 段**（ui_statusline_contribute / manifest.statusline_segments）：插件直接给 TUI 供文本段（agena.terminal.title/activity/notify、plan:{session_id}）。
2. **TUI 内容块**（manifest.content_blocks，location = composer_footer 等）：插件直接指定渲染位置与文本。
3. **Studio 命令/控件/视图 + 命令输出**（PluginCommandOutput：message / submit_prompt / open_route / open_url / invoke_tool）：插件控制前端行为。

问题：三套结构互不相同、渲染细节（位置、优先级、颜色）泄漏进插件、宿主无法统一治理。

### 3.2 目标：插件只「声明 + 发送意图」

- **声明式**：manifest 只描述「我有哪些贡献」，不含渲染位置/颜色细节；位置/优先级由宿主根据 kind/scope 决定。
  - 例如 `agena.terminal` 不再贡献 `agena.terminal.title` 段，而是声明 `kind = TerminalTitle`、`kind = TerminalNotify`、`kind = TerminalActivity`；宿主聚合进统一 Notification，再按 surface 渲染。
  - plan 进度改为 `kind = Progress { current, total }`，不再用 `plan:{session_id}` 段 id 暗示位置。
- **命令式只留一个统一入口**：插件要「现在告诉用户一件事」，只调用 `host.notify(NotifyRequest)`（对应现有 ui_statusline_contribute 的收敛）；宿主负责落 Notification、去重、决定 surface。
- **禁止直接操作 UI**：移除/冻结 `tui content block location=composer_footer` 这类「插件指定显示位置」的能力；改为插件声明贡献内容，宿主分配到 footer/chip/页面。
- **Studio 命令输出**：把 `PluginCommandOutput` 收敛为 `ActionTarget` 的一子集（Message -> 生成 kind=Notice 通知；SubmitPrompt / InvokeTool / InvokeCommand -> Command；OpenUrl / OpenPluginWorkbench -> Navigate），前端只消费统一入口。

### 3.3 trait 签名草案

```rust
// agena-plugin-sdk：插件侧可见的统一通知入口（命令式，唯一）
#[async_trait]
pub trait PluginNotificationSink: Send + Sync {
    /// 发送一条用户可见通知意图。宿主决定展示位置/优先级/去重。
    async fn notify(&self, req: PluginNotifyRequest) -> Result<(), PluginError>;
}

// 声明式贡献（manifest 收敛后）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginDisplayContribution {
    pub id: String,
    pub kind: ContributionKind,       // StatusLineText | Progress | TerminalTitle | TerminalNotify | FooterBlock | StudioCommand | StudioView | ...
    pub priority: i32,
    pub content: PluginDisplayContent, // 纯内容，无位置/颜色
}

// host 聚合端口（内部）
#[async_trait]
pub trait PluginDisplayHost: Send + Sync {
    fn contributions(&self) -> Vec<HostDisplayContribution>;   // 只读 catalog
    async fn notify(&self, plugin_id: &str, req: PluginNotifyRequest) -> Result<(), PluginError>;
}
```

### 3.4 迁移后的插件行为示例

| 现有插件写法 | 重构后 |
|---|---|
| `host.ui_statusline_contribute(segment_id="agena.terminal.activity", content="\"running\"", priority=i32::MAX-1)` | 声明 `kind=TerminalActivity`；宿主在 run.pre/run.post 钩子时自动更新 |
| `host.ui_statusline_contribute(segment_id="plan:3", content="2/5 done", priority=120)` | 声明 `kind=Progress{2,5}`，scope=Session(3)；宿主决定右下 chip 还是 footer |
| `PluginCommandOutput::Message("done")` | 生成 kind=Notice 通知；无需入口 |
| `manifest.ui.tui.content_blocks` 指定 composer_footer | `manifest.ui.display.contributions` 声明 FooterBlock，位置由宿主分配 |

---

## 4. 显示端统一

### 4.1 共享渲染语义（domain 提供纯函数）

```rust
// agena-notification 提供（不依赖任何 UI 框架）
pub fn severity_color(severity: NotificationSeverity) -> SemanticColor;  // 映射到主题语义色
pub fn scope_label(scope: &NotificationScope) -> String;
pub fn sort_and_group(notifications: &[Notification]) -> Vec<NotificationGroup>;
pub fn should_dedupe(a: &Notification, b: &Notification) -> bool;         // 基于 dedup_key + kind + scope
pub fn is_expired(notification: &Notification, now_ms: i64) -> bool;
```

### 4.2 TUI 适配（agena-tui-app）

- `NotificationStore`：持有当前可见通知列表（替代单一 `Option<UiNotice>`），由 reducer 纯函数维护。
- `TuiNotificationSink`：实现 `NotificationService::subscribe` 的本地消费；把 `Notification` 路由到：
  - footer 横幅（最高优先级一条）
  - composer chip（Progress / ActivityChanged / model 状态）
  - 头部 right（Run 状态 / spinner）
  - activities 面板（BackgroundTask scope）
- `TuiActionDispatcher`：把 `ActionTarget` 转交对应系统（Recovery -> 命令注册表、Command -> 命令、Navigate -> 路由、Copy -> 剪贴板；复用 commands.rs 的 CommandId 注册表）；`NotificationControl`（Dismiss/Copy/Pin）由前端本地直接处理。
- 删除：`app_transcript_actions.rs` 的 flash_* 自研 emit；`UiNotice` 收敛为 domain `Notification` 的 TUI 视图。

### 4.3 Web 适配（packages/agena-web-ui）

- `useNotifications` composable：REST 查询 + SSE 订阅统一 `/notifications/stream`。
- `NotificationToaster` / `NotificationBanner`：同一渲染器消费 NotificationResource，替代 errorMessage/localCommandNotice 的分散写入。
- `notificationActions`：把 `ActionTarget` 转成 router.push / api 调用 / 剪贴板；`NotificationControl`（Dismiss/Copy/Pin）本地处理。
- Activities / RuntimeOverview / Chat 面板全部改为读同一 store。

### 4.4 终端 chrome 适配

- `agena.terminal.*` 插件段收敛后，`app_terminal_integration.rs` 从统一 Notification 流投影 OSC 帧：
  - kind=TerminalTitle -> title_frames
  - kind=TerminalNotify -> notification_frames
  - kind=Progress -> progress_frames
- 保留现有能力探测与开关（terminal_title/notifications/progress）。

---

## 5. 用户交互统一（控制 + 入口转交）

- **通知自身控制（NotificationControl，≤3 种）**：Dismiss / Copy / Pin。它们是对「通知对象」的操作，由前端本地直接处理，不需要服务端参与（dismissed 状态同步走 REST）。
- **外部入口（NotificationAction → ActionTarget，4 种）**：通知只渲染按钮；点击后把 ActionTarget 转交对应系统：
  - `Recovery(RecoveryDirective)` → 失败恢复命令注册表（agena_failure 领域；现状 recovery_action() 已把 directive 翻译成命令字符串，如 provider.authenticate）
  - `Command { command, input }` → 应用命令注册表 / 插件命令
  - `Navigate { route }` → 前端路由（含 open_url）
  - `Copy { text }` → 剪贴板
- 服务端提供统一入口执行端点 `POST /api/v1/notifications/{id}/actions/{action_id}`：服务端把 target 映射到应用命令；前端不再各自实现「Sign in / Open settings / Retry」等命令映射。
- **权限 / 用户输入不是通知动作**：它们是会话执行资源的专用交互（PermissionRequest / UserInputRequest，已统一在 SessionExecutionResource），各自前端保留专用对话框；通知最多携带指向它们的入口（Navigate）。

---

## 6. 迁移路径（分阶段）

### Phase 0 — 基线冻结（0.5 周）
- 合并现状盘点文档（docs/notification-and-status-display.md）为权威基线。
- 为新 crate 建 feature/分支（本 worktree 即起点）。

### Phase 1 — 领域 crate（1 周）
- 新建 `crates/agena-notification`：模型 + trait + 纯逻辑 + 单元测试。
- 编写 `NotificationResource`（agena-api）与 serde 契约测试。
- 产出：无行为变更，纯新增。

### Phase 2 — 运行时聚合器（1.5 周）
- 新建 `crates/agena-runtime-notifications`：
  - 订阅 RuntimeEvent 流，把 NoticePart / ActivityPayload / BackgroundActivity 转成 Notification。
  - 内存 store + 去重 + 过期清理；可选 SQLite 历史表。
- 在 application 层接线 `NotificationService`。
- 产出：后端已有统一通知模型，但前端尚未消费。

### Phase 3 — REST/SSE API（1 周）
- `agena-api-server` 增加 `/api/v1/notifications*` 与 `/api/v1/notifications/stream`。
- 保留旧端点兼容（`/events/stream`、session events 继续工作）。
- 产出：任何客户端都能查询/订阅统一通知。

### Phase 4 — Web 迁移（1.5 周）
- 新建 `useNotifications` + toaster/banner 渲染器。
- 逐步把 ChatPage / RuntimeOverview / ActivitiesPage / UsagePage 的错误与操作反馈迁到统一 store。
- 删除 errorMessage / localCommandNotice 的分散写入（保留兼容过渡期）。
- 产出：Web 只消费统一 API。

### Phase 5 — TUI 迁移（2 周）
- `NotificationStore` 替换 `Option<UiNotice>`；flash_* 改为本地 emit 到统一 sink。
- composer chips / footer / header 改读统一 store；activities 面板读 BackgroundTask scope 子集。
- 删除 UiNotice 自研 emit；保留渲染样式映射到 domain 语义色。
- 产出：TUI 只消费统一模型。

### Phase 6 — 插件契约收敛（2 周）
- manifest 收敛：`PluginUiContributions` 三套结构合并为 `PluginDisplayContribution`。
- host 侧：移除 `ui_statusline_contribute` 命令式（或保留 deprecated 桥接层）、tui content blocks 位置字段、studio 命令输出消息直通。
- agena.terminal 与 workflow plan 插件改为声明式 + 统一 notify。
- 产出：插件无侵入。

### Phase 7 — 清理与收尾（1 周）
- 删除旧通道（UiNotice、errorMessage 直写、tui content block location 特殊化）。
- 合并重复代码、补全集成测试、更新两份文档。
- 全量回归（TUI 冒烟 + Web e2e + 插件测试）。

总工期约 10 周（可按需裁剪：Phase 1-3 单独交付即先拿到统一后端）。

---

## 7. Rust 最佳实践要点

1. **trait 对象 vs 泛型**：`NotificationService` 用 `dyn`（运行时多态、便于 mock 与 feature 组合）；纯逻辑函数用泛型/普通 fn。
2. **async_trait 约定**：统一 async_trait（现状已普遍使用），SSE 订阅用 `async-stream` 或 `tokio_stream::wrappers::ReceiverStream`（api-server 已有先例 sse.rs）。
3. **serde 契约**：所有跨进程类型 `#[serde(deny_unknown_fields)]` + JsonSchema；用 `#[serde(tag=...)]` 枚举保证可演化。
4. **错误类型**：`NotificationError` 用 thiserror，区分 NotFound / Validation / Conflict / Unavailable。
5. **事件驱动**：内存用 `tokio::sync::broadcast`（多订阅者、滞后检测 -> lagged 事件），避免 mpsc 单消费者。
6. **持久化可选**：默认内存 + 可选 SQLite；历史查询走 `NotificationHistoryStore` trait，便于替换。
7. **测试**：
   - domain 纯逻辑单测（去重、过期、排序、action 映射）
   - 聚合器单测（RuntimeEvent -> Notification）
   - REST/SSE 契约测试（api-server 已有 router_contract_tests 先例）
   - TUI reducer 单测（app_tests 先例）
   - Web composable 单测（bun test，chatPageModel 先例）
8. **feature 门控**：notifications 后端独立 feature；不强制所有 host 启用。
9. **兼容层**：新旧通道并存期用「适配器 + 弃用告警」，一次删除、避免双写。
10. **命名**：统一 `Notification*` / `*Sink` / `*Store` 命名，禁止再引入 Notice 双义词（保留 `NoticePart` 作为历史 wire 类型别名）。

---

## 8. 风险与取舍

| 风险 | 缓解 |
|---|---|
| 迁移期双通道不一致 | Phase 1-3 后端先行，新旧 API 并存；Phase 4-5 逐个前端切换；用集成测试锁行为 |
| 插件生态破坏 | 保留 deprecated 桥接层至少一个发布周期；manifest 字段用 serde default 兼容旧插件 |
| 性能：通知量突增 | broadcast 滞后检测 -> lagged；内存 store 上限 + 过期清理；SSE 分页查询 |
| Web toast 体验差异 | 保留 toast 视觉样式，仅数据源统一；严重级/过期语义由 domain 决定 |
| TUI 最近一条 vs 多条 | NotificationStore 支持「多活动 + 一条置顶横幅」；置顶规则为 priority+时间 |
| 过度设计 | Phase 1-3 先交付统一后端；若插件收敛阻力大，可先只做 API 统一，插件段保留 deprecated |

---

## 9. 验收标准

1. 新增 `agena-notification` crate，域模型与纯逻辑 100% 单测覆盖。
2. `GET/POST /api/v1/notifications*` + SSE 流可用；同一通知在 REST 与 SSE 载荷一致。
3. TUI 与 Web 均通过统一 API 消费通知；仓库中不再存在 `self.notice = Some(...)` 与 `errorMessage.value = ...` 直写通知的模式。
4. 插件不再直接指定渲染位置/颜色；所有插件贡献收敛为声明式 + 统一 notify trait。
5. 旧通道删除后全量回归通过（TUI 冒烟、Web e2e、插件测试）。
6. 文档更新：docs/notification-and-status-display.md 反映新架构。

---

## 10. 建议的下一步

1. 在本 worktree 分支先落地 Phase 1（agena-notification 领域 crate + 单测），作为可独立评审的 PR。
2. 同时产出《插件贡献迁移指南》供 agena.terminal / workflow plan / 第三方插件作者参考。
3. 建立通知 API 契约测试夹具（golden JSON），防止 wire 形状漂移。

---

## 附录 A：通知与显示分类全表（三轴：Kind × Surface × 交互）

> 回答「通知和显示会有哪些类型、一共多少种」：**内容 16 种 Kind、位置 16 处 Surface；交互分两层——通知自身操作 3 种（NotificationControl）+ 外部动作入口 4 种（ActionTarget）**。
> 任何一次显示 = 一个 Kind（内容） + 一个 Surface（位置） + (0..n 自身操作 + 0..n 外部入口)。

### A.1 内容轴：NotificationKind（16 种）

| # | Kind | 语义 | 现状对应 | 典型 Surface |
|---|---|---|---|---|
| 1 | Notice { code } | 一次性消息（成功/警告/错误/信息） | UiNotice / flash_* / NoticePart / Web errorMessage | Banner / Toast |
| 2 | Progress { current, total } | 一般进度（total 可空） | 后台任务进度 | Toast / BackgroundTask |
| 3 | Status { state } | 状态切换（idle/running/awaiting/blocked/...） | 终端 activity 段 / RuntimeStatus | StatusLine / TerminalProgress |
| 4 | ModelStatus { model, thinking, speed } | 模型状态与速度 | TUI model chip / speed 显示 | ComposerChip / StatusLine |
| 5 | PlanProgress { current, total } | plan 执行进度 | plan:{session_id} 段 | ComposerChip / PlanPanel |
| 6 | RunState { state } | run/workflow 执行状态 | workflow 状态 | PlanPanel / StatusLine |
| 7 | CommandExecution { command, stream, exit_code } | 命令执行反馈 | 命令运行状态 | ComposerFooter / Toast |
| 8 | ToolCall { call_id, name } | 工具调用反馈 | ToolCallNotice / 工具调用状态 | StatusLine / ComposerChip |
| 9 | BackgroundActivity { activity_id } | 后台活动状态变化 | BackgroundActivity / 后台计数 | ActivitiesPanel / ComposerChip |
| 10 | PermissionRequest { request_id } | 权限请求 | 待审批 chip / PermissionRequest | PermissionDialog / ComposerChip |
| 11 | UserInputRequest { request_id } | 用户输入请求 | UserInputRequest | InputPrompt / ComposerChip |
| 12 | HistorySearch { query, current, total } | 历史搜索反馈 | 历史搜索状态 | HistorySearch |
| 13 | TerminalTitle { title } | 终端窗口标题 | OSC 0/2 帧（title_frames） | TerminalTitle |
| 14 | TerminalNotify { text } | 终端通知（铃响/OSC 9/系统通知） | NotificationMethod{Bell,Osc9,...} | TerminalBell |
| 15 | UsageUpdate { current_tokens, projected_tokens, context_window } | 上下文用量更新 | token% / 用量显示 | StatusLine / ComposerChip |
| 16 | Custom(CustomNotification) | 插件自定义（扩展点） | 插件贡献（manifest/命令输出） | 宿主指定 |

### A.2 位置轴：NotificationSurface（16 处）

| # | Surface | 说明 | 现状对应 |
|---|---|---|---|
| 1 | Banner | 顶部横幅 | Web .notice（errorMessage/localCommandNotice）、TUI 顶栏 |
| 2 | Toast | 浮动提示 | Web RuntimeOverviewPanel toast（teleport body 右上 fixed） |
| 3 | ComposerChip | 输入区四角 chip | TUI render_composer 四角（状态/搜索/后台/plan） |
| 4 | ComposerFooter | 输入框上方 footer | TUI transcript_footer（notice 优先渲染行 + 插件段） |
| 5 | StatusLine | 状态行 | TUI status_line + 插件段（agena.terminal.activity 等）、Web 面板状态段 |
| 6 | TerminalTitle | 终端窗口标题 | OSC 0/2（title_frames） |
| 7 | TerminalProgress | 终端进度 | OSC 9;4（ProgressState{Clear,Working,Awaiting,Blocked}） |
| 8 | TerminalBell | 铃响/系统通知 | NotificationMethod{Bell,Osc9,Osc9AndItermAttention} |
| 9 | ActivitiesPanel | 后台活动面板 | TUI activities 面板、Web ActivitiesPage |
| 10 | HistorySearch | 历史搜索浮动条 | TUI 历史搜索状态 |
| 11 | PermissionDialog | 权限请求对话框 | TUI permission overlay、Web 待审批面板 |
| 12 | InputPrompt | 用户输入请求对话框 | TUI user_input overlay、Web 输入请求面板 |
| 13 | Settings | 设置面板 | 设置页 |
| 14 | PlanPanel | 计划/进度面板 | plan 面板 / workflow 视图 |
| 15 | BackgroundTask | 后台任务面板 | Web 后台任务卡片、TUI 活动面板任务分组 |
| 16 | Log | 仅记录（不主动弹出） | 活动日志流（e> 前缀）、日志面板 |

### A.3 交互轴 1：NotificationControl（通知自身操作，3 种）

| # | Control | 语义 | 前端处理 |
|---|---|---|---|
| 1 | Dismiss | 关闭/忽略本通知 | toast/banner 关闭按钮；dismissed=true 走 REST |
| 2 | Copy | 复制通知内容 | 剪贴板 |
| 3 | Pin | 置顶/稍后处理（可选） | 前端置顶状态 |

### A.4 交互轴 2：ActionTarget（外部入口，4 种；不属于通知领域）

| # | Target | 语义 | 归属领域 | 现状对应 |
|---|---|---|---|---|
| 1 | Recovery(RecoveryDirective) | 失败恢复指令（8 种：Refresh / Reauthenticate / OpenSettings / OpenPermissions / Retry / ChooseAlternative / RestartPlugin / RestartRuntime） | agena_failure | RecoveryDirective -> 命令字符串（session.refresh / provider.authenticate / ...） |
| 2 | Command { command, input } | 执行命令 / 提交提示 | 应用命令注册表 / 插件命令 | PluginCommandOutput{SubmitPrompt, InvokeTool, InvokeCommand}、submit_prompt |
| 3 | Navigate { route } | 前端跳转（含 open_url） | 前端路由 | open_route / open_url / OpenPluginWorkbench |
| 4 | Copy { text } | 复制内容 | 剪贴板 | 复制按钮 |

> 曾并入通知模型的 12 个系统操作全部回到各自领域：8 个 RecoveryDirective（agena_failure）、StopActivity/DismissActivity（activities REST API）、ApprovePermission/ReplyUserInput（SessionExecutionResource 会话交互）。

### A.5 组合规则（一个 Kind 可落多个 Surface）

- **主 Surface**：宿主按 kind + scope + priority 决定主位置（如 Notice → Banner/Toast、Progress → Toast、Status → StatusLine）。
- **次 Surface（镜像）**：同一通知可同步镜像到 Log（可追溯）、TerminalTitle/TerminalBell（终端可达性）、ComposerChip（常驻计数）。
- **控制与入口分离**：NotificationControl 由前端本地处理；ActionTarget 经 `POST /api/v1/notifications/{id}/actions/{action_id}` 转交对应系统。
- 例：`ToolCall` 主 Surface = StatusLine，同时镜像 Log；`PermissionRequest` 主 Surface = PermissionDialog + ComposerChip 待审批角标。

### A.6 计数汇总

- **Kind：16**（含 1 个插件扩展点 Custom）
- **Surface：16**
- **NotificationControl：3**（通知自身操作）
- **ActionTarget：4**（外部入口；其中 Recovery 承载 8 个 agena_failure 指令，不属于通知领域）
- 任何一次显示 = 1 Kind × ≥1 Surface × (0..n 自身操作 + 0..n 外部入口)
