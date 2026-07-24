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
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
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
        let message = if suggestions.is_empty() {
            format!("unknown tool '{requested}'")
        } else {
            format!(
                "unknown tool '{requested}'. Did you mean {}?",
                suggestions
                    .iter()
                    .map(|tool| format!("`{tool}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Err(PluginError::invalid_params(message))
    }

    pub(in crate::plugins::provided::workflow) async fn switch_agent_for_tool(
        &self,
        agent: Option<String>,
        push_previous: bool,
    ) -> SdkResult<HostAgentSwitchResponse> {
        self.host()?
            .agent_switch(HostAgentSwitchRequest {
                agent,
                session_id: None,
                push_previous,
            })
            .await
    }

    pub(in crate::plugins::provided::workflow) async fn restore_agent_for_tool(
        &self,
    ) -> SdkResult<HostAgentRestoreResponse> {
        self.host()?
            .agent_restore(HostAgentRestoreRequest { session_id: None })
            .await
    }

    pub(in crate::plugins::provided::workflow) fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("workflow plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
    }

    pub(in crate::plugins::provided::workflow) fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("workflow plugin workspace root not initialized"))
    }

    pub(in crate::plugins::provided::workflow) fn host_ask_user_questions(
        input: &AskUserToolInput,
    ) -> Vec<HostAskUserQuestion> {
        input
            .questions
            .iter()
            .map(|question| HostAskUserQuestion {
                id: question.id.clone(),
                header: question.header.clone(),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| HostAskUserOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                        preview_markdown: option.preview_markdown.clone(),
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
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(records)
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
            .map(|(tag, tool_count)| ToolTagRecord { tag, tool_count })
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
            .map_err(|err| PluginError::new(err.to_string()))
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
            .map_err(|err| PluginError::new(format!("invalid stored plan payload: {err}")))
    }

    pub(in crate::plugins::provided::workflow) async fn save_active_plan(
        &self,
        plan: &WorkflowPlan,
    ) -> SdkResult<()> {
        let value =
            serde_json::to_string_pretty(plan).map_err(|err| PluginError::new(err.to_string()))?;
        let host = self.host()?;
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Shared,
            namespace: PLAN_NAMESPACE.to_string(),
            key: PLAN_KEY_ACTIVE.to_string(),
            value,
        })
        .await?;
        self.clear_autorun_signature().await?;
        self.sync_plan_statusline(Some(plan)).await?;
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
        self.clear_autorun_signature().await?;
        self.sync_plan_statusline(None).await?;
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) async fn load_autorun_signature(
        &self,
    ) -> SdkResult<Option<String>> {
        Ok(self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
            })
            .await?
            .value)
    }

    pub(in crate::plugins::provided::workflow) async fn save_autorun_signature(
        &self,
        signature: &str,
    ) -> SdkResult<()> {
        self.host()?
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
                value: signature.to_string(),
            })
            .await
    }

    pub(in crate::plugins::provided::workflow) async fn clear_autorun_signature(
        &self,
    ) -> SdkResult<()> {
        self.host()?
            .storage_delete(HostStorageDeleteRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
            })
            .await
    }

    pub(in crate::plugins::provided::workflow) async fn sync_plan_statusline(
        &self,
        plan: Option<&WorkflowPlan>,
    ) -> SdkResult<()> {
        let host = self.host()?;
        match plan {
            Some(plan) => {
                host.ui_statusline_contribute(HostStatuslineContributeRequest {
                    segment_id: PLAN_STATUSLINE_SEGMENT_ID.to_string(),
                    content: Self::plan_statusline_content(plan),
                    priority: 120,
                    color: None,
                })
                .await?;
            }
            None => {
                let _ = host
                    .ui_statusline_remove(HostStatuslineRemoveRequest {
                        segment_id: PLAN_STATUSLINE_SEGMENT_ID.to_string(),
                    })
                    .await?;
            }
        }
        Ok(())
    }

    pub(in crate::plugins::provided::workflow) fn plan_payload(
        plan: &WorkflowPlan,
    ) -> SdkResult<serde_json::Value> {
        serde_json::to_value(serde_json::json!({ "plan": plan }))
            .map_err(|err| PluginError::new(err.to_string()))
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
                        id: checkpoint
                            .id
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| {
                                format!("step_{}_check_{}", step_index + 1, checkpoint_index + 1)
                            }),
                        text: text.to_string(),
                        status: checkpoint.status.unwrap_or_default(),
                    })
                })
                .collect::<SdkResult<Vec<_>>>()?;
            steps.push(WorkflowPlanStep {
                id: step
                    .id
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("step_{}", step_index + 1)),
                title: resolved_title.to_string(),
                description: description.to_string(),
                executor: step.executor,
                status: step.status.unwrap_or_default(),
                wait_until_ms: step.wait_until_ms,
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

    pub(in crate::plugins::provided::workflow) fn normalize_identifier(value: &str) -> String {
        value
            .trim()
            .chars()
            .filter_map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    Some(ch.to_ascii_lowercase())
                } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
                    Some('_')
                } else {
                    None
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    pub(in crate::plugins::provided::workflow) fn parse_1_based_index_hint(
        value: &str,
        prefixes: &[&str],
    ) -> Option<usize> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(index) = trimmed.parse::<usize>() {
            return index.checked_sub(1);
        }
        let normalized = Self::normalize_identifier(trimmed);
        for prefix in prefixes {
            for candidate in [
                prefix.to_string(),
                format!("{prefix}_"),
                format!("{prefix}-"),
            ] {
                if let Some(rest) = normalized.strip_prefix(candidate.as_str())
                    && let Ok(index) = rest.parse::<usize>()
                {
                    return index.checked_sub(1);
                }
            }
        }
        None
    }

    pub(in crate::plugins::provided::workflow) fn resolve_plan_step_index(
        plan: &WorkflowPlan,
        step_id: &str,
    ) -> Option<usize> {
        let normalized_target = Self::normalize_identifier(step_id);
        plan.steps
            .iter()
            .position(|step| step.id == step_id)
            .or_else(|| {
                plan.steps.iter().position(|step| {
                    let title = Self::normalize_identifier(step.title.as_str());
                    let description = Self::normalize_identifier(step.description.as_str());
                    !normalized_target.is_empty()
                        && (title == normalized_target || description == normalized_target)
                })
            })
            .or_else(|| Self::parse_1_based_index_hint(step_id, &["step", "s"]))
            .filter(|index| *index < plan.steps.len())
    }

    pub(in crate::plugins::provided::workflow) fn resolve_checkpoint_index(
        step: &WorkflowPlanStep,
        checkpoint_id: &str,
    ) -> Option<usize> {
        let normalized_target = Self::normalize_identifier(checkpoint_id);
        step.checkpoints
            .iter()
            .position(|checkpoint| checkpoint.id == checkpoint_id)
            .or_else(|| {
                step.checkpoints.iter().position(|checkpoint| {
                    let text = Self::normalize_identifier(checkpoint.text.as_str());
                    !normalized_target.is_empty() && text == normalized_target
                })
            })
            .or_else(|| {
                Self::parse_1_based_index_hint(checkpoint_id, &["check", "checkpoint", "cp", "c"])
            })
            .filter(|index| *index < step.checkpoints.len())
    }

    pub(in crate::plugins::provided::workflow) fn plan_step_identifier_hint(
        step: &WorkflowPlanStep,
        index: usize,
    ) -> String {
        format!("step_id={} (step {})", step.id, index + 1)
    }

    pub(in crate::plugins::provided::workflow) fn checkpoint_identifier_hint(
        checkpoint: &WorkflowPlanCheckpoint,
        index: usize,
    ) -> String {
        format!("check_id={} (check {})", checkpoint.id, index + 1)
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

    pub(in crate::plugins::provided::workflow) fn plan_statusline_content(
        plan: &WorkflowPlan,
    ) -> String {
        let (completed_steps, total_steps, _, _) = Self::plan_progress_counts(plan);
        if total_steps == 0 {
            return format!(
                "plan:{} autorun:{}",
                Self::plan_phase_label(plan.phase),
                if plan.autorun { "on" } else { "off" }
            );
        }
        format!(
            "plan:{} steps:{}/{} autorun:{}",
            Self::plan_phase_label(plan.phase),
            completed_steps,
            total_steps,
            if plan.autorun { "on" } else { "off" }
        )
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
                "Current step {}: '{}' [{}].\nGoal: {}\nStatus: {}.",
                index + 1,
                step.title,
                Self::plan_step_identifier_hint(step, index),
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

    pub(in crate::plugins::provided::workflow) fn normalized_optional_identifier(
        value: Option<&str>,
        field_name: &str,
    ) -> SdkResult<Option<String>> {
        match value.map(str::trim) {
            Some("") => Err(PluginError::invalid_params(format!(
                "{field_name} must not be empty when provided"
            ))),
            Some(value) => Ok(Some(value.to_string())),
            None => Ok(None),
        }
    }

    pub(in crate::plugins::provided::workflow) fn validate_plan_update_input(
        input: &PlanUpdateInput,
    ) -> SdkResult<PlanUpdateTarget> {
        let phase_update_requested =
            input.phase.is_some() || input.autorun.is_some() || input.summary.is_some();
        let step_id = Self::normalized_optional_identifier(input.step_id.as_deref(), "step_id")?;
        let checkpoint_id =
            Self::normalized_optional_identifier(input.checkpoint_id.as_deref(), "check_id")?;

        if phase_update_requested {
            if step_id.is_some()
                || checkpoint_id.is_some()
                || input.status.is_some()
                || input.wait_until_ms.is_some()
                || input.note.is_some()
            {
                return Err(PluginError::invalid_params(
                    "plan.update must target either the plan itself or a step/check, not both"
                        .to_string(),
                ));
            }
            if input.summary.is_some() && input.phase != Some(WorkflowPlanPhase::Completed) {
                return Err(PluginError::invalid_params(
                    "plan.update summary is only valid when phase is `completed`".to_string(),
                ));
            }
            if input.phase.is_none() && input.autorun.is_none() {
                return Err(PluginError::invalid_params(
                    "plan.update requires `phase` or `autorun` for plan-level updates".to_string(),
                ));
            }
            return Ok(PlanUpdateTarget::Plan);
        }

        let Some(step_id) = step_id else {
            if checkpoint_id.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates require step_id".to_string(),
                ));
            }
            return Err(PluginError::invalid_params(
                "plan.update requires either `phase` / `autorun` or `step_id`".to_string(),
            ));
        };

        if let Some(checkpoint_id) = checkpoint_id {
            if input.status.is_none() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates require `status`".to_string(),
                ));
            }
            if input.wait_until_ms.is_some() || input.note.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates do not support `wait_until_ms` or `note`"
                        .to_string(),
                ));
            }
            return Ok(PlanUpdateTarget::Check {
                step_id,
                checkpoint_id,
            });
        }

        if input.status.is_none() && input.wait_until_ms.is_none() && input.note.is_none() {
            return Err(PluginError::invalid_params(
                "plan.update step updates require at least one of `status`, `wait_until_ms`, or `note`"
                    .to_string(),
            ));
        }

        Ok(PlanUpdateTarget::Step(step_id))
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

    pub(in crate::plugins::provided::workflow) fn plan_auto_signature(
        plan: &WorkflowPlan,
        step_index: usize,
        step: &WorkflowPlanStep,
    ) -> SdkResult<String> {
        let serialized =
            serde_json::to_string(plan).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(format!("{serialized}:{step_index}:{}", step.id))
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

    pub(in crate::plugins::provided::workflow) fn phase_review_transition_summary(
        phase: WorkflowPlanPhase,
        effective_autorun: bool,
    ) -> String {
        match phase {
            WorkflowPlanPhase::Active => format!(
                "Move the plan to `active` with autorun {}.",
                if effective_autorun { "on" } else { "off" }
            ),
            WorkflowPlanPhase::Blocked => {
                "Move the plan to `blocked` after review approval.".to_string()
            }
            WorkflowPlanPhase::Completed => {
                "Mark the plan `completed` after review approval.".to_string()
            }
            WorkflowPlanPhase::Planning => {
                "Return the plan to `planning` after review approval.".to_string()
            }
            WorkflowPlanPhase::Cancelled => {
                "Move the plan to `cancelled` after review approval.".to_string()
            }
        }
    }

    pub(in crate::plugins::provided::workflow) fn phase_review_body_markdown(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> String {
        let effective_autorun = requested_autorun.unwrap_or(plan.autorun);
        let mut sections = vec![
            "## Requested Status Change".to_string(),
            String::new(),
            Self::phase_review_transition_summary(phase, effective_autorun),
        ];
        if phase == WorkflowPlanPhase::Completed
            && let Some(summary) = completion_summary
                .map(str::trim)
                .filter(|value| !value.is_empty())
        {
            sections.push(String::new());
            sections.push("### Completion Summary".to_string());
            sections.push(String::new());
            sections.push(summary.to_string());
        }
        sections.push(String::new());
        sections.push(Self::workflow_plan_markdown(plan));
        sections.join("\n")
    }

    pub(in crate::plugins::provided::workflow) fn phase_review_request(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> AskUserRequest {
        let requested_auto = requested_autorun.unwrap_or(plan.autorun);
        let mut options = if phase == WorkflowPlanPhase::Active {
            let approve_on = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON.to_string(),
                description: "Approve the plan, move it to active, and keep autorun on."
                    .to_string(),
                preview_markdown: String::new(),
            };
            let approve_off = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF.to_string(),
                description: "Approve the plan, move it to active, and keep autorun off."
                    .to_string(),
                preview_markdown: String::new(),
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
                preview_markdown: String::new(),
            }]
        };
        options.extend([
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string(),
                description: "Return to planning so the plan can be edited further.".to_string(),
                preview_markdown: String::new(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_REJECT.to_string(),
                description: "Reject the current plan and mark the review as rejected.".to_string(),
                preview_markdown: String::new(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_CANCELLED.to_string(),
                description: "Cancel the plan entirely and stop work on it.".to_string(),
                preview_markdown: String::new(),
            },
        ]);
        AskUserRequest {
            title: "Review Plan Status Change".to_string(),
            body_markdown: Self::phase_review_body_markdown(
                plan,
                phase,
                requested_autorun,
                completion_summary,
            ),
            kind: "review".to_string(),
            submit_label: "Submit decision".to_string(),
            cancel_label: "Keep in planning".to_string(),
            auto_resolution_ms: None,
            questions: vec![HostAskUserQuestion {
                id: "decision".to_string(),
                header: "Decision".to_string(),
                question: format!(
                    "Choose whether this plan should move to {}.",
                    Self::plan_phase_label(phase)
                ),
                options,
                multiple: false,
                allow_custom: false,
            }],
            prompt: String::new(),
            options: Vec::new(),
            allow_free_text: false,
        }
    }

    pub(in crate::plugins::provided::workflow) async fn review_plan_status_transition(
        &self,
        mut plan: WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
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
            ))
            .await?;

        let decision = if response.cancelled {
            PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string()
        } else {
            Self::review_decision(&response)
                .unwrap_or_else(|| PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string())
        };

        match decision.as_str() {
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                plan.autorun = true;
            }
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF
            | PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                plan.autorun = false;
            }
            PLAN_REVIEW_DECISION_APPROVE | PLAN_REVIEW_DECISION_APPROVE_REQUESTED => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                if let Some(autorun) = requested_autorun {
                    plan.autorun = autorun;
                }
            }
            PLAN_REVIEW_DECISION_CANCELLED => {
                Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Cancelled, None)?;
            }
            _ => {
                plan.phase = WorkflowPlanPhase::Planning;
            }
        }
        self.save_active_plan(&plan).await?;

        let output_text =
            Self::plan_output_text(format!("Plan review decision: {decision}.").as_str(), &plan);
        let payload = serde_json::json!({
            "plan": plan,
            "decision": decision,
        });
        Ok(ToolInvokeOutput::from_parts(
            "plan review",
            output_text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }
}
use super::{
    Arc, AskUserRequest, AskUserToolInput, AvailableToolRecord, BTreeMap, HashMap, HashSet,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostAskUserOption, HostAskUserQuestion, HostClient,
    HostGetSessionRequest, HostRegisteredToolDescriptor, HostRenameSessionRequest, HostSession,
    HostStatuslineContributeRequest, HostStatuslineRemoveRequest, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    OnceLock, PLAN_KEY_ACTIVE, PLAN_NAMESPACE, PLAN_REVIEW_DECISION_APPROVE,
    PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF,
    PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON, PLAN_REVIEW_DECISION_APPROVE_REQUESTED,
    PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE, PLAN_REVIEW_DECISION_CANCELLED,
    PLAN_REVIEW_DECISION_KEEP_PLANNING, PLAN_REVIEW_DECISION_REJECT,
    PLAN_RUNTIME_AUTO_SIGNATURE_KEY, PLAN_RUNTIME_NAMESPACE, PLAN_STATUSLINE_SEGMENT_ID, Path,
    PathBuf, PlanGetView, PlanUpdateInput, PlanUpdateTarget, PluginError, RwLock, SdkResult,
    SessionRenameToolInput, SessionToolResponse, ToolDescriptor, ToolInvokeOutput,
    ToolSearchDocument, ToolTagRecord, WorkflowPlan, WorkflowPlanCheckpoint, WorkflowPlanExecutor,
    WorkflowPlanPhase, WorkflowPlanStep, WorkflowPlanStepInput, WorkflowPlanStepStatus,
    WorkflowPlugin, WorkflowPluginConfig,
};
