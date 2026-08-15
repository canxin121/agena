use super::super::transcript_ast::{MarkdownNode, markdown_inline_line, render_attachment_image};
use super::super::{
    I18n, Local, Modifier, RenderedLine, RenderedTranscriptNode, RunStatus,
    SessionExecutionResource, Style, TOOL_CARD_PREVIEW_CHARS, TOOL_CARD_PREVIEW_LINES,
    ToolOutputPreview, TranscriptDetailDefaults, TranscriptEntry, TranscriptNodeKey,
    TranscriptNodeKind, UnicodeWidthStr, concise_text, format_occurred_time, format_timestamp,
    json_value_to_markdown, push_activity_headline, push_expanded_markdown,
    push_expanded_tool_text, push_label_value, push_markdown_document, push_markdown_rule,
    push_section_heading, push_single_line, push_wrapped_line, render_entry_detailed,
    render_expanded_tool_text_block, strip_terminal_ansi_sequences, style_for_role,
    tool_output_copy_text, transcript_message_parts, transcript_part_content,
    transcript_spinner_placeholder, trim_empty_line_edges, truncate_display_width,
};
use super::operation_render::render_tool_execution_with_sections;
use super::request_render::{preview_for_part, render_user_input_request};
use crate::snapshot::activity_presentation;
use crate::ui_text;
use crate::{
    OperationPartResource, RequestPartResource, TranscriptActivityContent,
    TranscriptActivitySection, TranscriptAssistantReplyLifecycle, TranscriptEntryPart,
    TranscriptPartContent,
};
use agena_api::resource::{PartAttachment, PartAttachmentKind, PartAttachmentSource, RunResource};
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
    defaults: &TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    // UI sections default to folded, but exports are durable documents rather
    // than viewport state. Preserve the previous export projection: legacy
    // Operations include both input and output, while canonical Operations
    // include their result body (canonical input was already folded).
    let mut expansions = std::collections::BTreeMap::new();
    for part in transcript_message_parts(message) {
        let sections = match transcript_part_content(part) {
            TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_)) => &[
                TranscriptActivitySection::Input,
                TranscriptActivitySection::Result,
            ][..],
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
                agena_domain::ActivityPayload::Operation(_),
            )) => &[TranscriptActivitySection::Result][..],
            _ => &[],
        };
        for section in sections {
            expansions.insert(
                TranscriptNodeKey::ActivitySection {
                    entry_id: message.id,
                    content_id: part.id,
                    section: *section,
                },
                true,
            );
        }
    }
    agena_tui_media::with_text_math_rendering(|| {
        render_entry_detailed(
            message,
            TRANSCRIPT_EXPORT_WIDTH,
            i18n,
            defaults,
            &expansions,
        )
        .lines
    })
}

