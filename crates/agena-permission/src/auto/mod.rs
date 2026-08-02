//! Layered automatic-approval engine for `PermissionMode::Auto`.

mod budget;
mod classifier;
mod fast_path;
mod heuristic;

pub use budget::DenialBudget;
pub use classifier::{
    AUTO_APPROVAL_CLASSIFY_TIMEOUT, AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS,
    AUTO_DENY_GUIDANCE, AutoApprovalClient, AutoApprovalError, ClassifierRequest,
    build_classifier_action_message, build_classifier_context_message,
    build_classifier_user_prompt, classifier_json_schema, deny_reason, parse_classifier_verdict,
};
pub use fast_path::{AUTO_APPROVAL_SYSTEM_PROMPT, AutoFastPath, auto_fast_path};
pub use heuristic::heuristic_decision;
