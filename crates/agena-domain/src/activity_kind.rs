use serde::{Deserialize, Serialize};

/// Stable identifier of an activity kind used by transcript expansion
/// settings and the dynamic settings catalog.
pub type ActivityKindId = String;

/// Origin of an activity kind in the catalog.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::module_name_repetitions)]
pub enum ActivityKindCategory {
    /// Part of the built-in catalog every install exposes.
    Builtin,
    /// Declared by a plugin manifest.
    Plugin,
}

/// A discoverable activity kind shown in settings and used to compute the
/// default transcript expansion for activities of that kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[allow(clippy::module_name_repetitions)]
pub struct ActivityKind {
    /// Stable id such as `reasoning` or a plugin-declared kind id.
    pub id: String,
    pub category: ActivityKindCategory,
    /// Display label. Built-in kinds use stable English labels; hosts
    /// localize them via i18n keys derived from `id` when available.
    pub label: String,
}

/// Built-in reasoning kind id.
pub const ACTIVITY_KIND_REASONING: &str = "reasoning";
/// Built-in operation kind id.
pub const ACTIVITY_KIND_OPERATION: &str = "operation";
/// Built-in resource kind id.
pub const ACTIVITY_KIND_RESOURCE: &str = "resource";
/// Built-in skill reference kind id.
pub const ACTIVITY_KIND_SKILL_REFERENCE: &str = "skill_reference";
/// Built-in interaction kind id.
pub const ACTIVITY_KIND_INTERACTION: &str = "interaction";
/// Built-in hook kind id.
pub const ACTIVITY_KIND_HOOK: &str = "hook";
/// Built-in error kind id.
pub const ACTIVITY_KIND_ERROR: &str = "error";
/// Built-in notice kind id.
pub const ACTIVITY_KIND_NOTICE: &str = "notice";
/// Built-in text kind id.
pub const ACTIVITY_KIND_TEXT: &str = "text";

/// The nine built-in activity kinds every install exposes.
pub fn builtin_activity_kinds() -> Vec<ActivityKind> {
    vec![
        ActivityKind {
            id: ACTIVITY_KIND_REASONING.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Reasoning".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_OPERATION.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Operation".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_RESOURCE.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Resource".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_SKILL_REFERENCE.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Skill reference".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_INTERACTION.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Interaction".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_HOOK.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Hook".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_ERROR.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Error".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_NOTICE.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Notice".to_owned(),
        },
        ActivityKind {
            id: ACTIVITY_KIND_TEXT.to_owned(),
            category: ActivityKindCategory::Builtin,
            label: "Text".to_owned(),
        },
    ]
}

/// Built-in default expansion overrides applied before user configuration:
/// reasoning expands by default; every other kind follows the global
/// transcript default.
pub fn builtin_activity_kind_defaults() -> Vec<(String, bool)> {
    vec![(ACTIVITY_KIND_REASONING.to_owned(), true)]
}
