//! The synchronous permission decision pipeline.
//!
//! The host composes the static policy decision with the rule snapshot and
//! hands the result to [`decide_sync`] together with the action. The pipeline
//! walks the automatic-approval layers that do not need a model call and
//! returns either a final verdict or a classifier candidate for the host to
//! evaluate asynchronously.

use agena_domain::{ActionSpec, PermissionDecision};

use crate::auto::{DenialBudget, auto_fast_path, heuristic_decision};

/// Static context for one synchronous decision.
#[derive(Debug, Clone, Default)]
/// Context of a permission decision.
pub struct DecisionContext<'a> {
    /// Runtime-owned project-state directory; writes inside it are safe.
    pub managed_project_root: Option<&'a str>,
}

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

/// Walk every synchronous layer. `base` must already be the static-policy
/// decision composed with the persisted rule snapshot.
pub fn decide_sync(
    base: &PermissionDecision,
    action: &ActionSpec,
    context: &DecisionContext,
    budget: &DenialBudget,
) -> SyncOutcome {
    match base {
        PermissionDecision::Allow
        | PermissionDecision::Ask { .. }
        | PermissionDecision::Deny { .. } => SyncOutcome::Final(base.clone()),
        PermissionDecision::Auto { reason } => {
            // Fast path.
            match auto_fast_path(action, context.managed_project_root) {
                crate::auto::AutoFastPath::Allow => {
                    return SyncOutcome::Final(PermissionDecision::Allow);
                }
                crate::auto::AutoFastPath::Ask { reason } => {
                    return SyncOutcome::Final(PermissionDecision::Ask { reason });
                }
                crate::auto::AutoFastPath::Defer => {}
            }
            // Heuristics.
            if let Some(decision) = heuristic_decision(action) {
                return SyncOutcome::Final(decision);
            }
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

    fn tool(name: &str, tags: &[&str], command: Option<&str>) -> ActionSpec {
        let mut contract = agena_domain::ToolPermissionContract::default();
        for tag in tags {
            match *tag {
                "read_only" => contract.read_only = true,
                "filesystem_read" | "filesystem_write" => {
                    contract.input_paths.push(agena_domain::InputPathSpec {
                        jsonpath: "$.path".to_owned(),
                        kind: if *tag == "filesystem_write" {
                            agena_domain::PathKind::Write
                        } else {
                            agena_domain::PathKind::Read
                        },
                        fallback: None,
                        optional: false,
                    });
                }
                "shell" => contract.shell = true,
                _ => {}
            }
        }
        ActionSpec::Tool {
            tool_name: name.to_owned(),
            contract,
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
        let context = DecisionContext::default();
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
                decide_sync(&base, &tool("fs.write", &[], None), &context, &budget),
                SyncOutcome::Final(base)
            );
        }
    }

    #[test]
    fn fast_path_and_heuristics_terminate_auto() {
        let context = DecisionContext::default();
        let budget = DenialBudget::default();
        assert_eq!(
            decide_sync(
                &auto("auto"),
                &tool("mcp.read", &["read_only", "filesystem_read"], None),
                &context,
                &budget
            ),
            SyncOutcome::Final(PermissionDecision::Allow)
        );
        let outcome = decide_sync(
            &auto("auto"),
            &tool("shell.run", &["shell"], Some("rm -rf /")),
            &context,
            &budget,
        );
        assert!(matches!(
            outcome,
            SyncOutcome::Final(PermissionDecision::Deny { reason })
                if reason.starts_with(
                    "automatic approval heuristic blocked a dangerous shell command"
                )
        ));
    }

    #[test]
    fn ambiguous_actions_become_classifier_candidates() {
        let context = DecisionContext::default();
        let budget = DenialBudget::default();
        let outcome = decide_sync(
            &auto("tool is eligible for automatic approval"),
            &tool("fs.write", &["filesystem_write"], None),
            &context,
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
        let context = DecisionContext::default();
        let mut budget = DenialBudget::default();
        budget.record_decision(false);
        budget.record_decision(false);
        budget.record_decision(false);
        let outcome = decide_sync(
            &auto("auto"),
            &tool("fs.write", &["filesystem_write"], None),
            &context,
            &budget,
        );
        assert!(matches!(
            outcome,
            SyncOutcome::Final(PermissionDecision::Ask { .. })
        ));
    }

    fn candidate(policy_reason: &str) -> ClassifierCandidate {
        ClassifierCandidate {
            action: tool("fs.write", &["filesystem_write"], None),
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
