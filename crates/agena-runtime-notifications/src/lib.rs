//! 运行时通知聚合器与内存存储。
//!
//! 把现有运行时/领域事件（NoticePart / ActivityPayload / BackgroundActivity）统一转成
//! [`agena_notification::model::Notification`]，并提供实现 [`NotificationService`] 的内存 store
//! （去重、过期清理、broadcast 订阅 + lagged 检测）。
//!
//! 设计：聚合器只做「事件 -> Notification」的纯转换；存储/推送由 store 承担；
//! 真正执行 `ActionTarget` 的宿主命令执行器不在本 crate（应用层接线）。

pub mod aggregate;
pub mod store;

pub use aggregate::{
    from_activity_payload, from_background_activity, from_notice_part, notification_id_for_activity,
};
pub use store::{
    BroadcastSubscription, InMemoryNotificationStore, SubscriptionEvent, filter_matches, now_ms,
};
