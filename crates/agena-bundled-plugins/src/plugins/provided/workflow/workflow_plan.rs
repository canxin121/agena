impl WorkflowPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            config: OnceLock::new(),
            workspace_root: OnceLock::new(),
        }
    }

    pub(in crate::plugins::provided::workflow) fn config(
        &self,
    ) -> SdkResult<&WorkflowPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("workflow plugin invoked before init"))
    }

    pub(in crate::plugins::provided::workflow) fn resolve_tool_descriptor<'a>(
        requested: &str,
        tools: &'a [ToolDescriptor],
    ) -> SdkResult<&'a ToolDescriptor> {
        let exact = tools
            .iter()
            .filter(|tool| tool.name == requested)
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [tool] => return Ok(*tool),
            [] => {}
            _ => {
                return Err(PluginError::invalid_params(format!(
                    "tool `{requested}` is ambiguous; call `tools_list` again and use the returned fully-qualified tool name"
                )));
            }
        }

        // Built-in canonical registry identities are valid payload data too,
        // but resolution remains exact: no trimming or case folding.
        let requested_without_namespace = requested.strip_prefix("agena.").unwrap_or(requested);
        let aliases = tools
            .iter()
            .filter(|tool| tool.name == requested_without_namespace)
            .collect::<Vec<_>>();
        match aliases.as_slice() {
            [tool] => return Ok(*tool),
            [] => {}
            _ => {
                return Err(PluginError::invalid_params(format!(
                    "tool `{requested}` is ambiguous; call `tools_list` again and use the returned fully-qualified tool name"
                )));
            }
        }

        let suggestions = Self::suggest_tool_names(requested, tools);
        let suggestion_text = if suggestions.is_empty() {
            String::new()
        } else {
            format!(
                " Similar live names are {}, but suggestions are not proof of the intended tool or its input contract.",
                suggestions
                    .iter()
                    .map(|tool| format!("`{tool}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let message = format!(
            "unknown tool '{requested}'.{suggestion_text} Do not guess a replacement name or its arguments. Call Tool API function `tools_search` for the capability you need, choose an exact returned identifier, call `tools_help` for its live input contract, then retry `tools_call`."
        );
        Err(PluginError::invalid_params_with_data(
            message,
            serde_json::json!({
                "kind": "unknown_execution_tool",
                "requested": requested,
                "suggestions": suggestions,
                "recovery": [
                    {
                        "function": "tools_search",
                        "arguments": { "query": requested }
                    },
                    {
                        "function": "tools_help",
                        "tool_from": "exact identifier returned by tools_search"
                    },
                    {
                        "function": "tools_call",
                        "tool_from": "same exact identifier passed to tools_help",
                        "input_from": "one complete object derived from the live tools_help contract"
                    }
                ]
            }),
        ))
    }

    pub(crate) fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("workflow plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("workflow plugin invoked before init"))
    }

    pub(in crate::plugins::provided::workflow) fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("workflow plugin workspace root not initialized"))
    }

    pub(in crate::plugins::provided::workflow) fn host_ask_user_questions(
        input: &AskUserToolInput,
    ) -> Vec<HostAskUserQuestion> {
        input
            .questions
            .iter()
            .map(|question| HostAskUserQuestion {
                header: question.header.clone(),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| HostAskUserOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect()
    }

    pub(in crate::plugins::provided::workflow) fn tool_search_document(
        record: &AvailableToolRecord,
    ) -> ToolSearchDocument {
        ToolSearchDocument::new(
            record.name.clone(),
            record.summary.clone(),
            record.tags.clone(),
            None,
        )
    }

    pub(in crate::plugins::provided::workflow) fn plugin_search_document(
        record: &AvailablePluginRecord,
    ) -> ToolSearchDocument {
        ToolSearchDocument::new(
            record.plugin_id.clone(),
            record.summary.clone(),
            record.tags.clone(),
            Some(record.plugin_id.clone()),
        )
    }

    pub(in crate::plugins::provided::workflow) fn filter_available_plugins_by_tag(
        mut records: Vec<AvailablePluginRecord>,
        tag: Option<&str>,
        tags: Option<&[String]>,
    ) -> Vec<AvailablePluginRecord> {
        let required_tags = Self::normalized_tag_filters(tag, tags);
        if required_tags.is_empty() {
            return records;
        }
        records.retain(|record| {
            required_tags.iter().all(|required_tag| {
                record
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(required_tag.as_str()))
            })
        });
        records
    }

    pub(in crate::plugins::provided::workflow) fn filter_available_plugins_by_id(
        mut records: Vec<AvailablePluginRecord>,
        plugin: Option<&ToolApiStringBatch>,
    ) -> Vec<AvailablePluginRecord> {
        let filters = normalized_plugin_filters(plugin);
        if filters.is_empty() {
            return records;
        }
        records.retain(|record| {
            filters
                .iter()
                .any(|filter| plugin_id_matches(record.plugin_id.as_str(), filter))
        });
        records
    }

    pub(in crate::plugins::provided::workflow) fn normalized_tag_filters(
        tag: Option<&str>,
        tags: Option<&[String]>,
    ) -> Vec<String> {
        let mut filters = Vec::new();
        if let Some(tag) = tag
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        {
            filters.push(tag);
        }
        if let Some(tags) = tags {
            for tag in tags {
                let normalized = tag.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    filters.push(normalized);
                }
            }
        }
        filters.sort();
        filters.dedup();
        filters
    }

    pub(in crate::plugins::provided::workflow) fn paginate<T: Clone>(
        items: &[T],
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> (Vec<T>, usize, usize) {
        let total = items.len();
        let offset = offset.unwrap_or(0) as usize;
        if offset >= total {
            return (Vec::new(), total, offset);
        }
        let limit = limit
            .map(|value| value as usize)
            .unwrap_or(total.saturating_sub(offset));
        let end = offset.saturating_add(limit).min(total);
        (items[offset..end].to_vec(), total, offset)
    }

    pub(in crate::plugins::provided::workflow) async fn available_tool_records(
        &self,
    ) -> SdkResult<Vec<AvailableToolRecord>> {
        let host = self.host()?;
        let tools = host.list_tools().await?;
        let visible = tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<HashSet<_>>();
        let registered = host.list_registered_tools().await?;
        let mut tags_by_name = Self::tool_tags_by_visible_name(&visible, registered.tools);

        let mut records = tools
            .into_iter()
            .map(|tool| AvailableToolRecord {
                tags: tags_by_name.remove(&tool.name).unwrap_or_default(),
                name: tool.name,
                summary: tool.summary.unwrap_or_default(),
                plugin_id: tool.plugin_id.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(records)
    }

    /// Filter the tool inventory by plugin id, fully qualified plugin id,
    /// or trailing plugin segment (e.g. `agena.fs`, `fs`).
    pub(in crate::plugins::provided::workflow) fn filter_available_tools_by_plugin(
        records: Vec<AvailableToolRecord>,
        plugin: Option<&ToolApiStringBatch>,
    ) -> Vec<AvailableToolRecord> {
        let filters = normalized_plugin_filters(plugin);
        if filters.is_empty() {
            return records;
        }
        records
            .into_iter()
            .filter(|record| {
                filters
                    .iter()
                    .any(|filter| plugin_id_matches(record.plugin_id.as_str(), filter))
            })
            .collect()
    }

    pub(in crate::plugins::provided::workflow) fn tool_tags_by_visible_name(
        visible: &HashSet<String>,
        registered: impl IntoIterator<Item = HostRegisteredToolDescriptor>,
    ) -> HashMap<String, Vec<String>> {
        let mut tags_by_name = HashMap::<String, Vec<String>>::new();
        for entry in registered {
            let canonical_name = entry.tool_key.to_string();
            let compact_name =
                agena_runtime_tools::tool::compact_tool_call_name(canonical_name.as_str());
            let visible_name = if visible.contains(&canonical_name) {
                canonical_name
            } else if visible.contains(&compact_name) {
                compact_name
            } else {
                continue;
            };
            let mut tags = entry
                .tool
                .effective_tags()
                .into_iter()
                .map(|tag| tag.to_string())
                .collect::<Vec<_>>();
            tags.sort();
            tags.dedup();
            tags_by_name.insert(visible_name, tags);
        }
        tags_by_name
    }

    pub(in crate::plugins::provided::workflow) fn filter_available_tools_by_tag(
        mut records: Vec<AvailableToolRecord>,
        tag: Option<&str>,
        tags: Option<&[String]>,
    ) -> Vec<AvailableToolRecord> {
        let required_tags = Self::normalized_tag_filters(tag, tags);
        if required_tags.is_empty() {
            return records;
        }
        records.retain(|record| {
            required_tags.iter().all(|required_tag| {
                record
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(required_tag.as_str()))
            })
        });
        records
    }

    pub(in crate::plugins::provided::workflow) fn available_tool_tag_records(
        records: &[AvailableToolRecord],
    ) -> Vec<ToolTagRecord> {
        let mut counts = BTreeMap::<String, usize>::new();
        for record in records {
            for tag in &record.tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .map(|(tag, tool_count)| ToolTagRecord {
                tag,
                tool_count,
                plugin_count: None,
            })
            .collect()
    }

    pub(in crate::plugins::provided::workflow) fn pretty_json_text(
        payload: &serde_json::Value,
    ) -> String {
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    }

    pub(in crate::plugins::provided::workflow) fn session_tool_payload(
        session: HostSession,
    ) -> SdkResult<serde_json::Value> {
        serde_json::to_value(SessionToolResponse { session })
            .map_err(|err| PluginError::internal_error(&err))
    }

    pub(in crate::plugins::provided::workflow) fn session_summary(session: &HostSession) -> String {
        let mut parts = vec![format!("Session #{} title: {}", session.id, session.title)];
        if let Some(parent_id) = session.parent_id {
            parts.push(format!("parent #{parent_id}"));
        }
        if session.root_id != session.id {
            parts.push(format!("root #{}", session.root_id));
        }
        if session.is_subagent {
            parts.push("subagent".to_string());
        }
        parts.join(" | ")
    }

    pub(crate) async fn invoke_get_session(&self) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .get_session(HostGetSessionRequest::default())
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::from_parts(
            "session",
            format!(
                "Session #{} · {}",
                response.session.id, response.session.title
            ),
            format!(
                "{}\n\n{}",
                Self::session_summary(&response.session),
                Self::pretty_json_text(&payload)
            ),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(crate) async fn invoke_rename_session(
        &self,
        input: &SessionRenameToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .rename_session(HostRenameSessionRequest {
                session_id: None,
                title: input.title.clone(),
            })
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::from_parts(
            "session",
            format!(
                "Renamed #{} · {}",
                response.session.id, response.session.title
            ),
            format!(
                "Renamed session #{} to {}.\n\n{}",
                response.session.id,
                response.session.title,
                Self::pretty_json_text(&payload)
            ),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(in crate::plugins::provided::workflow) async fn load_active_plan(
        &self,
    ) -> SdkResult<Option<WorkflowPlan>> {
        let response = self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Shared,
                namespace: PLAN_NAMESPACE.to_string(),
                key: PLAN_KEY_ACTIVE.to_string(),
            })
            .await?;
        let Some(value) = response.value else {
            return Ok(None);
        };
        serde_json::from_str::<WorkflowPlan>(&value)
            .map(Some)
            .map_err(|err| PluginError::internal(format!("invalid stored plan payload: {err}")))
    }

    pub(in crate::plugins::provided::workflow) async fn save_active_plan(
        &self,
        plan: &WorkflowPlan,
    ) -> SdkResult<()> {
        let value =
            serde_json::to_string_pretty(plan).map_err(|err| PluginError::internal_error(&err))?;
        let host = self.host()?;
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Shared,
            namespace: PLAN_NAMESPACE.to_string(),
            key: PLAN_KEY_ACTIVE.to_string(),
            value,
        })
        .await?;
        self.sync_plan_display(Some(plan)).await?;
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) async fn clear_active_plan(&self) -> SdkResult<()> {
        let host = self.host()?;
        host.storage_delete(HostStorageDeleteRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Shared,
            namespace: PLAN_NAMESPACE.to_string(),
            key: PLAN_KEY_ACTIVE.to_string(),
        })
        .await?;
        self.sync_plan_display(None).await?;
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) async fn sync_plan_display(
        &self,
        plan: Option<&WorkflowPlan>,
    ) -> SdkResult<()> {
        let host = self.host()?;
        let session_id = host
            .get_session(HostGetSessionRequest::default())
            .await?
            .session
            .id;
        // The plan contribution is qualified by the contributing session so
        // the UI never renders one session's plan while another session is
        // active.
        let contribution_id = format!("{PLAN_DISPLAY_CONTRIBUTION_ID}:{session_id}");
        match plan {
            Some(plan) => {
                host.display_contribute(HostDisplayContributeRequest {
                    contribution: PluginDisplayContribution {
                        id: contribution_id,
                        kind: ContributionKind::StatusLineText,
                        priority: 120,
                        content: PluginDisplayContent::Text {
                            text: Self::plan_display_content(plan),
                        },
                    },
                })
                .await?;
            }
            None => {
                let _ = host
                    .display_remove(HostDisplayRemoveRequest { contribution_id })
                    .await?;
            }
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn plan_payload(
        plan: &WorkflowPlan,
    ) -> SdkResult<serde_json::Value> {
        serde_json::to_value(serde_json::json!({ "plan": plan }))
            .map_err(|err| PluginError::internal_error(&err))
    }

    pub(in crate::plugins::provided::workflow) fn validate_plan_objective(
        value: &str,
    ) -> SdkResult<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(PluginError::invalid_params(
                "plan objective must not be empty",
            ));
        }
        Ok(trimmed.to_string())
    }

    pub(in crate::plugins::provided::workflow) fn default_plan_title(objective: &str) -> String {
        let line = objective
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(objective);
        let title = line.trim();
        if title.chars().count() <= 80 {
            return title.to_string();
        }
        title.chars().take(80).collect()
    }

    pub(in crate::plugins::provided::workflow) fn normalize_plan_steps(
        inputs: &[WorkflowPlanStepInput],
    ) -> SdkResult<Vec<WorkflowPlanStep>> {
        let mut steps = Vec::with_capacity(inputs.len());
        for (step_index, step) in inputs.iter().enumerate() {
            let title = step.title.trim();
            let description = step.description.trim();
            let resolved_title = if !title.is_empty() {
                title
            } else if !description.is_empty() {
                description
            } else {
                return Err(PluginError::invalid_params(format!(
                    "plan step {} requires a non-empty title",
                    step_index + 1
                )));
            };
            let checkpoints = step
                .checkpoints
                .iter()
                .enumerate()
                .map(|(checkpoint_index, checkpoint)| {
                    let text = checkpoint.text.trim();
                    if text.is_empty() {
                        return Err(PluginError::invalid_params(format!(
                            "plan check {}.{} requires non-empty text",
                            step_index + 1,
                            checkpoint_index + 1
                        )));
                    }
                    Ok(WorkflowPlanCheckpoint {
                        text: text.to_string(),
                        status: checkpoint.status.unwrap_or_default(),
                    })
                })
                .collect::<SdkResult<Vec<_>>>()?;
            steps.push(WorkflowPlanStep {
                title: resolved_title.to_string(),
                description: description.to_string(),
                executor: step.executor,
                status: step.status.unwrap_or_default(),
                note: step.note.clone().unwrap_or_default().trim().to_string(),
                checkpoints,
            });
        }
        Ok(steps)
    }

    pub(in crate::plugins::provided::workflow) fn build_plan(
        &self,
        objective: &str,
        title: Option<&str>,
        document_markdown: Option<&str>,
        steps: &[WorkflowPlanStepInput],
        autorun: Option<bool>,
        previous: Option<&WorkflowPlan>,
    ) -> SdkResult<WorkflowPlan> {
        let objective = Self::validate_plan_objective(objective)?;
        let title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Self::default_plan_title(&objective));
        let autorun = match autorun {
            Some(value) => value,
            None => previous
                .map(|plan| plan.autorun)
                .unwrap_or(self.config()?.plan.default_autorun),
        };
        Ok(WorkflowPlan {
            title,
            objective,
            phase: WorkflowPlanPhase::Planning,
            autorun,
            document_markdown: document_markdown
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_default(),
            steps: Self::normalize_plan_steps(steps)?,
        })
    }

    pub(in crate::plugins::provided::workflow) fn plan_phase_label(
        phase: WorkflowPlanPhase,
    ) -> &'static str {
        match phase {
            WorkflowPlanPhase::Planning => "planning",
            WorkflowPlanPhase::Active => "active",
            WorkflowPlanPhase::Blocked => "blocked",
            WorkflowPlanPhase::Completed => "completed",
            WorkflowPlanPhase::Cancelled => "cancelled",
        }
    }

    pub(in crate::plugins::provided::workflow) fn plan_step_status_label(
        status: WorkflowPlanStepStatus,
    ) -> &'static str {
        match status {
            WorkflowPlanStepStatus::Pending => "pending",
            WorkflowPlanStepStatus::InProgress => "in_progress",
            WorkflowPlanStepStatus::Blocked => "blocked",
            WorkflowPlanStepStatus::Completed => "completed",
            WorkflowPlanStepStatus::Skipped => "skipped",
        }
    }

    pub(in crate::plugins::provided::workflow) fn step_status_marker(
        status: WorkflowPlanStepStatus,
    ) -> &'static str {
        match status {
            WorkflowPlanStepStatus::Pending => "[ ]",
            WorkflowPlanStepStatus::InProgress => "[>]",
            WorkflowPlanStepStatus::Blocked => "[!]",
            WorkflowPlanStepStatus::Completed => "[x]",
            WorkflowPlanStepStatus::Skipped => "[-]",
        }
    }

    pub(in crate::plugins::provided::workflow) fn step_status_is_terminal(
        status: WorkflowPlanStepStatus,
    ) -> bool {
        matches!(
            status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        )
    }

    /// Resolve a 1-based `step` index to a 0-based index, validating the range.
    pub(in crate::plugins::provided::workflow) fn resolve_step_index(
        plan: &WorkflowPlan,
        step: usize,
    ) -> SdkResult<usize> {
        if step == 0 {
            return Err(PluginError::invalid_params(
                "plan step indices are 1-based; `step` must be at least 1".to_string(),
            ));
        }
        let index = step - 1;
        if index >= plan.steps.len() {
            return Err(PluginError::invalid_params(format!(
                "unknown plan step {step}; this plan has {} step(s)",
                plan.steps.len()
            )));
        }
        Ok(index)
    }

    /// Resolve a 1-based `check` index within a step to a 0-based index,
    /// validating the range.
    pub(in crate::plugins::provided::workflow) fn resolve_check_index(
        step: &WorkflowPlanStep,
        check: usize,
    ) -> SdkResult<usize> {
        if check == 0 {
            return Err(PluginError::invalid_params(
                "plan check indices are 1-based; `check` must be at least 1".to_string(),
            ));
        }
        let index = check - 1;
        if index >= step.checkpoints.len() {
            return Err(PluginError::invalid_params(format!(
                "unknown check {check}; this step has {} check(s)",
                step.checkpoints.len()
            )));
        }
        Ok(index)
    }

    pub(in crate::plugins::provided::workflow) fn step_listing(plan: &WorkflowPlan) -> String {
        plan.steps
            .iter()
            .enumerate()
            .map(|(index, step)| format!("{}: '{}'", index + 1, step.title))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(in crate::plugins::provided::workflow) fn check_listing(step: &WorkflowPlanStep) -> String {
        step.checkpoints
            .iter()
            .enumerate()
            .map(|(index, checkpoint)| format!("{}: '{}'", index + 1, checkpoint.text))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(in crate::plugins::provided::workflow) fn plan_progress_counts(
        plan: &WorkflowPlan,
    ) -> (usize, usize, usize, usize) {
        let total_steps = plan.steps.len();
        let completed_steps = plan
            .steps
            .iter()
            .filter(|step| Self::step_status_is_terminal(step.status))
            .count();
        let total_checkpoints = plan
            .steps
            .iter()
            .map(|step| step.checkpoints.len())
            .sum::<usize>();
        let completed_checkpoints = plan
            .steps
            .iter()
            .flat_map(|step| step.checkpoints.iter())
            .filter(|checkpoint| Self::step_status_is_terminal(checkpoint.status))
            .count();
        (
            completed_steps,
            total_steps,
            completed_checkpoints,
            total_checkpoints,
        )
    }

    pub(in crate::plugins::provided::workflow) fn workflow_plan_markdown(
        plan: &WorkflowPlan,
    ) -> String {
        let document_markdown = plan.document_markdown.trim();
        let mut sections = Vec::new();

        if document_markdown.is_empty() {
            sections.push(format!("# {}", plan.title));
            if plan.objective.trim() != plan.title.trim() {
                sections.push(String::new());
                sections.push(plan.objective.trim().to_string());
            }
        } else {
            sections.push(document_markdown.to_string());
        }

        let metadata = format!("Autorun: {}", if plan.autorun { "on" } else { "off" });
        sections.push(String::new());
        sections.push(format!("_{metadata}_"));

        if !plan.steps.is_empty() {
            sections.push(String::new());
            sections.push("## Steps".to_string());
            for (index, step) in plan.steps.iter().enumerate() {
                sections.push(format!(
                    "{}. {} {} ({})",
                    index + 1,
                    Self::step_status_marker(step.status),
                    step.title,
                    match step.executor {
                        WorkflowPlanExecutor::Ai => "ai",
                        WorkflowPlanExecutor::Human => "human",
                    }
                ));
                if !step.description.trim().is_empty()
                    && step.description.trim() != step.title.trim()
                {
                    sections.push(format!("   - Details: {}", step.description.trim()));
                }
                for checkpoint in &step.checkpoints {
                    sections.push(format!(
                        "   - {} {}",
                        Self::step_status_marker(checkpoint.status),
                        checkpoint.text
                    ));
                }
                if !step.note.trim().is_empty() {
                    sections.push(format!("   - Note: {}", step.note.trim()));
                }
            }
        }
        sections.join("\n")
    }

    pub(in crate::plugins::provided::workflow) fn plan_display_content(
        plan: &WorkflowPlan,
    ) -> String {
        let (completed_steps, total_steps, _, _) = Self::plan_progress_counts(plan);
        let mut parts = vec![Self::plan_phase_symbol(plan.phase).to_string()];
        if total_steps > 0 {
            parts.push(format!("{completed_steps}/{total_steps}"));
        }
        if plan.autorun {
            parts.push("↻".to_string());
        }
        parts.join(" ")
    }

    pub(in crate::plugins::provided::workflow) fn plan_phase_symbol(
        phase: WorkflowPlanPhase,
    ) -> &'static str {
        match phase {
            WorkflowPlanPhase::Planning => "⏳",
            WorkflowPlanPhase::Active => "▶",
            WorkflowPlanPhase::Blocked => "⚠",
            WorkflowPlanPhase::Completed => "✓",
            WorkflowPlanPhase::Cancelled => "✕",
        }
    }

    pub(in crate::plugins::provided::workflow) fn next_actionable_step(
        plan: &WorkflowPlan,
    ) -> Option<(usize, &WorkflowPlanStep)> {
        if matches!(
            plan.phase,
            WorkflowPlanPhase::Completed | WorkflowPlanPhase::Cancelled
        ) {
            return None;
        }
        plan.steps
            .iter()
            .enumerate()
            .find(|(_, step)| !Self::step_status_is_terminal(step.status))
    }

    pub(in crate::plugins::provided::workflow) fn step_goal(step: &WorkflowPlanStep) -> &str {
        let description = step.description.trim();
        if !description.is_empty() {
            description
        } else {
            step.title.trim()
        }
    }

    pub(in crate::plugins::provided::workflow) fn plan_summary_text(plan: &WorkflowPlan) -> String {
        let (completed_steps, total_steps, _, _) = Self::plan_progress_counts(plan);
        let mut parts = vec![format!("phase {}", Self::plan_phase_label(plan.phase))];
        if total_steps > 0 {
            parts.push(format!("steps {completed_steps}/{total_steps}"));
        }
        parts.push(format!(
            "autorun {}",
            if plan.autorun { "on" } else { "off" }
        ));
        parts.join(" | ")
    }

    pub(in crate::plugins::provided::workflow) fn plan_output_text(
        prefix: &str,
        plan: &WorkflowPlan,
    ) -> String {
        format!("{prefix}\n{}", Self::plan_summary_text(plan))
    }

    pub(in crate::plugins::provided::workflow) fn plan_current_text(plan: &WorkflowPlan) -> String {
        match Self::next_actionable_step(plan) {
            Some((index, step)) => format!(
                "Current step {}: '{}' (step {}).\nGoal: {}\nStatus: {}.",
                index + 1,
                step.title,
                index + 1,
                Self::step_goal(step),
                Self::plan_step_status_label(step.status)
            ),
            None => "The active plan has no current actionable step.".to_string(),
        }
    }

    pub(in crate::plugins::provided::workflow) fn plan_get_text(
        plan: &WorkflowPlan,
        view: PlanGetView,
    ) -> String {
        match view {
            PlanGetView::Current => Self::plan_current_text(plan),
            PlanGetView::Summary => Self::plan_summary_text(plan),
            PlanGetView::Full => Self::workflow_plan_markdown(plan),
        }
    }

    pub(in crate::plugins::provided::workflow) fn plan_get_payload(
        plan: &WorkflowPlan,
        view: PlanGetView,
    ) -> serde_json::Value {
        match Self::next_actionable_step(plan) {
            Some((index, step)) => serde_json::json!({
                "plan": plan,
                "view": view,
                "current_step": step,
                "current_step_index": index,
                "current_step_goal": Self::step_goal(step),
            }),
            None => serde_json::json!({
                "plan": plan,
                "view": view,
                "current_step": serde_json::Value::Null,
                "current_step_goal": serde_json::Value::Null,
            }),
        }
    }

    pub(in crate::plugins::provided::workflow) fn cascade_terminal_step_status(
        step: &mut WorkflowPlanStep,
        status: WorkflowPlanStepStatus,
    ) {
        if !matches!(
            status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        ) {
            return;
        }
        for checkpoint in &mut step.checkpoints {
            if !Self::step_status_is_terminal(checkpoint.status) {
                checkpoint.status = status;
            }
        }
    }

    pub(in crate::plugins::provided::workflow) fn reconcile_step_status_from_checkpoints(
        step: &mut WorkflowPlanStep,
    ) {
        if step.checkpoints.is_empty() {
            return;
        }
        let all_terminal = step
            .checkpoints
            .iter()
            .all(|checkpoint| Self::step_status_is_terminal(checkpoint.status));
        if all_terminal {
            if step
                .checkpoints
                .iter()
                .all(|checkpoint| checkpoint.status == WorkflowPlanStepStatus::Skipped)
            {
                step.status = WorkflowPlanStepStatus::Skipped;
            } else {
                step.status = WorkflowPlanStepStatus::Completed;
            }
            return;
        }
        if matches!(
            step.status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        ) {
            step.status = if step
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.status == WorkflowPlanStepStatus::Blocked)
            {
                WorkflowPlanStepStatus::Blocked
            } else if step.checkpoints.iter().any(|checkpoint| {
                matches!(
                    checkpoint.status,
                    WorkflowPlanStepStatus::InProgress | WorkflowPlanStepStatus::Completed
                )
            }) {
                WorkflowPlanStepStatus::InProgress
            } else {
                WorkflowPlanStepStatus::Pending
            };
        }
    }

    pub(in crate::plugins::provided::workflow) fn plan_completion_blocker(
        plan: &WorkflowPlan,
    ) -> Option<String> {
        for (step_index, step) in plan.steps.iter().enumerate() {
            if !Self::step_status_is_terminal(step.status) {
                return Some(format!(
                    "step {} ('{}') is still {}",
                    step_index + 1,
                    step.title,
                    Self::plan_step_status_label(step.status)
                ));
            }
            for (checkpoint_index, checkpoint) in step.checkpoints.iter().enumerate() {
                if !Self::step_status_is_terminal(checkpoint.status) {
                    return Some(format!(
                        "check {}.{} ('{}') is still {}",
                        step_index + 1,
                        checkpoint_index + 1,
                        checkpoint.text,
                        Self::plan_step_status_label(checkpoint.status)
                    ));
                }
            }
        }
        None
    }

    pub(in crate::plugins::provided::workflow) fn ensure_plan_ready_for_completion(
        plan: &WorkflowPlan,
    ) -> SdkResult<()> {
        if let Some(blocker) = Self::plan_completion_blocker(plan) {
            return Err(PluginError::invalid_params(format!(
                "cannot complete plan: {blocker}"
            )));
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn append_completion_summary(
        plan: &mut WorkflowPlan,
        summary: Option<&str>,
    ) {
        let Some(summary) = summary.map(str::trim).filter(|value| !value.is_empty()) else {
            return;
        };
        let summary_section = format!("## Completion Summary\n\n{summary}");
        if plan.document_markdown.trim().is_empty() {
            plan.document_markdown = summary_section;
            return;
        }
        if plan.document_markdown.contains(summary_section.as_str()) {
            return;
        }
        plan.document_markdown = format!("{}\n\n{summary_section}", plan.document_markdown.trim());
    }

    /// Validate a `plan.edit` input and classify it as a step or check update.
    /// This tool never touches the plan phase and never requests approval.
    pub(in crate::plugins::provided::workflow) fn validate_plan_edit_input(
        input: &PlanEditInput,
    ) -> SdkResult<PlanEditTarget> {
        let step = input.step;
        let check = input.check;

        let Some(step) = step else {
            if check.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.edit check updates require `step`".to_string(),
                ));
            }
            return Err(PluginError::invalid_params(
                "plan.edit requires `step` to address a step or check".to_string(),
            ));
        };

        if let Some(check) = check {
            if input.status.is_none() {
                return Err(PluginError::invalid_params(
                    "plan.edit check updates require `status`".to_string(),
                ));
            }
            if input.note.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.edit check updates do not support `note`".to_string(),
                ));
            }
            return Ok(PlanEditTarget::Check {
                step_index: step,
                check_index: check,
            });
        }

        if input.status.is_none() && input.note.is_none() {
            return Err(PluginError::invalid_params(
                "plan.edit step updates require at least one of `status` or `note`".to_string(),
            ));
        }

        Ok(PlanEditTarget::Step(step))
    }

    /// Validate a `plan.phase` input. At least one of `phase` or `autorun` must
    /// be present; `summary` is only meaningful with `phase: completed`.
    pub(in crate::plugins::provided::workflow) fn validate_plan_phase_input(
        input: &PlanPhaseInput,
    ) -> SdkResult<()> {
        if input.phase.is_none() && input.autorun.is_none() {
            return Err(PluginError::invalid_params(
                "plan.phase requires `phase` or `autorun`".to_string(),
            ));
        }
        if input.summary.is_some() && input.phase != Some(WorkflowPlanPhase::Completed) {
            return Err(PluginError::invalid_params(
                "plan.phase summary is only valid when phase is `completed`".to_string(),
            ));
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn plan_phase_requires_approval(
        phase: WorkflowPlanPhase,
    ) -> bool {
        matches!(
            phase,
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked | WorkflowPlanPhase::Completed
        )
    }

    pub(in crate::plugins::provided::workflow) fn plan_phase_is_approved(
        phase: WorkflowPlanPhase,
    ) -> bool {
        matches!(
            phase,
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked | WorkflowPlanPhase::Completed
        )
    }

    pub(in crate::plugins::provided::workflow) fn mark_plan_completed(
        plan: &mut WorkflowPlan,
        summary: Option<&str>,
    ) -> SdkResult<()> {
        Self::ensure_plan_ready_for_completion(plan)?;
        plan.phase = WorkflowPlanPhase::Completed;
        Self::append_completion_summary(plan, summary);
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn validate_plan_phase_change(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
    ) -> SdkResult<()> {
        match phase {
            WorkflowPlanPhase::Completed => Self::ensure_plan_ready_for_completion(plan),
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked => {
                if Self::plan_completion_blocker(plan).is_none() {
                    return Err(PluginError::invalid_params(format!(
                        "cannot set plan status to {}: all steps and checks are already complete; reopen a step or check first",
                        Self::plan_phase_label(phase)
                    )));
                }
                Ok(())
            }
            WorkflowPlanPhase::Planning | WorkflowPlanPhase::Cancelled => Ok(()),
        }
    }

    pub(in crate::plugins::provided::workflow) fn set_plan_phase(
        plan: &mut WorkflowPlan,
        phase: WorkflowPlanPhase,
        completion_summary: Option<&str>,
    ) -> SdkResult<()> {
        Self::validate_plan_phase_change(plan, phase)?;
        if phase == WorkflowPlanPhase::Completed {
            return Self::mark_plan_completed(plan, completion_summary);
        }
        plan.phase = phase;
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn review_decision(
        response: &agena_plugin_host::sdk::host_api::AskUserResponse,
    ) -> Option<String> {
        response
            .answers
            .get("decision")
            .and_then(|values| values.first())
            .cloned()
            .or_else(|| {
                response
                    .answers
                    .values()
                    .find_map(|values| values.first().cloned())
            })
            .or_else(|| {
                response
                    .answers
                    .get("reply")
                    .and_then(|values| values.first())
                    .cloned()
            })
            .or_else(|| {
                let reply = response.reply.trim();
                (!reply.is_empty()).then_some(reply.to_string())
            })
    }

    /// Resolve a host ask_user response into a review decision string. A
    /// cancelled or timed-out request (no decision content) resolves to
    /// "keep in planning": the plan was already returned to planning before
    /// the review, so a missed review never strands the plan in a requested
    /// phase waiting forever.
    pub(in crate::plugins::provided::workflow) fn review_decision_from_response(
        response: &agena_plugin_host::sdk::host_api::AskUserResponse,
    ) -> String {
        if response.cancelled {
            return PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string();
        }
        Self::review_decision(response)
            .unwrap_or_else(|| PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string())
    }

    /// Apply a review decision to the plan. Approvals move the plan to the
    /// requested phase; every other decision (keep in planning, reject, free
    /// text feedback, timeout fallback) leaves it in planning so the agent can
    /// revise and propose again.
    pub(in crate::plugins::provided::workflow) fn apply_review_decision(
        plan: &mut WorkflowPlan,
        decision: &str,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> SdkResult<()> {
        match decision {
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON => {
                Self::set_plan_phase(plan, phase, completion_summary)?;
                plan.autorun = true;
            }
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF
            | PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE => {
                Self::set_plan_phase(plan, phase, completion_summary)?;
                plan.autorun = false;
            }
            PLAN_REVIEW_DECISION_APPROVE | PLAN_REVIEW_DECISION_APPROVE_REQUESTED => {
                Self::set_plan_phase(plan, phase, completion_summary)?;
                if let Some(autorun) = requested_autorun {
                    plan.autorun = autorun;
                }
            }
            PLAN_REVIEW_DECISION_CANCELLED => {
                Self::set_plan_phase(plan, WorkflowPlanPhase::Cancelled, None)?;
            }
            _ => {
                plan.phase = WorkflowPlanPhase::Planning;
            }
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn phase_review_request(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        _completion_summary: Option<&str>,
        review_kind: PlanReviewKind,
    ) -> AskUserRequest {
        let requested_auto = requested_autorun.unwrap_or(plan.autorun);
        let mut options = if phase == WorkflowPlanPhase::Active {
            let approve_on = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON.to_string(),
                description: "Approve the plan, move it to active, and keep autorun on."
                    .to_string(),
            };
            let approve_off = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF.to_string(),
                description: "Approve the plan, move it to active, and keep autorun off."
                    .to_string(),
            };
            if requested_auto {
                vec![approve_on, approve_off]
            } else {
                vec![approve_off, approve_on]
            }
        } else {
            vec![HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE.to_string(),
                description: match phase {
                    WorkflowPlanPhase::Blocked => {
                        "Approve the plan and move it to blocked.".to_string()
                    }
                    WorkflowPlanPhase::Completed => {
                        "Approve the plan and mark it completed.".to_string()
                    }
                    WorkflowPlanPhase::Planning => {
                        "Approve the plan and return it to planning.".to_string()
                    }
                    WorkflowPlanPhase::Cancelled => "Approve the plan and cancel it.".to_string(),
                    WorkflowPlanPhase::Active => unreachable!(),
                },
            }]
        };
        options.extend([
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string(),
                description: "Return to planning so the plan can be edited further.".to_string(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_REJECT.to_string(),
                description: "Reject the current plan and mark the review as rejected.".to_string(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_CANCELLED.to_string(),
                description: "Cancel the plan entirely and stop work on it.".to_string(),
            },
        ]);
        // The review dialog renders this body above the decision options, so
        // the user sees the plan they are approving. The plan markdown is the
        // same full document `plan.get` with `PlanGetView::Full` produces.
        let body_markdown = Self::workflow_plan_markdown(plan);
        AskUserRequest {
            title: match review_kind {
                PlanReviewKind::Creation => "Approve New Plan".to_string(),
                PlanReviewKind::StatusChange => "Review Plan Status Change".to_string(),
            },
            kind: "review".to_string(),
            body_markdown,
            auto_resolution_ms: None,
            questions: vec![HostAskUserQuestion {
                header: "Decision".to_string(),
                question: format!(
                    "Choose whether this plan should move to {}, or type feedback for the agent to revise the plan.",
                    Self::plan_phase_label(phase)
                ),
                options,
                multiple: false,
                allow_custom: true,
            }],
            prompt: String::new(),
            options: Vec::new(),
            allow_free_text: true,
        }
    }

    pub(in crate::plugins::provided::workflow) async fn review_plan_status_transition(
        &self,
        mut plan: WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
        review_kind: PlanReviewKind,
    ) -> SdkResult<ToolInvokeOutput> {
        Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Planning, None)?;
        self.save_active_plan(&plan).await?;

        let response = self
            .host()?
            .ask_user(Self::phase_review_request(
                &plan,
                phase,
                requested_autorun,
                completion_summary,
                review_kind,
            ))
            .await?;

        let decision = Self::review_decision_from_response(&response);
        Self::apply_review_decision(
            &mut plan,
            &decision,
            phase,
            requested_autorun,
            completion_summary,
        )?;
        self.save_active_plan(&plan).await?;

        let output_text = if Self::is_review_feedback_decision(&decision) {
            Self::plan_output_text(
                format!(
                    "Plan review decision: {decision}. The user left feedback instead of picking an option; revise the plan to address it (for example with plan.edit, then propose it again via plan.review or plan.phase) and propose it again."
                )
                .as_str(),
                &plan,
            )
        } else {
            Self::plan_output_text(format!("Plan review decision: {decision}.").as_str(), &plan)
        };
        let payload = serde_json::json!({
            "plan": &plan,
            "decision": decision,
        });
        Ok(ToolInvokeOutput::from_parts(
            "plan review",
            format!("{decision} · {:?}", plan.phase),
            output_text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(in crate::plugins::provided::workflow) fn is_review_feedback_decision(
        decision: &str,
    ) -> bool {
        !matches!(
            decision,
            PLAN_REVIEW_DECISION_APPROVE
                | PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON
                | PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF
                | PLAN_REVIEW_DECISION_APPROVE_REQUESTED
                | PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE
                | PLAN_REVIEW_DECISION_KEEP_PLANNING
                | PLAN_REVIEW_DECISION_REJECT
                | PLAN_REVIEW_DECISION_CANCELLED
        )
    }
}
use super::{
    Arc, AskUserRequest, AskUserToolInput, AvailablePluginRecord, AvailableToolRecord, BTreeMap,
    ContributionKind, HashMap, HashSet, HostAskUserOption, HostAskUserQuestion, HostClient,
    HostDisplayContributeRequest, HostDisplayRemoveRequest, HostGetSessionRequest,
    HostRegisteredToolDescriptor, HostRenameSessionRequest, HostSession, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    OnceLock, PLAN_DISPLAY_CONTRIBUTION_ID, PLAN_KEY_ACTIVE, PLAN_NAMESPACE,
    PLAN_REVIEW_DECISION_APPROVE, PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF,
    PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON, PLAN_REVIEW_DECISION_APPROVE_REQUESTED,
    PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE, PLAN_REVIEW_DECISION_CANCELLED,
    PLAN_REVIEW_DECISION_KEEP_PLANNING, PLAN_REVIEW_DECISION_REJECT, Path, PathBuf, PlanEditInput,
    PlanEditTarget, PlanGetView, PlanPhaseInput, PlanReviewKind, PluginDisplayContent,
    PluginDisplayContribution, PluginError, RwLock, SdkResult, SessionRenameToolInput,
    SessionToolResponse, ToolApiStringBatch, ToolDescriptor, ToolInvokeOutput, ToolSearchDocument,
    ToolTagRecord, WorkflowPlan, WorkflowPlanCheckpoint, WorkflowPlanExecutor, WorkflowPlanPhase,
    WorkflowPlanStep, WorkflowPlanStepInput, WorkflowPlanStepStatus, WorkflowPlugin,
    WorkflowPluginConfig,
};

fn normalized_plugin_filters(plugin: Option<&ToolApiStringBatch>) -> Vec<String> {
    let mut filters = plugin
        .map(ToolApiStringBatch::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|value| value.trim().trim_end_matches('.'))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    filters.sort();
    filters.dedup();
    filters
}

fn plugin_id_matches(plugin_id: &str, filter: &str) -> bool {
    plugin_id == filter
        || plugin_id
            .strip_suffix(filter)
            .is_some_and(|prefix| prefix.ends_with('.'))
        || plugin_id.rsplit('.').next() == Some(filter)
}
