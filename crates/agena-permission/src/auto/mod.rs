//! Layered automatic-approval engine for `PermissionMode::Auto`.

mod budget;
mod classifier;
mod fast_path;
mod heuristic;

pub use budget::DenialBudget;
pub use classifier::{
    AUTO_APPROVAL_APPROVE_TOOL, AUTO_APPROVAL_BLOCK_RULES, AUTO_APPROVAL_BLOCK_TOOL,
    AUTO_APPROVAL_CLASSIFY_TIMEOUT, AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS, AUTO_DENY_GUIDANCE,
    AutoApprovalClient, AutoApprovalError, CLASSIFIER_VERDICT_INSTRUCTION, ClassifierRequest,
    ClassifierVerdict, ClassifyFailure, auto_approval_decision_tools,
    build_classifier_action_message, build_classifier_context_message,
    build_classifier_user_prompt, cited_block_rule, deny_reason, parse_classifier_reason,
    parse_classifier_verdict,
};
pub use fast_path::{AUTO_APPROVAL_SYSTEM_PROMPT, AutoFastPath, auto_fast_path};
pub use heuristic::heuristic_decision;