#[derive(Debug, Clone)]
/// A rendered message block.
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
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) {
    // Like Markdown blocks, non-text parts start after the message header so
    // selecting the first activity part never highlights `assistant`.
    let start_line = lines.len();
    let node = render_part_node(
        message,
        part,
        width,
        lines,
        i18n,
        defaults,
        expansions,
        interactions,
    );
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

/// Stable activity-kind id for a transcript part, used to resolve default
/// expansion from transcript settings. Returns `None` for lifecycle-only
/// markers that have no user-visible activity kind.
pub(crate) fn activity_kind_id_for_part(part: &TranscriptEntryPart) -> Option<&'static str> {
    match transcript_part_content(part) {
        TranscriptPartContent::Text(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
        // User documents group multiple activities and text; no single kind.
        TranscriptPartContent::UserDocument(_) => None,
        TranscriptPartContent::Activity(content) => match content {
            TranscriptActivityContent::Reasoning(_) => Some(agena_domain::ACTIVITY_KIND_REASONING),
            TranscriptActivityContent::Operation(_) => Some(agena_domain::ACTIVITY_KIND_OPERATION),
            TranscriptActivityContent::Attachment(_) => Some(agena_domain::ACTIVITY_KIND_RESOURCE),
            TranscriptActivityContent::SkillReference(_) => {
                Some(agena_domain::ACTIVITY_KIND_SKILL_REFERENCE)
            }
            TranscriptActivityContent::Error(_) => Some(agena_domain::ACTIVITY_KIND_ERROR),
            TranscriptActivityContent::Hook(_) => Some(agena_domain::ACTIVITY_KIND_HOOK),
            TranscriptActivityContent::Request(_) => Some(agena_domain::ACTIVITY_KIND_INTERACTION),
            TranscriptActivityContent::TextSegment(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
            // The answer defaults to expanded independently of the global
            // activity default; resolve kind overrides as usual so a user may
            // still collapse assistant answers globally if they choose.
            TranscriptActivityContent::Answer(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
            TranscriptActivityContent::Canonical(payload) => activity_kind_id_for_payload(payload),
            TranscriptActivityContent::AssistantReplyLifecycle(_) => None,
        },
    }
}

fn activity_kind_id_for_payload(payload: &agena_domain::ActivityPayload) -> Option<&'static str> {
    match payload {
        agena_domain::ActivityPayload::Resource(_) => Some(agena_domain::ACTIVITY_KIND_RESOURCE),
        agena_domain::ActivityPayload::SkillReference(_) => {
            Some(agena_domain::ACTIVITY_KIND_SKILL_REFERENCE)
        }
        agena_domain::ActivityPayload::TextArtifact(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
        agena_domain::ActivityPayload::Reasoning(_) => Some(agena_domain::ACTIVITY_KIND_REASONING),
        agena_domain::ActivityPayload::TextSegment(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
        agena_domain::ActivityPayload::Operation(_) => Some(agena_domain::ACTIVITY_KIND_OPERATION),
        agena_domain::ActivityPayload::Interaction(_) => {
            Some(agena_domain::ACTIVITY_KIND_INTERACTION)
        }
        agena_domain::ActivityPayload::Error(_) => Some(agena_domain::ACTIVITY_KIND_ERROR),
        agena_domain::ActivityPayload::Notice(_) => Some(agena_domain::ACTIVITY_KIND_NOTICE),
    }
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
    i18n: &I18n,
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
            if !operation.invocation.input.is_empty() {
                // Tool arguments as nested Markdown bullets (matching the
                // single-activity `render_tool_execution` input section)
                // instead of a raw JSON fence.
                let input = json_value_to_markdown(&serde_json::Value::from(
                    operation.invocation.input.clone(),
                ));
                details.push(CanonicalActivityDetail::identified_section(
                    TranscriptActivitySection::Input,
                    "Input",
                    input,
                    CanonicalActivityDetailFormat::Markdown,
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
                        false,
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
                        false,
                    ));
                    has_result_presentation = true;
                }
            }
            // `summary` is the compact collapsed projection. Expanded
            // Operations render the actual output/sections instead. It is
            // only a fallback result when the producer supplied no detailed
            // result at all; failures are rendered exclusively from `error`.
            // An approval/authorization-phase summary ("Awaiting approval",
            // "Permission allowed once", …) is transcript prose about the
            // permission gate and must not masquerade as tool output.
            if operation.error.is_none()
                && !has_result_presentation
                && !summary.trim().is_empty()
                && !crate::snapshot::is_authorization_phase_summary(summary)
            {
                details.push(CanonicalActivityDetail::identified_section(
                    TranscriptActivitySection::Result,
                    "Output",
                    summary,
                    CanonicalActivityDetailFormat::Auto,
                    false,
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
        agena_domain::ActivityPayload::Notice(notice) => notice
            .detail
            .as_ref()
            .map(|detail| {
                vec![CanonicalActivityDetail::section(
                    "Notice",
                    detail.clone(),
                    CanonicalActivityDetailFormat::Plain,
                )]
            })
            .unwrap_or_default(),
        // The problem is rendered once by the shared red Error section.
        agena_domain::ActivityPayload::Error(error) => {
            let problem = &error.problem;
            let lines = [
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-code"),
                    problem.code
                ),
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-category"),
                    failure_category_label(problem.category, i18n)
                ),
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-responsibility"),
                    failure_responsibility_label(problem.responsibility, i18n)
                ),
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-impact"),
                    failure_impact_label(problem.impact, i18n)
                ),
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-recovery"),
                    recovery_directive_label(problem.recovery, i18n)
                ),
                format!(
                    "{}: {}",
                    ui_text::t(i18n, "failure-detail-retry"),
                    retry_directive_label(problem.retry, i18n)
                ),
            ];
            vec![CanonicalActivityDetail::section(
                ui_text::t(i18n, "failure-detail-section-title").as_str(),
                lines.join(
                    "
",
                ),
                CanonicalActivityDetailFormat::Plain,
            )]
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
        agena_domain::ActivityPayload::Reasoning(_)
        | agena_domain::ActivityPayload::TextSegment(_) => Vec::new(),
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

/// Join the headline title with the currently visible content sections of a
/// canonical Activity. Sections whose text is equivalent to an already
/// included section are dropped (a summary that repeats the title, an error
/// that repeats the output, …) so copy text never duplicates the projection.
fn join_canonical_copy_sections(sections: Vec<String>) -> String {
    let mut out = Vec::<String>::new();
    for section in sections {
        if section.trim().is_empty() {
            continue;
        }
        if out
            .iter()
            .any(|existing| canonical_text_equivalent(existing.as_str(), section.as_str()))
        {
            continue;
        }
        out.push(section);
    }
    out.join("\n")
}

fn patch_rendered_lines_style(lines: &mut [RenderedLine], style: Style) {
    for line in lines {
        line.style = line.style.patch(style);
        if let Some(rich_line) = line.rich_line.take() {
            line.rich_line = Some(rich_line.patch_style(style));
        }
    }
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

fn canonical_resource_attachment(resource: &agena_domain::ResourceActivity) -> PartAttachment {
    let kind = match resource.kind {
        agena_domain::ResourceKind::Image => PartAttachmentKind::Image,
        agena_domain::ResourceKind::Audio => PartAttachmentKind::Audio,
        agena_domain::ResourceKind::Video => PartAttachmentKind::Video,
        agena_domain::ResourceKind::Pdf => PartAttachmentKind::Pdf,
        agena_domain::ResourceKind::File
        | agena_domain::ResourceKind::Directory
        | agena_domain::ResourceKind::Url
        | agena_domain::ResourceKind::Artifact => PartAttachmentKind::File,
    };
    let (source, sha256) = match &resource.reference {
        agena_domain::ResourceReference::Artifact { sha256, uri } => (
            PartAttachmentSource::FileId {
                file_id: uri.clone(),
            },
            Some(sha256.clone()),
        ),
        agena_domain::ResourceReference::WorkspacePath { path } => {
            (PartAttachmentSource::LocalPath { path: path.clone() }, None)
        }
        agena_domain::ResourceReference::Url { url } => {
            (PartAttachmentSource::Url { url: url.clone() }, None)
        }
        agena_domain::ResourceReference::ProviderFile { file_id, .. } => (
            PartAttachmentSource::FileId {
                file_id: file_id.clone(),
            },
            None,
        ),
    };
    PartAttachment {
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// A rendered markdown block.
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
                &TranscriptDetailDefaults {
                    activity_default_expanded: true,
                    kind_defaults: std::collections::BTreeMap::new(),
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

pub fn rewind_message_preview(message: &RunResource, i18n: &I18n) -> String {
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

/// Export markdown from the v2 parts projection (the live wire transcript).
/// Mirrors [`render_transcript_snapshot_export_markdown`] but renders the
/// ordered part list, projecting it through [`crate::parts_entries`].
pub fn render_parts_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    parts: &[agena_api::resource::SessionTranscriptPart],
) -> String {
    let entries = crate::parts_entries(parts);
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
        RunStatus::Completed => role,
        RunStatus::Pending => format!("{role} ○"),
        RunStatus::InProgress => format!("{role} {}", transcript_spinner_placeholder()),
        RunStatus::PolicyDenied => format!("{role} ⊘"),
        RunStatus::UserDeclined => format!("{role} –"),
        RunStatus::CapabilityUnavailable | RunStatus::ToolUnavailable => {
            format!("{role} ◇")
        }
        RunStatus::Failed => format!("{role} ×"),
        RunStatus::Cancelled => format!("{role} –"),
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
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
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
            render_activity_canonical(
                message, part, payload, None, out, width, i18n, defaults, expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(segment)) => {
            render_activity_canonical(
                message,
                part,
                &agena_domain::ActivityPayload::TextSegment(segment.as_ref().clone()),
                None,
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        // The final reply text is a part that defaults to expanded: it reads
        // as a normal assistant answer on arrival (headline + full Markdown
        // body), but the user may collapse or expand it like any other
        // activity part. Interstitial TextSegments render through the
        // canonical path and stay collapsed; the answer is its own toggleable
        // node so it is never folded with the working notes. Like canonical
        // activities, the node owns only its headline range and the body is a
        // child section, so selection and navigation match every other block.
        TranscriptPartContent::Activity(TranscriptActivityContent::Answer(answer)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or(true);
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                "Answer",
                answer.text.as_str(),
                width,
            );
            let headline_end = out.len();
            let mut children = Vec::new();
            let body_start = out.len();
            if expanded {
                push_markdown_document(out, "    ", answer.text.as_str(), width);
            }
            if let Some(child) = rendered_activity_section_node(
                TranscriptNodeKey::ActivitySection {
                    entry_id: message.id,
                    content_id: part.id,
                    section: TranscriptActivitySection::Detail(0),
                },
                body_start,
                out.len(),
                answer.text.clone(),
                false,
                true,
                out,
            ) {
                children.push(child);
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: if expanded {
                    answer.text.clone()
                } else {
                    String::new()
                },
                toggleable: true,
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
            // Reasoning must never be truncated: it is the model's full
            // thought trail, for both the provider and the human reading the
            // transcript. It defaults to expanded (so the full trail is
            // immediately visible) but the user may still collapse it via the
            // normal toggle or a per-kind expansion setting.
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults
                    .kind_defaults
                    .get(agena_domain::ACTIVITY_KIND_REASONING)
                    .copied()
                    .unwrap_or(true)
            });
            let summary = reasoning.preferred_text();
            let headline = summary
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("thinking");
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                "thinking",
                headline,
                width,
            );
            if expanded {
                let body_start = out.len();
                // Render reasoning as plain multi-line text, never through the
                // markdown path, so the full thought trail is shown verbatim
                // with no ellipsis or reformatting.
                push_expanded_tool_text(out, "    ", summary.as_str(), Style::default(), width);
                patch_rendered_lines_style(
                    &mut out[body_start..],
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                );
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: if expanded { summary } else { String::new() },
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
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_OPERATION))
            });
            // Canonical single-activity shape: a tool operation carrying a
            // user-input record IS the interaction part. Render the pending
            // interaction body while awaiting a reply (live view), the
            // read-only plan + questions + answers once answered; fall through
            // to the plain tool execution renderer for operations without
            // user input.
            let user_input_rendered =
                render_operation_user_input(part, tool, out, width, i18n, expanded, interactions);
            if user_input_rendered {
                return RenderedNodeDraft {
                    key,
                    kind: TranscriptNodeKind::Activity,
                    copy_text: if expanded {
                        tool_output_copy_text(part, tool, i18n)
                    } else {
                        String::new()
                    },
                    toggleable: true,
                    expanded,
                    end_line: None,
                    children: Vec::new(),
                };
            }

            let input_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Input,
            };
            let output_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Result,
            };
            let input_expanded = expansions.get(&input_key).copied().unwrap_or(false);
            let output_expanded = expansions.get(&output_key).copied().unwrap_or(false);
            let execution = render_tool_execution_with_sections(
                part,
                input_expanded,
                output_expanded,
                tool,
                out,
                width,
                i18n,
                expanded,
            );
            let mut children = Vec::new();
            if let Some(section) = execution.input
                && let Some(child) = rendered_activity_section_node(
                    input_key,
                    section.start_line,
                    section.end_line,
                    section.copy_text,
                    true,
                    section.expanded,
                    out,
                )
            {
                children.push(child);
            }
            if let Some(section) = execution.output
                && let Some(child) = rendered_activity_section_node(
                    output_key,
                    section.start_line,
                    section.end_line,
                    section.copy_text,
                    true,
                    section.expanded,
                    out,
                )
            {
                children.push(child);
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: execution.visible_copy_text,
                toggleable: true,
                expanded,
                end_line: Some(execution.headline_end),
                children,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            render_activity_canonical(
                message,
                part,
                &agena_domain::ActivityPayload::Error(agena_domain::ErrorActivity {
                    problem: error.problem.clone(),
                }),
                None,
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Hook(hook)) => {
            render_activity_canonical(
                message,
                part,
                &agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
                    kind: "hook".to_owned(),
                    summary: hook.summary.clone(),
                    detail: hook
                        .message
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                        .map(str::to_owned)
                        .or_else(|| hook.detail.clone()),
                    occurred_at_ms: None,
                    title: None,
                }),
                None,
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => {
            let problem = match status {
                TranscriptAssistantReplyLifecycle::Failed { problem } => problem,
                _ => &None,
            };
            let failed_title = ui_text::t(i18n, "message-activity-response-failed");
            if let Some(problem) = problem.as_ref() {
                // A failed reply reuses the canonical Error Activity
                // presentation: same headline, expandable detail sections,
                // copy text and node structure as every other tool-call
                // error, so the transcript does not carry a bespoke failure
                // row.
                return render_activity_canonical(
                    message,
                    part,
                    &agena_domain::ActivityPayload::Error(agena_domain::ErrorActivity {
                        problem: problem.clone(),
                    }),
                    Some(failed_title.as_str()),
                    out,
                    width,
                    i18n,
                    defaults,
                    expansions,
                );
            }
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
            push_activity_headline(out, part.status, true, false, title.as_str(), "", width);
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: title,
                toggleable: false,
                expanded: true,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_RESOURCE))
            });
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
                copy_text: if expanded {
                    labels.join("\n")
                } else {
                    String::new()
                },
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
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_SKILL_REFERENCE))
            });
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
                copy_text: if expanded {
                    labels.join("\n")
                } else {
                    String::new()
                },
                toggleable: true,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                RequestPartResource::UserInput { request, reply } => {
                    let key = TranscriptNodeKey::Activity {
                        entry_id: message.id,
                        content_id: part.id,
                    };
                    // Answered user-input requests render as a foldable
                    // Activity, so the full "用户输入已答复" dump does not read
                    // like AI reply prose. The interaction kind id is already
                    // wired through `activity_kind_id_for_part`, so the per-kind
                    // transcript expansion setting applies here too.
                    let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                        defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_INTERACTION))
                    });
                    let title = if request.kind == "review" {
                        "Plan review"
                    } else {
                        "User input"
                    };
                    let summary = request
                        .questions
                        .first()
                        .map(|question| question.question.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(request.title.as_str());
                    push_activity_headline(out, part.status, expanded, true, title, summary, width);
                    if expanded {
                        // A pending part with a live interaction view renders
                        // its plan body and decision rows natively (the part IS
                        // the interaction surface, "everything is a part").
                        // Answered parts keep the plain replied/awaiting body.
                        if reply.is_none()
                            && let Some(view) = interactions.get(&request.request_id)
                        {
                            render_pending_interaction_body(out, request, view, width, i18n);
                        } else {
                            render_user_input_request(request, reply.as_ref(), out, width, i18n);
                        }
                    }
                    RenderedNodeDraft {
                        key,
                        kind: TranscriptNodeKind::Activity,
                        copy_text: if expanded {
                            request
                                .questions
                                .iter()
                                .map(|question| question.question.clone())
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            String::new()
                        },
                        toggleable: true,
                        expanded,
                        end_line: None,
                        children: Vec::new(),
                    }
                }
            }
        }
    }
}

