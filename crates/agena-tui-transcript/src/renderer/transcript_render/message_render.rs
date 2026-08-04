use super::super::transcript_ast::{MarkdownNode, markdown_inline_line, render_attachment_image};
use super::super::{
    I18n, Local, MessageStatus, Modifier, RenderedLine, RenderedTranscriptNode,
    SessionExecutionResource, Style, TOOL_CARD_PREVIEW_CHARS, TOOL_CARD_PREVIEW_LINES,
    ToolOutputPreview, TranscriptDetailDefaults, TranscriptEntry, TranscriptNodeKey,
    TranscriptNodeKind, UnicodeWidthStr, concise_text, format_timestamp, push_activity_headline,
    push_expanded_markdown, push_expanded_tool_text, push_label_value, push_markdown_document,
    push_section_heading, push_wrapped_line, render_entry_detailed,
    render_expanded_tool_text_block, strip_terminal_ansi_sequences, style_for_role,
    tool_output_copy_text, transcript_message_parts, transcript_part_content,
    transcript_spinner_placeholder, trim_empty_line_edges, truncate_display_width,
};
use super::operation_render::render_tool_execution;
use super::request_render::{preview_for_part, render_user_input_request};
use crate::snapshot::activity_presentation;
use crate::ui_text;
use crate::{
    MessageRequestPartResource, TranscriptActivityContent, TranscriptActivitySection,
    TranscriptAssistantReplyLifecycle, TranscriptEntryPart, TranscriptPartContent,
};
use agena_api::resource::{
    MessageAttachment, MessageAttachmentKind, MessageAttachmentSource, MessageResource,
};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

/// Export and pager output is a document, not an infinitely wide terminal.
/// Keeping this width bounded prevents visual rules and code-card borders from
/// expanding to `u16::MAX`-sized lines while remaining comfortable to read in
/// both terminal pagers and text editors.
pub(crate) const TRANSCRIPT_EXPORT_WIDTH: u16 = 120;

pub fn render_entry_export(
    message: &TranscriptEntry,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    agena_tui_media::with_text_math_rendering(|| {
        render_entry_detailed(
            message,
            TRANSCRIPT_EXPORT_WIDTH,
            i18n,
            defaults,
            &std::collections::BTreeMap::new(),
        )
        .lines
    })
}

