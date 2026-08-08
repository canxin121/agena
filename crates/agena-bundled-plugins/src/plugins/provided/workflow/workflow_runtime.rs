impl WorkflowPlugin {
    pub(in crate::plugins::provided::workflow) fn plan_lock_active(plan: &WorkflowPlan) -> bool {
        plan.phase == WorkflowPlanPhase::Planning
    }

    pub(in crate::plugins::provided::workflow) fn tool_allowed_during_planning(
        input: &ToolBeforeInput,
    ) -> bool {
        match input.plugin_key().to_string().as_str() {
            "agena.plan" => return matches!(input.tool_name(), "get" | "set" | "update" | "clear"),
            "agena.session" => return matches!(input.tool_name(), "get" | "rename"),
            "agena.interaction" => return matches!(input.tool_name(), "ask" | "notify"),
            "agena.tools" => {
                return matches!(
                    input.tool_name(),
                    "list" | "search" | "tags" | "help" | "call"
                );
            }
            "agena.tasks" if input.tool_name() == "run" => {
                return TaskToolInput::parse_input(input.input.clone())
                    .is_ok_and(|task| task.access == TaskAccess::ReadOnly);
            }
            _ => {}
        }
        // Planning eligibility reads the permission contract, never tags:
        // tags are metadata for discovery/UI and carry no authority.
        if input.contract.shell {
            return true;
        }
        if input.contract.read_only && !input.contract.shell && !input.contract.interactive {
            return true;
        }
        if input.contract.interactive {
            return true;
        }
        input.tags.iter().any(|tag| {
            matches!(
                tag,
                ToolTag::Discovery | ToolTag::Planning | ToolTag::Snapshot
            )
        })
    }

    pub(in crate::plugins::provided::workflow) fn is_probably_read_only_shell(
        command: &str,
    ) -> bool {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return true;
        }
        if trimmed.contains('>')
            || trimmed.contains(">>")
            || trimmed.contains("<<")
            || trimmed.contains("rm ")
            || trimmed.contains("mv ")
            || trimmed.contains("cp ")
            || trimmed.contains("chmod ")
            || trimmed.contains("chown ")
            || trimmed.contains("touch ")
            || trimmed.contains(';')
            || trimmed.contains("&&")
            || trimmed.contains("||")
        {
            return false;
        }
        let Some(tokens) = shlex::split(trimmed) else {
            return false;
        };
        let Some(command_name) = tokens.first().map(String::as_str) else {
            return true;
        };
        match command_name {
            "cat" | "sed" | "grep" | "rg" | "ls" | "find" | "pwd" | "head" | "tail" | "wc"
            | "stat" | "tree" | "readlink" | "realpath" | "file" | "echo" => true,
            "git" => matches!(
                tokens.get(1).map(String::as_str),
                Some(
                    "status"
                        | "diff"
                        | "show"
                        | "log"
                        | "branch"
                        | "rev-parse"
                        | "remote"
                        | "ls-files"
                        | "grep"
                )
            ),
            _ => false,
        }
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
            "Update the plan state as you make progress: mark checks completed via `plan.update` with `step` and `check` (1-based indices), mark steps complete when their checks are done, and move to the next step when this one is finished.".to_string(),
        );
        lines.push(
            "When the whole plan is finished, call `plan.update` with `phase: \"completed\"` (or `\"blocked\"`/`\"cancelled\"` as appropriate) so autorun stops cleanly.".to_string(),
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
            let _ = self.sync_plan_display(None).await;
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
                "plan display sync failed during plan.get: {error}"
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
        let previous = self.load_active_plan().await?;
        let plan = self.build_plan(
            input.objective.as_str(),
            input.title.as_deref(),
            input.document_markdown.as_deref(),
            input.steps.as_slice(),
            input.autorun,
            previous.as_ref(),
        )?;
        self.save_active_plan(&plan).await?;
        if input.request_approval.unwrap_or(true) {
            // By default a new plan must pass through user approval before it
            // may become active; the agent can opt out explicitly when the user
            // has already declared that the plan needs no approval. The review
            // keeps the plan in planning when the user rejects or leaves
            // feedback, so the agent can revise it.
            return self
                .review_plan_status_transition(
                    plan,
                    WorkflowPlanPhase::Active,
                    input.autorun,
                    None,
                    PlanReviewKind::Creation,
                )
                .await;
        }
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            format!("Saved · {:?} · {} steps", plan.phase, plan.steps.len()),
            Self::plan_output_text("Saved the plan.", &plan),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_plan_update(
        &self,
        input: &PlanUpdateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        let target = Self::validate_plan_update_input(input)?;
        let message = match target {
            PlanUpdateTarget::Plan => {
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
                "Updated the plan.".to_string()
            }
            PlanUpdateTarget::Step(step) => {
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
                if let Some(wait_until_ms) = input.wait_until_ms {
                    step.wait_until_ms = Some(wait_until_ms);
                }
                if let Some(note) = input.note.as_deref() {
                    step.note = note.to_string();
                }
                format!("Updated step '{}'.", step.title)
            }
            PlanUpdateTarget::Check {
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
        self.save_active_plan(&plan).await?;
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

    pub(crate) async fn invoke_plan_clear(&self) -> SdkResult<ToolInvokeOutput> {
        let existing = self.load_active_plan().await?;
        self.clear_active_plan().await?;
        let payload = serde_json::json!({
            "cleared": existing.is_some(),
        });
        let text = if existing.is_some() {
            "Cleared the active plan."
        } else {
            "No active plan to clear."
        };
        Ok(ToolInvokeOutput::from_parts(
            "plan",
            text,
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_snapshot_enter(
        &self,
        args: &EnterSnapshotCommandInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let request = match args {
            EnterSnapshotCommandInput::New { name } => HostEnterSnapshotRequest {
                name: name.clone(),
                path: None,
            },
            EnterSnapshotCommandInput::Existing { path } => HostEnterSnapshotRequest {
                name: None,
                path: Some(path.clone()),
            },
        };
        self.host()?.enter_snapshot(request).await
    }

    pub(crate) async fn invoke_snapshot_exit(
        &self,
        args: &ExitSnapshotCommandInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.host()?
            .exit_snapshot(HostExitSnapshotRequest {
                action: args.exit_action.to_string(),
                discard_changes: args.discard_changes,
            })
            .await
    }

    pub(crate) async fn permission_snapshot_enter(
        &self,
        args: &EnterSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        snapshot_enter_permission_paths(self.workspace_root()?, args)
    }

    pub(crate) async fn permission_snapshot_exit(
        &self,
        _args: &ExitSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(Vec::new())
    }

    pub(crate) async fn invoke_ask_user(
        &self,
        input: &AskUserToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        ask_user::validate(input).map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let host = self.host()?;
        let response = host
            .ask_user(AskUserRequest {
                title: input.title.clone(),
                body_markdown: input.body_markdown.clone(),
                kind: input.kind.clone(),
                submit_label: input.submit_label.clone(),
                cancel_label: input.cancel_label.clone(),
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
            && let Some(question) = input.questions.first()
            && !response.reply.trim().is_empty()
        {
            answers.insert(question.id.clone(), vec![response.reply]);
        }

        let execution = ask_user::execution_from_answers(input, answers);
        Ok(crate::plugins::provided::router::tool_execution_to_invoke_output(execution))
    }

    pub(crate) async fn invoke_task(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let response = host
            .run_subtask(RunSubtaskRequest {
                parent_session_id: None,
                access: match input.access {
                    TaskAccess::Inherit => RunSubtaskAccess::Inherit,
                    TaskAccess::ReadOnly => RunSubtaskAccess::ReadOnly,
                },
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                skills: input.skills.clone(),
                task_id: input.task_id.clone(),
                selection: input
                    .selection
                    .as_ref()
                    .map(|selection| RunSubtaskModelSelection {
                        provider: selection.provider.clone(),
                        adapter: selection.adapter.clone(),
                        model: selection.model.clone(),
                        thinking_mode: selection.thinking_mode.clone(),
                        speed_mode: selection.speed_mode.clone(),
                        verbosity: selection.verbosity.clone(),
                        parallel_tool_calls: selection.parallel_tool_calls,
                    }),
                timeout_ms: input.timeout_ms,
                max_tokens: input.max_tokens,
                max_cost_microusd: input.max_cost_microusd,
            })
            .await?;

        let status = match response.status {
            RunSubtaskStatus::Created => "created",
            RunSubtaskStatus::Running => "running",
            RunSubtaskStatus::Completed => "completed",
            RunSubtaskStatus::Failed => "failed",
            RunSubtaskStatus::Cancelled => "cancelled",
            RunSubtaskStatus::TimedOut => "timed_out",
            RunSubtaskStatus::Interrupted => "interrupted",
        };
        let failure_message = response
            .problem
            .as_ref()
            .map(|failure| failure.user.fallback.as_str());
        let output_text = match (&response.final_text, failure_message) {
            (final_text, Some(error)) if !error.trim().is_empty() => match final_text {
                Some(text) if !text.trim().is_empty() => format!(
                    "Subtask {} {status}: {error}\n\nLast assistant output:\n{text}",
                    response.task_id
                ),
                _ => format!("Subtask {} {status}: {error}", response.task_id),
            },
            (Some(text), _) if !text.trim().is_empty() => text.clone(),
            _ => format!(
                "Subtask {} finished with status {status}.",
                response.task_id
            ),
        };
        let access = match input.access {
            TaskAccess::Inherit => "inherit",
            TaskAccess::ReadOnly => "read_only",
        };
        let mut view =
            ToolExecutionView::simple(format!("Task {}", input.description), status, output_text);
        view.metadata
            .insert("description".to_string(), input.description.clone());
        view.metadata
            .insert("access".to_string(), access.to_string());
        view.metadata
            .insert("task_id".to_string(), response.task_id.clone());
        view.metadata
            .insert("session_id".to_string(), response.session_id.to_string());
        view.metadata
            .insert("status".to_string(), status.to_string());
        view.metadata
            .insert("resumed".to_string(), response.resumed.to_string());
        if let Some(model_provider_id) = response.model_provider_id.clone() {
            view.metadata
                .insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_adapter_id) = response.model_adapter_id.clone() {
            view.metadata
                .insert("model_adapter_id".to_string(), model_adapter_id);
        }
        if let Some(model_id) = response.model_id.clone() {
            view.metadata.insert("model_id".to_string(), model_id);
        }

        let output = ToolPayloadOutput::Task {
            task_id: response.task_id,
            session_id: response.session_id,
            parent_session_id: response.parent_session_id,
            access: access.to_string(),
            status: status.to_string(),
            resumed: response.resumed,
            final_text: response.final_text,
            model_feedback: response.model_feedback,
            model_provider_id: response.model_provider_id,
            model_adapter_id: response.model_adapter_id,
            model_id: response.model_id,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            reasoning_tokens: response.usage.reasoning_tokens,
            cache_write_tokens: response.usage.cache_write_tokens,
            cache_read_tokens: response.usage.cache_read_tokens,
            total_cost_microusd: (response.usage.total_cost.max(0.0) * 1_000_000.0).round() as u64,
        };
        Ok(
            crate::plugins::provided::router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(output, view),
            ),
        )
    }

    pub(crate) async fn invoke_tool_api_search(
        &self,
        input: &ToolApiSearchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
        let config = self.config()?;
        let max_query_length = config.tool_discovery.search.max_query_length as usize;
        if query.chars().count() > max_query_length {
            return Err(PluginError::invalid_params(format!(
                "search query is too long: {} characters (max {})",
                query.chars().count(),
                max_query_length
            )));
        }
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.search.default_limit)
            .clamp(1, config.tool_discovery.search.max_limit);
        let max_summary_chars = config.tool_discovery.search.max_summary_chars as usize;
        let records = Self::filter_available_tools_by_tag(
            Self::filter_available_tools_by_plugin(
                self.available_tool_records().await?,
                input.plugin.as_deref(),
            ),
            input.tag.as_deref(),
            input.tags.as_deref(),
        );
        let documents = records
            .iter()
            .map(Self::tool_search_document)
            .collect::<Vec<_>>();
        let results = search_tools(&documents, query, documents.len())
            .map_err(|err| PluginError::internal(format!("tool search failed: {err}")))?;
        let (results, total, offset) = Self::paginate(&results, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Matching tools for {}: returned {} of {} starting at offset {}.",
            serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_owned()),
            results.len(),
            total,
            offset,
        )];
        for tool in &results {
            let summary = compact_tool_summary(&tool.description, max_summary_chars);
            if summary.is_empty() {
                lines.push(format!(
                    "- {} [{}]",
                    tool.name,
                    tags_summary(tool.tags.as_slice())
                ));
            } else {
                lines.push(format!(
                    "- {} [{}]: {}",
                    tool.name,
                    tags_summary(tool.tags.as_slice()),
                    summary
                ));
            }
        }
        if !results.is_empty() {
            lines.push(format!(
                "Call `{}` with an exact tool name for detailed usage.",
                agena_runtime_tools::tool::tools_help_function_name()
            ));
        }
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::Search.function_name(),
            total,
            offset,
            results.len(),
        );
        let title_query = compact_tool_summary(query, 80);
        Ok(discovery_text_output(
            format!(
                "Search tools · {} · {}/{}",
                title_query,
                results.len(),
                total
            ),
            discovery_page_summary("matching tools", total, offset, results.len()),
            lines.join("\n"),
        ))
    }

    pub(crate) async fn invoke_tool_api_list(
        &self,
        input: &ToolApiListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.list.default_limit)
            .clamp(1, config.tool_discovery.list.max_limit);
        let max_summary_chars = config.tool_discovery.list.max_summary_chars as usize;
        let records = Self::filter_available_tools_by_tag(
            Self::filter_available_tools_by_plugin(
                self.available_tool_records().await?,
                input.plugin.as_deref(),
            ),
            input.tag.as_deref(),
            input.tags.as_deref(),
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

    pub(crate) async fn invoke_tool_api_tags(
        &self,
        input: &ToolApiTagsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.tags.default_limit)
            .clamp(1, config.tool_discovery.tags.max_limit);
        let tags =
            Self::available_tool_tag_records(self.available_tool_records().await?.as_slice());
        let (tags, total, offset) = Self::paginate(&tags, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available tool tags: returned {} of {} starting at offset {}.",
            tags.len(),
            total,
            offset
        )];
        for tag in &tags {
            lines.push(format!("- {}: {}", tag.tag, tag.tool_count));
        }
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::Tags.function_name(),
            total,
            offset,
            tags.len(),
        );
        Ok(discovery_text_output(
            format!("List tool tags · {}/{}", tags.len(), total),
            discovery_page_summary("tool tags", total, offset, tags.len()),
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
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.list.default_limit)
            .clamp(1, config.tool_discovery.list.max_limit);
        let max_summary_chars = config.tool_discovery.list.max_summary_chars as usize;
        let records = Self::filter_available_plugins_by_tag(
            self.available_plugin_records().await?,
            input.tag.as_deref(),
            input.tags.as_deref(),
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
            if summary.is_empty() {
                lines.push(format!(
                    "- {} [{}]",
                    plugin.plugin_id,
                    tags_summary(&plugin.tags)
                ));
            } else {
                lines.push(format!(
                    "- {} [{}] (v{}): {} · tools: {}",
                    plugin.plugin_id,
                    tags_summary(&plugin.tags),
                    plugin.version,
                    summary,
                    plugin.tools.join(", ")
                ));
            }
        }
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

    pub(crate) async fn invoke_plugins_search(
        &self,
        input: &ToolApiSearchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
        let config = self.config()?;
        let max_query_length = config.tool_discovery.search.max_query_length as usize;
        if query.chars().count() > max_query_length {
            return Err(PluginError::invalid_params(format!(
                "search query is too long: {} characters (max {})",
                query.chars().count(),
                max_query_length
            )));
        }
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.search.default_limit)
            .clamp(1, config.tool_discovery.search.max_limit);
        let max_summary_chars = config.tool_discovery.search.max_summary_chars as usize;
        let records = Self::filter_available_plugins_by_tag(
            self.available_plugin_records().await?,
            input.tag.as_deref(),
            input.tags.as_deref(),
        );
        let documents = records
            .iter()
            .map(Self::plugin_search_document)
            .collect::<Vec<_>>();
        let results = search_tools(&documents, query, documents.len())
            .map_err(|err| PluginError::internal(format!("plugin search failed: {err}")))?;
        let (results, total, offset) = Self::paginate(&results, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Matching plugins for {}: returned {} of {} starting at offset {}.",
            serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_owned()),
            results.len(),
            total,
            offset,
        )];
        for result in &results {
            let summary = compact_tool_summary(&result.description, max_summary_chars);
            lines.push(format!(
                "- {} [{}]: {}",
                result.name,
                tags_summary(&result.tags),
                summary
            ));
        }
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::PluginsSearch.function_name(),
            total,
            offset,
            results.len(),
        );
        Ok(discovery_text_output(
            format!("Search plugins · {}/{}", results.len(), total),
            discovery_page_summary("matching plugins", total, offset, results.len()),
            lines.join("\n"),
        ))
    }

    pub(crate) async fn invoke_plugins_tags(
        &self,
        input: &ToolApiTagsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.tags.default_limit)
            .clamp(1, config.tool_discovery.tags.max_limit);
        let records = self.available_plugin_records().await?;
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for plugin in &records {
            for tag in &plugin.tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        let tags = counts
            .into_iter()
            .map(|(tag, plugin_count)| ToolTagRecord {
                tag,
                tool_count: 0,
                plugin_count: Some(plugin_count),
            })
            .collect::<Vec<_>>();
        let (tags, total, offset) = Self::paginate(&tags, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available plugin tags: returned {} of {} starting at offset {}.",
            tags.len(),
            total,
            offset
        )];
        for tag in &tags {
            lines.push(format!(
                "- {}: {}",
                tag.tag,
                tag.plugin_count.unwrap_or(tag.tool_count)
            ));
        }
        append_discovery_page_hint(
            &mut lines,
            agena_domain::ToolApiFunction::PluginsTags.function_name(),
            total,
            offset,
            tags.len(),
        );
        Ok(discovery_text_output(
            format!("List plugin tags · {}/{}", tags.len(), total),
            discovery_page_summary("plugin tags", total, offset, tags.len()),
            lines.join("\n"),
        ))
    }

    pub(crate) async fn invoke_tool_api_help(
        &self,
        input: &ToolApiHelpInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.as_str();
        Self::ensure_execution_tool_target(requested)?;
        let tools = self.host()?.list_tools().await?;
        let tag_index = self
            .available_tool_records()
            .await?
            .into_iter()
            .map(|record| (record.name, record.tags))
            .collect::<HashMap<_, _>>();
        let descriptor = Self::resolve_tool_descriptor(requested, &tools)?;
        Ok(Self::render_tool_api_help(
            descriptor,
            tag_index.get(descriptor.name.as_str()).map(Vec::as_slice),
        ))
    }

    pub(in crate::plugins::provided::workflow) fn render_tool_api_help(
        descriptor: &ToolDescriptor,
        tags: Option<&[String]>,
    ) -> ToolInvokeOutput {
        let mut lines = vec![format!("Tool: {}", descriptor.name)];
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
        let declared_examples = descriptor.examples.clone();
        let generated_examples = descriptor
            .input_schema
            .as_ref()
            .map(agena_runtime_tools::tool::definition::schema_example_texts)
            .unwrap_or_default();
        if !declared_examples.is_empty() || !generated_examples.is_empty() {
            lines.push("Examples:".to_string());
            let mut seen_examples = HashSet::new();
            if !declared_examples.is_empty() {
                lines.push("Declared examples:".to_string());
                for example in &declared_examples {
                    if seen_examples.insert(example.clone()) {
                        lines.push(format!("- {example}"));
                    }
                }
            }
            if !generated_examples.is_empty() {
                lines.push("Generated examples:".to_string());
                for example in &generated_examples {
                    if seen_examples.insert(example.clone()) {
                        lines.push(format!("- {example}"));
                    }
                }
            }
        }
        if let Some(help) = descriptor.help.as_deref().filter(|value| !value.is_empty()) {
            lines.push("Help:".to_string());
            lines.push(help.to_string());
        }
        let routing_input_example = generated_examples
            .iter()
            .chain(declared_examples.iter())
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
            serde_json::to_string(&routing_arguments_example).unwrap_or_else(|_| "{}".to_owned()),
        ));
        lines.push(
            "- Replace example placeholders with the user's exact task values. Make one complete call with every supplied key; never make a preliminary, empty, or default-input call."
                .to_string(),
        );
        lines.push(format!(
            "This help is reusable. Call Tool API function `{}` any number of times for execution tool `{}` with complete inputs; parallel calls are allowed when the tool is concurrency-safe.",
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
            if let Ok(results) = search_tools(&documents, requested, 3) {
                for tool in results {
                    if !tool.name.eq_ignore_ascii_case(requested)
                        && !suggestions.contains(&tool.name)
                    {
                        suggestions.push(tool.name);
                    }
                    if suggestions.len() >= 3 {
                        break;
                    }
                }
            }
        }
        suggestions
    }
}
use super::{
    AskUserRequest, AskUserToolInput, AvailablePluginRecord, AvailableToolRecord, BTreeMap,
    CommandBeforeInput, EnterSnapshotCommandInput, ExitSnapshotCommandInput, HashMap, HashSet,
    HostEnterSnapshotRequest, HostExitSnapshotRequest, PathRequest, PlanGetInput, PlanReviewKind,
    PlanSetInput, PlanUpdateInput, PlanUpdateTarget, PluginError, RunSubtaskAccess,
    RunSubtaskModelSelection, RunSubtaskRequest, RunSubtaskStatus, SdkResult, TaskAccess,
    TaskToolInput, ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput,
    ToolBeforeInput, ToolDescriptor, ToolExecutionView, ToolInvokeOutput, ToolPayloadExecution,
    ToolPayloadOutput, ToolTag, ToolTagRecord, WorkflowPlan, WorkflowPlanPhase, WorkflowPlanStep,
    WorkflowPlanStepStatus, WorkflowPlugin, ask_user, compact_tool_summary, search_tools,
    snapshot_enter_permission_paths, tags_summary,
};

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
