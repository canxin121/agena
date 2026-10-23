fn workflow_json_text<T: serde::Serialize>(value: &T, context: &'static str) -> String {
    match serde_json::to_string(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "workflow guidance could not serialize a JSON value"
            );
            "[value could not be serialized]".to_owned()
        }
    }
}

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
            "agena.tools" => {
                return matches!(input.tool_name(), "list" | "tags" | "help" | "call");
            }
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

    pub(crate) async fn invoke_tool_api_list(
        &self,
        input: &ToolApiListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        validate_plugin_selector(input.plugin.as_ref())?;
        let limit = discovery_limit(input.limit, config.tool_discovery.list.default_limit);
        let max_summary_chars = config.tool_discovery.list.max_summary_chars as usize;
        let records = Self::filter_available_tools_by_plugin(
            self.available_tool_records().await?,
            input.plugin.as_ref(),
        );
        let (tools, total, offset) = Self::paginate(&records, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available tools: returned {} of {} starting at offset {}.",
            tools.len(),
            total,
            offset
        )];
        for tool in &tools {
            let summary = compact_tool_summary(&tool.summary, max_summary_chars);
            let tags_part = tags_summary(tool.tags.as_slice());
            if summary.is_empty() {
                lines.push(format!(
                    "- {} [{}] ({})",
                    tool.name, tags_part, tool.plugin_id
                ));
            } else {
                lines.push(format!(
                    "- {} [{}] ({}): {}",
                    tool.name, tags_part, tool.plugin_id, summary
                ));
            }
        }
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::List.function_name(),
            total,
            offset,
            tools.len(),
        );
        Ok(discovery_text_output(
            format!("List tools · {}/{}", tools.len(), total),
            discovery_page_summary("tools", total, offset, tools.len()),
            lines.join("\n"),
        ))
    }

    /// Enumerate the current live plugin inventory for the plugins_* family.
    async fn available_plugin_records(&self) -> SdkResult<Vec<AvailablePluginRecord>> {
        Ok(self
            .host()?
            .list_plugins()
            .await?
            .plugins
            .into_iter()
            .map(|plugin| AvailablePluginRecord {
                plugin_id: plugin.plugin_id.to_string(),
                summary: plugin.summary.unwrap_or_default(),
                version: plugin.version,
                tags: plugin.tags,
                tools: plugin.tools,
            })
            .collect())
    }

    pub(crate) async fn invoke_plugins_list(
        &self,
        input: &ToolApiListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        validate_plugin_selector(input.plugin.as_ref())?;
        let limit = discovery_limit(input.limit, config.tool_discovery.list.default_limit);
        let max_summary_chars = config.tool_discovery.list.max_summary_chars as usize;
        let records = Self::filter_available_plugins_by_id(
            self.available_plugin_records().await?,
            input.plugin.as_ref(),
        );
        let (plugins, total, offset) = Self::paginate(&records, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available plugins: returned {} of {} starting at offset {}.",
            plugins.len(),
            total,
            offset
        )];
        for plugin in &plugins {
            let summary = compact_tool_summary(&plugin.summary, max_summary_chars);
            let tool_count = plugin.tools.len();
            if summary.is_empty() {
                lines.push(format!(
                    "- {} [{}] (v{}): {tool_count} tool(s)",
                    plugin.plugin_id,
                    tags_summary(&plugin.tags),
                    plugin.version
                ));
            } else {
                lines.push(format!(
                    "- {} [{}] (v{}): {} · {tool_count} tool(s)",
                    plugin.plugin_id,
                    tags_summary(&plugin.tags),
                    plugin.version,
                    summary
                ));
            }
        }
        lines.push(format!(
            "List a plugin's tools with `{}` and its `plugin` selector.",
            agena_domain::ToolApiFunction::List.function_name()
        ));
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::PluginsList.function_name(),
            total,
            offset,
            plugins.len(),
        );
        Ok(discovery_text_output(
            format!("List plugins · {}/{}", plugins.len(), total),
            discovery_page_summary("plugins", total, offset, plugins.len()),
            lines.join("\n"),
        ))
    }

    pub(crate) async fn invoke_tool_api_help(
        &self,
        input: &ToolApiHelpInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.as_slice();
        if requested.is_empty() {
            return Err(PluginError::invalid_params(
                "tools_help.tool must contain at least one execution-tool name",
            ));
        }
        for tool_name in requested {
            if tool_name.is_empty() {
                return Err(PluginError::invalid_params(
                    "tools_help.tool cannot contain an empty execution-tool name",
                ));
            }
            Self::ensure_execution_tool_target(tool_name)?;
        }
        let tools = self.host()?.list_tools().await?;
        let records = self.available_tool_records().await?;
        let shell_hint = Self::shell_discovery_hint(&records);
        let tag_index = records
            .iter()
            .map(|record| (record.name.clone(), record.tags.clone()))
            .collect::<HashMap<_, _>>();
        let mut outputs = Vec::new();
        let mut failures = 0usize;
        for name in requested {
            match Self::resolve_tool_descriptor(name, &tools) {
                Ok(descriptor) => outputs.push(Self::render_tool_api_help(
                    descriptor,
                    tag_index.get(descriptor.name.as_str()).map(Vec::as_slice),
                    shell_hint.as_deref(),
                )),
                Err(error) if requested.len() == 1 => return Err(error),
                Err(error) => {
                    failures += 1;
                    outputs.push(discovery_text_output(
                        format!("Unavailable tool: {name}"),
                        "Tool help failed",
                        format!(
                            "Tool: {name}\nHelp unavailable: {}",
                            error.failure.user.fallback
                        ),
                    ));
                }
            }
        }
        if let [output] = outputs.as_slice() {
            return Ok(output.clone());
        }
        Ok(discovery_text_output(
            format!("Inspect {} tools", outputs.len()),
            format!(
                "{} tool contracts returned · {failures} unavailable",
                outputs.len() - failures
            ),
            outputs
                .iter()
                .map(|output| output.output_text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n---\n\n"),
        ))
    }

    pub(in crate::plugins::provided::workflow) fn render_tool_api_help(
        descriptor: &ToolDescriptor,
        tags: Option<&[String]>,
        shell_hint: Option<&str>,
    ) -> ToolInvokeOutput {
        let mut lines = vec![format!("Tool: {}", descriptor.name)];
        if (descriptor.name.starts_with("shell.") || descriptor.name.starts_with("agena.shell."))
            && let Some(hint) = shell_hint
        {
            lines.push(hint.to_owned());
        }
        if let Some(tags) = tags.filter(|tags| !tags.is_empty()) {
            lines.push(format!("Tags: {}", tags.join(", ")));
        }
        lines.push("Usage:".to_string());
        if let Some(schema) = descriptor.input_schema.as_ref() {
            if let Some(arguments) =
                agena_runtime_tools::tool::definition::schema_usage_text(schema)
            {
                lines.push(arguments);
            } else {
                lines.push("- No input arguments.".to_string());
            }
        } else {
            lines.push("- No input arguments.".to_string());
        }
        let generated_examples = descriptor
            .input_schema
            .as_ref()
            .map(agena_runtime_tools::tool::definition::schema_example_texts)
            .unwrap_or_default();
        if !generated_examples.is_empty() {
            lines.push("Examples:".to_string());
            for example in &generated_examples {
                lines.push(format!("- {example}"));
            }
        }
        if let Some(help) = descriptor.help.as_deref().filter(|value| !value.is_empty()) {
            lines.push("Help:".to_string());
            lines.push(help.to_string());
        }
        let routing_input_example = generated_examples
            .iter()
            .find_map(|example| {
                serde_json::from_str::<serde_json::Value>(example)
                    .ok()
                    .filter(serde_json::Value::is_object)
            })
            .unwrap_or_else(|| serde_json::json!({}));
        let routing_arguments_example = serde_json::json!({
            "tool": descriptor.name,
            "input": routing_input_example,
        });
        lines.push("To run this execution tool:".to_string());
        lines.push(format!(
            "- `{}` is an execution-tool name, not a Tool API function name. Never use `{}` as the function name.",
            descriptor.name, descriptor.name,
        ));
        lines.push(format!(
            "- Call Tool API function `{}` with arguments shaped exactly like {}.",
            agena_runtime_tools::tool::tools_call_function_name(),
            workflow_json_text(
                &routing_arguments_example,
                "serialize a workflow execution-tool routing example",
            ),
        ));
        lines.push(
            "- Replace example placeholders with the user's exact task values. Make one complete call with every supplied key; never make a preliminary, empty, or default-input call."
                .to_string(),
        );
        lines.push(format!(
            "This help is reusable. Call Tool API function `{}` any number of times for execution tool `{}` with complete inputs; parallel calls in one turn are allowed.",
            agena_runtime_tools::tool::tools_call_function_name(),
            descriptor.name,
        ));

        discovery_text_output(
            format!("Inspect {}", descriptor.name),
            format!("Usage and input contract for {}.", descriptor.name),
            lines.join("\n"),
        )
    }

    pub(in crate::plugins::provided::workflow) fn ensure_execution_tool_target(
        requested: &str,
    ) -> SdkResult<()> {
        if let Some(api_function) = agena_domain::ToolApiFunction::from_function_name(requested) {
            return Err(PluginError::invalid_params(format!(
                "`{requested}` identifies protocol function `{}` and is not an execution tool; protocol functions cannot inspect or invoke themselves",
                api_function.function_name(),
            )));
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn suggest_tool_names(
        requested: &str,
        tools: &[ToolDescriptor],
    ) -> Vec<String> {
        let candidate_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        let mut suggestions =
            agena_runtime_tools::tool::suggest_tool_names(requested, candidate_names, 1);
        if suggestions.is_empty() {
            let documents = tools
                .iter()
                .map(|tool| AvailableToolRecord {
                    name: tool.name.clone(),
                    summary: tool.summary.clone().unwrap_or_default(),
                    tags: Vec::new(),
                    plugin_id: tool.plugin_id.clone().unwrap_or_default(),
                })
                .map(|record| Self::tool_search_document(&record))
                .collect::<Vec<_>>();
            for tool in search_tools(&documents, requested, 3) {
                if !tool.name.eq_ignore_ascii_case(requested) && !suggestions.contains(&tool.name) {
                    suggestions.push(tool.name);
                }
                if suggestions.len() >= 3 {
                    break;
                }
            }
        }
        suggestions
    }
}
use super::{
    AskUserRequest, AskUserToolInput, AvailablePluginRecord, AvailableToolRecord, BTreeMap,
    CommandBeforeInput, HashMap, PlanEditInput, PlanEditTarget, PlanGetInput, PlanPhaseInput,
    PlanReviewInput, PlanReviewKind, PlanSetInput, PluginError, SdkResult, ToolApiHelpInput,
    ToolApiListInput, ToolBeforeInput, ToolDescriptor, ToolInvokeOutput, ToolTag, WorkflowPlan,
    WorkflowPlanPhase, WorkflowPlanStep, WorkflowPlanStepStatus, WorkflowPlugin, ask_user,
    compact_tool_summary, search_tools, tags_summary,
};

fn validate_plugin_selector(plugin: Option<&super::ToolApiStringBatch>) -> SdkResult<()> {
    let Some(plugin) = plugin else {
        return Ok(());
    };
    if plugin.as_slice().is_empty() {
        return Err(PluginError::invalid_params(
            "plugin selector batch must contain at least one plugin id",
        ));
    }
    if plugin
        .as_slice()
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(PluginError::invalid_params(
            "plugin selectors must not contain empty plugin ids",
        ));
    }
    Ok(())
}

pub(in crate::plugins::provided::workflow) fn discovery_limit(
    requested: Option<u32>,
    default_limit: u32,
) -> u32 {
    requested.unwrap_or(default_limit).max(1)
}

fn append_discovery_page_hint(
    lines: &mut Vec<String>,
    function_name: &str,
    total: usize,
    offset: usize,
    returned: usize,
) {
    let next_offset = offset.saturating_add(returned);
    if returned > 0 && next_offset < total {
        lines.push(format!(
            "More available: yes. Continue with `{function_name}` using `offset: {next_offset}`."
        ));
    } else {
        lines.push("More available: no.".to_owned());
    }
}

fn discovery_page_summary(item_name: &str, total: usize, offset: usize, returned: usize) -> String {
    let next_offset = offset.saturating_add(returned);
    if returned > 0 && next_offset < total {
        format!("Returned {returned} of {total} {item_name}; continue at offset {next_offset}.")
    } else {
        format!("Returned {returned} of {total} {item_name}; no more results.")
    }
}

pub(super) fn discovery_text_output(
    title: impl Into<String>,
    summary: impl Into<String>,
    output_text: impl Into<String>,
) -> ToolInvokeOutput {
    ToolInvokeOutput::from_parts(
        title,
        summary,
        output_text,
        None,
        BTreeMap::new(),
        Vec::new(),
    )
}
