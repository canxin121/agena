//! `enter_plan_mode` / `exit_plan_mode` plugin tools.
//!
//! Plan mode pattern (mirrors claude-code's EnterPlanMode/ExitPlanMode):
//!
//! 1. The model invokes `enter_plan_mode` to declare it's about to draft a
//!    plan.  The executor allocates a fresh markdown file at
//!    `<workspace>/.agena/plans/<slug>.md`, records the path on the
//!    session, and the tool returns the path so the model knows where to
//!    write its plan.
//!
//! 2. While plan mode is active, tools follow one uniform rule:
//!    `ReadOnly` tools are allowed, shell tools are allowed only when the
//!    command itself is classified as read-only, and everything else is
//!    refused. This is enforced in `ToolExecutor::enforce_plan_mode_for`.
//!
//! 3. The model writes the plan to the file (via apply_patch is fine
//!    since plan files live under workspace) and then calls
//!    `exit_plan_mode`.  ExitPlanMode does NOT auto-approve; it surfaces
//!    a permission ask so the human gets to read the plan first.  On
//!    approval, plan mode is cleared and subsequent mutating calls are
//!    allowed again.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::message::{EnterPlanModeToolInput, ExitPlanModeToolInput};
use crate::session::PlanState;

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

/// Process-wide plan state — keyed by session id.  Conceptually this
/// belongs in `SessionRuntimeState`, but plan-mode checks happen inside
/// the sync `ToolExecutor` path which doesn't have a session handle.  A
/// shared map is the pragmatic place; entries are cleaned up on exit.
pub type PlanRegistry = Arc<RwLock<std::collections::HashMap<i64, PlanState>>>;

pub fn registry_for_executor() -> PlanRegistry {
    Arc::new(RwLock::new(std::collections::HashMap::new()))
}

pub(super) fn execute_enter(
    executor: &ToolExecutor,
    _input: &EnterPlanModeToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("enter_plan_mode: no session in execution context".to_string())
    })?;
    let registry = executor
        .plan_registry()
        .ok_or_else(|| ToolError::Plugin("enter_plan_mode: registry not configured".to_string()))?;

    let slug = generate_slug(session_id);
    let plans_dir = executor.workspace_root().join(".agena").join("plans");
    executor.ensure_edit_permission(&plans_dir)?;
    if let Err(e) = std::fs::create_dir_all(&plans_dir) {
        return Err(ToolError::Plugin(format!(
            "enter_plan_mode: failed to create plans dir {plans_dir:?}: {e}"
        )));
    }
    let file_path: PathBuf = plans_dir.join(format!("{slug}.md"));
    executor.ensure_edit_permission(&file_path)?;
    if !file_path.exists()
        && let Err(e) = std::fs::write(&file_path, "# Plan\n\n_(write your plan here)_\n")
    {
        return Err(ToolError::Plugin(format!(
            "enter_plan_mode: failed to seed plan file: {e}"
        )));
    }

    let state = PlanState {
        file_path: file_path.clone(),
        slug: slug.clone(),
        started_at: chrono::Utc::now(),
    };
    registry.write().insert(session_id, state);

    let view = ToolExecutionView::simple(
        format!("Plan mode entered ({slug})"),
        format!(
            "Plan mode is now ON.  Write your full plan to:\n  {}\n\n\
             While plan mode is active, mutating tools (Bash that mutates, \
             Edit, Write, ApplyPatch) are blocked.  When the plan is \
             complete call `exit_plan_mode` to ask the user to approve it.",
            file_path.display()
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::EnterPlanMode {
            plan_path: file_path.to_string_lossy().to_string(),
            slug,
        },
        view,
    ))
}

pub(super) fn execute_exit(
    executor: &ToolExecutor,
    _input: &ExitPlanModeToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("exit_plan_mode: no session in execution context".to_string())
    })?;
    let registry = executor
        .plan_registry()
        .ok_or_else(|| ToolError::Plugin("exit_plan_mode: registry not configured".to_string()))?;

    let state = registry.write().remove(&session_id).ok_or_else(|| {
        ToolError::Plugin("exit_plan_mode: not currently in plan mode".to_string())
    })?;

    let plan_path = state.file_path.to_string_lossy().to_string();
    let view = ToolExecutionView::simple(
        "Plan mode exited",
        format!(
            "Plan mode OFF.  Plan stayed at:\n  {plan_path}\n\n\
             Mutating tools are unblocked again."
        ),
    );

    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::ExitPlanMode {
            approved: true,
            plan_path,
        },
        view,
    ))
}

/// Returns true when the executor is currently in plan mode for this
/// session id (or when session id is unknown — be safe and refuse).
#[allow(dead_code)]
pub fn is_active(executor: &ToolExecutor, session_id: Option<i64>) -> bool {
    let Some(registry) = executor.plan_registry() else {
        return false;
    };
    match session_id {
        Some(id) => registry.read().contains_key(&id),
        None => false,
    }
}

fn generate_slug(session_id: i64) -> String {
    // Cheap, stable-ish slug — timestamp + session id, suffixed with a
    // word so paths are easy to read.
    let now = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let words = [
        "alpaca", "blossom", "comet", "dahlia", "ember", "frost", "glade", "harbor", "indigo",
        "juniper",
    ];
    let pick = words[(session_id.unsigned_abs() as usize) % words.len()];
    format!("{now}-{pick}")
}
