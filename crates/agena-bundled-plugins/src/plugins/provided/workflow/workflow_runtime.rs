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
        if input.tags.iter().any(|tag| matches!(tag, ToolTag::Shell)) {
            return true;
        }
        if input.tags.iter().any(|tag| {
            matches!(
                tag,
                ToolTag::Mutating | ToolTag::FilesystemWrite | ToolTag::Snapshot
            )
        }) {
            return false;
        }
        input.tags.iter().any(|tag| {
            matches!(
                tag,
                ToolTag::ReadOnly | ToolTag::Discovery | ToolTag::Interactive | ToolTag::Planning
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
    ) -> String {
        let mut lines = vec![
            "<plan_context>".to_string(),
            "Continue the active approved plan.".to_string(),
            format!("Plan: {}", plan.title),
            format!("Objective: {}", plan.objective),
            format!("Current step {}: {}", step_index + 1, step.title),
        ];
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
            "Update the plan state as you make progress. If the next step needs human input, stop and say exactly what is needed.".to_string(),
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
            let payload = serde_json::json!({
                "plan": serde_json::Value::Null,
                "view": view,
                "current_step": serde_json::Value::Null,
                "current_step_goal": serde_json::Value::Null,
            });
            return Ok(ToolInvokeOutput::from_parts(
                "plan",
                "No plan.",
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ));
        };
        let payload = Self::plan_get_payload(&plan, view);
        Ok(ToolInvokeOutput::from_parts(
            "plan",
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
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::from_parts(
            "plan",
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
                let allow_direct_approval = self.config()?.plan.allow_direct_approval;
                if let Some(phase) = input.phase {
                    Self::validate_plan_phase_change(&plan, phase)?;
                    if Self::plan_phase_requires_approval(phase)
                        && !Self::plan_phase_is_approved(plan.phase)
                        && !allow_direct_approval
                    {
                        return self
                            .review_plan_status_transition(
                                plan,
                                phase,
                                input.autorun,
                                completion_summary,
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
            PlanUpdateTarget::Step(step_id) => {
                let Some(step_index) = Self::resolve_plan_step_index(&plan, step_id.as_str())
                else {
                    return Err(PluginError::invalid_params(format!(
                        "unknown plan step '{}'; available steps: {}",
                        step_id,
                        plan.steps
                            .iter()
                            .enumerate()
                            .map(|(index, step)| format!(
                                "'{}' [{}]",
                                step.title,
                                Self::plan_step_identifier_hint(step, index)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                };
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
                step_id,
                checkpoint_id,
            } => {
                let Some(step_index) = Self::resolve_plan_step_index(&plan, step_id.as_str())
                else {
                    return Err(PluginError::invalid_params(format!(
                        "unknown plan step '{}'; available steps: {}",
                        step_id,
                        plan.steps
                            .iter()
                            .enumerate()
                            .map(|(index, step)| format!(
                                "'{}' [{}]",
                                step.title,
                                Self::plan_step_identifier_hint(step, index)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                };
                let step = &mut plan.steps[step_index];
                let checkpoint_text = {
                    let Some(checkpoint_index) =
                        Self::resolve_checkpoint_index(step, checkpoint_id.as_str())
                    else {
                        return Err(PluginError::invalid_params(format!(
                            "unknown check '{}' for step '{}'; available checks: {}",
                            checkpoint_id,
                            step_id,
                            step.checkpoints
                                .iter()
                                .enumerate()
                                .map(|(index, checkpoint)| format!(
                                    "'{}' [{}]",
                                    checkpoint.text,
                                    Self::checkpoint_identifier_hint(checkpoint, index)
                                ))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    };
                    let checkpoint = &mut step.checkpoints[checkpoint_index];
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
            ToolExecutionView::simple(format!("Task {}", input.description), output_text);
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
        let records = Self::filter_available_tools_by_tag(
            self.available_tool_records().await?,
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
        let names = results
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let mut lines = vec![format!(
            "Found {} matching tool(s); returned {} starting at offset {} for '{}'.",
            total,
            names.len(),
            offset,
            query
        )];
        for tool in &results {
            lines.push(format!(
                "- {} [{}]: {}",
                tool.name,
                tags_summary(tool.tags.as_slice()),
                tool.description
            ));
        }
        if !names.is_empty() {
            lines.push(format!(
                "Call `{}` with an exact tool name for detailed usage.",
                agena_runtime_tools::tool::tools_help_function_name()
            ));
        }
        let payload = serde_json::json!({
            "results": names,
            "query": query,
            "tag": input.tag.as_deref(),
            "tags": input.tags.as_deref(),
            "total": total,
            "offset": offset,
            "returned": results.len(),
        });
        Ok(ToolInvokeOutput::from_parts(
            "Tool search",
            lines.join("\n"),
            Some(payload),
            BTreeMap::from([
                ("query".to_string(), query.to_string()),
                ("matched_tools".to_string(), total.to_string()),
                ("returned_tools".to_string(), results.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_tool_api_list(
        &self,
        input: &ToolApiListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.search.default_limit)
            .clamp(1, config.tool_discovery.search.max_limit);
        let records = Self::filter_available_tools_by_tag(
            self.available_tool_records().await?,
            input.tag.as_deref(),
            input.tags.as_deref(),
        );
        let (tools, total, offset) = Self::paginate(&records, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available tool(s): returned {}/{} starting at offset {}.",
            tools.len(),
            total,
            offset
        )];
        for tool in &tools {
            let summary = tool.summary.trim();
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

        let payload = serde_json::json!({
            "tools": tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "summary": tool.summary,
                "tags": tool.tags,
            })).collect::<Vec<_>>(),
            "total": total,
            "offset": offset,
            "returned": tools.len(),
            "tag": input.tag.as_deref(),
            "tags": input.tags.as_deref(),
        });
        Ok(ToolInvokeOutput::from_parts(
            "Tool list",
            lines.join("\n"),
            Some(payload),
            BTreeMap::from([
                ("total_tools".to_string(), total.to_string()),
                ("returned_tools".to_string(), tools.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_tool_api_tags(
        &self,
        input: &ToolApiTagsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_discovery.search.default_limit)
            .clamp(1, config.tool_discovery.search.max_limit);
        let tags =
            Self::available_tool_tag_records(self.available_tool_records().await?.as_slice());
        let (tags, total, offset) = Self::paginate(&tags, input.offset, Some(limit));
        let mut lines = vec![format!(
            "Available tool tag(s): returned {}/{} starting at offset {}.",
            tags.len(),
            total,
            offset
        )];
        for tag in &tags {
            lines.push(format!("- {}: {}", tag.tag, tag.tool_count));
        }
        let payload = serde_json::json!({
            "tags": tags,
            "total": total,
            "offset": offset,
            "returned": tags.len(),
        });
        Ok(ToolInvokeOutput::from_parts(
            "Tool tags",
            lines.join("\n"),
            Some(payload),
            BTreeMap::from([
                ("total_tags".to_string(), total.to_string()),
                ("returned_tags".to_string(), tags.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
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

        ToolInvokeOutput::from_parts(
            format!("{} help", descriptor.name),
            lines.join("\n"),
            None,
            std::collections::BTreeMap::new(),
            Vec::new(),
        )
    }

    pub(in crate::plugins::provided::workflow) fn invalid_tool_input_with_embedded_help(
        descriptor: &ToolDescriptor,
        validation_error: &str,
    ) -> PluginError {
        let help = Self::render_tool_api_help(descriptor, None);
        let message = format!(
            "Input for execution tool `{}` failed validation, so the tool was not run. A separate `tools_help` call is unnecessary because Agena attached the complete help below. Correct the input and retry Tool API function `tools_call` directly with the same tool name and one complete input object.\n\nValidation error:\n{}\n\nTool help for `{}`:\n{}",
            descriptor.name, validation_error, descriptor.name, help.output_text,
        );
        PluginError::invalid_params_with_data(
            message,
            serde_json::json!({
                "kind": "tool_input_rejected_with_help",
                "tool": descriptor.name,
                "validation_error": validation_error,
                "help": {
                    "title": help.title,
                    "output_text": help.output_text,
                    "input_schema": descriptor.input_schema,
                    "examples": descriptor.examples,
                    "help_text": descriptor.help,
                },
                "retry": {
                    "function": agena_runtime_tools::tool::tools_call_function_name(),
                    "arguments": {
                        "tool": descriptor.name,
                        "input": "<one complete corrected object>"
                    }
                }
            }),
        )
    }

    pub(crate) async fn invoke_tool_api_call(
        &self,
        input: &ToolApiCallInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.as_str();
        Self::ensure_execution_tool_target(requested)?;
        let tools = self.host()?.list_tools().await?;
        let descriptor = Self::resolve_tool_descriptor(requested, &tools)?;
        if let Some(schema) = descriptor.input_schema.as_ref()
            && let Err(validation_error) =
                agena_plugin_host::loader::validate_json_schema_value(schema, &input.input)
        {
            return Err(Self::invalid_tool_input_with_embedded_help(
                descriptor,
                validation_error.as_str(),
            ));
        }
        self.host()?
            .invoke_tool(descriptor.name.clone(), input.input.clone())
            .await
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
    AskUserRequest, AskUserToolInput, AvailableToolRecord, BTreeMap, CommandBeforeInput,
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, HashMap, HashSet,
    HostEnterSnapshotRequest, HostExitSnapshotRequest, PathRequest, PlanGetInput, PlanSetInput,
    PlanUpdateInput, PlanUpdateTarget, PluginError, RunSubtaskAccess, RunSubtaskModelSelection,
    RunSubtaskRequest, RunSubtaskStatus, SdkResult, TaskAccess, TaskToolInput, ToolApiCallInput,
    ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput, ToolBeforeInput,
    ToolDescriptor, ToolExecutionView, ToolInvokeOutput, ToolPayloadExecution, ToolPayloadOutput,
    ToolTag, WorkflowPlan, WorkflowPlanPhase, WorkflowPlanStep, WorkflowPlanStepStatus,
    WorkflowPlugin, ask_user, search_tools, snapshot_enter_permission_paths, tags_summary,
};
