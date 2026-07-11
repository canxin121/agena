use super::{app_detail_labeled_line, app_detail_plain_line};

pub(in crate::app) fn style_for_role(role: MessageRole) -> Style {
    match role {
        MessageRole::User => Style::default().fg(agena_tui_components::theme::success_color()),
        MessageRole::Assistant => Style::default().fg(agena_tui_components::theme::accent_color()),
        MessageRole::System => Style::default().fg(agena_tui_components::theme::special_color()),
        MessageRole::Tool => Style::default().fg(agena_tui_components::theme::warning_color()),
    }
}

pub(in crate::app) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub(in crate::app) fn build_timeline_item(i18n: &I18n, record: &DomainEvent) -> TimelineItem {
    let event_type = timeline_event_type_label(i18n, record);
    let summary_suffix = timeline_event_summary(i18n, record);
    let summary = if summary_suffix.is_empty() {
        format!("#{}  {}", record.meta.seq_global, event_type)
    } else {
        format!(
            "#{}  {}  {}",
            record.meta.seq_global, event_type, summary_suffix
        )
    };

    let mut detail_lines = vec![
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-seq",
            record.meta.seq_global.to_string(),
        ),
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-created",
            format_timestamp(record.meta.created_at),
        ),
        timeline_detail_labeled_line(i18n, "timeline-label-type", event_type.clone()),
        timeline_detail_labeled_line(i18n, "timeline-label-event-id", record.meta.id.to_string()),
    ];
    if let Some(causation_id) = record.meta.causation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-causation-id",
            causation_id.to_string(),
        ));
    }
    if let Some(correlation_id) = record.meta.correlation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-correlation-id",
            correlation_id.to_string(),
        ));
    }
    detail_lines.push(app_detail_plain_line(String::new()));
    detail_lines.extend(timeline_event_detail_lines(i18n, record));

    let detail_document =
        build_detail_document(detail_lines.as_slice(), &DetailTextSpec::label_width(16));
    let copy_text = format!("{summary}\n\n{}", detail_document.plain);
    let search_text = format!(
        "{} {} {}",
        summary.to_ascii_lowercase(),
        detail_document.plain.to_ascii_lowercase(),
        record.kind.tag_str().to_ascii_lowercase(),
    );
    let linked_message_id = timeline_event_message_id(record);

    TimelineItem {
        summary,
        detail_body: detail_document.text,
        search_text,
        copy_text,
        linked_message_id,
    }
}

pub(in crate::app) fn timeline_event_message_id(record: &DomainEvent) -> Option<i64> {
    match &record.kind {
        AgenaSessionEvent::MessagePartUpdated(event) => Some(event.message_id),
        AgenaSessionEvent::MessagePartDelta(event) => Some(event.message_id),
        AgenaSessionEvent::CommandBegin(event) => event.context.message_id,
        AgenaSessionEvent::CommandOutputDelta(event) => event.context.message_id,
        AgenaSessionEvent::CommandEnd(event) => event.context.message_id,
        AgenaSessionEvent::UserMessageAppended(event) => Some(event.message_id.into()),
        AgenaSessionEvent::AssistantMessageCompleted(event) => Some(event.message_id.into()),
        AgenaSessionEvent::SystemNoticeAppended(event) => Some(event.message_id.into()),
        AgenaSessionEvent::ExecutionStarted(_)
        | AgenaSessionEvent::ExecutionFailed(_)
        | AgenaSessionEvent::StreamError(_)
        | AgenaSessionEvent::PermissionRequested(_)
        | AgenaSessionEvent::PermissionReplied(_)
        | AgenaSessionEvent::PermissionRuleCreated(_)
        | AgenaSessionEvent::PermissionRuleUpdated(_)
        | AgenaSessionEvent::PermissionRuleRevoked(_)
        | AgenaSessionEvent::RunStarted(_)
        | AgenaSessionEvent::RunCompleted(_)
        | AgenaSessionEvent::RunAborted(_)
        | AgenaSessionEvent::ToolCallIssued(_)
        | AgenaSessionEvent::ToolCallCompleted(_)
        | AgenaSessionEvent::PluginEvent(_)
        | AgenaSessionEvent::PluginToolRegistryChanged(_) => None,
    }
}

