use super::super::transcript_ast::{
    MarkdownBlockLayout, MarkdownNode, markdown_inline_line, push_markdown_block_spacing,
    render_attachment_image,
};
use super::super::{
    I18n, Local, Modifier, RenderedLine, RenderedTranscriptNode, RunStatus,
    SessionExecutionResource, Style, TOOL_CARD_PREVIEW_CHARS, TOOL_CARD_PREVIEW_LINES,
    ToolOutputPreview, TranscriptDetailDefaults, TranscriptEntry, TranscriptNodeKey,
    TranscriptNodeKind, UnicodeWidthStr, concise_text, format_occurred_time, format_timestamp,
    markdown_blocks, push_activity_headline, push_expanded_markdown, push_expanded_tool_text,
    push_label_value, push_literal_activity_headline, push_markdown_document, push_markdown_rule,
    push_section_heading, push_wrapped_line, push_wrapped_rich_line, render_entry_detailed,
    render_expanded_tool_text_block, render_markdown_block, should_suppress_markdown_block,
    strip_terminal_ansi_sequences, style_for_role, transcript_message_parts,
    transcript_part_content, transcript_spinner_placeholder, trim_empty_line_edges,
    truncate_display_width,
};
use super::operation_render::{
    render_tool_detail_sections_with_sections, render_tool_execution_with_sections,
};
use super::request_render::preview_for_part;
use crate::activity_presentation::activity_presentation;
use crate::display_types::TranscriptRun;
use crate::layout::{BODY_PREFIX, DETAIL_PREFIX};
use crate::part_headline::{
    attachment_label, compaction_notice, hook_notice, notification_notice, part_headline,
};
use crate::ui_text;
use crate::{
    ToolCallView, TranscriptActivityContent, TranscriptActivitySection,
    TranscriptAssistantReplyLifecycle, TranscriptEntryPart, TranscriptPartContent,
};
use agena_api::resource::{PartAttachment, PartAttachmentKind, PartAttachmentSource};
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
    let expansions = export_expansions(message);
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

/// Render one part without a viewport budget or an extra message header.
/// Source-backed bodies can be streamed separately by the export caller.
pub fn render_entry_part_export(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    let expansions = export_expansions(message);
    agena_tui_media::with_text_math_rendering(|| {
        let mut lines = Vec::new();
        render_part_node(
            message,
            part,
            TRANSCRIPT_EXPORT_WIDTH,
            &mut lines,
            i18n,
            defaults,
            &expansions,
            &Default::default(),
        );
        lines
    })
}

fn export_expansions(
    message: &TranscriptEntry,
) -> std::collections::BTreeMap<TranscriptNodeKey, bool> {
    // Exported documents expand the same canonical tool detail sections as
    // the interactive transcript.
    let mut expansions = std::collections::BTreeMap::new();
    for part in transcript_message_parts(message) {
        let sections = match transcript_part_content(part) {
            TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_)) => &[
                TranscriptActivitySection::TechnicalDetails,
                TranscriptActivitySection::RenderData,
                TranscriptActivitySection::Metadata,
                TranscriptActivitySection::Input,
                TranscriptActivitySection::Output,
                TranscriptActivitySection::Presentation,
            ][..],
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
    expansions
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
            copy_text: node.copy_text.into(),
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

/// Every contiguous activity run of one entry as `(start, end)` indexes into
/// `parts`. A run starts at an Activity node and extends over adjacent
/// Activity nodes and invisible blank text bridges, exactly like the collapse
/// scan above; this is the single source of truth for run identity, shared by
/// the fold renderer and the entry's activity-summary keys.
pub(crate) fn activity_runs(parts: &[TranscriptEntryPart]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut index = 0_usize;
    while index < parts.len() {
        if let Some(end) = collapsed_activity_run_end(parts, index) {
            runs.push((index, end));
            index = end;
        } else {
            index += 1;
        }
    }
    runs
}

pub const COLLAPSED_ACTIVITY_VISIBLE_COUNT: usize = 5;

fn is_invisible_activity_run_bridge(parts: &[TranscriptEntryPart], index: usize) -> bool {
    matches!(
        parts.get(index).map(transcript_part_content),
        Some(TranscriptPartContent::Text(text)) if text.text.trim().is_empty()
    )
}

pub(crate) fn is_activity_node(part: &TranscriptEntryPart) -> bool {
    matches!(transcript_part_content(part), TranscriptPartContent::Activity(content) if !matches!(content, TranscriptActivityContent::Fold { .. }))
}

fn activity_kind_id_for_payload(payload: &agena_domain::ActivityPayload) -> Option<&'static str> {
    match payload {
        agena_domain::ActivityPayload::Resource(_) => Some(agena_domain::ACTIVITY_KIND_RESOURCE),
        agena_domain::ActivityPayload::CommandReference(_) => {
            Some(agena_domain::ACTIVITY_KIND_COMMAND_REFERENCE)
        }
        agena_domain::ActivityPayload::TextArtifact(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
        agena_domain::ActivityPayload::Reasoning(_) => Some(agena_domain::ACTIVITY_KIND_REASONING),
        agena_domain::ActivityPayload::TextSegment(_) => Some(agena_domain::ACTIVITY_KIND_TEXT),
        agena_domain::ActivityPayload::Interaction(_) => {
            Some(agena_domain::ACTIVITY_KIND_INTERACTION)
        }
        agena_domain::ActivityPayload::Error(_) => Some(agena_domain::ACTIVITY_KIND_ERROR),
        agena_domain::ActivityPayload::Notice(_) => Some(agena_domain::ACTIVITY_KIND_NOTICE),
    }
}

fn operation_tool_preference_ids(name: &str, plugin_name: Option<&str>) -> Vec<String> {
    let raw = name.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return Vec::new();
    }
    let plugin = plugin_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let mut candidates = Vec::new();
    let mut push = |candidate: String| {
        if !candidate.is_empty() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    if raw.starts_with("agena.") {
        push(raw.clone());
    } else if let Some(plugin) = plugin.as_deref() {
        let short_plugin = plugin.strip_prefix("agena.").unwrap_or(plugin);
        if raw == short_plugin || raw.starts_with(format!("{short_plugin}.").as_str()) {
            push(format!("agena.{raw}"));
        } else {
            push(format!("{plugin}.{raw}"));
        }
    } else if raw.contains('.') {
        push(format!("agena.{raw}"));
    }

    push(raw.clone());
    if let Some(short) = raw.strip_prefix("agena.") {
        push(short.to_owned());
    }
    candidates
}

fn operation_default_expanded(
    defaults: &TranscriptDetailDefaults,
    name: &str,
    plugin_name: Option<&str>,
) -> bool {
    let tool_ids = operation_tool_preference_ids(name, plugin_name);
    for tool_id in &tool_ids {
        if let Some(expanded) = defaults
            .kind_defaults
            .get(format!("tool:{tool_id}").as_str())
        {
            return *expanded;
        }
    }
    for tool_id in &tool_ids {
        if let Some(category) = operation_tool_activity_category(tool_id)
            && let Some(expanded) = defaults
                .kind_defaults
                .get(format!("tool-category:{category}").as_str())
        {
            return *expanded;
        }
    }
    defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_OPERATION))
}

