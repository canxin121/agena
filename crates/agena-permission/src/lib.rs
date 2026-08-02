//! Pure permission decision core for Agena.
//!
//! This crate owns *every* permission decision that is not a raw policy
//! table lookup executed inside the tool executor:
//!
//! - rule synthesis against the persisted rule snapshot ([`rules`]),
//! - the layered automatic-approval engine ([`auto`]),
//! - the synchronous decision pipeline ([`pipeline`]) that composes static
//!   policy, rules, fast path, heuristics, and the classifier hand-off.
//!
//! The crate has no runtime, session, storage, or provider dependency: the
//! host supplies the compiled policy, the rule snapshot, and an
//! [`auto::AutoApprovalClient`] implementation.

pub mod auto;
pub mod pipeline;
pub mod rules;

pub use auto::{
    AUTO_APPROVAL_CLASSIFY_TIMEOUT, AUTO_APPROVAL_SYSTEM_PROMPT,
    AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS, AUTO_DENY_GUIDANCE, AutoApprovalClient,
    AutoApprovalError, ClassifierRequest, DenialBudget, build_classifier_action_message,
    build_classifier_context_message, build_classifier_user_prompt, classifier_json_schema,
    deny_reason, parse_classifier_verdict,
};
pub use pipeline::{ClassifierCandidate, DecisionContext, SyncOutcome, decide_sync};
pub use rules::{RuleEntry, apply_rules};