pub(in crate::app) fn timeline_event_type_key(record: &DomainEvent) -> &'static str {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(_) => "timeline-type-execution-started",
        AgenaSessionEvent::ExecutionFailed(_) => "timeline-type-execution-failed",
        AgenaSessionEvent::StreamError(_) => "timeline-type-stream-error",
        AgenaSessionEvent::MessagePartUpdated(_) => "timeline-type-message-part-updated",
        AgenaSessionEvent::MessagePartDelta(_) => "timeline-type-message-part-delta",
        AgenaSessionEvent::CommandBegin(_) => "timeline-type-command-begin",
        AgenaSessionEvent::CommandOutputDelta(_) => "timeline-type-command-output-delta",
        AgenaSessionEvent::CommandEnd(_) => "timeline-type-command-end",
        AgenaSessionEvent::PermissionRequested(_) => "timeline-type-permission-requested",
        AgenaSessionEvent::PermissionReplied(_) => "timeline-type-permission-replied",
        AgenaSessionEvent::PermissionRuleCreated(_) => "timeline-type-permission-rule-created",
        AgenaSessionEvent::PermissionRuleUpdated(_) => "timeline-type-permission-rule-updated",
        AgenaSessionEvent::PermissionRuleRevoked(_) => "timeline-type-permission-rule-revoked",
        AgenaSessionEvent::RunStarted(_) => "timeline-type-run-started",
        AgenaSessionEvent::RunCompleted(_) => "timeline-type-run-completed",
        AgenaSessionEvent::RunAborted(_) => "timeline-type-run-aborted",
        AgenaSessionEvent::UserMessageAppended(_) => "timeline-type-user-message-appended",
        AgenaSessionEvent::AssistantMessageCompleted(_) => {
            "timeline-type-assistant-message-completed"
        }
        AgenaSessionEvent::ToolCallIssued(_) => "timeline-type-tool-call-issued",
        AgenaSessionEvent::ToolCallCompleted(_) => "timeline-type-tool-call-completed",
        AgenaSessionEvent::SystemNoticeAppended(_) => "timeline-type-system-notice-appended",
        AgenaSessionEvent::PluginEvent(_) | AgenaSessionEvent::PluginToolRegistryChanged(_) => {
            "timeline-type-plugin-event"
        }
    }
}

pub(in crate::app) fn timeline_event_type_label(i18n: &I18n, record: &DomainEvent) -> String {
    ui_text::t(i18n, timeline_event_type_key(record))
}