fn operation_tool_activity_category(tool_id: &str) -> Option<&'static str> {
    let tool = tool_id.strip_prefix("agena.").unwrap_or(tool_id);
    match tool {
        "read" | "fs.read" | "fs.read_many" | "fs.stat" => Some("read"),
        "list" | "fs.list" => Some("list"),
        "glob" | "fs.glob" => Some("glob"),
        "grep" | "fs.grep" => Some("grep"),
        "edit" | "fs.replace" => Some("edit"),
        "write" | "fs.write" => Some("write"),
        "apply_patch" | "fs.apply_patch" => Some("apply_patch"),
        "multiedit" => Some("multiedit"),
        "bash" | "shell.exec" | "shell.spawn" | "shell.watch" | "shell.open" => Some("bash"),
        "agent.spawn" => Some("agent"),
        "webfetch" | "web.read" | "web.fetch" => Some("webfetch"),
        "websearch" | "web.search" => Some("websearch"),
        "codesearch" | "code.search" => Some("codesearch"),
        "command" | "commands.run" => Some("command"),
        "lsp" | "lsp.hover" | "lsp.definition" | "lsp.references" => Some("lsp"),
        "todowrite" | "todo.write" => Some("todowrite"),
        "todoread" | "todo.read" => Some("todoread"),
        "question" | "interaction.ask" => Some("question"),
        "batch" | "tools.batch" => Some("batch"),
        "plan_enter" | "plan.enter" => Some("plan_enter"),
        "plan_exit" | "plan.exit" => Some("plan_exit"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalActivityDetailFormat {
    /// Render line-oriented output as plain text, but promote recognisable
    /// Markdown documents to the shared AST renderer.
    Auto,
    Json,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalActivityDetail {
    title: Option<String>,
    body: String,
    format: CanonicalActivityDetailFormat,
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
            default_expanded: Some(true),
        }
    }

    fn copy_text(&self) -> String {
        self.title.as_ref().map_or_else(
            || self.body.clone(),
            |title| format!("{title}\n{}", self.body),
        )
    }

    fn navigation_section(&self, detail_index: &mut usize) -> TranscriptActivitySection {
        let section = TranscriptActivitySection::Detail(*detail_index);
        *detail_index = detail_index.saturating_add(1);
        section
    }
}

fn canonical_activity_details(
    i18n: &I18n,
    payload: &agena_domain::ActivityPayload,
    _summary: &str,
) -> Vec<CanonicalActivityDetail> {
    match payload {
        agena_domain::ActivityPayload::CommandReference(command) => {
            vec![CanonicalActivityDetail::section(
                ui_text::t(i18n, "activity-detail-source").as_str(),
                format!("{} · {}", command.source, command.content_hash),
                CanonicalActivityDetailFormat::Plain,
            )]
        }
        agena_domain::ActivityPayload::Notice(notice) => notice
            .detail
            .as_ref()
            .map(|detail| {
                vec![CanonicalActivityDetail::section(
                    ui_text::t(i18n, "activity-detail-notice").as_str(),
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
                    match error.retry.as_ref() {
                        None => retry_directive_label(problem.retry, i18n),
                        Some(retry) => format!(
                            "{} · {}/{} · {}",
                            retry_directive_label(problem.retry, i18n),
                            retry.attempt,
                            retry.max_retries,
                            ui_text::t(
                                i18n,
                                match retry.outcome {
                                    agena_failure::RetryOutcome::Retrying =>
                                        "message-activity-response-running",
                                    agena_failure::RetryOutcome::Recovered =>
                                        "message-activity-response-completed",
                                    agena_failure::RetryOutcome::Failed =>
                                        "message-activity-response-failed",
                                    agena_failure::RetryOutcome::Cancelled =>
                                        "message-activity-response-cancelled",
                                }
                            )
                        ),
                    }
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
                details.push(i18n.text_args(
                    "activity-detail-type",
                    &agena_tui::fl_args!("media_type" => media_type.as_str()),
                ));
            }
            if let Some(size) = resource.size_bytes {
                details.push(
                    i18n.text_args("activity-detail-size", &agena_tui::fl_args!("size" => size)),
                );
            }
            if let (Some(width), Some(height)) = (resource.width, resource.height) {
                details.push(i18n.text_args(
                    "activity-detail-dimensions",
                    &agena_tui::fl_args!(
                        "width" => width,
                        "height" => height,
                    ),
                ));
            }
            (!details.is_empty())
                .then(|| {
                    CanonicalActivityDetail::section(
                        ui_text::t(i18n, "activity-detail-details").as_str(),
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
                    ui_text::t(i18n, "activity-detail-language").as_str(),
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
                        ui_text::t(i18n, "activity-detail-request").as_str(),
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
    i18n: &I18n,
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
            .then(|| canonical_activity_detail_preview(detail, i18n))
            .flatten()
            .map(|preview| format!(" · {preview}"))
            .unwrap_or_default();
        push_section_heading(
            out,
            format!("  {disclosure} {title}{preview}").as_str(),
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
        DETAIL_PREFIX
    } else {
        BODY_PREFIX
    };
    let body_start = out.len();
    match detail.format {
        CanonicalActivityDetailFormat::Auto => {
            render_expanded_tool_text_block(out, body_prefix, detail.body.as_str(), width, i18n);
        }
        CanonicalActivityDetailFormat::Json => {
            let markdown = format!("```json\n{}\n```", detail.body.trim());
            push_expanded_markdown(out, body_prefix, markdown.as_str(), width, i18n);
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

fn canonical_activity_detail_preview(
    detail: &CanonicalActivityDetail,
    i18n: &I18n,
) -> Option<String> {
    match detail.format {
        CanonicalActivityDetailFormat::Json => {
            serde_json::from_str::<serde_json::Value>(detail.body.as_str())
                .ok()
                .and_then(|value| match value {
                    serde_json::Value::Object(fields) => Some(i18n.text_args(
                        "activity-detail-preview-fields",
                        &ui_text::count_args("count", fields.len()),
                    )),
                    serde_json::Value::Array(items) => Some(i18n.text_args(
                        "activity-detail-preview-items",
                        &ui_text::count_args("count", items.len()),
                    )),
                    _ => None,
                })
        }
        CanonicalActivityDetailFormat::Auto => detail
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| canonical_activity_markdown_preview(line, i18n)),
        CanonicalActivityDetailFormat::Plain => detail
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| concise_text(line, 48)),
    }
}

fn canonical_activity_markdown_preview(line: &str, i18n: &I18n) -> String {
    let line = line.trim();
    let line = if line.starts_with('#') {
        line.trim_start_matches('#').trim_start()
    } else {
        ["- [x] ", "- [X] ", "- [ ] ", "- ", "* ", "+ ", "> "]
            .into_iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .unwrap_or(line)
    };
    let plain = markdown_inline_line(line, Style::default(), i18n)
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
        copy_text: copy_text.into(),
        atomic: lines[start_line..end_line]
            .iter()
            .any(|line| !line.math.is_empty()),
        toggleable,
        expanded,
    })
}

/// Plain message text and expanded answers share block layout and selection.
/// Spacing stays outside navigation nodes so a selected block never captures
/// a neighboring separator or the message header.
pub(crate) fn append_message_markdown_blocks(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    text: &str,
    width: u16,
    out: &mut Vec<RenderedLine>,
    children: &mut Vec<RenderedTranscriptNode>,
    i18n: &I18n,
) {
    let blocks = markdown_blocks(text);
    let mut previous = None;
    let mut layout = MarkdownBlockLayout::default();
    for (block_index, block) in blocks.iter().enumerate() {
        if should_suppress_markdown_block(blocks.as_slice(), block_index) {
            continue;
        }
        push_markdown_block_spacing(out, BODY_PREFIX, previous, &block.parsed);
        let block_prefix = layout.prefix(BODY_PREFIX, &block.parsed, width);
        let start_line = out.len();
        render_markdown_block(out, &block_prefix, block, width, i18n);
        if out.len() > start_line {
            let semantic_unit_count = out[start_line..]
                .iter()
                .filter_map(|line| line.navigation_unit)
                .fold((None, 0_usize), |(previous, count), unit| {
                    (
                        Some(unit),
                        count.saturating_add(usize::from(previous != Some(unit))),
                    )
                })
                .1;
            let atomic = if block.kind == TranscriptNodeKind::MarkdownMath {
                semantic_unit_count <= 1
            } else {
                block.kind.uses_atomic_navigation()
            };
            children.push(RenderedTranscriptNode {
                key: TranscriptNodeKey::MarkdownBlock {
                    entry_id: message.id,
                    content_id: part.id,
                    block_index,
                },
                kind: block.kind,
                start_line,
                end_line: out.len(),
                copy_text: block.copy_text.clone().into(),
                atomic,
                toggleable: false,
                expanded: true,
            });
        }
        previous = Some(&block.parsed);
    }
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

pub fn rewind_message_preview(message: &TranscriptRun, i18n: &I18n) -> String {
    entry_preview(&TranscriptEntry::from(message), i18n)
}

/// Export markdown from the canonical parts projection (the live wire
/// transcript), projecting it through [`crate::parts_entries`].
pub fn render_parts_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    parts: &[agena_api::part::PartResource],
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
            push_markdown_document(out, BODY_PREFIX, text.text.as_str(), width, i18n);
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Fold { hidden_count }) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or(false);
            let summary = super::super::transcript_tool_summary::push_fold_marker_row(
                out,
                i18n,
                *hidden_count,
                width,
            );
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: summary,
                toggleable: false,
                expanded,
                end_line: None,
                children: Vec::new(),
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            render_activity_canonical(
                message, part, payload, None, out, width, i18n, defaults, expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::TextArtifact(artifact)) => {
            render_activity_canonical(
                message,
                part,
                &agena_domain::ActivityPayload::TextArtifact(artifact.as_ref().clone()),
                None,
                out,
                width,
                i18n,
                defaults,
                expansions,
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
        // node so it is never folded with the working notes. The node owns
        // only its headline range; the body renders as one MarkdownBlock
        // child per block — the same projection plain message text uses — so
        // per-block/per-line selection, text objects (`vim`/`yam`/`gq`) and
        // copy operate on the answer exactly as on body text instead of
        // forcing whole-part selection.
        TranscriptPartContent::Activity(TranscriptActivityContent::Answer(answer)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or(true);
            let headline = part_headline(part, i18n);
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                &headline.title,
                &headline.summary,
                width,
                i18n,
            );
            let headline_end = out.len();
            let mut children = Vec::new();
            if expanded {
                append_message_markdown_blocks(
                    message,
                    part,
                    answer.text.as_str(),
                    width,
                    out,
                    &mut children,
                    i18n,
                );
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                // The Markdown block children own the answer body's copy; the
                // toggle node contributes nothing so an aggregate copy of the
                // entry contains each block exactly once.
                copy_text: String::new(),
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
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_REASONING))
            });
            let summary = reasoning.preferred_text();
            let headline = part_headline(part, i18n);
            push_activity_headline(
                out,
                part.status,
                expanded,
                true,
                &headline.title,
                &headline.summary,
                width,
                i18n,
            );
            if expanded && !summary.trim().is_empty() {
                let body_start = out.len();
                // Render reasoning as plain multi-line text, never through the
                // markdown path, so the full thought trail is shown verbatim
                // with no ellipsis or reformatting.
                push_expanded_tool_text(
                    out,
                    BODY_PREFIX,
                    summary.as_str(),
                    Style::default(),
                    width,
                );
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
                tool.has_interaction()
                    || operation_default_expanded(
                        defaults,
                        tool.operation.invocation.name.as_str(),
                        tool.operation.invocation.plugin_name.as_deref(),
                    )
            });
            // Canonical single-activity shape: a tool operation carrying a
            // user-input record IS the interaction part. Render the pending
            // interaction body while awaiting a reply (live view), the
            // read-only plan + questions + answers once answered; fall through
            // to the plain tool execution renderer for operations without
            // user input.
            let interaction_start = out.len();
            let user_input_rendered =
                render_operation_user_input(part, tool, out, width, i18n, expanded, interactions);
            let permission_rendered = render_operation_permission(
                part,
                tool,
                out,
                width,
                i18n,
                expanded,
                !user_input_rendered,
            );
            let user_input_rendered = user_input_rendered || permission_rendered;

            let details_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::TechnicalDetails,
            };
            let render_data_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::RenderData,
            };
            let metadata_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Metadata,
            };
            let input_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Input,
            };
            let output_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Output,
            };
            let presentation_key = TranscriptNodeKey::ActivitySection {
                entry_id: message.id,
                content_id: part.id,
                section: TranscriptActivitySection::Presentation,
            };
            let metadata_expanded = expansions.get(&metadata_key).copied().unwrap_or(false);
            let input_expanded = expansions.get(&input_key).copied().unwrap_or(false);
            let output_expanded = expansions.get(&output_key).copied().unwrap_or(false);
            let details_expanded = expansions.get(&details_key).copied().unwrap_or(false);
            let render_data_expanded = expansions.get(&render_data_key).copied().unwrap_or(false);
            let execution = if user_input_rendered {
                render_tool_detail_sections_with_sections(
                    part,
                    metadata_expanded,
                    input_expanded,
                    output_expanded,
                    details_expanded,
                    render_data_expanded,
                    tool,
                    out,
                    width,
                    i18n,
                    expanded,
                )
            } else {
                render_tool_execution_with_sections(
                    part,
                    metadata_expanded,
                    input_expanded,
                    output_expanded,
                    details_expanded,
                    render_data_expanded,
                    tool,
                    out,
                    width,
                    i18n,
                    expanded,
                )
            };
            let mut children = Vec::new();
            if let Some(section) = execution.presentation
                && let Some(child) = rendered_activity_section_node(
                    presentation_key,
                    section.start_line,
                    section.end_line,
                    section.copy_text,
                    false,
                    section.expanded,
                    out,
                )
            {
                children.push(child);
            }
            if let Some(section) = execution.details
                && let Some(child) = rendered_activity_section_node(
                    details_key,
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
            if let Some(section) = execution.metadata
                && let Some(child) = rendered_activity_section_node(
                    metadata_key,
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
            if let Some(section) = execution.render_data
                && let Some(child) = rendered_activity_section_node(
                    render_data_key,
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
                copy_text: if user_input_rendered {
                    if expanded {
                        let interaction = out[interaction_start..execution.headline_end]
                            .iter()
                            .map(|line| line.copy_text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        [interaction, execution.visible_copy_text]
                            .into_iter()
                            .filter(|section| !section.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    } else {
                        String::new()
                    }
                } else {
                    execution.visible_copy_text
                },
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
                    retry: error.retry.clone(),
                }),
                None,
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Notice(notice)) => {
            render_notice_row(
                message,
                part,
                agena_domain::NoticeActivity {
                    kind: if notice.kind.trim().is_empty() {
                        "notice".to_owned()
                    } else {
                        notice.kind.clone()
                    },
                    summary: notice.summary.clone(),
                    detail: notice.detail.clone(),
                    occurred_at_ms: None,
                    title: notice.title.clone(),
                },
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Hook(hook)) => {
            render_notice_row(
                message,
                part,
                hook_notice(hook),
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Compaction(compaction)) => {
            render_notice_row(
                message,
                part,
                compaction_notice(compaction, i18n),
                out,
                width,
                i18n,
                defaults,
                expansions,
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::SystemNotification(
            notification,
        )) => render_notice_row(
            message,
            part,
            notification_notice(notification),
            out,
            width,
            i18n,
            defaults,
            expansions,
        ),
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
                    TranscriptAssistantReplyLifecycle::Cancelled => {
                        "message-activity-response-cancelled"
                    }
                },
            );
            push_activity_headline(
                out,
                part.status,
                true,
                false,
                title.as_str(),
                "",
                width,
                i18n,
            );
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
                let label = attachment_label(item, i18n);
                labels.push(label);
            }
            let headline = part_headline(part, i18n);
            push_literal_activity_headline(
                out,
                part.status,
                expanded,
                true,
                &headline.title,
                &headline.summary,
                width,
                i18n,
            );
            if expanded {
                for item in &attachment.attachments {
                    let label = attachment_label(item, i18n);
                    if !render_attachment_image(out, BODY_PREFIX, item, width, i18n) {
                        push_label_value(out, "  - ", label.as_str(), Style::default(), width);
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
        TranscriptPartContent::Activity(TranscriptActivityContent::CommandReference(reference)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions.get(&key).copied().unwrap_or_else(|| {
                defaults.default_expanded(Some(agena_domain::ACTIVITY_KIND_COMMAND_REFERENCE))
            });
            let mut labels = Vec::new();
            for command in &reference.commands {
                labels.push(command.name.clone());
            }
            let headline = part_headline(part, i18n);
            push_literal_activity_headline(
                out,
                part.status,
                expanded,
                true,
                &headline.title,
                &headline.summary,
                width,
                i18n,
            );
            if expanded {
                for command in &reference.commands {
                    render_canonical_activity_detail(
                        out,
                        &CanonicalActivityDetail::section(
                            command.name.as_str(),
                            command.description.as_str(),
                            CanonicalActivityDetailFormat::Auto,
                        ),
                        width,
                        None,
                        true,
                        i18n,
                    );
                    render_canonical_activity_detail(
                        out,
                        &CanonicalActivityDetail::section(
                            ui_text::t(i18n, "activity-detail-source").as_str(),
                            format!("{} · {}", command.source, command.content_hash),
                            CanonicalActivityDetailFormat::Plain,
                        ),
                        width,
                        None,
                        true,
                        i18n,
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
    }
}

/// Draw interaction content and attach actions to the rows actually rendered.
/// Descriptions wrap freely; keyboard and pointer routing share the metadata.
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
        push_markdown_document(
            out,
            BODY_PREFIX,
            request.body_markdown.as_str(),
            width,
            i18n,
        );
        push_markdown_rule(out, BODY_PREFIX, width);
        render_review_decision_rows(out, request, view, width, i18n);
    } else {
        // The transcript cursor owns both option selection and page actions.
        render_ask_user_body(out, request, view, width, i18n);
    }
}

/// Pending permission decisions use the same inline cursor as questions.
/// The detailed permission surface remains available for scope/rule editing.
#[allow(clippy::too_many_arguments)]
fn render_operation_permission(
    part: &TranscriptEntryPart,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    with_headline: bool,
) -> bool {
    use agena_domain::{PermissionAction, PermissionReplyKind};
    if part.status.is_terminal() || tool.operation.status().is_terminal() {
        return false;
    }
    let pending: Vec<_> = tool
        .operation
        .authorization
        .permissions
        .iter()
        .filter(|record| record.reply.is_none())
        .collect();
    if pending.is_empty() {
        return false;
    }
    if with_headline {
        push_activity_headline(
            out,
            part.status,
            expanded,
            true,
            &tool.operation.invocation.name,
            &i18n.text("overlay-permission-title"),
            width,
            i18n,
        );
    }
    if !expanded {
        return true;
    }
    for record in pending {
        let request = &record.request;
        let actions = std::iter::once(&request.action)
            .chain(
                request
                    .requested_actions
                    .iter()
                    .filter(|action| **action != request.action),
            )
            .collect::<Vec<_>>();
        for action in actions {
            let label = match action {
                PermissionAction::Tool {
                    tool_name,
                    qualifier,
                } => qualifier
                    .as_ref()
                    .map(|qualifier| format!("{tool_name} · {qualifier}"))
                    .unwrap_or_else(|| tool_name.clone()),
                PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => format!("{access_kind} · {target_path}"),
                PermissionAction::NetworkAccess { target, .. } => target.clone(),
            };
            push_expanded_tool_text(
                out,
                BODY_PREFIX,
                &label,
                Style::default().add_modifier(Modifier::BOLD),
                width,
            );
        }
        let reason = if request.reason.trim().is_empty() {
            &request.explanation
        } else {
            &request.reason
        };
        push_expanded_tool_text(
            out,
            BODY_PREFIX,
            reason,
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
        for decision in [
            Some(PermissionReplyKind::AllowOnce),
            Some(PermissionReplyKind::AllowAlways),
            Some(PermissionReplyKind::DenyOnce),
            None,
        ] {
            let label = decision
                .map(|kind| {
                    let label = ui_text::permission_reply_label(i18n, kind);
                    if kind == PermissionReplyKind::AllowAlways {
                        format!("{label}…")
                    } else {
                        label
                    }
                })
                .unwrap_or_else(|| i18n.text("overlay-permission-details-title"));
            let start = out.len();
            push_expanded_tool_text(
                out,
                BODY_PREFIX,
                &format!("[ {label} ]"),
                Style::default().fg(if matches!(decision, Some(PermissionReplyKind::DenyOnce)) {
                    agena_tui_components::theme::danger_color()
                } else {
                    agena_tui_components::theme::accent_color()
                }),
                width,
            );
            for row in &mut out[start..] {
                let end = UnicodeWidthStr::width(row.text.as_str())
                    .min(usize::from(width))
                    .max(1);
                row.controls.push(crate::RenderedTranscriptControl {
                    columns: BODY_PREFIX.len().min(end - 1)..end,
                    action: crate::TranscriptControlAction::Permission {
                        request_id: request.request_id.clone(),
                        decision,
                    },
                    enabled: true,
                });
            }
        }
    }
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        &i18n.text("overlay-user-input-review-footer-hint"),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
    true
}

/// Renders the canonical single-activity user-input surface on a `tool_call`
/// operation part: the current pending interaction body followed by retained
/// request history, or read-only plans, questions and answers once terminal.
/// Returns `false` (rendering nothing) when the operation carries no
/// user-input record, so the caller falls through to the plain tool execution
/// renderer.
#[allow(clippy::too_many_arguments)]
fn render_operation_user_input(
    part: &TranscriptEntryPart,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> bool {
    let records = &tool.operation.user_input.requests;
    let pending_id = crate::interaction_view::interaction_request_id_for_part(part);
    let primary_index = pending_id
        .and_then(|id| {
            records
                .iter()
                .position(|record| record.request.request_id == id)
        })
        .unwrap_or(0);
    let Some(record) = records.get(primary_index) else {
        return false;
    };
    let request = crate::parts::user_input_request_resource(record.request.clone());
    let headline = part_headline(part, i18n);
    push_activity_headline(
        out,
        part.status,
        expanded,
        true,
        &headline.title,
        &headline.summary,
        width,
        i18n,
    );
    if !expanded {
        return true;
    }
    if pending_id == Some(request.request_id.as_str())
        && let Some(view) = interactions.get(&request.request_id)
    {
        render_pending_interaction_body(out, &request, view, width, i18n);
    } else {
        let reply = record
            .reply
            .clone()
            .map(crate::parts::user_input_reply_resource);
        render_user_input_history_body(out, &request, reply.as_ref(), width, i18n);
    }
    // Keep the actionable request at the normal body offset so key routing
    // stays aligned. Earlier records remain visible after the live controls;
    // an operation may ask more than once during a single tool invocation.
    for (index, record) in records.iter().enumerate() {
        if index == primary_index {
            continue;
        }
        let request = crate::parts::user_input_request_resource(record.request.clone());
        push_markdown_rule(out, BODY_PREFIX, width);
        if !request.title.trim().is_empty() {
            push_expanded_tool_text(
                out,
                BODY_PREFIX,
                request.title.as_str(),
                Style::default().add_modifier(Modifier::BOLD),
                width,
            );
        }
        let reply = record
            .reply
            .clone()
            .map(crate::parts::user_input_reply_resource);
        render_user_input_history_body(out, &request, reply.as_ref(), width, i18n);
    }
    true
}

/// Read-only history for a user-input request on a tool_call operation:
/// the plan body + separator, then each question with its committed answer
/// marked, and the custom feedback when provided. No interactive controls or
/// key hints — the activity stays expandable but is no longer an interaction
/// surface.
fn render_user_input_history_body(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    reply: Option<&agena_api::resource::UserInputReply>,
    width: u16,
    i18n: &I18n,
) {
    if !request.body_markdown.trim().is_empty() {
        push_markdown_document(
            out,
            BODY_PREFIX,
            request.body_markdown.as_str(),
            width,
            i18n,
        );
        push_markdown_rule(out, BODY_PREFIX, width);
    }
    if let Some(reply) = reply {
        let status = match reply.kind {
            agena_api::resource::UserInputReplyKind::Cancel => {
                Some("session-work-status-cancelled")
            }
            agena_api::resource::UserInputReplyKind::Timeout => Some("failure-category-timeout"),
            agena_api::resource::UserInputReplyKind::Submit => None,
        };
        if let Some(status) = status {
            push_expanded_tool_text(
                out,
                BODY_PREFIX,
                &i18n.text(status),
                Style::default().add_modifier(Modifier::BOLD),
                width,
            );
            if let Some(reason) = &reply.reason {
                push_interaction_detail_row(out, reason, width);
            }
            return;
        }
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

/// Read-only decisions contain only committed values, never old choices.
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
    render_committed_answers(
        out,
        question,
        values,
        "overlay-user-input-review-feedback",
        width,
        i18n,
    );
}

/// Question history preserves the full question and its submitted answers.
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
    out.push(RenderedLine::rich(Line::from(vec![
        Span::raw(BODY_PREFIX),
        Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
    ])));
    push_interaction_detail_row(out, question.question.as_str(), width);
    render_committed_answers(
        out,
        question,
        values,
        "overlay-user-input-other",
        width,
        i18n,
    );
}

fn render_committed_answers(
    out: &mut Vec<RenderedLine>,
    question: &agena_api::resource::UserInputQuestion,
    values: &[String],
    custom_label: &str,
    width: u16,
    i18n: &I18n,
) {
    if values.is_empty() {
        push_interaction_detail_row(out, &i18n.text("overlay-user-input-unanswered"), width);
    }
    for value in values {
        let option = question
            .options
            .iter()
            .find(|option| &option.label == value);
        let label = option
            .map(|option| option.label.clone())
            .unwrap_or_else(|| i18n.text(custom_label));
        push_expanded_tool_text(
            out,
            "  ✓ ",
            &label,
            Style::default().add_modifier(Modifier::BOLD),
            width,
        );
        if let Some(option) = option {
            push_interaction_detail_row(out, &option.description, width);
        } else {
            push_interaction_detail_row(out, value, width);
        }
    }
}

/// Each action owns its wrapped label and description. Input routing reads
/// this metadata directly, so changing the layout never changes the action.
fn mark_interaction_control(
    lines: &mut [RenderedLine],
    request_id: &str,
    kind: crate::InteractionLineKind,
    enabled: bool,
    width: u16,
) {
    for line in lines {
        let end = UnicodeWidthStr::width(line.text.as_str())
            .min(usize::from(width))
            .max(1);
        line.controls.push(crate::RenderedTranscriptControl {
            columns: BODY_PREFIX.len().min(end - 1)..end,
            action: crate::TranscriptControlAction::UserInput {
                request_id: request_id.to_owned(),
                kind,
            },
            enabled,
        });
    }
}

fn push_interaction_control(
    out: &mut Vec<RenderedLine>,
    request_id: &str,
    kind: crate::InteractionLineKind,
    label: &str,
    detail: &str,
    width: u16,
) {
    let start = out.len();
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        label,
        Style::default().add_modifier(Modifier::BOLD),
        width,
    );
    if !detail.trim().is_empty() {
        push_expanded_tool_text(
            out,
            DETAIL_PREFIX,
            detail,
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
    }
    mark_interaction_control(&mut out[start..], request_id, kind, true, width);
}

/// A compact action bar wraps whole buttons on small terminals. Columns are
/// measured in display cells, including wide CJK labels.
fn push_interaction_actions(
    out: &mut Vec<RenderedLine>,
    request_id: &str,
    actions: &[(crate::InteractionLineKind, String, bool)],
    width: u16,
) {
    let indent = BODY_PREFIX.len();
    let new_line =
        || RenderedLine::plain(BODY_PREFIX, Style::default()).with_copy_projection("", indent);
    let mut line = new_line();
    let mut column = indent;
    for (kind, label, enabled) in actions {
        let text = format!("[ {label} ]");
        let style = Style::default().fg(if *enabled {
            agena_tui_components::theme::accent_color()
        } else {
            agena_tui_components::theme::muted_color()
        });
        let cells = UnicodeWidthStr::width(text.as_str());
        let separator = if line.controls.is_empty() { 0 } else { 2 };
        if column > indent && column + separator + cells > usize::from(width) {
            out.push(line);
            line = new_line();
            column = indent;
        }
        if cells + indent > usize::from(width) {
            if !line.controls.is_empty() {
                out.push(line);
                line = new_line();
                column = indent;
            }
            let start = out.len();
            push_expanded_tool_text(out, BODY_PREFIX, &text, style, width);
            mark_interaction_control(&mut out[start..], request_id, *kind, *enabled, width);
            continue;
        }
        if !line.controls.is_empty() {
            line.text.push_str("  ");
            line.rich_line
                .as_mut()
                .expect("plain lines have rich text")
                .spans
                .push(Span::raw("  "));
            column += 2;
        }
        line.text.push_str(&text);
        line.copy_text = line.text.trim().to_owned();
        line.rich_line
            .as_mut()
            .expect("plain lines have rich text")
            .spans
            .push(Span::styled(text, style));
        line.controls.push(crate::RenderedTranscriptControl {
            columns: column..column + cells,
            action: crate::TranscriptControlAction::UserInput {
                request_id: request_id.to_owned(),
                kind: *kind,
            },
            enabled: *enabled,
        });
        column += cells;
    }
    if !line.controls.is_empty() {
        out.push(line);
    }
}

fn render_review_decision_rows(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    use crate::InteractionLineKind::*;
    let Some(question) = request.questions.first() else {
        return;
    };
    for (index, option) in question.options.iter().enumerate() {
        push_interaction_control(
            out,
            &request.request_id,
            ReviewOption {
                option_index: index,
            },
            &format!("› {}", option.label),
            &option.description,
            width,
        );
    }
    if question.allow_custom {
        push_interaction_control(
            out,
            &request.request_id,
            ReviewCustomLabel,
            &format!("＋ {}", i18n.text("overlay-user-input-review-feedback")),
            &view.custom_text,
            width,
        );
        if view.editing_custom {
            let start = out.len();
            push_interaction_editor_row(out, &view.custom_draft, view.custom_cursor, width);
            mark_interaction_control(
                &mut out[start..],
                &request.request_id,
                ReviewEditor,
                true,
                width,
            );
        }
    }
    push_interaction_actions(
        out,
        &request.request_id,
        &[(AskCancel, i18n.text("overlay-user-input-defer"), true)],
        width,
    );
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        &i18n.text("overlay-user-input-review-footer-hint"),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

/// A compact read-only detail row for an answered interaction.
fn push_interaction_detail_row(out: &mut Vec<RenderedLine>, text: &str, width: u16) {
    if text.trim().is_empty() {
        return;
    }
    push_expanded_tool_text(
        out,
        DETAIL_PREFIX,
        text,
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

/// A bounded, soft-wrapped draft viewport that follows its byte-offset caret.
/// Graphemes stay intact and widths are terminal cells, including CJK/emoji.
fn push_interaction_editor_row(
    out: &mut Vec<RenderedLine>,
    draft: &str,
    cursor: usize,
    width: u16,
) {
    let width = usize::from(width.max(1));
    let indent = width.saturating_sub(1).min(DETAIL_PREFIX.len());
    let available = width - indent;
    let cursor = draft
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(draft.len()))
        .take_while(|offset| *offset <= cursor.min(draft.len()))
        .last()
        .unwrap_or(0);
    let mut glyphs = Vec::new();
    for (offset, grapheme) in draft
        .grapheme_indices(true)
        .chain(std::iter::once((draft.len(), "")))
    {
        if offset == cursor {
            glyphs.push(("▏", true));
        }
        if !grapheme.is_empty() {
            glyphs.push((grapheme, false));
        }
    }
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    let mut column = 0;
    let mut caret_row = 0;
    for (grapheme, caret) in glyphs {
        if grapheme.contains('\n') {
            rows.push(Line::from(std::mem::take(&mut spans)));
            column = 0;
            continue;
        }
        let grapheme = if UnicodeWidthStr::width(grapheme) > available {
            "…"
        } else {
            grapheme
        };
        let cells = UnicodeWidthStr::width(grapheme);
        if column + cells > available {
            rows.push(Line::from(std::mem::take(&mut spans)));
            column = 0;
        }
        if caret {
            caret_row = rows.len();
        }
        spans.push(Span::styled(
            grapheme.to_owned(),
            if caret {
                agena_tui_components::theme::selection_style()
            } else {
                Style::default()
            },
        ));
        column += cells;
    }
    rows.push(Line::from(spans));
    for mut row in rows.into_iter().skip(caret_row.saturating_sub(3)).take(4) {
        row.spans.insert(0, Span::raw(" ".repeat(indent)));
        out.push(RenderedLine::rich(row));
    }
}

/// Render one question and its navigation, or an optional editable summary.
fn render_ask_user_body(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    use crate::InteractionLineKind::*;
    let page = view.question_page.min(request.questions.len());
    if page == 0 {
        push_markdown_document(out, BODY_PREFIX, &request.body_markdown, width, i18n);
    }
    push_markdown_rule(out, BODY_PREFIX, width);
    let summary = page == request.questions.len();
    let title = if summary {
        i18n.text("overlay-user-input-summary")
    } else {
        i18n.text_args("overlay-user-input-page", &agena_tui::fl_args!("current" => (page + 1) as i64, "total" => request.questions.len() as i64))
    };
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        &title,
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    if let Some(question) = request.questions.get(page) {
        render_ask_user_question_block(out, request, page, question, view, width, i18n);
    } else {
        for index in 0..request.questions.len() {
            let start = out.len();
            for line in crate::interaction_view::ask_user_summary_lines(
                request,
                index,
                view.answers.get(&index),
                width,
            ) {
                out.push(
                    RenderedLine::plain(format!("{BODY_PREFIX}{line}"), Style::default())
                        .with_copy_projection(line, BODY_PREFIX.len()),
                );
            }
            mark_interaction_control(
                &mut out[start..],
                &request.request_id,
                AskSummary {
                    question_index: index,
                },
                true,
                width,
            );
        }
    }
    let mut actions = vec![(
        AskPrevious,
        format!("‹ {}", i18n.text("overlay-user-input-previous")),
        page > 0,
    )];
    if summary {
        actions.push((AskSubmit, i18n.text("overlay-user-input-submit"), true));
        actions.push((AskCancel, i18n.text("overlay-user-input-defer"), true));
    } else {
        let next = if page + 1 == request.questions.len() {
            "overlay-user-input-summary"
        } else {
            "overlay-user-input-next"
        };
        actions.push((AskNext, format!("{} ›", i18n.text(next)), true));
        if page + 1 == request.questions.len() {
            actions.push((AskSubmit, i18n.text("overlay-user-input-submit"), true));
        }
    }
    // Horizontal keys now change questions. Keep every footer action on its
    // own row so the shared Up/Down cursor can reach all of them.
    for action in &actions {
        push_interaction_actions(
            out,
            &request.request_id,
            std::slice::from_ref(action),
            width,
        );
    }
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        &i18n.text("overlay-user-input-question-footer-hint"),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

fn render_ask_user_question_block(
    out: &mut Vec<RenderedLine>,
    request: &agena_api::resource::UserInputRequest,
    index: usize,
    question: &agena_api::resource::UserInputQuestion,
    view: &crate::interaction_view::PendingInteractionView,
    width: u16,
    i18n: &I18n,
) {
    use crate::InteractionLineKind::*;
    let answer = view.answers.get(&index);
    if !question.header.trim().is_empty() {
        push_expanded_tool_text(
            out,
            BODY_PREFIX,
            &question.header,
            Style::default().add_modifier(Modifier::BOLD),
            width,
        );
    }
    push_expanded_tool_text(
        out,
        BODY_PREFIX,
        &question.question,
        Style::default(),
        width,
    );
    let legacy_custom =
        question.allow_custom && !question.options.iter().any(|option| option.custom);
    let options = question
        .options
        .iter()
        .map(Some)
        .chain(legacy_custom.then_some(None));
    for (option_index, option) in options.enumerate() {
        let custom = option.is_none_or(|option| option.custom);
        let picked = answer.is_some_and(|answer| answer.picked.contains(&option_index));
        let marker = if question.multiple {
            if picked { "[✓]" } else { "[ ]" }
        } else if picked {
            "(●)"
        } else {
            "( )"
        };
        let label = option
            .map(|option| option.label.clone())
            .unwrap_or_else(|| i18n.text("overlay-user-input-other"));
        let description = option
            .map(|option| option.description.as_str())
            .unwrap_or_default();
        let custom_values = if custom && picked {
            answer
                .map(|answer| answer.custom_values.join("\n"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let detail = if custom_values.is_empty() {
            description
        } else {
            &custom_values
        };
        let kind = if custom {
            AskCustomRow {
                question_index: index,
            }
        } else {
            AskOption {
                question_index: index,
                option_index,
            }
        };
        push_interaction_control(
            out,
            &request.request_id,
            kind,
            &format!("{marker} {label}"),
            detail,
            width,
        );
        if custom && view.editing_custom && view.editing_question == Some(index) {
            let start = out.len();
            push_interaction_editor_row(out, &view.custom_draft, view.custom_cursor, width);
            mark_interaction_control(
                &mut out[start..],
                &request.request_id,
                AskCustomEditor {
                    question_index: index,
                },
                true,
                width,
            );
        }
    }
}

/// Render one notification-class activity. Notices, hooks, compactions, and
/// background notifications share one canonical Notice presentation, so their
/// rows stay one vocabulary while their content shapes stay distinct.
fn render_notice_row(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    notice: agena_domain::NoticeActivity,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedNodeDraft {
    render_activity_canonical(
        message,
        part,
        &agena_domain::ActivityPayload::Notice(notice),
        None,
        out,
        width,
        i18n,
        defaults,
        expansions,
    )
}

/// Shared renderer for canonical Activity payloads. Tool-call operations,
/// resources, reasoning and errors all flow through this single path so the
/// transcript keeps one headline/expandable-section contract. The canonical
/// `Error` part projects into `ActivityPayload::Error` here.
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
    let is_text_segment = matches!(payload, agena_domain::ActivityPayload::TextSegment(_));
    let default_expanded = defaults.default_expanded(activity_kind_id_for_payload(payload));
    let expanded = expansions.get(&key).copied().unwrap_or(default_expanded);
    let (_, _, summary, error) = activity_presentation(payload);
    let retry_headline = match payload {
        agena_domain::ActivityPayload::Error(error) => error.retry.as_ref().map(|retry| {
            crate::error_part::retry_headline(retry, chrono::Utc::now().timestamp_millis(), i18n)
        }),
        _ => None,
    };
    let headline = part_headline(part, i18n);
    let localized_title = title_override.map(str::to_owned).unwrap_or(headline.title);
    let title = retry_headline
        .as_ref()
        .map(|header| header.title.as_str())
        .unwrap_or(localized_title.as_str());
    // A notice row carries the wall-clock time it was recorded so the hook
    // timeline reads at a glance instead of by position alone.
    let headline_summary = match payload {
        agena_domain::ActivityPayload::Notice(notice) => notice
            .occurred_at_ms
            .map(|ms| format!("{} · {}", headline.summary, format_occurred_time(ms)))
            .unwrap_or_else(|| headline.summary.clone()),
        _ => headline.summary.clone(),
    };
    let error_text = error.as_ref().map(|e| e.user.fallback.clone());
    let details = canonical_activity_details(i18n, payload, summary.as_str());
    let toggleable = !summary.trim().is_empty() || error.is_some() || !details.is_empty();
    push_activity_headline(
        out,
        retry_headline
            .as_ref()
            .map(|header| header.status)
            .unwrap_or(part.status),
        expanded,
        toggleable,
        title,
        headline_summary.as_str(),
        width,
        i18n,
    );
    if let Some(header) = &retry_headline {
        let prefix = if usize::from(width) > BODY_PREFIX.len() {
            BODY_PREFIX
        } else {
            ""
        };
        push_wrapped_rich_line(
            out,
            prefix,
            prefix,
            Line::from(vec![Span::styled(
                header.progress.clone(),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )]),
            width,
        );
    }
    let headline_end = out.len();
    let mut children = Vec::new();
    let mut copy_sections = Vec::<String>::new();
    let mut detail_index = 0_usize;
    let render_summary = expanded && error.is_none() && !summary.trim().is_empty();
    if render_summary {
        let summary_start = out.len();
        render_expanded_tool_text_block(out, BODY_PREFIX, summary.as_str(), width, i18n);
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
            render_canonical_activity_detail(out, detail, width, None, section_expanded, i18n);
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
            let _ = render_attachment_image(out, BODY_PREFIX, &attachment, width, i18n);
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
                ui_text::t(i18n, "activity-title-error").as_str(),
                error_str,
                CanonicalActivityDetailFormat::Auto,
            ),
            width,
            Some(Style::default().fg(agena_tui_components::theme::danger_color())),
            section_expanded,
            i18n,
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
                        crate::TranscriptUserActivityStyle::Command => Style::default()
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
    let line_width = usize::from(width).saturating_sub(BODY_PREFIX.len()).max(1);
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
    let mut spans = vec![Span::raw(BODY_PREFIX)];
    for token in tokens {
        if let Some(last) = spans.last_mut()
            && last.style == token.style
        {
            last.content.to_mut().push_str(token.text.as_str());
        } else {
            spans.push(Span::styled(token.text, token.style));
        }
    }
    out.push(
        RenderedLine::rich(Line::from(spans)).with_copy_projection(copy_text, BODY_PREFIX.len()),
    );
}