/// Renders the expanded body of a pending interaction part natively. Plan
/// review draws the plan body through the standard Markdown pipeline at the
/// activity detail indent, a separator rule, and the decision rows whose
/// markers track the live [`PendingInteractionView`] — the transcript cursor IS
/// the review cursor, so the App derives row semantics from the same
/// [`crate::interaction_view::classify_interaction_line`] arithmetic.
///
/// Ask-user renders as one **continuous body** (plan + separator + every
/// question block + a footer key-hint): the transcript cursor IS the option
/// cursor, so the App derives row semantics from the same
/// [`crate::interaction_view::classify_ask_user_line`] arithmetic. The body
/// row count is pinned by [`crate::interaction_view::ask_user_body_rows`].
///
/// Review decision rows keep a FIXED row budget (2 rows per option, 2 per
/// custom slot, plus header/text/preview) matching the classifier, so a
/// rendered row always has a semantic kind and Enter/submit routing can never
/// drift from what the user sees.
fn render_pending_interaction_body(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    if crate::interaction_view::request_is_review_decision(request) {
        // Plan body: the standard transcript Markdown pipeline with the same
        // activity detail indent as every other expanded part.
        push_markdown_document(out, "    ", request.body_markdown.as_str(), width);
        push_markdown_rule(out, "    ", width);
        render_review_decision_rows(out, request, view, width, i18n);
    } else {
        // Ask-user: the whole body (plan + every question) in one continuous
        // markdown-like part; the transcript cursor is the option cursor.
        render_ask_user_body(out, request, view, width, i18n);
    }
}

