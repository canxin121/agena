//! Pure permission decision core for Agena.
//!
//! This crate owns *every* permission decision that is not a raw policy
//! table lookup executed inside the tool executor:
//!
//! - rule synthesis against the persisted rule snapshot ([`rules`]),
//! - the automatic-approval engine ([`auto`]) that the static policy layer
//!   hands its `Auto` verdicts to,
//! - the synchronous decision pipeline ([`pipeline`]) that checks the denial
//!   budget and resolves the classifier hand-off.
//!
//! The crate has no runtime, session, storage, or provider dependency: the
//! host supplies the compiled policy, the rule snapshot, and an
//! [`auto::AutoApprovalClient`] implementation.

pub mod auto;
pub mod pipeline;
pub mod rules;

pub use auto::{
    AUTO_APPROVAL_APPROVE_TOOL, AUTO_APPROVAL_BLOCK_RULES, AUTO_APPROVAL_BLOCK_TOOL,
    AUTO_APPROVAL_CLASSIFY_TIMEOUT, AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS, AUTO_DENY_GUIDANCE,
    AutoApprovalClient, AutoApprovalError, CLASSIFIER_VERDICT_INSTRUCTION, ClassifierRequest,
    ClassifierVerdict, ClassifyFailure, DenialBudget, PathClassPromptFlags,
    auto_approval_decision_tools, auto_approval_system_prompt, build_classifier_action_message,
    build_classifier_context_message, build_classifier_user_prompt, cited_block_rule, deny_reason,
    parse_classifier_reason, parse_classifier_verdict,
};
pub use pipeline::{ClassifiedCandidate, ClassifierCandidate, SyncOutcome, decide_sync};
pub use rules::{RuleEntry, apply_rules};
