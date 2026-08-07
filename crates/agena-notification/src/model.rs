//! 统一通知领域模型：内容（Kind）、位置（Surface）、控制（Control）与入口（ActionTarget）。
//!
//! 序列化契约说明：`ActionTarget` 承载 `agena-failure::RecoveryDirective`（该类型未实现
//! `schemars::JsonSchema`），因此 `ActionTarget` / `NotificationAction` / `Notification` 只实现
//! serde 契约；API 层的 resource 类型（Phase 3）负责补充 JsonSchema。

use agena_failure::RecoveryDirective;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 通知唯一标识（由实现方生成，如 `notif_<uuid>`）。
pub type NotificationId = String;

/// 严重级别（替代 TUI NoticeSeverity 与 Web toast kind 的重复定义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

/// 作用域（替代 TUI NoticeScope 与 Web scope_kind 的重复定义）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// 通知来源（谁发出）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSource {
    #[default]
    Runtime,
    App,
    Plugin,
    Background,
    Frontend,
}

/// 状态类通知的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// Run 状态类通知的取值（plan / workflow 执行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// 插件自定义通知载荷（kind = Custom）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomNotification {
    pub plugin_id: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// 机器可读类别（枚举 + 插件扩展点），全系统收敛为 16 种，见文档附录 A。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationKind {
    Notice {
        code: String,
    },
    Progress {
        current: Option<u64>,
        total: Option<u64>,
    },
    Status {
        state: NotificationState,
    },
    ModelStatus {
        model: String,
        thinking: Option<String>,
        speed: Option<String>,
    },
    PlanProgress {
        current: u64,
        total: u64,
    },
    RunState {
        state: RunNotificationState,
    },
    CommandExecution {
        command: String,
        stream: Option<String>,
        exit_code: Option<i32>,
    },
    ToolCall {
        call_id: String,
        name: String,
    },
    BackgroundActivity {
        activity_id: String,
    },
    PermissionRequest {
        request_id: String,
    },
    UserInputRequest {
        request_id: String,
    },
    HistorySearch {
        query: String,
        current: u64,
        total: u64,
    },
    TerminalTitle {
        title: String,
    },
    TerminalNotify {
        text: String,
    },
    UsageUpdate {
        current_tokens: u64,
        projected_tokens: Option<u64>,
        context_window: Option<u32>,
    },
    Custom(CustomNotification),
}

/// 物理渲染位置（Surface）：宿主根据 kind/scope 决定，前端只按 surface 渲染。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSurface {
    /// 顶部横幅（Web .notice / TUI 顶栏）。
    Banner,
    /// 浮动 toast（Web 右上 / TUI 覆盖层）。
    Toast,
    /// 输入区四角 chip（状态/搜索/后台/plan）。
    ComposerChip,
    /// 输入框上方 footer 行（TUI）。
    ComposerFooter,
    /// 状态行（TUI 底部 / Web 面板状态段）。
    StatusLine,
    /// 终端窗口标题（OSC 0/2）。
    TerminalTitle,
    /// 终端进度（OSC 9;4 / 任务栏）。
    TerminalProgress,
    /// 终端铃响 / 系统通知。
    TerminalBell,
    /// 后台活动面板。
    ActivitiesPanel,
    /// 历史搜索浮动条。
    HistorySearch,
    /// 权限请求对话框。
    PermissionDialog,
    /// 用户输入请求对话框。
    InputPrompt,
    /// 设置面板。
    Settings,
    /// 计划/进度面板。
    PlanPanel,
    /// 后台任务面板。
    BackgroundTask,
    /// 仅记录（活动日志流），不主动弹出。
    Log,
}

/// 通知自身的操作（对通知对象的操作；前端本地处理，不经服务端）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationControl {
    #[default]
    Dismiss,
    Copy,
    Pin,
}

/// 通知上渲染的按钮：外部动作入口（不属于通知领域，点击后转交各自系统执行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum ActionTarget {
    /// 失败恢复指令（agena-failure::RecoveryDirective；运行时翻译为命令）。
    Recovery(RecoveryDirective),
    /// 通用命令（应用命令注册表 / 插件命令）。
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    /// 前端路由（含 open_url）。
    Navigate { route: String },
    /// 复制文本。
    Copy { text: String },
}

/// 用户可执行的单一动作（入口）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
    pub target: ActionTarget,
}

/// 一条完整通知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub id: NotificationId,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub scope: NotificationScope,
    /// 物理渲染位置（宿主根据 kind/scope 分配）。
    pub surface: NotificationSurface,
    pub source: NotificationSource,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 通知自身允许的控制操作（Dismiss/Copy/Pin）。
    pub control: NotificationControl,
    /// 外部动作入口（点击后转交各自系统）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotificationAction>,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub dismissed: bool,
}

impl Notification {
    /// 是否已过期（now_ms 大于等于 expires_at_ms）。
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|exp| now_ms >= exp)
    }

    /// 按 id 查找入口。
    pub fn find_action(&self, action_id: &str) -> Option<&NotificationAction> {
        self.actions.iter().find(|a| a.id == action_id)
    }
}
