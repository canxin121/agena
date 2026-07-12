impl WorkflowPlugin {
    pub(in crate::plugins::provided::workflow) fn plan_lock_active(plan: &WorkflowPlan) -> bool {
        plan.phase == WorkflowPlanPhase::Planning
    }

    pub(in crate::plugins::provided::workflow) fn tool_allowed_during_planning(
        input: &ToolBeforeInput,
    ) -> bool {
        match input.plugin_key().to_string().as_str() {
            "agena.plan" => return matches!(input.tool_name(), "get" | "set" | "update" | "clear"),
            "agena.agent" => return matches!(input.tool_name(), "switch" | "restore"),
            "agena.session" => return matches!(input.tool_name(), "get" | "rename"),
            "agena.interaction" => return matches!(input.tool_name(), "ask" | "notify"),
            "agena.tools" => {
                return matches!(
                    input.tool_name(),
                    "list" | "search" | "tags" | "help" | "call"
                );
            }
            "agena.tasks" if input.tool_name() == "run" => {
                return TaskToolInput::parse_input(input.input.clone()).is_ok_and(|task| {
                    matches!(
                        task.subagent_type,
                        crate::message::TaskSubagentType::Explore
                            | crate::message::TaskSubagentType::Verify
                    )
                });
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
            return Err(PluginError::new(reason));
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
            .spawn_subtask(SpawnSubtaskRequest {
                subagent_type: input.subagent_type.to_string(),
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                task_id: input.task_id.clone(),
                command: input.command.clone(),
                model: None,
            })
            .await?;

        let session_id = response.metadata.get("session_id").cloned();
        let model_provider_id = response.metadata.get("model_provider_id").cloned();
        let model_id = response.metadata.get("model_id").cloned();
        let output_text = if response.final_text.trim().is_empty() {
            format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session_id.as_deref().unwrap_or("unknown"),
                input.subagent_type,
                "<unknown>"
            )
        } else {
            response.final_text
        };
        let mut view = ToolExecutionView::simple(
            format!("Task {} ({})", input.description, input.subagent_type),
            output_text,
        );
        view.metadata
            .insert("description".to_string(), input.description.clone());
        view.metadata
            .insert("subagent_type".to_string(), input.subagent_type.to_string());
        view.metadata.insert(
            "profile_guidance".to_string(),
            input.subagent_type.guidance().to_string(),
        );
        if let Some(session_id_value) = session_id.clone() {
            view.metadata
                .insert("session_id".to_string(), session_id_value);
        }
        if let Some(command) = input.command.clone() {
            view.metadata.insert("command".to_string(), command);
        }
        if let Some(model_provider_id) = model_provider_id.clone() {
            view.metadata
                .insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = model_id.clone() {
            view.metadata.insert("model_id".to_string(), model_id);
        }
        for (key, value) in response.metadata {
            view.metadata.entry(key).or_insert(value);
        }

        let output = ToolPayloadOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(
            crate::plugins::provided::router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(output, view),
            ),
        )
    }

    pub(crate) async fn invoke_agent_switch(
        &self,
        input: &AgentSwitchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .switch_agent_for_tool(input.agent.clone(), input.push_previous)
            .await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let current = response
            .current_agent
            .as_deref()
            .unwrap_or("default runtime context");
        let previous = response.previous_agent.as_deref().unwrap_or("none");
        Ok(ToolInvokeOutput::from_parts(
            "agent switch",
            format!(
                "Switched session {} agent to {current}. Previous agent: {previous}.",
                response.session_id
            ),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_agent_restore(&self) -> SdkResult<ToolInvokeOutput> {
        let response = self.restore_agent_for_tool().await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let text = if response.restored {
            let current = response
                .current_agent
                .as_deref()
                .unwrap_or("default runtime context");
            let previous = response.previous_agent.as_deref().unwrap_or("none");
            format!(
                "Restored session {} agent to {current}. Previous agent: {previous}.",
                response.session_id
            )
        } else {
            format!(
                "No agent restore point is available for session {}.",
                response.session_id
            )
        };
        Ok(ToolInvokeOutput::from_parts(
            "agent restore",
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_tool_search(
        &self,
        input: &CatalogSearchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
        let config = self.config()?;
        let max_query_length = config.tool_catalog.search.max_query_length as usize;
        if query.chars().count() > max_query_length {
            return Err(PluginError::invalid_params(format!(
                "search query is too long: {} characters (max {})",
                query.chars().count(),
                max_query_length
            )));
        }
        let limit = input
            .limit
            .unwrap_or(config.tool_catalog.search.default_limit)
            .clamp(1, config.tool_catalog.search.max_limit);
        let records = Self::filter_catalog_records_by_tag(
            self.catalog_tool_records().await?,
            input.tag.as_deref(),
            input.tags.as_deref(),
        );
        let catalog = records
            .iter()
            .map(Self::tool_search_document)
            .collect::<Vec<_>>();
        let results = search_tool_catalog(&catalog, query, catalog.len())
            .map_err(|err| PluginError::new(format!("tool search failed: {err}")))?;
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
                crate::tool::gateway_help_tool_name()
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

    pub(crate) async fn invoke_tool_list(
        &self,
        input: &ToolListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_catalog.search.default_limit)
            .clamp(1, config.tool_catalog.search.max_limit);
        let records = Self::filter_catalog_records_by_tag(
            self.catalog_tool_records().await?,
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

    pub(crate) async fn invoke_tool_tags(
        &self,
        input: &ToolTagsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_catalog.search.default_limit)
            .clamp(1, config.tool_catalog.search.max_limit);
        let tags = Self::catalog_tag_records(self.catalog_tool_records().await?.as_slice());
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

    pub(crate) async fn invoke_tool_help(
        &self,
        input: &ToolsHelpInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.as_str();
        let tools = self.host()?.list_tools().await?;
        let tag_index = self
            .catalog_tool_records()
            .await?
            .into_iter()
            .map(|record| (record.name, record.tags))
            .collect::<HashMap<_, _>>();
        let descriptor = Self::resolve_tool_descriptor(requested, &tools)?;

        let mut lines = vec![format!("Tool: {}", descriptor.name)];
        if let Some(tags) = tag_index.get(descriptor.name.as_str())
            && !tags.is_empty()
        {
            lines.push(format!("Tags: {}", tags.join(", ")));
        }
        lines.push("Usage:".to_string());
        if let Some(schema) = descriptor.input_schema.as_ref() {
            if let Some(arguments) = crate::tool::definition::schema_usage_text(schema) {
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
            .map(crate::tool::definition::schema_example_texts)
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
        lines.push(format!(
            "Preflight: this help authorizes one `{}` call for `{}` in this session. It remains available while you inspect other tools and is consumed by that call.",
            crate::tool::gateway_call_tool_name(),
            descriptor.name,
        ));
        self.save_help_preflight(descriptor.name.as_str()).await?;

        Ok(ToolInvokeOutput::from_parts(
            format!("{} help", descriptor.name),
            lines.join("\n"),
            None,
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_tool_call(
        &self,
        input: &ToolCallInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.trim();
        if requested.eq_ignore_ascii_case(crate::tool::gateway_call_tool_name()) {
            return Err(PluginError::invalid_params(format!(
                "{} cannot invoke itself",
                crate::tool::gateway_call_tool_name()
            )));
        }
        let tools = self.host()?.list_tools().await?;
        let descriptor = Self::resolve_tool_descriptor(requested, &tools)?;
        if Self::is_gateway_tool(descriptor.name.as_str()) {
            return Err(PluginError::invalid_params(format!(
                "`{}` can invoke only catalog targets such as `web.search`; gateway function `{}` must be called directly",
                crate::tool::gateway_call_tool_name(),
                descriptor.name,
            )));
        }
        if !self
            .consume_help_preflight(descriptor.name.as_str())
            .await?
        {
            return Err(PluginError::invalid_params(format!(
                "`{}` requires a one-call help preflight for `{}`. Call `{}({{\"tool\":\"{}\"}})` once, then call `{}` once with that target. The preflight remains valid while inspecting other tools, but is consumed by this call.",
                crate::tool::gateway_call_tool_name(),
                descriptor.name,
                crate::tool::gateway_help_tool_name(),
                descriptor.name,
                crate::tool::gateway_call_tool_name()
            )));
        }
        self.host()?
            .invoke_tool(descriptor.name.clone(), input.input.clone())
            .await
    }

    pub(in crate::plugins::provided::workflow) fn suggest_tool_names(
        requested: &str,
        tools: &[ToolDescriptor],
    ) -> Vec<String> {
        let candidate_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        let mut suggestions = crate::tool::suggest_tool_names(requested, candidate_names, 1);
        if suggestions.is_empty() {
            let catalog = tools
                .iter()
                .map(|tool| CatalogToolRecord {
                    name: tool.name.clone(),
                    summary: tool.summary.clone().unwrap_or_default(),
                    tags: Vec::new(),
                })
                .map(|record| Self::tool_search_document(&record))
                .collect::<Vec<_>>();
            if let Ok(results) = search_tool_catalog(&catalog, requested, 3) {
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

    pub(in crate::plugins::provided::workflow) fn is_gateway_tool(name: &str) -> bool {
        matches!(
            name.trim(),
            "agena.tools.list"
                | "agena.tools.search"
                | "agena.tools.help"
                | "agena.tools.tags"
                | "agena.tools.call"
                | "tools.list"
                | "tools.search"
                | "tools.help"
                | "tools.tags"
                | "tools.call"
        )
    }
}
use super::{
    AgentSwitchToolInput, AskUserRequest, AskUserToolInput, BTreeMap, CatalogSearchInput,
    CatalogToolRecord, CommandBeforeInput, EnterSnapshotCommandInput, ExitSnapshotCommandInput,
    HashMap, HashSet, HostEnterSnapshotRequest, HostExitSnapshotRequest, PathRequest, PlanGetInput,
    PlanSetInput, PlanUpdateInput, PlanUpdateTarget, PluginError, SdkResult, SpawnSubtaskRequest,
    TaskToolInput, ToolBeforeInput, ToolCallInput, ToolDescriptor, ToolExecutionView,
    ToolInvokeOutput, ToolListInput, ToolPayloadExecution, ToolPayloadOutput, ToolTag,
    ToolTagsInput, ToolsHelpInput, WorkflowPlan, WorkflowPlanPhase, WorkflowPlanStep,
    WorkflowPlanStepStatus, WorkflowPlugin, ask_user, search_tool_catalog,
    snapshot_enter_permission_paths, tags_summary,
};