/// Renders the canonical single-activity user-input surface on a `tool_call`
/// operation part: the pending interaction body while a request is awaiting a
/// reply (live view), or the read-only plan + questions + answers once
/// answered. Returns `false` (rendering nothing) when the operation carries no
/// user-input record, so the caller falls through to the plain tool execution
/// renderer.
#[allow(clippy::too_many_arguments)]
fn render_operation_user_input(
    part: &TranscriptEntryPart,
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> bool {
    let Some(record) = tool.user_input.requests.first() else {
        return false;
    };
    let request = crate::parts::user_input_request_resource(record.request.clone());
    // The headline leads with the CALLED TOOL's name — "plan.review" or
    // "interaction.ask" — not an invented label: the interaction IS this
    // operation, so its title should be the tool the model invoked. The
    // question text follows as the summary.
    let title = tool.invocation.name.trim();
    let title = if title.is_empty() {
        // Malformed/legacy operation without an invocation name.
        if request.kind == "review" {
            "Plan review"
        } else {
            "User input"
        }
    } else {
        title
    };
    let summary = request
        .questions
        .first()
        .map(|question| question.question.as_str())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(request.title.as_str());
    push_activity_headline(out, part.status, expanded, true, title, summary, width);
    if !expanded {
        return true;
    }
    if record.reply.is_none()
        && let Some(view) = interactions.get(&request.request_id)
    {
        render_pending_interaction_body(out, &request, view, width, i18n);
    } else {
        let reply = record
            .reply
            .clone()
            .map(crate::parts::user_input_reply_resource);
        render_answered_user_input_body(out, &request, reply.as_ref(), width, i18n);
    }
    true
}

/// Read-only body for an answered user-input request on a tool_call operation:
/// the plan body + separator, then each question with its committed answer
/// marked, and the custom feedback when provided. No interactive controls or
/// key hints — the activity stays expandable but is no longer an interaction
/// surface.
fn render_answered_user_input_body(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    reply: Option<&agena_api::resource::UserInputReply>,
    width: u16,
    i18n: &I18n,
) {
    if !request.body_markdown.trim().is_empty() {
        push_markdown_document(out, "    ", request.body_markdown.as_str(), width);
        push_markdown_rule(out, "    ", width);
    }
    let answers = reply.map(|reply| &reply.answers);
    if crate::interaction_view::request_is_review_decision(request) {
        render_answered_review_rows(out, request, answers, width, i18n);
        return;
    }
    for (index, question) in request.questions.iter().enumerate() {
        let values = answers
            .and_then(|answers| answers.get(&index.to_string()))
            .cloned()
            .unwrap_or_default();
        render_answered_question_block(out, index, question, &values, width, i18n);
    }
}

/// Read-only review decision rows: each option on ONE row with `(x)` on the
/// answered one, then the custom feedback when the answer is free text.
fn render_answered_review_rows(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    answers: Option<&std::collections::BTreeMap<String, Vec<String>>>,
    width: u16,
    i18n: &I18n,
) {
    let values = answers
        .and_then(|answers| answers.get("0"))
        .map(|values| values.as_slice())
        .unwrap_or_default();
    let Some(question) = request.questions.first() else {
        return;
    };
    for (_index, option) in question.options.iter().enumerate() {
        let answered = values.iter().any(|value| value == &option.label);
        let marker = if answered { "(x)" } else { "( )" };
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                option.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])));
    }
    if question.allow_custom {
        let custom: Vec<&String> = values
            .iter()
            .filter(|value| {
                !question
                    .options
                    .iter()
                    .any(|option| &option.label == *value)
            })
            .collect();
        if !custom.is_empty() {
            let marker = "(x)";
            out.push(RenderedLine::rich(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{marker} "), Style::default()),
                Span::styled(
                    i18n.text("overlay-user-input-review-feedback"),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])));
            push_interaction_detail_row(out, &join_answer_values(&custom), width);
        }
    }
}

