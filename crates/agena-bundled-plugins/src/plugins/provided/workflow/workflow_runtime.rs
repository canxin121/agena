impl WorkflowPlugin {
    pub(in crate::plugins::provided::workflow) fn plan_lock_active(plan: &WorkflowPlan) -> bool {
        plan.phase == WorkflowPlanPhase::Planning
    }

    pub(in crate::plugins::provided::workflow) fn tool_allowed_during_planning(
        input: &ToolBeforeInput,
    ) -> bool {
        match input.plugin_key().to_string().as_str() {
            "agena.plan" => {
                return matches!(
                    input.tool_name(),
                    "get" | "set" | "edit" | "phase" | "review" | "clear"
                );
            }
            "agena.session" => {
                return matches!(
                    input.tool_name(),
                    "get" | "environment" | "executables" | "model" | "tokens" | "rename"
                );
            }
            "agena.interaction" => return matches!(input.tool_name(), "ask" | "notify"),
            _ => {}
        }
        // Planning eligibility reads the tool's declared tags. They are
        // self-description: they decide what planning may run, not what the
        // host will authorize.
        let tags = input.tags.as_slice();
        if ToolTag::is_shell(tags) || ToolTag::is_read_only(tags) || ToolTag::is_interactive(tags) {
            return true;
        }
        tags.iter()
            .any(|tag| matches!(tag, ToolTag::Discovery | ToolTag::Planning))
    }

    pub(in crate::plugins::provided::workflow) fn is_probably_read_only_shell(
        command: &str,
    ) -> bool {
        command.trim().is_empty()
            || matches!(
                agena_tool::shell_analysis::analyze_command(command).classification,
                agena_tool::shell_analysis::CommandClassification::ReadOnly
            )
    }

    pub(in crate::plugins::provided::workflow) fn command_text_for_policy(
        input: &CommandBeforeInput,
    ) -> String {
        if input.command == "sh"
            && input.args.len() >= 2
            && input.args.first().is_some_and(|arg| arg == "-c")
        {
            return input.args[1].clone();
        }
        std::iter::once(input.command.as_str())
            .chain(input.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(in crate::plugins::provided::workflow) fn autorun_prompt(
        plan: &WorkflowPlan,
        step_index: usize,
        step: &WorkflowPlanStep,
        run_error: Option<&str>,
    ) -> String {
        let mut lines = vec![
            "<plan_context>".to_string(),
            "Continue the active approved plan.".to_string(),
            format!("Plan: {}", plan.title),
            format!("Objective: {}", plan.objective),
            format!("Current step {}: {}", step_index + 1, step.title),
        ];
        if let Some(run_error) = run_error {
            lines.push(format!("Previous run error: {run_error}"));
        }
        if !step.description.trim().is_empty() {
            lines.push(format!("Step details: {}", step.description.trim()));
        }
        let pending_checks = step
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                !matches!(
                    checkpoint.status,
                    WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
                )
            })
            .map(|checkpoint| format!("- {}", checkpoint.text))
            .collect::<Vec<_>>();
        if !pending_checks.is_empty() {
            lines.push("Pending checks:".to_string());
            lines.extend(pending_checks);
        }
        lines.push(
            "Update the plan state as you make progress: mark checks completed via `plan.edit` with `step` and `check` (1-based indices), mark steps complete when their checks are done, and move to the next step when this one is finished.".to_string(),
        );
        lines.push(
            "When the whole plan is finished, call `plan.phase` with `phase: \"completed\"` (or `\"blocked\"`/`\"cancelled\"` as appropriate) so autorun stops cleanly.".to_string(),
        );
        lines.push(
            "If the next step needs human input or cannot be advanced, stop and say exactly what is needed.".to_string(),
        );
        lines.push("</plan_context>".to_string());
        lines.join("\n")
    }

    pub(crate) async fn invoke_plan_get(
        &self,
        input: &PlanGetInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let view = input.view;
        let Some(plan) = self.load_active_plan().await? else {
            // No active plan for this session: clear any stale display
            // contribution so the composer chip never keeps showing a plan
            // that no longer exists.
            if let Err(error) = self.sync_plan_display(None).await {
                tracing::warn!(
                    target: "agena::workflow",
                    diagnostic = %error.diagnostic_message(),
                    "stale plan display could not be cleared while reading the plan"
                );
            }
            let payload = serde_json::json!({
                "plan": serde_json::Value::Null,
                "view": view,
                "current_step": serde_json::Value::Null,
                "current_step_goal": serde_json::Value::Null,
            });
            return Ok(ToolInvokeOutput::from_parts(
                "plan",
                "No active plan",
                "No plan.",
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ));
        };
        // Re-publish the plan display contribution from durable storage. The
        // contribution is held in memory on the plugin host and starts empty
        // after a process restart or runtime reload; any read of the plan
        // restores the composer's bottom-right progress chip without mutating
        // the plan. Cosmetic only: a failure here must not fail the read.
        if let Err(error) = self.sync_plan_display(Some(&plan)).await {
            tracing::warn!(
                target: "agena::workflow",
                plan = %plan.title,
                diagnostic = %error.diagnostic_message(),
                "plan display sync failed during plan.get"
            );
        }
        let payload = Self::plan_get_payload(&plan, view);
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            format!("{:?} · {} steps", plan.phase, plan.steps.len()),
            Self::plan_get_text(&plan, view),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_plan_set(
        &self,
        input: &PlanSetInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.require_activation_grant(input.request_approval)?;
        let previous = self.load_active_plan().await?;
        Self::require_expected_plan(previous.as_ref(), input.expected_revision.as_deref())?;
        let expected = previous.as_ref().map(|plan| plan.revision.clone());
        let mut plan = self.build_plan(
            input.objective.as_str(),
            input.title.as_deref(),
            input.document_markdown.as_deref(),
            input.steps.as_slice(),
            input.autorun,
            previous.as_ref(),
        )?;
        if input.request_approval.unwrap_or(true) {
            // Default: save the plan in `planning` without blocking on the
            // user. This tool never triggers review by itself; the agent must
            // call `plan.review` to request user approval before the plan can
            // become active.
            self.save_plan_version(&mut plan, expected).await?;
            let payload = Self::plan_payload(&plan)?;
            return Ok(ToolInvokeOutput::from_parts(
                "plan",
                format!("Saved · {:?} · {} steps", plan.phase, plan.steps.len()),
                format!(
                    "{}\n\nThe plan is saved in the `planning` phase. Call `plan.review` to request user approval before it becomes active.",
                    Self::plan_output_text("Saved the plan.", &plan)
                ),
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ));
        }
        // request_approval: false — the user has already declared the plan
        // needs no approval, so start immediately: transition straight to
        // `active` (the plan already carries the resolved `autorun`) and
        // return without any user interaction.
        Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Active, None)?;
        self.save_plan_version(&mut plan, expected).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            format!("Saved · {:?} · {} steps", plan.phase, plan.steps.len()),
            Self::plan_output_text("Saved the plan and made it active.", &plan),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_plan_edit(
        &self,
        input: &PlanEditInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params(
                "no plan exists in this session; create one with plan.set first",
            ));
        };
        Self::require_expected_plan(Some(&plan), input.expected_revision.as_deref())?;
        let target = Self::validate_plan_edit_input(input)?;
        let message = match target {
            PlanEditTarget::Step(step) => {
                let step_index = Self::resolve_step_index(&plan, step).map_err(|err| {
                    PluginError::invalid_params(format!(
                        "{}; available steps: {}",
                        err.diagnostic.message,
                        Self::step_listing(&plan)
                    ))
                })?;
                let step = &mut plan.steps[step_index];
                if let Some(status) = input.status {
                    step.status = status;
                    Self::cascade_terminal_step_status(step, status);
                }
                if let Some(note) = input.note.as_deref() {
                    step.note = note.to_string();
                }
                format!("Updated step '{}'.", step.title)
            }
            PlanEditTarget::Check {
                step_index,
                check_index,
            } => {
                let step_index = Self::resolve_step_index(&plan, step_index).map_err(|err| {
                    PluginError::invalid_params(format!(
                        "{}; available steps: {}",
                        err.diagnostic.message,
                        Self::step_listing(&plan)
                    ))
                })?;
                let step = &mut plan.steps[step_index];
                let checkpoint_text = {
                    let check_index =
                        Self::resolve_check_index(step, check_index).map_err(|err| {
                            PluginError::invalid_params(format!(
                                "{}; available checks: {}",
                                err.diagnostic.message,
                                Self::check_listing(step)
                            ))
                        })?;
                    let checkpoint = &mut step.checkpoints[check_index];
                    checkpoint.status = input
                        .status
                        .expect("validated plan check update requires status");
                    checkpoint.text.clone()
                };
                Self::reconcile_step_status_from_checkpoints(step);
                format!("Updated check '{checkpoint_text}'.")
            }
        };
        self.update_active_plan(&mut plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            format!("{} · {:?}", message.trim_end_matches('.'), plan.phase),
            Self::plan_output_text(message.as_str(), &plan),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_plan_phase(
        &self,
        input: &PlanPhaseInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params(
                "no plan exists in this session; create one with plan.set first",
            ));
        };
        self.require_activation_grant(input.request_approval)?;
        Self::require_expected_plan(Some(&plan), input.expected_revision.as_deref())?;
        Self::validate_plan_phase_input(input)?;
        let completion_summary = match input.phase {
            Some(WorkflowPlanPhase::Completed) => input.summary.as_deref(),
            _ => None,
        };
        if let Some(phase) = input.phase {
            Self::validate_plan_phase_change(&plan, phase)?;
            if Self::plan_phase_requires_approval(phase)
                && !Self::plan_phase_is_approved(plan.phase)
                && input.request_approval.unwrap_or(true)
            {
                return self
                    .review_plan_status_transition(
                        plan,
                        phase,
                        input.autorun,
                        completion_summary,
                        PlanReviewKind::StatusChange,
                    )
                    .await;
            }
            Self::set_plan_phase(&mut plan, phase, completion_summary)?;
        }
        if let Some(autorun) = input.autorun {
            plan.autorun = autorun;
        }
        self.update_active_plan(&mut plan).await?;
        let payload = Self::plan_payload(&plan)?;
        let message = "Updated the plan phase.";
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            format!("{} · {:?}", message.trim_end_matches('.'), plan.phase),
            Self::plan_output_text(message, &plan),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_plan_review(
        &self,
        _input: &PlanReviewInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params(
                "no plan exists in this session; create one with plan.set, then call plan.review",
            ));
        };
        // `plan.review` requests user approval to move the current saved plan
        // from `planning` to `active` (creation review). The plan's own state
        // drives the dialog: the requested phase is Active, autorun defaults to
        // the plan's current value, and no completion summary applies.
        self.review_plan_status_transition(
            plan,
            WorkflowPlanPhase::Active,
            None,
            None,
            PlanReviewKind::Creation,
        )
        .await
    }

    pub(crate) async fn invoke_plan_clear(&self) -> SdkResult<ToolInvokeOutput> {
        let existing = self.load_active_plan().await?;
        let warning = self
            .clear_plan_version(existing.as_ref().map(|plan| plan.revision.clone()))
            .await?;
        let payload = serde_json::json!({
            "cleared": existing.is_some(),
            "committed": true,
            "display_warning": warning,
        });
        let text = if existing.is_some() {
            "Cleared the active plan."
        } else {
            "No active plan to clear."
        };
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            text,
            warning
                .as_ref()
                .map(|warning| format!("{text}\nWarning: {warning}"))
                .unwrap_or_else(|| text.into()),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_ask_user(
        &self,
        input: &AskUserToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        ask_user::validate(input).map_err(|err| PluginError::invalid_params_error(&err))?;
        let host = self.host()?;
        let response = host
            .ask_user(AskUserRequest {
                title: input.title.clone(),
                kind: input.kind.clone(),
                body_markdown: input.body_markdown.clone(),
                auto_resolution_ms: input.auto_resolution_ms,
                questions: Self::host_ask_user_questions(input),
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        if response.timed_out {
            let execution = ask_user::execution_from_timeout(input);
            return Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(execution),
            );
        }
        if response.cancelled {
            let reason = if response.reply.trim().is_empty() {
                "user declined to answer requested questions".to_string()
            } else {
                response.reply
            };
            return Err(PluginError::internal(reason));
        }

        let mut answers = response.answers;
        if answers.is_empty()
            && let Some(_question) = input.questions.first()
            && !response.reply.trim().is_empty()
        {
            answers.insert("0".to_string(), vec![response.reply]);
        }

        let execution = ask_user::execution_from_answers(input, answers);
        Ok(crate::plugins::provided::router::tool_execution_to_invoke_output(execution))
    }
}
use super::{
    AskUserRequest, AskUserToolInput, CommandBeforeInput, PlanEditInput, PlanEditTarget,
    PlanGetInput, PlanPhaseInput, PlanReviewInput, PlanReviewKind, PlanSetInput, PluginError,
    SdkResult, ToolBeforeInput, ToolInvokeOutput, ToolTag, WorkflowPlan, WorkflowPlanPhase,
    WorkflowPlanStep, WorkflowPlanStepStatus, WorkflowPlugin, ask_user,
};
