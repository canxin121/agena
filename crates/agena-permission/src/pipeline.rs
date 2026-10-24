//! The synchronous permission decision pipeline.
//!
//! The static policy layer runs first: it is the compiled permission policy
//! composed with the persisted rule snapshot, and it already contains every
//! static rule (read-only tools, temp paths, routine shell commands, …), so
//! its `Allow` / `Ask` / `Deny` verdicts are terminal. Only `Auto` reaches
//! [`decide_sync`], which is the boundary between the static layer and the
//! approval model: it checks the denial budget and hands everything else to
//! the classifier. There is no automatic-approval fast path.

use agena_domain::{ActionSpec, PermissionDecision};

use crate::auto::DenialBudget;

/// One classifier evaluation unit. The host groups candidates from the same
/// tool invocation so a single transcript/model setup serves them all.
#[derive(Debug, Clone, PartialEq, Eq)]
/// A candidate action for classifier review.
pub struct ClassifierCandidate {
    pub action: ActionSpec,
    pub policy_reason: String,
}

/// A candidate returned from classification with its outcome attached.
///
/// `verdict` is `None` when automatic approval could not resolve at all, in
/// which case `failure` explains why and the caller falls back to interactive
/// confirmation (fail closed). `verdict: Some(_)` with `failure: Some(_)` is
/// the third case: the model did answer, but with a block that cites no rule
/// from the taxonomy, so the answer is not honored as a denial — the caller
/// also falls back to confirmation, and `failure` carries the model's own
/// words into the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
/// Outcome of classifying one candidate.
pub struct ClassifiedCandidate {
    pub candidate: ClassifierCandidate,
    pub verdict: Option<crate::ClassifierVerdict>,
    pub failure: Option<crate::ClassifyFailure>,
}

impl ClassifiedCandidate {
    /// Convert a classifier outcome into the permission decision it licenses.
    ///
    /// This is the single place the automatic-approval contract is enforced,
    /// and both hosts (the interactive reply path and the tool-execution
    /// batch) route through it so they cannot drift apart:
    ///
    /// - an allow is an allow;
    /// - a block is a terminal `Deny` only when the verdict cites a BLOCK rule
    ///   from [`crate::AUTO_APPROVAL_BLOCK_RULES`] — the model had to justify
    ///   itself against the enumerated taxonomy;
    /// - everything else (an uncited block, an unparseable verdict, a provider
    ///   error, a timeout) falls back to an interactive `Ask`, which is the
    ///   fail-closed outcome, and `failure` explains why.
    pub fn decision(&self) -> PermissionDecision {
        let verdict_reason = |verdict: &crate::ClassifierVerdict| {
            let text = verdict.reason.trim();
            if text.is_empty() {
                self.candidate.policy_reason.clone()
            } else {
                text.to_owned()
            }
        };
        match self.verdict.as_ref() {
            Some(verdict) if verdict.allowed => PermissionDecision::Allow,
            Some(verdict) => match verdict.block_rule {
                Some(cited) => PermissionDecision::Deny {
                    reason: crate::deny_reason(format!(
                        "automatic approval classifier blocked the action [{cited}]: {}",
                        verdict_reason(verdict)
                    )),
                },
                None => PermissionDecision::Ask {
                    reason: format!(
                        "automatic approval classifier blocked the action without naming a \
                         block rule; confirm it yourself: {}",
                        verdict_reason(verdict)
                    ),
                },
            },
            None => PermissionDecision::Ask {
                reason: format!(
                    "automatic approval unavailable: {}",
                    self.failure.as_ref().map_or_else(
                        || "the classifier produced no verdict".to_owned(),
                        ToString::to_string,
                    )
                ),
            },
        }
    }

    /// Why this outcome could not be auto-approved.
    ///
    /// Every outcome whose [`Self::decision`] is `Ask` carries a failure; the
    /// fallback is unreachable and exists only so callers never need an
    /// `unwrap` on a path that must not panic.
    pub fn failure(&self) -> crate::ClassifyFailure {
        self.failure.clone().unwrap_or_else(|| {
            crate::ClassifyFailure::UnparseableVerdict(self.candidate.policy_reason.clone())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Outcome of a synchronous permission decision.
pub enum SyncOutcome {
    Final(PermissionDecision),
    Classifier(ClassifierCandidate),
}

/// Decide what happens after the static policy layer returned `Auto`.
///
/// `base` must already be the static-policy decision composed with the
/// persisted rule snapshot; anything other than `Auto` is returned untouched.
/// An `Auto` decision goes to the approval model, unless the denial budget has
/// been exhausted, in which case automatic approval is switched off and the
/// user is asked instead.
pub fn decide_sync(
    base: &PermissionDecision,
    action: &ActionSpec,
    budget: &DenialBudget,
) -> SyncOutcome {
    match base {
        PermissionDecision::Allow
        | PermissionDecision::Ask { .. }
        | PermissionDecision::Deny { .. } => SyncOutcome::Final(base.clone()),
        PermissionDecision::Auto { reason } => {
            // Denial budget: stop burning model calls after repeated denials.
            if budget.exceeded() {
                return SyncOutcome::Final(PermissionDecision::Ask {
                    reason: "automatic approval disabled after repeated denials".to_owned(),
                });
            }
            SyncOutcome::Classifier(ClassifierCandidate {
                action: action.clone(),
                policy_reason: reason.clone(),
            })
        }
    }
}