/// Join committed answer values (which may be `&String`) into a comma list.
fn join_answer_values(values: &[&String]) -> String {
    values
        .iter()
        .map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Read-only ask-user question block: header with its answered marker, the
/// question text, each option on its (label + detail) rows with picked
/// options marked, and the custom values when present.
fn render_answered_question_block(
    out: &mut Vec<RenderedLine>,
    index: usize,
    question: &agena_api::resource::UserInputQuestion,
    values: &[String],
    width: u16,
    i18n: &I18n,
) {
    let header = if question.header.trim().is_empty() {
        format!("Q{}", index + 1)
    } else {
        question.header.clone()
    };
    let answered = !values.is_empty();
    let header_marker = if answered { "[x]" } else { "[ ]" };
    out.push(RenderedLine::rich(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("{header_marker} "), Style::default()),
        Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
    ])));
    push_interaction_detail_row(out, question.question.as_str(), width);
    for (_option_index, option) in question.options.iter().enumerate() {
        let picked = values.iter().any(|value| value == &option.label);
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                option.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])));
        push_interaction_detail_row(out, option.description.as_str(), width);
    }
    if question.allow_custom {
        let custom: Vec<&String> = values
            .iter()
            .filter(|value| {
                !question
                    .options
                    .iter()
                    .any(|option| &option.label == *value)
            })
            .collect();
        if !custom.is_empty() {
            out.push(RenderedLine::rich(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    i18n.text("overlay-user-input-other"),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])));
            push_interaction_detail_row(out, &join_answer_values(&custom), width);
        }
    }
}