pub(in crate::app) fn timeline_event_summary(i18n: &I18n, record: &DomainEvent) -> String {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => i18n.text_args(
            "timeline-summary-execution-started",
            &crate::fl_args!("id" => event.session_id),
        ),
        AgenaSessionEvent::ExecutionFailed(event) => {
            format!(
                "{}: {}",
                event.error.code,
                timeline_excerpt(i18n, event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::MessagePartUpdated(event) => i18n.text_args(
            "timeline-summary-message-part-updated",
            &crate::fl_args!(
                "message_id" => event.message_id,
                "part_id" => event.part.id,
                "kind" => event.part.kind.to_string(),
            ),
        ),
        AgenaSessionEvent::MessagePartDelta(event) => i18n.text_args(
            "timeline-summary-message-part-delta",
            &crate::fl_args!(
                "message_id" => event.message_id,
                "part_id" => event.part_id,
                "field" => timeline_part_delta_field_token(&event.field),
                "count" => event.delta.chars().count() as i64,
            ),
        ),
        AgenaSessionEvent::CommandBegin(event) => {
            timeline_excerpt(i18n, event.command.as_str(), 72)
        }
        AgenaSessionEvent::CommandOutputDelta(event) => {
            let preview = if event.preview_text.trim().is_empty() {
                i18n.text_args(
                    "timeline-summary-command-output-bytes",
                    &crate::fl_args!("count" => event.chunk.len() as i64),
                )
            } else {
                timeline_excerpt(i18n, event.preview_text.as_str(), 56)
            };
            i18n.text_args(
                "timeline-summary-command-output-delta",
                &crate::fl_args!(
                    "stream" => timeline_command_output_stream_token(event.stream.clone()),
                    "preview" => preview,
                ),
            )
        }
        AgenaSessionEvent::CommandEnd(event) => i18n.text_args(
            "timeline-summary-command-end",
            &crate::fl_args!(
                "status" => ui_text::execution_status_label(i18n, event.status),
                "exit_code" => event.exit_code,
                "duration_ms" => event.duration_ms as i64,
            ),
        ),
        AgenaSessionEvent::StreamError(event) => {
            format!(
                "{}: {}",
                event.error.code,
                timeline_excerpt(i18n, event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::PermissionRequested(event) => i18n.text_args(
            "timeline-summary-permission-requested",
            &crate::fl_args!(
                "risk" => permission_risk_label(i18n, event.risk),
                "reason" => timeline_excerpt(i18n, event.reason.as_str(), 72),
            ),
        ),
        AgenaSessionEvent::PermissionReplied(event) => i18n.text_args(
            "timeline-summary-permission-replied",
            &crate::fl_args!("kind" => ui_text::permission_reply_label(i18n, event.kind)),
        ),
        AgenaSessionEvent::PermissionRuleCreated(event) => i18n.text_args(
            "timeline-summary-permission-rule-created",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::PermissionRuleUpdated(event) => i18n.text_args(
            "timeline-summary-permission-rule-updated",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::PermissionRuleRevoked(event) => i18n.text_args(
            "timeline-summary-permission-rule-revoked",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::RunStarted(p) => i18n.text_args(
            "timeline-summary-run-started",
            &crate::fl_args!("id" => p.run_id),
        ),
        AgenaSessionEvent::RunCompleted(p) => i18n.text_args(
            "timeline-summary-run-completed",
            &crate::fl_args!(
                "id" => p.run_id,
                "finish" => p.finish_reason.to_string(),
            ),
        ),
        AgenaSessionEvent::RunAborted(p) => i18n.text_args(
            "timeline-summary-run-aborted",
            &crate::fl_args!(
                "id" => p.run_id,
                "reason" => p.reason.to_string(),
            ),
        ),
        AgenaSessionEvent::UserMessageAppended(p) => i18n.text_args(
            "timeline-summary-user-message-appended",
            &crate::fl_args!("id" => p.message_id),
        ),
        AgenaSessionEvent::AssistantMessageCompleted(p) => i18n.text_args(
            "timeline-summary-assistant-message-completed",
            &crate::fl_args!(
                "id" => p.message_id,
                "finish" => p.finish_reason.to_string(),
            ),
        ),
        AgenaSessionEvent::ToolCallIssued(p) => i18n.text_args(
            "timeline-summary-tool-call-issued",
            &crate::fl_args!(
                "name" => p.name.as_str(),
                "call_id" => p.call_id,
            ),
        ),
        AgenaSessionEvent::ToolCallCompleted(p) => i18n.text_args(
            "timeline-summary-tool-call-completed",
            &crate::fl_args!("call_id" => p.call_id),
        ),
        AgenaSessionEvent::SystemNoticeAppended(p) => i18n.text_args(
            "timeline-summary-system-notice-appended",
            &crate::fl_args!(
                "message_id" => p.message_id,
                "kind" => p.kind.to_string(),
            ),
        ),
        AgenaSessionEvent::PluginEvent(p) => i18n.text_args(
            "timeline-summary-plugin-event",
            &crate::fl_args!(
                "plugin_id" => p.plugin_id.clone(),
                "kind_label" => p.kind_label.clone(),
            ),
        ),
        AgenaSessionEvent::PluginToolRegistryChanged(event) => format!(
            "{} {} {}",
            event.plugin,
            match event.kind {
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Registered => "registered",
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Updated => "updated",
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Removed => "removed",
            },
            event.tool_key
        ),
    }
}

pub(in crate::app) fn timeline_event_detail_lines(
    i18n: &I18n,
    record: &DomainEvent,
) -> Vec<DetailTextLine<'static>> {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => vec![timeline_detail_labeled_line(
            i18n,
            "timeline-label-session-id",
            event.session_id.to_string(),
        )],
        AgenaSessionEvent::ExecutionFailed(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-code",
                event.error.code.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-message",
                event.error.message.clone(),
            ),
        ],
        AgenaSessionEvent::MessagePartUpdated(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                event.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-part-id", event.part.id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-part-kind",
                event.part.kind.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-status",
                ui_text::execution_status_label(i18n, event.part.status),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-summary",
                event
                    .part
                    .summary
                    .clone()
                    .unwrap_or_else(|| ui_text::t(i18n, "value-none")),
            ),
        ],
        AgenaSessionEvent::MessagePartDelta(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                event.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-part-id", event.part_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-field",
                timeline_part_delta_field_token(&event.field),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-seq", event.seq.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-delta",
                timeline_excerpt(i18n, event.delta.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::CommandBegin(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-command", event.command.clone()),
            timeline_detail_labeled_line(i18n, "timeline-label-cwd", event.cwd.clone()),
        ],
        AgenaSessionEvent::CommandOutputDelta(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-stream",
                timeline_command_output_stream_token(event.stream.clone()),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-seq", event.seq.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-bytes",
                event.chunk.len().to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-preview",
                timeline_excerpt(i18n, event.preview_text.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::CommandEnd(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-status",
                ui_text::execution_status_label(i18n, event.status),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-exit-code",
                event.exit_code.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-duration-ms",
                event.duration_ms.to_string(),
            ),
        ],
        AgenaSessionEvent::StreamError(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-code",
                event.error.code.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-message",
                event.error.message.clone(),
            ),
        ],
        AgenaSessionEvent::PermissionRequested(event) => {
            let mut lines = vec![
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-session-id",
                    event.session_id.to_string(),
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-request-id",
                    event.request_id.clone(),
                ),
                app_detail_plain_line(permission_action_label(i18n, &event.action)),
                timeline_detail_labeled_line(i18n, "timeline-label-reason", event.reason.clone()),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-risk",
                    permission_risk_label(i18n, event.risk),
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-explanation",
                    timeline_excerpt(i18n, event.explanation.as_str(), 200),
                ),
            ];
            if let Some(source) = event.source.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-source",
                    source.to_string(),
                ));
            }
            if let Some(scope) = event.scope.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-scope",
                    scope.to_string(),
                ));
            }
            if let Some(operator) = event.operator.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-operator",
                    operator.to_string(),
                ));
            }
            append_permission_action_detail_lines(
                i18n,
                &mut lines,
                "timeline-label-requested-actions",
                permission_requested_actions_for_display(
                    Some(&event.action),
                    event.requested_actions.as_slice(),
                )
                .as_slice(),
            );
            append_permission_action_detail_lines(
                i18n,
                &mut lines,
                "timeline-label-related-actions",
                permission_related_actions_for_display(
                    Some(&event.action),
                    event.related_actions.as_slice(),
                    event.requested_actions.as_slice(),
                )
                .as_slice(),
            );
            append_permission_trace_detail_lines(i18n, &mut lines, &event.trace);
            lines
        }
        AgenaSessionEvent::PermissionReplied(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-request-id",
                event.request_id.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-reply-kind",
                ui_text::permission_reply_label(i18n, event.kind),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-reason",
                timeline_value_or_none(i18n, event.reason.clone()),
            ),
        ],
        AgenaSessionEvent::PermissionRuleCreated(event)
        | AgenaSessionEvent::PermissionRuleUpdated(event)
        | AgenaSessionEvent::PermissionRuleRevoked(event) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-rule-id", event.rule_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-action-key",
                event.action_key.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-mode",
                permission_mode_token_display(i18n, event.mode.as_str()),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-scope",
                permission_rule_scope_display(i18n, event.scope.as_str()),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-source", event.source.clone()),
        ],
        AgenaSessionEvent::RunStarted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-model",
                format!("{}/{}", p.provider_id, p.model_id),
            ),
        ],
        AgenaSessionEvent::RunCompleted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-finish",
                p.finish_reason.to_string(),
            ),
        ],
        AgenaSessionEvent::RunAborted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-reason", p.reason.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message",
                timeline_value_or_none(i18n, p.message.clone()),
            ),
        ],
        AgenaSessionEvent::UserMessageAppended(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::AssistantMessageCompleted(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-finish",
                p.finish_reason.to_string(),
            ),
        ],
        AgenaSessionEvent::ToolCallIssued(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-call-id", p.call_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-name", p.name.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::ToolCallCompleted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-call-id", p.call_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::SystemNoticeAppended(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-kind", p.kind.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-text",
                timeline_excerpt(i18n, p.text.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::PluginEvent(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-plugin-id", p.plugin_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-kind-label", p.kind_label.clone()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-payload",
                timeline_excerpt(i18n, &p.payload.to_string(), 200),
            ),
        ],
        AgenaSessionEvent::PluginToolRegistryChanged(event) => {
            let plugin_full_name = event.plugin.to_string();
            let mut lines = vec![
                timeline_detail_labeled_line(i18n, "timeline-label-plugin-id", plugin_full_name),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-kind",
                    match event.kind {
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Registered => {
                            "registered"
                        }
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Updated => "updated",
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Removed => "removed",
                    },
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-name",
                    event.tool_key.name().to_owned(),
                ),
                app_detail_plain_line(format!("tool_key: {}", event.tool_key)),
                app_detail_plain_line(format!("generation: {}", event.generation)),
                app_detail_plain_line(format!("timestamp_ms: {}", event.timestamp_ms)),
            ];
            if let Some(tool) = &event.tool {
                let summary = tool
                    .summary_text()
                    .or_else(|| tool.help_text())
                    .unwrap_or(tool.name.as_str())
                    .to_owned();
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-summary",
                    summary,
                ));
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-payload",
                    timeline_excerpt(i18n, &tool.contract.input_schema.to_string(), 200),
                ));
            }
            lines
        }
    }
}