#[derive(Debug, Clone)]
pub struct RenderedMessageBlock {
    pub lines: Vec<RenderedLine>,
    pub nodes: Vec<RenderedTranscriptNode>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedNodeDraft {
    key: TranscriptNodeKey,
    kind: TranscriptNodeKind,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
    /// Canonical Activities with independently navigable sections own only
    /// their headline range. Other nodes default to all lines they rendered.
    end_line: Option<usize>,
    children: Vec<RenderedTranscriptNode>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_rendered_part_node(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    width: u16,
    lines: &mut Vec<RenderedLine>,
    nodes: &mut Vec<RenderedTranscriptNode>,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) {
    // Like Markdown blocks, non-text parts start after the message header so
    // selecting the first activity part never highlights `assistant`.
    let start_line = lines.len();
    let node = render_part_node(message, part, width, lines, i18n, defaults, expansions);
    if lines.len() > start_line {
        let end_line = node.end_line.unwrap_or(lines.len());
        let atomic = node.kind.uses_atomic_navigation()
            || lines[start_line..end_line]
                .iter()
                .any(|line| !line.math.is_empty());
        nodes.push(RenderedTranscriptNode {
            key: node.key,
            kind: node.kind,
            start_line,
            end_line,
            copy_text: node.copy_text,
            atomic,
            toggleable: node.toggleable,
            expanded: node.expanded,
        });
        nodes.extend(node.children);
    }
}

pub(crate) fn collapsed_activity_run_end(
    parts: &[TranscriptEntryPart],
    start: usize,
) -> Option<usize> {
    is_activity_node(parts.get(start)?).then(|| {
        let mut end = start.saturating_add(1);
        while parts.get(end).is_some_and(|part| {
            is_activity_node(part) || is_invisible_activity_run_bridge(parts, end)
        }) {
            end = end.saturating_add(1);
        }
        end
    })
}

pub(crate) const COLLAPSED_ACTIVITY_VISIBLE_COUNT: usize = 5;

fn is_invisible_activity_run_bridge(parts: &[TranscriptEntryPart], index: usize) -> bool {
    matches!(
        parts.get(index).map(transcript_part_content),
        Some(TranscriptPartContent::Text(text)) if text.text.trim().is_empty()
    )
}

pub(crate) fn is_activity_node(part: &TranscriptEntryPart) -> bool {
    matches!(
        transcript_part_content(part),
        TranscriptPartContent::Activity(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalActivityDetailFormat {
    /// Render line-oriented output as plain text, but promote recognisable
    /// Markdown documents to the shared AST renderer.
    Auto,
    Markdown,
    Json,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalActivityDetail {
    title: Option<String>,
    body: String,
    format: CanonicalActivityDetailFormat,
    stable_section: Option<TranscriptActivitySection>,
    default_expanded: Option<bool>,
}

impl CanonicalActivityDetail {
    fn section(
        title: impl Into<String>,
        body: impl Into<String>,
        format: CanonicalActivityDetailFormat,
    ) -> Self {
        Self {
            title: Some(title.into()),
            body: body.into(),
            format,
            stable_section: None,
            default_expanded: Some(true),
        }
    }

    fn identified_section(
        stable_section: TranscriptActivitySection,
        title: impl Into<String>,
        body: impl Into<String>,
        format: CanonicalActivityDetailFormat,
        default_expanded: bool,
    ) -> Self {
        Self {
            title: Some(title.into()),
            body: body.into(),
            format,
            stable_section: Some(stable_section),
            default_expanded: Some(default_expanded),
        }
    }

    fn body(body: impl Into<String>, format: CanonicalActivityDetailFormat) -> Self {
        Self {
            title: None,
            body: body.into(),
            format,
            stable_section: None,
            default_expanded: None,
        }
    }

    fn copy_text(&self) -> String {
        self.title.as_ref().map_or_else(
            || self.body.clone(),
            |title| format!("{title}\n{}", self.body),
        )
    }

    fn navigation_section(&self, detail_index: &mut usize) -> TranscriptActivitySection {
        if let Some(section) = self.stable_section {
            return section;
        }
        let section = TranscriptActivitySection::Detail(*detail_index);
        *detail_index = detail_index.saturating_add(1);
        section
    }
}

fn canonical_activity_details(
    payload: &agena_domain::ActivityPayload,
    summary: &str,
    error_equivalence_text: Option<&str>,
) -> Vec<CanonicalActivityDetail> {
    match payload {
        agena_domain::ActivityPayload::Operation(operation) => {
            let mut details = Vec::new();
            let mut has_result_presentation = false;
            if !operation.authorization.permissions.is_empty() {
                let permissions = operation
                    .authorization
                    .permissions
                    .iter()
                    .map(|permission| {
                        let status = match permission.reply.as_ref().map(|reply| reply.kind) {
                            None => "Awaiting user approval",
                            Some(agena_domain::PermissionReplyKind::AllowOnce) => "Allowed once",
                            Some(agena_domain::PermissionReplyKind::AllowAlways) => {
                                "Allowed persistently"
                            }
                            Some(agena_domain::PermissionReplyKind::DenyOnce) => "Denied once",
                            Some(agena_domain::PermissionReplyKind::DenyAlways) => {
                                "Denied persistently"
                            }
                            // AutoApprove is downgraded before being recorded, so
                            // it cannot appear on a stored reply; keep a fallback.
                            Some(agena_domain::PermissionReplyKind::AutoApprove) => {
                                "Auto-approval requested"
                            }
                        };
                        let action = match &permission.request.action {
                            agena_domain::PermissionAction::Tool {
                                tool_name,
                                qualifier,
                            } => qualifier.as_deref().map_or_else(
                                || tool_name.clone(),
                                |qualifier| format!("{tool_name} · {qualifier}"),
                            ),
                            agena_domain::PermissionAction::PathAccess {
                                access_kind,
                                target_path,
                                ..
                            } => format!("{access_kind} {target_path}"),
                            agena_domain::PermissionAction::NetworkAccess { target, .. } => {
                                format!("network {target}")
                            }
                        };
                        let mut lines = vec![format!("{status} · {action}")];
                        if !permission.request.reason.trim().is_empty() {
                            lines.push(format!("Request: {}", permission.request.reason));
                        }
                        if !permission.request.explanation.trim().is_empty()
                            && permission.request.explanation.trim()
                                != permission.request.reason.trim()
                        {
                            lines.push(format!("Policy: {}", permission.request.explanation));
                        }
                        if let Some(reason) = permission
                            .reply
                            .as_ref()
                            .and_then(|reply| reply.reason.as_deref())
                            .filter(|reason| !reason.trim().is_empty())
                            && reason.trim() != permission.request.reason.trim()
                        {
                            lines.push(format!("Reply: {reason}"));
                        }
                        let provenance = [
                            permission.request.source.clone(),
                            permission.request.scope.map(|scope| format!("{scope}")),
                        ]
                        .into_iter()
                        .flatten()
                        .filter(|value| !value.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                        if !provenance.is_empty() {
                            lines.push(provenance);
                        }
                        lines.join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                details.push(CanonicalActivityDetail::identified_section(
                    TranscriptActivitySection::Permissions,
                    "Permissions",
                    permissions,
                    CanonicalActivityDetailFormat::Plain,
                    false,
                ));
            }
            if !operation.invocation.input.is_empty()
                && let Ok(input) = serde_json::to_string_pretty(&serde_json::Value::from(
                    operation.invocation.input.clone(),
                ))
            {
                details.push(CanonicalActivityDetail::identified_section(
                    TranscriptActivitySection::Input,
                    "Input",
                    input,
                    CanonicalActivityDetailFormat::Json,
                    false,
                ));
            }
            // The human-facing detail is derived from the compact tool result
            // by the runtime. The durable record carries only compact `data`;
            // the runtime attaches the derived `markdown` to the snapshot
            // projection, and clients may also fetch it lazily on expansion.
            // Structured data is the fallback when no derived Markdown exists.
            // Avoid duplicating the failure text as a separate Result when the
            // tool's detail is just the error message itself.
            let detail_equals_error = error_equivalence_text
                .is_some_and(|error| canonical_text_equivalent(operation.markdown.as_str(), error));
            if !detail_equals_error {
                if !operation.markdown.trim().is_empty() {
                    details.push(CanonicalActivityDetail::identified_section(
                        TranscriptActivitySection::Result,
                        "Output",
                        operation.markdown.clone(),
                        CanonicalActivityDetailFormat::Markdown,
                        true,
                    ));
                    has_result_presentation = true;
                } else if !operation.data.is_null()
                    && let Ok(output) = serde_json::to_string_pretty(&operation.data)
                {
                    details.push(CanonicalActivityDetail::identified_section(
                        TranscriptActivitySection::Result,
                        "Output",
                        output,
                        CanonicalActivityDetailFormat::Json,
                        true,
                    ));
                    has_result_presentation = true;
                }
            }
            // `summary` is the compact collapsed projection. Expanded
            // Operations render the actual output/sections instead. It is
            // only a fallback result when the producer supplied no detailed
            // result at all; failures are rendered exclusively from `error`.
            if operation.error.is_none() && !has_result_presentation && !summary.trim().is_empty() {
                details.push(CanonicalActivityDetail::identified_section(
                    TranscriptActivitySection::Result,
                    "Output",
                    summary,
                    CanonicalActivityDetailFormat::Auto,
                    true,
                ));
            }
            details
        }
        agena_domain::ActivityPayload::SkillReference(skill) => {
            let mut details = Vec::new();
            if !skill.instructions.trim().is_empty() {
                details.push(CanonicalActivityDetail::section(
                    "Instructions",
                    skill.instructions.clone(),
                    CanonicalActivityDetailFormat::Markdown,
                ));
            }
            details.push(CanonicalActivityDetail::section(
                "Source",
                format!("{} · {}", skill.source, skill.content_hash),
                CanonicalActivityDetailFormat::Plain,
            ));
            details
        }
        agena_domain::ActivityPayload::SkillExecution(skill) => {
            vec![CanonicalActivityDetail::section(
                "Execution",
                format!(
                    "{}{}",
                    skill.execution_id,
                    skill
                        .parent_activity_id
                        .map(|id| format!(" · parent {id}"))
                        .unwrap_or_default()
                ),
                CanonicalActivityDetailFormat::Plain,
            )]
        }
        agena_domain::ActivityPayload::Progress(progress) => {
            match (progress.current, progress.total) {
                (Some(current), Some(total)) => vec![CanonicalActivityDetail::body(
                    format!("{current}/{total}"),
                    CanonicalActivityDetailFormat::Plain,
                )],
                (Some(current), None) => vec![CanonicalActivityDetail::body(
                    current.to_string(),
                    CanonicalActivityDetailFormat::Plain,
                )],
                (None, Some(total)) => vec![CanonicalActivityDetail::body(
                    format!("total {total}"),
                    CanonicalActivityDetailFormat::Plain,
                )],
                (None, None) => Vec::new(),
            }
        }
        agena_domain::ActivityPayload::Checklist(checklist) => {
            let body = checklist
                .items
                .iter()
                .map(|item| {
                    format!(
                        "- [{checked}] **{:?}** · {}",
                        item.priority,
                        item.content,
                        checked = if format!("{:?}", item.status).eq_ignore_ascii_case("completed")
                        {
                            "x"
                        } else {
                            " "
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!body.is_empty())
                .then(|| {
                    CanonicalActivityDetail::section(
                        "Checklist",
                        body,
                        CanonicalActivityDetailFormat::Markdown,
                    )
                })
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::Search(search) => {
            serde_json::to_string_pretty(&search.results)
                .ok()
                .map(|results| {
                    CanonicalActivityDetail::section(
                        "Results",
                        results,
                        CanonicalActivityDetailFormat::Json,
                    )
                })
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::FileChanges(changes) => {
            serde_json::to_string_pretty(&changes.changes)
                .ok()
                .map(|changes| {
                    CanonicalActivityDetail::section(
                        "Changes",
                        changes,
                        CanonicalActivityDetailFormat::Json,
                    )
                })
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::NestedTask(task) => {
            vec![CanonicalActivityDetail::section(
                "Task",
                format!(
                    "{}{}",
                    task.task_id,
                    task.session_id
                        .map(|id| format!(" · session {id}"))
                        .unwrap_or_default()
                ),
                CanonicalActivityDetailFormat::Plain,
            )]
        }
        agena_domain::ActivityPayload::Maintenance(maintenance) => {
            serde_json::to_string_pretty(maintenance)
                .ok()
                .map(|maintenance| {
                    CanonicalActivityDetail::section(
                        "Details",
                        maintenance,
                        CanonicalActivityDetailFormat::Json,
                    )
                })
                .into_iter()
                .collect()
        }
        // The problem is rendered once by the shared red Error section.
        agena_domain::ActivityPayload::Error(_) => Vec::new(),
        agena_domain::ActivityPayload::Custom(custom) => {
            let mut details = vec![CanonicalActivityDetail::section(
                "Schema",
                format!("{} · version {}", custom.schema, custom.schema_version),
                CanonicalActivityDetailFormat::Plain,
            )];
            if let Ok(data) = serde_json::to_string_pretty(&custom.data) {
                details.push(CanonicalActivityDetail::section(
                    "Data",
                    data,
                    CanonicalActivityDetailFormat::Json,
                ));
            }
            details
        }
        agena_domain::ActivityPayload::Resource(resource) => {
            let mut details = Vec::new();
            if let Some(media_type) = resource.media_type.as_ref() {
                details.push(format!("Type: {media_type}"));
            }
            if let Some(size) = resource.size_bytes {
                details.push(format!("Size: {size} bytes"));
            }
            if let (Some(width), Some(height)) = (resource.width, resource.height) {
                details.push(format!("Dimensions: {width}×{height}"));
            }
            (!details.is_empty())
                .then(|| {
                    CanonicalActivityDetail::section(
                        "Details",
                        details.join("\n"),
                        CanonicalActivityDetailFormat::Plain,
                    )
                })
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::TextArtifact(artifact) => artifact
            .language
            .as_ref()
            .map(|language| {
                CanonicalActivityDetail::section(
                    "Language",
                    language,
                    CanonicalActivityDetailFormat::Plain,
                )
            })
            .into_iter()
            .collect(),
        agena_domain::ActivityPayload::Interaction(interaction) => {
            serde_json::to_string_pretty(interaction)
                .ok()
                .map(|interaction| {
                    CanonicalActivityDetail::section(
                        "Request",
                        interaction,
                        CanonicalActivityDetailFormat::Json,
                    )
                })
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::Reasoning(_) => Vec::new(),
    }
}

fn canonical_text_equivalent(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        sanitize_terminal_text(value)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let left = normalize(left);
    let right = normalize(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
}

fn canonical_activity_copy_text(
    title: String,
    summary: String,
    details: &[CanonicalActivityDetail],
    error_text: Option<String>,
    include_summary: bool,
) -> String {
    let mut sections = vec![title];
    let candidates = [
        (include_summary && error_text.is_none()).then_some(summary),
        (!details.is_empty()).then_some(
            details
                .iter()
                .map(CanonicalActivityDetail::copy_text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        error_text,
    ];
    for section in candidates.into_iter().flatten() {
        if section.trim().is_empty() {
            continue;
        }
        if !sections
            .iter()
            .any(|existing| canonical_text_equivalent(existing.as_str(), section.as_str()))
        {
            sections.push(section);
        }
    }
    sections.join("\n")
}

fn patch_rendered_lines_style(lines: &mut [RenderedLine], style: Style) {
    for line in lines {
        line.style = line.style.patch(style);
        if let Some(rich_line) = line.rich_line.take() {
            line.rich_line = Some(rich_line.patch_style(style));
        }
    }
}

/// Render the expandable detail body for a structured public failure
/// projection. Every field is derived from the already-public `UserProblem`;
/// the renderer never accepts a raw diagnostic string on this path.
fn render_failure_detail(
    out: &mut Vec<RenderedLine>,
    problem: &agena_failure::UserProblem,
    i18n: &I18n,
    width: u16,
) {
    let danger = Style::default().fg(agena_tui_components::theme::danger_color());
    let mut push = |label: &str, value: &str, style: Style| {
        push_label_value(out, "    ", &format!("{label}: {value}"), style, width);
    };
    push(
        &ui_text::t(i18n, "failure-detail-message"),
        problem.user.fallback.as_str(),
        danger,
    );
    push(
        &ui_text::t(i18n, "failure-detail-code"),
        problem.code.as_str(),
        Style::default(),
    );
    push(
        &ui_text::t(i18n, "failure-detail-category"),
        &failure_category_label(problem.category, i18n),
        Style::default(),
    );
    push(
        &ui_text::t(i18n, "failure-detail-responsibility"),
        &failure_responsibility_label(problem.responsibility, i18n),
        Style::default(),
    );
    push(
        &ui_text::t(i18n, "failure-detail-impact"),
        &failure_impact_label(problem.impact, i18n),
        Style::default(),
    );
    push(
        &ui_text::t(i18n, "failure-detail-recovery"),
        &recovery_directive_label(problem.recovery, i18n),
        Style::default(),
    );
    push(
        &ui_text::t(i18n, "failure-detail-retry"),
        &retry_directive_label(problem.retry, i18n),
        Style::default(),
    );
}

fn failure_category_label(category: agena_failure::FailureCategory, i18n: &I18n) -> String {
    ui_text::t(
        i18n,
        match category {
            agena_failure::FailureCategory::InvalidInput => "failure-category-invalid-input",
            agena_failure::FailureCategory::NotFound => "failure-category-not-found",
            agena_failure::FailureCategory::Conflict => "failure-category-conflict",
            agena_failure::FailureCategory::PermissionRequired => {
                "failure-category-permission-required"
            }
            agena_failure::FailureCategory::PermissionDenied => {
                "failure-category-permission-denied"
            }
            agena_failure::FailureCategory::AuthenticationRequired => {
                "failure-category-authentication-required"
            }
            agena_failure::FailureCategory::RateLimited => "failure-category-rate-limited",
            agena_failure::FailureCategory::QuotaExceeded => "failure-category-quota-exceeded",
            agena_failure::FailureCategory::Timeout => "failure-category-timeout",
            agena_failure::FailureCategory::DependencyUnavailable => {
                "failure-category-dependency-unavailable"
            }
            agena_failure::FailureCategory::ProtocolFailure => "failure-category-protocol-failure",
            agena_failure::FailureCategory::DataCorruption => "failure-category-data-corruption",
            agena_failure::FailureCategory::Internal => "failure-category-internal",
        },
    )
}

fn failure_responsibility_label(
    responsibility: agena_failure::FailureResponsibility,
    i18n: &I18n,
) -> String {
    ui_text::t(
        i18n,
        match responsibility {
            agena_failure::FailureResponsibility::Caller => "failure-responsibility-caller",
            agena_failure::FailureResponsibility::Policy => "failure-responsibility-policy",
            agena_failure::FailureResponsibility::Dependency => "failure-responsibility-dependency",
            agena_failure::FailureResponsibility::System => "failure-responsibility-system",
        },
    )
}

fn failure_impact_label(impact: agena_failure::FailureImpact, i18n: &I18n) -> String {
    ui_text::t(
        i18n,
        match impact {
            agena_failure::FailureImpact::RequestRejected => "failure-impact-request-rejected",
            agena_failure::FailureImpact::OperationFailed => "failure-impact-operation-failed",
            agena_failure::FailureImpact::OperationPaused => "failure-impact-operation-paused",
            agena_failure::FailureImpact::PartialSuccess => "failure-impact-partial-success",
            agena_failure::FailureImpact::BackgroundTaskFailed => {
                "failure-impact-background-task-failed"
            }
            agena_failure::FailureImpact::RuntimeDegraded => "failure-impact-runtime-degraded",
            agena_failure::FailureImpact::FatalStartupFailure => {
                "failure-impact-fatal-startup-failure"
            }
        },
    )
}

fn recovery_directive_label(recovery: agena_failure::RecoveryDirective, i18n: &I18n) -> String {
    ui_text::t(
        i18n,
        match recovery {
            agena_failure::RecoveryDirective::None => "failure-recovery-none",
            agena_failure::RecoveryDirective::Refresh => "failure-recovery-refresh",
            agena_failure::RecoveryDirective::Reauthenticate => "failure-recovery-reauthenticate",
            agena_failure::RecoveryDirective::OpenSettings => "failure-recovery-open-settings",
            agena_failure::RecoveryDirective::RequestPermission => {
                "failure-recovery-request-permission"
            }
            agena_failure::RecoveryDirective::AskUser => "failure-recovery-ask-user",
            agena_failure::RecoveryDirective::Retry => "failure-recovery-retry",
            agena_failure::RecoveryDirective::ChooseAlternative => {
                "failure-recovery-choose-alternative"
            }
            agena_failure::RecoveryDirective::RestartPlugin => "failure-recovery-restart-plugin",
            agena_failure::RecoveryDirective::RestartRuntime => "failure-recovery-restart-runtime",
        },
    )
}

fn retry_directive_label(retry: agena_failure::RetryDirective, i18n: &I18n) -> String {
    ui_text::t(
        i18n,
        match retry {
            agena_failure::RetryDirective::Never => "failure-retry-never",
            agena_failure::RetryDirective::CorrectInput => "failure-retry-correct-input",
            agena_failure::RetryDirective::AfterUserAction => "failure-retry-after-user-action",
            agena_failure::RetryDirective::AfterRefresh => "failure-retry-after-refresh",
            agena_failure::RetryDirective::ImmediateOnce => "failure-retry-immediate-once",
            agena_failure::RetryDirective::Backoff => "failure-retry-backoff",
            agena_failure::RetryDirective::UseAlternative => "failure-retry-use-alternative",
            agena_failure::RetryDirective::Unknown => "failure-retry-unknown",
        },
    )
}

fn render_canonical_activity_detail(
    out: &mut Vec<RenderedLine>,
    detail: &CanonicalActivityDetail,
    width: u16,
    accent: Option<Style>,
    expanded: bool,
) {
    if detail.body.trim().is_empty() {
        return;
    }
    if let Some(title) = detail
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        let disclosure = if detail.default_expanded.is_some() {
            if expanded { "▾" } else { "▸" }
        } else {
            "›"
        };
        let preview = (!expanded)
            .then(|| canonical_activity_detail_preview(detail))
            .flatten()
            .map(|preview| format!(" · {preview}"))
            .unwrap_or_default();
        push_section_heading(
            out,
            format!("    {disclosure} {title}{preview}").as_str(),
            accent.map_or_else(
                || {
                    Style::default()
                        .fg(agena_tui_components::theme::special_color())
                        .add_modifier(Modifier::BOLD)
                },
                |style| style.add_modifier(Modifier::BOLD),
            ),
            width,
        );
    }
    if !expanded {
        return;
    }
    let body_prefix = if detail.title.is_some() {
        "      "
    } else {
        "    "
    };
    let body_start = out.len();
    match detail.format {
        CanonicalActivityDetailFormat::Auto => {
            render_expanded_tool_text_block(out, body_prefix, detail.body.as_str(), width);
        }
        CanonicalActivityDetailFormat::Markdown => {
            push_expanded_markdown(out, body_prefix, detail.body.as_str(), width);
        }
        CanonicalActivityDetailFormat::Json => {
            let markdown = format!("```json\n{}\n```", detail.body.trim());
            push_expanded_markdown(out, body_prefix, markdown.as_str(), width);
        }
        CanonicalActivityDetailFormat::Plain => {
            push_expanded_tool_text(
                out,
                body_prefix,
                detail.body.as_str(),
                Style::default(),
                width,
            );
        }
    }
    if let Some(style) = accent {
        patch_rendered_lines_style(&mut out[body_start..], style);
    }
}

fn canonical_activity_detail_preview(detail: &CanonicalActivityDetail) -> Option<String> {
    if detail.stable_section == Some(TranscriptActivitySection::Permissions) {
        let count = detail
            .body
            .split("\n\n")
            .filter(|permission| !permission.trim().is_empty())
            .count();
        return (count > 0)
            .then(|| format!("{count} permission{}", if count == 1 { "" } else { "s" }));
    }
    match detail.format {
        CanonicalActivityDetailFormat::Json => {
            serde_json::from_str::<serde_json::Value>(detail.body.as_str())
                .ok()
                .and_then(|value| match value {
                    serde_json::Value::Object(fields) => Some(format!(
                        "{} field{}",
                        fields.len(),
                        if fields.len() == 1 { "" } else { "s" }
                    )),
                    serde_json::Value::Array(items) => Some(format!(
                        "{} item{}",
                        items.len(),
                        if items.len() == 1 { "" } else { "s" }
                    )),
                    _ => None,
                })
        }
        CanonicalActivityDetailFormat::Auto | CanonicalActivityDetailFormat::Markdown => detail
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(canonical_activity_markdown_preview),
        CanonicalActivityDetailFormat::Plain => detail
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| concise_text(line, 48)),
    }
}

fn canonical_activity_markdown_preview(line: &str) -> String {
    let line = line.trim();
    let line = if line.starts_with('#') {
        line.trim_start_matches('#').trim_start()
    } else {
        ["- [x] ", "- [X] ", "- [ ] ", "- ", "* ", "+ ", "> "]
            .into_iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .unwrap_or(line)
    };
    let plain = markdown_inline_line(line, Style::default())
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .unwrap_or_else(|| line.to_owned());
    concise_text(plain.as_str(), 48)
}

fn rendered_activity_section_node(
    key: TranscriptNodeKey,
    start_line: usize,
    end_line: usize,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
    lines: &[RenderedLine],
) -> Option<RenderedTranscriptNode> {
    (end_line > start_line).then(|| RenderedTranscriptNode {
        key,
        kind: TranscriptNodeKind::Activity,
        start_line,
        end_line,
        copy_text,
        atomic: lines[start_line..end_line]
            .iter()
            .any(|line| !line.math.is_empty()),
        toggleable,
        expanded,
    })
}

fn canonical_resource_attachment(resource: &agena_domain::ResourceActivity) -> MessageAttachment {
    let kind = match resource.kind {
        agena_domain::ResourceKind::Image => MessageAttachmentKind::Image,
        agena_domain::ResourceKind::Audio => MessageAttachmentKind::Audio,
        agena_domain::ResourceKind::Video => MessageAttachmentKind::Video,
        agena_domain::ResourceKind::Pdf => MessageAttachmentKind::Pdf,
        agena_domain::ResourceKind::File
        | agena_domain::ResourceKind::Directory
        | agena_domain::ResourceKind::Url
        | agena_domain::ResourceKind::Artifact => MessageAttachmentKind::File,
    };
    let (source, sha256) = match &resource.reference {
        agena_domain::ResourceReference::Artifact { sha256, uri } => (
            MessageAttachmentSource::FileId {
                file_id: uri.clone(),
            },
            Some(sha256.clone()),
        ),
        agena_domain::ResourceReference::WorkspacePath { path } => (
            MessageAttachmentSource::LocalPath { path: path.clone() },
            None,
        ),
        agena_domain::ResourceReference::Url { url } => {
            (MessageAttachmentSource::Url { url: url.clone() }, None)
        }
        agena_domain::ResourceReference::ProviderFile { file_id, .. } => (
            MessageAttachmentSource::FileId {
                file_id: file_id.clone(),
            },
            None,
        ),
    };
    MessageAttachment {
        kind,
        mime: resource.media_type.clone().unwrap_or_default(),
        source,
        filename: Some(resource.name.clone()),
        title: None,
        size_bytes: resource.size_bytes,
        sha256,
        width: resource.width,
        height: resource.height,
        duration_ms: resource.duration_ms,
        page_count: resource.page_count,
    }
}

pub(crate) fn activity_copy_text(part: &TranscriptEntryPart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            let (_, title, summary, error) = activity_presentation(payload);
            let error_text = error.as_ref().map(|e| e.user.fallback.clone());
            let error_equivalence_text = error.as_ref().map(|error| error.user.fallback.as_str());
            let details =
                canonical_activity_details(payload, summary.as_str(), error_equivalence_text);
            Some(canonical_activity_copy_text(
                title,
                summary,
                details.as_slice(),
                error_text,
                !matches!(
                    payload,
                    agena_domain::ActivityPayload::Operation(_)
                ),
            ))
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(reasoning)) => {
            Some(reasoning.preferred_text())
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) => {
            Some(tool_output_copy_text(part, tool, i18n))
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => Some(
            attachment
                .attachments
                .iter()
                .map(|item| {
                    item.title
                        .as_ref()
                        .or(item.filename.as_ref())
                        .cloned()
                        .unwrap_or_else(|| item.mime.clone())
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(reference)) => {
            Some(
                reference
                    .skills
                    .iter()
                    .map(|skill| format!("Skill: {}\n{}", skill.name, skill.instructions))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            Some(error.problem.user.fallback.clone())
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => Some(match status {
            TranscriptAssistantReplyLifecycle::Running => {
                ui_text::t(i18n, "message-activity-response-running")
            }
            TranscriptAssistantReplyLifecycle::Completed => {
                ui_text::t(i18n, "message-activity-response-completed")
            }
            TranscriptAssistantReplyLifecycle::Failed { problem } => match problem {
                Some(problem) => format!(
                    "{}: {}",
                    ui_text::t(i18n, "message-activity-response-failed"),
                    problem.user.fallback
                ),
                None => ui_text::t(i18n, "message-activity-response-failed"),
            },
            TranscriptAssistantReplyLifecycle::Cancelled => {
                ui_text::t(i18n, "message-activity-response-cancelled")
            }
        }),
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            Some(match request.as_ref() {
                MessageRequestPartResource::UserInput { request, .. } => request
                    .questions
                    .iter()
                    .map(|question| question.question.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub kind: TranscriptNodeKind,
    pub source: String,
    pub copy_text: String,
    pub leading_blank_line: bool,
    pub parsed: MarkdownNode,
}

fn entry_preview(message: &TranscriptEntry, i18n: &I18n) -> String {
    let preview = transcript_message_parts(message)
        .iter()
        .find_map(|part| preview_for_part(part, i18n))
        .unwrap_or_else(|| ui_text::t(i18n, "message-empty"));
    truncate_display_width(preview.as_str(), 64)
}

pub(crate) fn render_transcript_entries_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    messages: &[TranscriptEntry],
) -> String {
    if session_id.is_none() && messages.is_empty() {
        return String::new();
    }

    let title = if !session_title.trim().is_empty() {
        session_title.trim().to_string()
    } else if let Some(session_id) = session_id {
        ui_text::session_fallback_title(i18n, session_id)
    } else {
        ui_text::transcript_export_default_title(i18n)
    };

    let title = truncate_display_width(
        title.as_str(),
        usize::from(TRANSCRIPT_EXPORT_WIDTH).saturating_sub(2),
    );
    let mut out = vec![format!("# {title}"), String::new()];
    if let Some(session_id) = session_id {
        out.push(ui_text::transcript_export_session_id_line(i18n, session_id));
    }
    out.push(ui_text::transcript_export_exported_at_line(
        i18n,
        Local::now(),
    ));
    out.push(ui_text::transcript_export_messages_loaded_line(
        i18n,
        messages.len(),
    ));
    if let Some(execution) = execution {
        if let Some(parent_id) = execution.session.parent_id {
            out.push(ui_text::transcript_export_parent_session_line(
                i18n, parent_id,
            ));
        }
        out.push(ui_text::transcript_export_child_sessions_line(
            i18n,
            execution.session.child_session_count,
        ));
    }
    out.push(String::new());

    if messages.is_empty() {
        out.push(ui_text::transcript_export_empty_line(i18n));
        return out.join("\n");
    }

    for message in messages {
        let timestamp = format_timestamp(message.created_at);
        if let Some(role) = message.role {
            out.push(format!(
                "## {} · {} · {}",
                ui_text::role_label(i18n, role),
                ui_text::message_state_label(i18n, message.state),
                timestamp,
            ));
        } else {
            out.push(format!(
                "## {} · {}",
                ui_text::t(i18n, "transcript-node-kind-activity"),
                timestamp,
            ));
        }
        out.push(String::new());
        out.push("~~~~text".to_string());
        out.extend(
            render_entry_export(
                message,
                i18n,
                TranscriptDetailDefaults {
                    activity_expanded: true,
                },
            )
            .into_iter()
            .map(|line| line.text),
        );
        out.push("~~~~".to_string());
        out.push(String::new());
    }

    out.join("\n")
}

pub fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    entry_preview(&TranscriptEntry::from(message), i18n)
}

pub fn render_transcript_snapshot_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    snapshot: &agena_domain::TranscriptSnapshot,
) -> String {
    let entries = crate::transcript_entries(snapshot);
    render_transcript_entries_export_markdown(
        i18n,
        session_id,
        session_title,
        execution,
        entries.as_slice(),
    )
}

pub(crate) fn tool_output_preview(text: &str) -> ToolOutputPreview {
    tool_output_preview_with_limits(text, TOOL_CARD_PREVIEW_LINES, TOOL_CARD_PREVIEW_CHARS)
}

pub(crate) fn tool_output_preview_with_limits(
    text: &str,
    max_lines: usize,
    max_chars: usize,
) -> ToolOutputPreview {
    let normalized = trim_empty_line_edges(sanitize_terminal_text(text).as_str());
    if normalized.is_empty() {
        return ToolOutputPreview {
            text: String::new(),
            omitted_lines: 0,
        };
    }

    let total_lines = normalized.split('\n').count();
    let mut preview = String::new();
    let mut used_chars = 0_usize;
    let mut included_lines = 0_usize;
    let mut truncated = false;

    for (index, line) in normalized.split('\n').enumerate() {
        if index >= max_lines {
            truncated = true;
            break;
        }

        let separator_chars = usize::from(index > 0);
        let line_chars = line.chars().count();
        if used_chars
            .saturating_add(separator_chars)
            .saturating_add(line_chars)
            > max_chars
        {
            if index > 0 {
                preview.push('\n');
            }
            let remaining = max_chars
                .saturating_sub(used_chars)
                .saturating_sub(separator_chars);
            preview.extend(line.chars().take(remaining));
            included_lines = index + 1;
            truncated = true;
            break;
        }

        if index > 0 {
            preview.push('\n');
            used_chars += 1;
        }
        preview.push_str(line);
        used_chars += line_chars;
        included_lines = index + 1;
    }

    let mut omitted_lines = if truncated {
        total_lines.saturating_sub(included_lines)
    } else {
        0
    };
    if truncated && omitted_lines == 0 {
        omitted_lines = 1;
    }

    ToolOutputPreview {
        text: preview,
        omitted_lines,
    }
}

pub(crate) fn sanitize_terminal_text(text: &str) -> String {
    let stripped = strip_terminal_ansi_sequences(text).replace('\r', "");
    stripped
        .chars()
        .filter_map(|ch| match ch {
            '\n' | '\t' => Some(ch),
            '\u{200e}' | '\u{200f}' => None,
            '\u{202a}'..='\u{202e}' => None,
            '\u{2066}'..='\u{2069}' => None,
            ch if ch.is_control() => Some(' '),
            _ => Some(ch),
        })
        .collect()
}

pub(crate) fn push_message_header(
    out: &mut Vec<RenderedLine>,
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
) {
    let role = message
        .role
        .map(|role| ui_text::role_label(i18n, role))
        .expect("only message entries render a role header");
    let header = match message.state {
        MessageStatus::Completed => role,
        MessageStatus::Pending => format!("{role} ○"),
        MessageStatus::InProgress => format!("{role} {}", transcript_spinner_placeholder()),
        MessageStatus::PolicyDenied => format!("{role} ⊘"),
        MessageStatus::UserDeclined => format!("{role} –"),
        MessageStatus::CapabilityUnavailable | MessageStatus::ToolUnavailable => {
            format!("{role} ◇")
        }
        MessageStatus::Failed => format!("{role} ×"),
        MessageStatus::Cancelled => format!("{role} –"),
    };
    let header_style =
        style_for_role(message.role.expect("message role")).add_modifier(Modifier::BOLD);

    if UnicodeWidthStr::width(header.as_str()) <= width.max(1) as usize {
        out.push(RenderedLine::plain(header, header_style));
    } else {
        push_wrapped_line(out, "", "", header.as_str(), header_style, width);
    }
}

pub(crate) fn render_part_node(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    width: u16,
    out: &mut Vec<RenderedLine>,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedNodeDraft {
    match transcript_part_content(part) {
        TranscriptPartContent::UserDocument(document) => {
            let copy_text = render_user_document(document, out, width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::Content {
                    entry_id: message.id,
                    content_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text,
                toggleable: false,
                expanded: true,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Text(text) => {
            push_markdown_document(out, "  ", text.text.as_str(), width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::Content {
                    entry_id: message.id,
                    content_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: text.text.clone(),
                toggleable: false,
                expanded: true,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let (_, title, summary, error) = activity_presentation(payload);
            let error_text = error.as_ref().map(|e| e.user.fallback.clone());
            let error_equivalence_text = error.as_ref().map(|error| error.user.fallback.as_str());
            let details =
                canonical_activity_details(payload, summary.as_str(), error_equivalence_text);
            let toggleable = !summary.trim().is_empty() || error.is_some() || !details.is_empty();
            push_activity_headline(
                out,
                part.status,
                expanded,
                toggleable,
                title.as_str(),
                summary.as_str(),
                width,
            );
            let headline_end = out.len();
            let mut children = Vec::new();
            let mut detail_index = 0_usize;
            let is_operation = matches!(
                payload,
                agena_domain::ActivityPayload::Operation(_)
            );
            let render_summary =
                expanded && !is_operation && error.is_none() && !summary.trim().is_empty();
            if render_summary {
                let summary_start = out.len();
                render_expanded_tool_text_block(out, "    ", summary.as_str(), width);
                patch_rendered_lines_style(
                    &mut out[summary_start..],
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                );
                if let Some(child) = rendered_activity_section_node(
                    TranscriptNodeKey::ActivitySection {
                        entry_id: message.id,
                        content_id: part.id,
                        section: TranscriptActivitySection::Detail(detail_index),
                    },
                    summary_start,
                    out.len(),
                    summary.clone(),
                    false,
                    true,
                    out,
                ) {
                    children.push(child);
                    detail_index = detail_index.saturating_add(1);
                }
            }
            if expanded {
                for detail in &details {
                    let section = detail.navigation_section(&mut detail_index);
                    let section_key = TranscriptNodeKey::ActivitySection {
                        entry_id: message.id,
                        content_id: part.id,
                        section,
                    };
                    let section_expanded = detail.default_expanded.is_none_or(|default_expanded| {
                        expansions
                            .get(&section_key)
                            .copied()
                            .unwrap_or(default_expanded)
                    });
                    let section_start = out.len();
                    render_canonical_activity_detail(out, detail, width, None, section_expanded);
                    if let Some(child) = rendered_activity_section_node(
                        section_key,
                        section_start,
                        out.len(),
                        detail.copy_text(),
                        detail.default_expanded.is_some(),
                        section_expanded,
                        out,
                    ) {
                        children.push(child);
                    }
                }
                if let agena_domain::ActivityPayload::Resource(resource) = payload {
                    let section_start = out.len();
                    let attachment = canonical_resource_attachment(resource);
                    let _ = render_attachment_image(out, "    ", &attachment, width);
                    if let Some(child) = rendered_activity_section_node(
                        TranscriptNodeKey::ActivitySection {
                            entry_id: message.id,
                            content_id: part.id,
                            section: TranscriptActivitySection::Detail(detail_index),
                        },
                        section_start,
                        out.len(),
                        resource.name.clone(),
                        false,
                        true,
                        out,
                    ) {
                        children.push(child);
                    }
                }
            }
            if expanded && let Some(ref error_str) = error_text {
                let section_key = TranscriptNodeKey::ActivitySection {
                    entry_id: message.id,
                    content_id: part.id,
                    section: TranscriptActivitySection::Error,
                };
                let section_expanded = expansions.get(&section_key).copied().unwrap_or(true);
                let section_start = out.len();
                render_canonical_activity_detail(
                    out,
                    &CanonicalActivityDetail::section(
                        "Error",
                        error_str,
                        CanonicalActivityDetailFormat::Auto,
                    ),
                    width,
                    Some(Style::default().fg(agena_tui_components::theme::danger_color())),
                    section_expanded,
                );
                if let Some(child) = rendered_activity_section_node(
                    section_key,
                    section_start,
                    out.len(),
                    error_str.clone(),
                    true,
                    section_expanded,
                    out,
                ) {
                    children.push(child);
                }
            }
            let copy_text = canonical_activity_copy_text(
                title,
                summary,
                details.as_slice(),
                error_text,
                !is_operation,
            );
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text,
                toggleable,
                expanded,
                end_line: Some(headline_end),
                children,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(reasoning)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let summary = reasoning.preferred_text();
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                "thinking",
                concise_text(summary.as_str(), 112).as_str(),
                width,
            );
            if expanded {
                let body_start = out.len();
                render_expanded_tool_text_block(out, "    ", summary.as_str(), width);
                patch_rendered_lines_style(
                    &mut out[body_start..],
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                );
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: summary,
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            render_tool_execution(part, tool, out, width, i18n, expanded);
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: tool_output_copy_text(part, tool, i18n),
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or(true);
            let title = error.problem.user.fallback.clone();
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                title.as_str(),
                "",
                width,
            );
            if expanded {
                render_failure_detail(out, &error.problem, i18n, width);
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: title,
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let title = ui_text::t(
                i18n,
                match status {
                    TranscriptAssistantReplyLifecycle::Running => {
                        "message-activity-response-running"
                    }
                    TranscriptAssistantReplyLifecycle::Completed => {
                        "message-activity-response-completed"
                    }
                    TranscriptAssistantReplyLifecycle::Failed { .. } => {
                        "message-activity-response-failed"
                    }
                    TranscriptAssistantReplyLifecycle::Cancelled => {
                        "message-activity-response-cancelled"
                    }
                },
            );
            let problem = match status {
                TranscriptAssistantReplyLifecycle::Failed { problem } => problem,
                _ => &None,
            };
            let summary = problem
                .as_ref()
                .map(|problem| problem.user.fallback.as_str())
                .unwrap_or("");
            // A failed reply is expandable whenever the runtime persisted a
            // structured failure; the collapsed row shows the readable
            // summary and the expanded body carries the full field set.
            let toggleable = problem.is_some();
            let expanded = if toggleable {
                expansions
                    .get(&key)
                    .copied()
                    .unwrap_or(defaults.activity_expanded)
            } else {
                true
            };
            push_activity_headline(
                out,
                part.status,
                expanded,
                toggleable,
                title.as_str(),
                summary,
                width,
            );
            if toggleable && expanded
                && let Some(problem) = problem.as_ref()
            {
                render_failure_detail(out, problem, i18n, width);
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: if summary.is_empty() {
                    title
                } else {
                    format!("{title}: {summary}")
                },
                toggleable,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let mut labels = Vec::new();
            for item in &attachment.attachments {
                let label = item
                    .title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone());
                labels.push(label);
            }
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                ui_text::t(i18n, "message-input-activity-attachment").as_str(),
                labels.join(", ").as_str(),
                width,
            );
            if expanded {
                for item in &attachment.attachments {
                    let label = item
                        .title
                        .as_ref()
                        .or(item.filename.as_ref())
                        .cloned()
                        .unwrap_or_else(|| item.mime.clone());
                    if !render_attachment_image(out, "    ", item, width) {
                        push_label_value(out, "    - ", label.as_str(), Style::default(), width);
                    }
                }
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: labels.join("\n"),
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(reference)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let mut labels = Vec::new();
            for skill in &reference.skills {
                labels.push(skill.name.clone());
            }
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                ui_text::t(i18n, "message-input-activity-skill").as_str(),
                labels.join(", ").as_str(),
                width,
            );
            if expanded {
                for skill in &reference.skills {
                    render_canonical_activity_detail(
                        out,
                        &CanonicalActivityDetail::section(
                            skill.name.as_str(),
                            skill.description.as_str(),
                            CanonicalActivityDetailFormat::Auto,
                        ),
                        width,
                        None,
                        true,
                    );
                    render_canonical_activity_detail(
                        out,
                        &CanonicalActivityDetail::section(
                            "Instructions",
                            skill.instructions.as_str(),
                            CanonicalActivityDetailFormat::Markdown,
                        ),
                        width,
                        None,
                        true,
                    );
                    render_canonical_activity_detail(
                        out,
                        &CanonicalActivityDetail::section(
                            "Source",
                            format!("{} · {}", skill.source, skill.content_hash),
                            CanonicalActivityDetailFormat::Plain,
                        ),
                        width,
                        None,
                        true,
                    );
                }
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: labels.join("\n"),
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                MessageRequestPartResource::UserInput { request, .. } => {
                    render_user_input_request(request, out, width, i18n);
                    RenderedNodeDraft {
                        key: TranscriptNodeKey::Activity {
                            entry_id: message.id,
                            content_id: part.id,
                        },
                        kind: TranscriptNodeKind::Activity,
                        copy_text: request
                            .questions
                            .iter()
                            .map(|question| question.question.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        toggleable: false,
                        expanded: true,
                        end_line: None,
                        children: Vec::new(),
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct UserDocumentToken {
    text: String,
    style: Style,
    width: usize,
    newline: bool,
}

fn render_user_document(
    document: &crate::TranscriptUserDocument,
    out: &mut Vec<RenderedLine>,
    width: u16,
) -> String {
    let mut tokens = Vec::new();
    for node in &document.nodes {
        match node {
            crate::TranscriptUserDocumentNode::Text { text, .. } => {
                let sanitized = sanitize_terminal_text(text);
                tokens.extend(sanitized.graphemes(true).map(|grapheme| UserDocumentToken {
                    text: grapheme.to_owned(),
                    style: Style::default(),
                    width: UnicodeWidthStr::width(grapheme),
                    newline: grapheme == "\n",
                }));
            }
            crate::TranscriptUserDocumentNode::Activity {
                placeholder, style, ..
            } => {
                let placeholder = sanitize_terminal_text(placeholder);
                tokens.push(UserDocumentToken {
                    width: UnicodeWidthStr::width(placeholder.as_str()),
                    text: placeholder,
                    style: match style {
                        crate::TranscriptUserActivityStyle::Resource => Style::default()
                            .fg(agena_tui_components::theme::info_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::Skill => Style::default()
                            .fg(agena_tui_components::theme::accent_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::TextArtifact => Style::default()
                            .fg(agena_tui_components::theme::warning_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::Other => Style::default()
                            .fg(agena_tui_components::theme::accent_color())
                            .add_modifier(Modifier::BOLD),
                    },
                    newline: false,
                });
            }
        }
    }

    let copy_text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>();
    let line_width = usize::from(width).saturating_sub(2).max(1);
    let mut line_tokens = Vec::new();
    let mut used_width = 0_usize;
    let mut last_was_newline = false;
    let start_len = out.len();
    for token in tokens {
        if token.newline {
            push_user_document_line(out, std::mem::take(&mut line_tokens));
            used_width = 0;
            last_was_newline = true;
            continue;
        }
        if !line_tokens.is_empty() && used_width.saturating_add(token.width) > line_width {
            push_user_document_line(out, std::mem::take(&mut line_tokens));
            used_width = 0;
        }
        used_width = used_width.saturating_add(token.width);
        line_tokens.push(token);
        last_was_newline = false;
    }
    if !line_tokens.is_empty() || last_was_newline || out.len() == start_len {
        push_user_document_line(out, line_tokens);
    }
    copy_text
}

fn push_user_document_line(out: &mut Vec<RenderedLine>, tokens: Vec<UserDocumentToken>) {
    let copy_text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>();
    let mut spans = vec![Span::raw("  ")];
    for token in tokens {
        if let Some(last) = spans.last_mut()
            && last.style == token.style
        {
            last.content.to_mut().push_str(token.text.as_str());
        } else {
            spans.push(Span::styled(token.text, token.style));
        }
    }
    out.push(RenderedLine::rich(Line::from(spans)).with_copy_projection(copy_text, 2));
}

#[cfg(test)]
pub(crate) fn thinking_collapsed_summary(
    status: agena_api::message_part::PartExecutionStatusResource,
    text: &str,
) -> String {
    let normalized = trim_empty_line_edges(sanitize_terminal_text(text).as_str());
    let preview = normalized
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let additional_content = normalized
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .skip(1)
        .any(|line| !line.trim().is_empty());
    let suffix = if additional_content { " …" } else { "" };
    format!(
        "{} thinking · {}{suffix}",
        super::super::transcript_tool_summary::activity_status_icon(status),
        concise_text(preview, 112)
    )
}