/// Review decision rows: ONE row per option (marker + label — the description
/// detail line is gone), then the custom feedback slot, then a muted localized
/// footer hint. Markers `(x)`/`( )` track `view.selected_option`. The row
/// budget matches [`crate::interaction_view::review_decision_rows_count`] plus
/// the footer row.
fn render_review_decision_rows(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    let Some(question) = request.questions.first() else {
        return;
    };
    for (index, option) in question.options.iter().enumerate() {
        let marker = if view.selected_option == Some(index) {
            "(x)"
        } else {
            "( )"
        };
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                option.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])));
    }
    if question.allow_custom {
        let custom_index = question.options.len();
        if view.editing_custom {
            // The inline editor replaces the custom label row (one row, so the
            // fixed budget `review_decision_rows_count` counts it exactly).
            push_interaction_editor_row(out, &view.custom_draft, view.custom_cursor, width);
        } else {
            let marker = if view.selected_option == Some(custom_index) {
                "(x)"
            } else {
                "( )"
            };
            out.push(RenderedLine::rich(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{marker} "), Style::default()),
                Span::styled(
                    i18n.text("overlay-user-input-review-feedback"),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])));
        }
    }
    // Localized footer hint: Enter on an option submits, Enter on the title
    // collapses/expands. This row is never a submit target.
    out.push(RenderedLine::plain(
        format!("    {}", i18n.text("overlay-user-input-review-footer-hint")),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ));
}