pub(in crate::app) fn timeline_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    app_detail_labeled_line(ui_text::t(i18n, label_key), value.into())
}

pub(in crate::app) fn timeline_excerpt(i18n: &I18n, text: &str, max_chars: usize) -> String {
    if text.trim().is_empty() {
        ui_text::t(i18n, "value-none")
    } else {
        detail_excerpt(text, max_chars)
    }
}

pub(in crate::app) fn timeline_value_or_none<T: ToString>(i18n: &I18n, value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| ui_text::t(i18n, "value-none"))
}

pub(in crate::app) fn timeline_part_delta_field_token(
    field: &agena::event::PartDeltaField,
) -> String {
    match field {
        agena::event::PartDeltaField::Text => "text".to_string(),
        agena::event::PartDeltaField::ReasoningSummary => "reasoning_summary".to_string(),
        agena::event::PartDeltaField::ReasoningRawContent => "reasoning_raw_content".to_string(),
        agena::event::PartDeltaField::CommandStdout => "command_stdout".to_string(),
        agena::event::PartDeltaField::CommandStderr => "command_stderr".to_string(),
        agena::event::PartDeltaField::ToolOutputText => "tool_output_text".to_string(),
        agena::event::PartDeltaField::Custom { name } => format!("custom/{name}"),
    }
}

