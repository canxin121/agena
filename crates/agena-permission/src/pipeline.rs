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
pub struct DecisionContext<'a> {
    /// Runtime-owned project-state directory; writes inside it are safe.
    pub managed_project_root: Option<&'a str>,
}

/// One classifier evaluation unit. The host groups candidates from the same
/// tool invocation so a single transcript/model setup serves them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierCandidate {
    pub action: ActionSpec,
    pub policy_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
                    reason: format!(
                        "automatic approval denial budget exceeded; falling back to confirmation: {reason}"
                    ),
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
}