/// One muted detail row below a decision label. Always pushed (even when
/// empty) so the fixed 2-rows-per-option budget the classifier uses holds.
fn push_interaction_detail_row(out: &mut Vec<RenderedLine>, text: &str, width: u16) {
    if text.trim().is_empty() {
        out.push(RenderedLine::plain("        ", Style::default()));
        return;
    }
    push_single_line(
        out,
        "        ",
        text,
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

/// The inline custom-feedback editor row: the live draft with a visible caret
/// at `cursor` (a byte offset into `draft`).
fn push_interaction_editor_row(
    out: &mut Vec<RenderedLine>,
    draft: &str,
    cursor: usize,
    width: u16,
) {
    let available = (width.max(1) as usize).saturating_sub(8);
    // Visible draft clamped to the row width.
    let visible: String = draft.chars().take(available).collect();
    // Cursor position measured in visible chars, clamped to the visible range.
    let caret_column = visible
        .char_indices()
        .take_while(|(offset, _)| *offset < cursor)
        .count()
        .min(visible.chars().count());
    let caret_at_end = caret_column == visible.chars().count();
    let mut spans = vec![Span::raw("        ")];
    for (index, ch) in visible.chars().enumerate() {
        if index == caret_column {
            if caret_at_end {
                spans.push(Span::styled(
                    "▏",
                    agena_tui_components::theme::selection_style(),
                ));
            } else {
                spans.push(Span::styled(
                    ch.to_string(),
                    agena_tui_components::theme::selection_style(),
                ));
                continue;
            }
        }
        spans.push(Span::raw(ch.to_string()));
    }
    if caret_at_end {
        spans.push(Span::styled(
            "▏",
            agena_tui_components::theme::selection_style(),
        ));
    }
    out.push(RenderedLine::rich(Line::from(spans)));
}

/// Ask-user continuous body: the plan body + separator, then EVERY question's
/// block (header, muted question-text row, ONE row per option, the custom
/// label + detail slot, an answered-preview row), then a muted footer key-hint
/// row. No paging, no summary page, no `▸` option cursor — the transcript
/// cursor IS the option cursor, highlighted whole-line by the App. Markers
/// `(x)`/`[x]`/`( )`/`[ ]` track `view.answers`. The row budget matches
/// [`crate::interaction_view::classify_ask_user_line`] via
/// [`crate::interaction_view::ask_user_question_block_rows`].
fn render_ask_user_body(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    push_markdown_document(out, "    ", request.body_markdown.as_str(), width);
    push_markdown_rule(out, "    ", width);
    for (index, question) in request.questions.iter().enumerate() {
        render_ask_user_question_block(out, request, index, question, view, width, i18n);
    }
    // Localized footer hint: the continuous-body key contract. This row is
    // never a submit target.
    out.push(RenderedLine::plain(
        format!("    {}", i18n.text("overlay-user-input-wizard-keys")),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ));
}

/// One ask-user question block in the continuous body: header row, muted
/// question-text row, ONE row per option, 2 per custom slot, and an
/// answered-preview row — the exact budget
/// [`crate::interaction_view::ask_user_question_block_rows`] counts. No cursor
/// rendering: the whole-line highlight is the App's job.
fn render_ask_user_question_block(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    index: usize,
    question: &agena_api::resource::UserInputQuestion,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    let answer = view.answers.get(&index);
    let answered = answer.is_some_and(|answer| answer.is_answered());
    let header = if question.header.trim().is_empty() {
        format!("Q{}", index + 1)
    } else {
        question.header.clone()
    };
    let header_marker = if answered { "[x]" } else { "[ ]" };
    out.push(RenderedLine::rich(Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("{header_marker} "), Style::default()),
        Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
    ])));
    push_interaction_detail_row(out, question.question.as_str(), width);
    for (option_index, option) in question.options.iter().enumerate() {
        let picked = answer.is_some_and(|answer| answer.picked.contains(&option_index));
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                option.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])));
    }
    if question.allow_custom {
        let custom_values = answer
            .map(|answer| answer.custom_values.as_slice())
            .unwrap_or_default();
        let picked = !custom_values.is_empty();
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                i18n.text("overlay-user-input-other"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])));
        if view.editing_custom && view.editing_question == Some(index) {
            // The inline editor replaces the custom detail row, so the fixed
            // 2-row custom budget is unchanged.
            push_interaction_editor_row(out, &view.custom_draft, view.custom_cursor, width);
        } else {
            let text = if custom_values.is_empty() {
                i18n.text("overlay-user-input-custom-empty")
            } else {
                custom_values.join(", ")
            };
            push_interaction_detail_row(out, &text, width);
        }
    }
    if answered {
        let summary = answer
            .map(|answer| ask_user_answer_summary(request, index, answer))
            .unwrap_or_default();
        push_interaction_detail_row(out, &summary, width);
    }
}