pub(in crate::app) fn timeline_command_output_stream_token(
    stream: agena::event::CommandOutputStream,
) -> &'static str {
    match stream {
        agena::event::CommandOutputStream::Stdout => "stdout",
        agena::event::CommandOutputStream::Stderr => "stderr",
    }
}

pub(in crate::app) fn append_permission_action_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    label_key: &str,
    actions: &[&PermissionAction],
) {
    if actions.is_empty() {
        return;
    }
    lines.push(app_detail_plain_line(format!(
        "{}:",
        ui_text::t(i18n, label_key)
    )));
    lines.extend(actions.iter().map(|action| {
        app_detail_plain_line(format!("  {}", permission_action_label(i18n, action)))
    }));
}

pub(in crate::app) fn append_permission_trace_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    trace: &[DecisionTraceStep],
) {
    if trace.is_empty() {
        return;
    }
    lines.push(app_detail_plain_line(format!(
        "{}:",
        ui_text::t(i18n, "timeline-label-trace")
    )));
    lines.extend(trace.iter().map(|step| {
        app_detail_plain_line(format!("  {}", permission_trace_step_label(i18n, step)))
    }));
}

pub(in crate::app) fn permission_risk_label(i18n: &I18n, risk: PermissionRiskLevel) -> String {
    ui_text::t(
        i18n,
        match risk {
            PermissionRiskLevel::Low => "value-risk-low",
            PermissionRiskLevel::Medium => "value-risk-medium",
            PermissionRiskLevel::High => "value-risk-high",
            PermissionRiskLevel::Critical => "value-risk-critical",
        },
    )
}

pub(in crate::app) fn permission_trace_step_label(i18n: &I18n, step: &DecisionTraceStep) -> String {
    let source_kind = match step.source_kind {
        PolicySourceKind::StaticPolicy => "static_policy",
        PolicySourceKind::PersistedRule => "persisted_rule",
        PolicySourceKind::PluginAdvice => "plugin_advice",
        PolicySourceKind::ManagedPolicy => "managed_policy",
    };
    let mut facts = vec![source_kind.to_string()];
    if let Some(source) = step.source.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-source").as_str(),
            source,
        ));
    }
    if let Some(scope) = step.scope {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-scope").as_str(),
            permission_scope_label(i18n, scope).as_str(),
        ));
    }
    if let Some(operator) = step.operator.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-operator").as_str(),
            operator,
        ));
    }
    format!("- {} — {}", join_inline_segments(facts), step.summary)
}

pub(in crate::app) fn permission_scope_label(i18n: &I18n, scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => ui_text::t(i18n, "value-session"),
        PermissionScope::Workspace => ui_text::t(i18n, "value-workspace"),
        PermissionScope::Global => ui_text::t(i18n, "value-global"),
    }
}

pub(in crate::app) fn detail_excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(empty)".to_string();
    }
    let compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
use crate::app::{
    AgenaSessionEvent, DateTime, DecisionTraceStep, DetailTextLine, DetailTextSpec, DomainEvent,
    I18n, Local, MessageRole, PermissionAction, PermissionRiskLevel, PermissionScope,
    PolicySourceKind, Style, TimelineItem, Utc, build_detail_document, format_key_value_segment,
    join_inline_segments, permission_action_label, permission_mode_token_display,
    permission_related_actions_for_display, permission_requested_actions_for_display,
    permission_rule_scope_display, ui_text,
};
