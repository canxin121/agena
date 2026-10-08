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

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::ActionSpec;

    fn tool(name: &str, command: Option<&str>) -> ActionSpec {
        ActionSpec::Tool {
            tool_name: name.to_owned(),
            command: command.map(ToOwned::to_owned),
        }
    }

    fn auto(reason: &str) -> PermissionDecision {
        PermissionDecision::Auto {
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn final_decisions_do_not_reenter_the_pipeline() {
        let budget = DenialBudget::default();
        for base in [
            PermissionDecision::Allow,
            PermissionDecision::Ask {
                reason: "ask".into(),
            },
            PermissionDecision::Deny {
                reason: "deny".into(),
            },
        ] {
            assert_eq!(
                decide_sync(&base, &tool("fs.write", None), &budget),
                SyncOutcome::Final(base)
            );
        }
    }

    #[test]
    fn auto_always_reaches_the_classifier() {
        // Every action whose static policy verdict is `Auto` goes to the model.
        // The class defaults that used to resolve these in-process are now static
        // policy rules, so they are decided before `decide_sync` is reached.
        let budget = DenialBudget::default();
        for action in [
            tool("mcp.read", None),
            tool("shell.exec", Some("rm -rf /")),
            tool("shell.exec", Some("cargo test")),
            tool("fs.write", None),
        ] {
            let outcome = decide_sync(&auto("auto"), &action, &budget);
            assert!(
                matches!(
                    outcome,
                    SyncOutcome::Classifier(ClassifierCandidate { ref policy_reason, .. })
                        if policy_reason == "auto"
                ),
                "{action:?} should reach the classifier, got {outcome:?}"
            );
        }
    }

    #[test]
    fn ambiguous_actions_become_classifier_candidates() {
        let budget = DenialBudget::default();
        let outcome = decide_sync(
            &auto("tool is eligible for automatic approval"),
            &tool("fs.write", None),
            &budget,
        );
        assert!(matches!(
            outcome,
            SyncOutcome::Classifier(ClassifierCandidate {
                policy_reason,
                ..
            }) if policy_reason == "tool is eligible for automatic approval"
        ));
    }

    #[test]
    fn exhausted_budget_asks_instead_of_classifying() {
        let mut budget = DenialBudget::default();
        budget.record_decision(false);
        budget.record_decision(false);
        budget.record_decision(false);
        let outcome = decide_sync(&auto("auto"), &tool("fs.write", None), &budget);
        assert!(matches!(
            outcome,
            SyncOutcome::Final(PermissionDecision::Ask { .. })
        ));
    }

    fn candidate(policy_reason: &str) -> ClassifierCandidate {
        ClassifierCandidate {
            action: tool("fs.write", None),
            policy_reason: policy_reason.to_owned(),
        }
    }

    fn verdict(text: &str) -> crate::ClassifierVerdict {
        crate::ClassifierVerdict::parse(text).expect("verdict")
    }

    #[test]
    fn an_allowed_verdict_allows() {
        let classified = ClassifiedCandidate {
            candidate: candidate("eligible"),
            verdict: Some(verdict(
                r#"{"shouldBlock":false,"reason":"routine test run"}"#,
            )),
            failure: None,
        };
        assert_eq!(classified.decision(), PermissionDecision::Allow);
    }

    #[test]
    fn a_cited_block_is_a_terminal_denial_naming_the_rule() {
        let classified = ClassifiedCandidate {
            candidate: candidate("eligible"),
            verdict: Some(verdict(
                r#"{"shouldBlock":true,"reason":"[Exfiltration] uploads the .env to a paste site."}"#,
            )),
            failure: None,
        };
        let PermissionDecision::Deny { reason } = classified.decision() else {
            panic!("a cited block must be a denial");
        };
        assert!(reason.contains("[Exfiltration]"));
        assert!(reason.contains("uploads the .env"));
    }

    #[test]
    fn an_uncited_block_asks_with_the_models_own_words() {
        let classified = ClassifiedCandidate {
            candidate: candidate("eligible"),
            verdict: Some(verdict(
                r#"{"shouldBlock":true,"reason":"This feels risky."}"#,
            )),
            failure: Some(crate::ClassifyFailure::UncitedBlock("risky".to_owned())),
        };
        let PermissionDecision::Ask { reason } = classified.decision() else {
            panic!("an uncited block must fall back to confirmation");
        };
        assert!(reason.contains("without naming a block rule"));
        assert!(reason.contains("This feels risky."));
    }

    #[test]
    fn an_empty_reason_falls_back_to_the_policy_reason() {
        // The user must never see a denial with no explanation at all.
        let classified = ClassifiedCandidate {
            candidate: candidate("tool is eligible for automatic approval"),
            verdict: Some(verdict(r#"{"shouldBlock":true,"reason":"   "}"#)),
            failure: Some(crate::ClassifyFailure::UncitedBlock("  ".to_owned())),
        };
        let PermissionDecision::Ask { reason } = classified.decision() else {
            panic!("expected a fallback ask");
        };
        assert!(reason.contains("tool is eligible for automatic approval"));
    }

    #[test]
    fn an_unresolved_candidate_asks_and_carries_its_failure() {
        for failure in [
            crate::ClassifyFailure::Timeout,
            crate::ClassifyFailure::EmptyResponse,
            crate::ClassifyFailure::Provider("connection reset".to_owned()),
            crate::ClassifyFailure::ApprovalModelUnavailable("no model".to_owned()),
            crate::ClassifyFailure::UnparseableVerdict("not json".to_owned()),
        ] {
            let classified = ClassifiedCandidate {
                candidate: candidate("eligible"),
                verdict: None,
                failure: Some(failure.clone()),
            };
            let PermissionDecision::Ask { reason } = classified.decision() else {
                panic!("{failure} must fall back to confirmation");
            };
            assert!(reason.contains("automatic approval unavailable"));
            assert_eq!(classified.failure(), failure);
        }
    }
}