/// Joins one question's committed answer into a single summary string: the
/// picked option labels followed by the custom values.
fn ask_user_answer_summary(
    request: &agena_api::resource::UserInputRequest,
    index: usize,
    answer: &crate::interaction_view::PendingInteractionAnswerView,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(question) = request.questions.get(index) {
        for &picked in &answer.picked {
            if let Some(option) = question.options.get(picked) {
                parts.push(option.label.clone());
            }
        }
    }
    parts.extend(answer.custom_values.iter().cloned());
    parts.join(", ")
}

/// Shared renderer for canonical Activity payloads. Tool-call operations,
/// resources, reasoning and errors all flow through this single path so the
/// transcript keeps one headline/expandable-section contract. The legacy
/// `Error` part and a failed reply lifecycle both project into
/// `ActivityPayload::Error` here instead of owning separate renderers.
#[allow(clippy::too_many_arguments)]
fn render_activity_canonical(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    payload: &agena_domain::ActivityPayload,
    title_override: Option<&str>,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedNodeDraft {
    let key = TranscriptNodeKey::Activity {
        entry_id: message.id,
        content_id: part.id,
    };
    // Interstitial body segments are the working notes of a reply: they stay
    // collapsed until the user opens them, regardless of the global activity
    // default, so a multi-step tool run reads as a stack of blocks with one
    // visible answer at the end.
    let is_text_segment = matches!(payload, agena_domain::ActivityPayload::TextSegment(_));
    let default_expanded = if is_text_segment {
        false
    } else {
        defaults.default_expanded(activity_kind_id_for_payload(payload))
    };
    let expanded = expansions.get(&key).copied().unwrap_or(default_expanded);
    let (_, canonical_title, summary, error) = activity_presentation(payload);
    let title = title_override.unwrap_or(canonical_title.as_str());
    // A notice row carries the wall-clock time it was recorded so the hook
    // timeline reads at a glance instead of by position alone.
    let headline_summary = match payload {
        agena_domain::ActivityPayload::Notice(notice) => notice
            .occurred_at_ms
            .map(|ms| format!("{} · {}", summary, format_occurred_time(ms)))
            .unwrap_or_else(|| summary.clone()),
        _ => summary.clone(),
    };
    let error_text = error.as_ref().map(|e| e.user.fallback.clone());
    let error_equivalence_text = error.as_ref().map(|error| error.user.fallback.as_str());
    let details =
        canonical_activity_details(i18n, payload, summary.as_str(), error_equivalence_text);
    let toggleable = !summary.trim().is_empty() || error.is_some() || !details.is_empty();
    push_activity_headline(
        out,
        part.status,
        expanded,
        toggleable,
        title,
        headline_summary.as_str(),
        width,
    );
    let headline_end = out.len();
    let mut children = Vec::new();
    let mut copy_sections = Vec::<String>::new();
    let mut detail_index = 0_usize;
    let is_operation = matches!(payload, agena_domain::ActivityPayload::Operation(_));
    let render_summary = expanded && !is_operation && error.is_none() && !summary.trim().is_empty();
    if render_summary {
        let summary_start = out.len();
        render_expanded_tool_text_block(out, "    ", summary.as_str(), width);
        if !is_text_segment {
            patch_rendered_lines_style(
                &mut out[summary_start..],
                Style::default().fg(agena_tui_components::theme::muted_color()),
            );
        }
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
        copy_sections.push(summary.clone());
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
            if section_expanded {
                copy_sections.push(detail.copy_text());
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
            copy_sections.push(resource.name.clone());
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
        if section_expanded {
            copy_sections.push(error_str.clone());
        }
    }
    // Copy text mirrors the visible expansion state: a collapsed Activity
    // contributes nothing, and an expanded one carries only the sections
    // that are actually expanded.
    let mut sections = Vec::with_capacity(copy_sections.len() + 1);
    sections.push(title.to_owned());
    sections.extend(copy_sections);
    let copy_text = if expanded {
        join_canonical_copy_sections(sections)
    } else {
        String::new()
    };
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
    status: agena_api::part::PartExecutionStatusResource,
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
