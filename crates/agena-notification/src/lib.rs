//! 统一通知与显示领域模型（Unified Notification & Display Architecture）。
//!
//! 所有通知/显示意图的唯一领域对象：内容（Kind）、位置（Surface）、
//! 通知自身操作（Control）与外部动作入口（ActionTarget）。
//!
//! 设计原则：
//! - 通知模型不携带任何「业务动作语义」，只携带展示载体 + 入口指针。
//! - 业务动作语义留在各自领域：失败恢复（`agena-failure::RecoveryDirective`）、
//!   活动操作（activities REST API）、会话交互（SessionExecutionResource）。
//! - 领域层纯逻辑，适配层薄：本 crate 零运行时依赖（无 tokio），全部可单测。

pub mod logic;
pub mod model;
pub mod service;

pub use logic::{
    default_surface, is_expired, recovery_command, resolve_action_target, scope_label,
    severity_color, should_dedupe, sort_and_group, NotificationGroup, SemanticColor,
};
pub use model::*;
pub use service::{EmitNotificationRequest, NotificationError, NotificationFilter, NotificationService, NotificationSubscription};
