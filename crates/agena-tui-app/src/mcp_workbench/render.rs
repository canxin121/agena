use agena_tui::help::{HelpOverlay, HelpOverlayKind};
use agena_tui_components::{
    HelpDialogEntry, HelpDialogSection, ListPanelSpec, ScrollState, SurfaceMode, TextPanelSpec,
    TwoLineListItemSpec, WorkbenchFrameSpec, build_two_line_list_item, panel_highlight_style,
    render_confirm_dialog, render_list_panel, render_overlay_masked_line_input_dialog,
    render_text_panel, render_workbench_editor_dialog, render_workbench_frame, theme,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{ListItem, Paragraph, Wrap},
};
use serde_json::{Value, json};

use super::state::{McpDraft, McpImport, McpPane, PENDING_RELOAD_HINT, scope_label};
use super::{App, McpWorkbench};

impl App {
    pub(crate) fn render_mcp_workbench(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &McpWorkbench,
    ) {
        draw(frame, area, dialog);
    }

    pub(crate) fn help_for_mcp_workbench(&self, dialog: &McpWorkbench) -> HelpOverlay {
        let section = |title: &str, rows: &[(&str, &str)]| HelpDialogSection {
            title: title.to_owned(),
            entries: rows
                .iter()
                .map(|(keys, description)| HelpDialogEntry {
                    keys: (*keys).to_owned(),
                    description: (*description).to_owned(),
                })
                .collect(),
        };
        let mut sections = vec![
            section(
                "Connections",
                &[
                    ("↑ / ↓, Home / End", "Select a connection or capability"),
                    ("Tab", "Switch connections and capability panes"),
                    ("/", "Search connections"),
                    ("G", "Filter all, workspace, or global copies"),
                    (
                        "S",
                        "Choose the target scope for new connections and imports",
                    ),
                    ("N / E", "Create or edit a connection"),
                    ("Space", "Enable or disable the selected connection"),
                    ("D / Delete", "Remove a connection with confirmation"),
                    ("I", "Import mcpServers or native servers JSON"),
                    ("B", "Enable or disable the MCP bridge in the target scope"),
                    ("R / Ctrl+R", "Refresh live connection status"),
                    (
                        "C",
                        "Reconnect the selected service once saved settings are active",
                    ),
                    ("T", "Test its saved connection without saving changes"),
                ],
            ),
            section(
                "Capabilities and credentials",
                &[
                    (
                        "M",
                        "Cycle tools, resources, resource templates, and prompts",
                    ),
                    ("< / >", "Read previous or next capability page"),
                    ("Enter", "Inspect a tool, read a resource, or get a prompt"),
                    ("PgUp / PgDn", "Scroll the detail preview"),
                    (
                        "A",
                        "Start OAuth authorization; paste the callback during an active flow",
                    ),
                    ("V", "Set a stored bearer credential using masked input"),
                    (
                        "X",
                        "Remove the selected stored credential or cancel pending authorization",
                    ),
                    (
                        "Esc",
                        "Return to settings; pending authorization is cancelled explicitly",
                    ),
                ],
            ),
        ];
        if let Some(draft) = &dialog.draft {
            sections.insert(
                0,
                section(
                    "Connection editor",
                    &[
                        ("↑ / ↓", "Select a field"),
                        ("Enter / E", "Edit or switch the selected field"),
                        (
                            "Delete",
                            "Clear a bearer token or optional OAuth client field",
                        ),
                        ("Ctrl+S / S", "Validate and save the complete connection"),
                        (
                            "O",
                            "Edit native configuration JSON with redacted secret placeholders",
                        ),
                        ("Esc", "Return, confirming unsaved changes"),
                    ],
                ),
            );
            sections.insert(
                1,
                section(
                    "Selected field",
                    &[(draft.field().label(), draft.field().hint())],
                ),
            );
        }
        if dialog.import.is_some() {
            sections.insert(
                0,
                section(
                    "Import preview",
                    &[
                        ("Space", "Select a valid candidate"),
                        ("A", "Select or clear all valid candidates"),
                        ("O", "Allow replacement of selected existing records"),
                        ("Enter", "Confirm the import"),
                        ("Esc", "Discard this preview"),
                    ],
                ),
            );
        }
        HelpOverlay{kind:HelpOverlayKind::ContextHelp,modal_title:self.i18n.text("context-help-title"),eyebrow:self.i18n.text("context-help-eyebrow"),footer:self.i18n.text("context-help-footer"),context:"MCP connections".to_owned(),
            summary:"Manage external MCP services connected to the running Agena server.".to_owned(),sections,
            tips:vec![
                format!("The server workspace is {}. Selecting another chat directory does not change this connection manager.",dialog.catalog.as_ref().map(|c|c.workspace_root.as_str()).unwrap_or("shown after loading")),
                "Existing secret values never return to the editor. Cancelling keeps them; replacement or removal is explicit.".to_owned(),
                "Stdio commands and working directories run on the Agena server, which may be a different computer from this terminal.".to_owned(),
            ],scroll:ScrollState::default(),max_scroll:0}
    }
}

fn draw(frame: &mut Frame, area: Rect, dialog: &McpWorkbench) {
    let pending_reload = dialog
        .selected_connection()
        .is_some_and(|connection| connection.state == "pending_reload");
    let footer = if dialog.authorization.is_some() {
        "C copy URL · A paste callback · X cancel · PgUp/PgDn scroll · F1 help"
    } else if dialog.import.is_some() {
        "↑/↓ inspect · Space select · A all · O replace · Enter import · Esc back"
    } else if dialog.map_editor.is_some() {
        "↑/↓ select · N new key · Enter replace value · D remove · Esc back"
    } else if dialog.draft.is_some() {
        "↑/↓ field · Enter edit · Ctrl+S save · O JSON · F1 help · Esc back"
    } else if pending_reload {
        "R refresh · T test · E edit · Diagnostics to reload · F1 help · Esc back"
    } else if area.width < 100 {
        "↑/↓ select · Tab pane · N new · E edit · C reconnect · T test · F1 help · Esc back"
    } else {
        "↑/↓ select · Tab pane · / search · N new · E edit · I import · Space enable · D remove\nC reconnect · T test · M capability kind · < > page · A authorize · V token · X clear credential · F1 help · Esc back"
    };
    let summary = if dialog.busy {
        "Working…".to_owned()
    } else if dialog.loading {
        "Loading…".to_owned()
    } else {
        dialog
            .catalog
            .as_ref()
            .map(|catalog| format!("Server workspace: {}", catalog.workspace_root))
            .unwrap_or_else(|| "External MCP services".to_owned())
    };
    let surface = render_workbench_frame(
        frame,
        area,
        SurfaceMode::Route,
        &WorkbenchFrameSpec::new("MCP connections".into(), footer.into(), 144, 38)
            .summary(Some(summary.into())),
    );
    let mut body = surface.body;
    if let Some(message) = dialog.error.as_ref().or(dialog.notice.as_ref()) {
        let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(body);
        frame.render_widget(
            Paragraph::new(clean(message))
                .style(Style::default().fg(if dialog.error.is_some() {
                    theme::danger_color()
                } else {
                    theme::muted_color()
                }))
                .wrap(Wrap { trim: false }),
            rows[0],
        );
        body = rows[1];
    }
    if let Some(flow) = &dialog.authorization {
        text_panel(
            frame,
            body,
            "OAuth authorization",
            Text::from(clean(&format!(
                "Open this URL in your browser:\n\n{}\n\nExpires: {}\n\nC copies the URL. After sign-in, press A and paste the complete callback URL. X cancels this pending flow.",
                flow.authorization_url, flow.expires_at
            ))),
            dialog.scroll,
        );
    } else if let Some(import) = &dialog.import {
        render_import(frame, body, import);
    } else if let Some(draft) = &dialog.draft {
        if dialog.map_editor.is_some() {
            render_map(frame, body, dialog);
        } else {
            render_form(frame, body, dialog, draft);
        }
    } else {
        let compact = body.width < 92;
        dialog.compact.set(compact);
        if compact {
            match dialog.pane {
                McpPane::Connections => render_connections(frame, body, dialog),
                McpPane::Capabilities => render_details(frame, body, dialog),
            }
        } else {
            let columns =
                Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)])
                    .split(body);
            render_connections(frame, columns[0], dialog);
            render_details(frame, columns[1], dialog);
        }
    }
    if let Some(editor) = &dialog.editor {
        let prompt = dialog
            .error
            .as_ref()
            .map(|error| format!("{error}\n{}", editor.prompt))
            .unwrap_or_else(|| editor.prompt.clone());
        let footer = if dialog.busy {
            "Working…".to_owned()
        } else {
            editor.footer.clone()
        };
        if editor.action.secret(editor.input.text()) {
            render_overlay_masked_line_input_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                clean(&editor.title).into(),
                clean(&prompt).into(),
                footer.into(),
                &editor.input,
            );
        } else {
            render_workbench_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                clean(&editor.title).into(),
                clean(&prompt).into(),
                footer.into(),
                editor.multiline,
                &editor.input,
            );
        }
    }
    if let Some(confirm) = &dialog.confirm {
        render_confirm_dialog(frame, area, confirm, clean);
    }
}

fn render_connections(frame: &mut Frame, area: Rect, dialog: &McpWorkbench) {
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    let search = if dialog.query.is_empty() {
        "/ Search connections".to_owned()
    } else {
        format!("Search: {}", clean(&dialog.query))
    };
    let scope = dialog
        .scope_filter
        .as_ref()
        .map(scope_label)
        .unwrap_or("All scopes");
    frame.render_widget(
        Paragraph::new(format!(
            "{search}\n{scope} · new: {}",
            scope_label(&dialog.target_scope)
        ))
        .style(theme::muted_style()),
        rows[0],
    );
    let visible = dialog.visible();
    if visible.is_empty() {
        let message = if dialog.loading {
            "Loading configured connections…"
        } else if !dialog.query.is_empty() || dialog.scope_filter.is_some() {
            "No connections match this filter. Press / or G to change it."
        } else {
            "No external MCP connections configured. Press N to create one, or I to import mcpServers JSON."
        };
        text_panel(frame, rows[1], "External services", Text::from(message), 0);
        return;
    }
    let items: Vec<_> = visible
        .iter()
        .map(|connection| {
            let style = state_style(&connection.state);
            build_two_line_list_item(TwoLineListItemSpec {
                label: clean(&connection.name).into(),
                value: Some(
                    format!(
                        "{} · {}",
                        connection.transport,
                        connection.state.replace('_', " ")
                    )
                    .into(),
                ),
                detail: Some(
                    format!(
                        "{} · {} tools",
                        scope_label(&connection.scope),
                        connection.tool_count
                    )
                    .into(),
                ),
                label_style: Style::default().add_modifier(Modifier::BOLD),
                value_style: style,
                detail_style: theme::muted_style(),
                separator: "  ".into(),
            })
        })
        .collect();
    render_list_panel(
        frame,
        rows[1],
        &ListPanelSpec::new(
            Some(format!("External services · {}", visible.len()).into()),
            &items,
            Some(dialog.selected),
            panel_highlight_style(dialog.pane == McpPane::Connections),
            "› ".into(),
        ),
    );
}

fn render_details(frame: &mut Frame, area: Rect, dialog: &McpWorkbench) {
    let Some(summary) = dialog.selected_connection() else {
        let body=dialog.catalog.as_ref().map(|c|format!("Select an external MCP service to inspect its configuration and capabilities.\n\nRunning server workspace\n{}\n\nGlobal configuration\n{}\n\nWorkspace configuration\n{}\n\nBridge: {}",c.workspace_root,c.global_path,c.workspace_path,if c.bridge_enabled{"Enabled"}else{"Disabled · B enables the bridge in the target scope"})).unwrap_or_else(||"Loading connection configuration…".to_owned());
        text_panel(
            frame,
            area,
            "Connection details",
            Text::from(clean(&body)),
            dialog.scroll,
        );
        return;
    };
    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Min(3),
    ])
    .split(area);
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "{} · {} · {}",
                summary.state.replace('_', " "),
                summary.transport,
                if summary.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
            state_style(&summary.state),
        )),
        Line::from(scope_label(&summary.scope)),
    ];
    if let Some(error) = &summary.error {
        lines.push(Line::from(Span::styled(
            clean(error),
            Style::default().fg(theme::danger_color()),
        )));
    }
    text_panel(frame, rows[0], &clean(&summary.name), Text::from(lines), 0);
    // A refreshed catalog can arrive before detail/capability requests finish.
    // Only render live data when both records describe the current connection.
    let live = summary.state == "connected"
        && dialog.detail.as_ref().is_some_and(|detail| {
            detail.summary.state == "connected"
                && detail.summary.id == summary.id
                && detail.summary.revision == summary.revision
        });
    let title = if summary.state == "pending_reload" {
        "Capabilities · pending reload".to_owned()
    } else {
        format!("{} · M changes kind · < > page", dialog.kind.title())
    };
    let capabilities = dialog.capabilities.as_ref().filter(|_| live);
    let items: Vec<_> = capabilities
        .map(|p| {
            p.items
                .iter()
                .map(|item| ListItem::new(clean(&capability_name(item))))
                .collect()
        })
        .unwrap_or_default();
    if dialog.capability_loading || dialog.detail_loading {
        text_panel(
            frame,
            rows[1],
            &title,
            Text::from("Loading capabilities…"),
            0,
        );
    } else if items.is_empty() {
        let message = if summary.state == "pending_reload" {
            PENDING_RELOAD_HINT
        } else if summary.state == "connected" {
            "No entries in this capability list."
        } else {
            "Connect or enable this service to discover capabilities."
        };
        text_panel(frame, rows[1], &title, Text::from(message), 0);
    } else {
        render_list_panel(
            frame,
            rows[1],
            &ListPanelSpec::new(
                Some(title.into()),
                &items,
                Some(dialog.capability_index),
                panel_highlight_style(dialog.pane == McpPane::Capabilities),
                "› ".into(),
            ),
        );
    }
    let preview = dialog
        .preview
        .as_ref()
        .filter(|(title, _)| live || title == "Connection test");
    let (title, body) = if let Some((title, value)) = preview {
        (title.clone(), pretty(value))
    } else if let Some(item) = capabilities.and_then(|p| p.items.get(dialog.capability_index)) {
        (
            "Capability details · Enter inspect".to_owned(),
            pretty(item),
        )
    } else if let Some(detail) = &dialog.detail {
        let mut value = json!({"configuration":detail.config,"server_workspace":dialog.catalog.as_ref().map(|c|&c.workspace_root)});
        if live {
            value["protocol_version"] = json!(detail.protocol_version);
            value["server_info"] = json!(detail.server_info);
            value["capabilities"] = json!(detail.capabilities);
            value["oauth_health"] = json!(detail.oauth_health);
        }
        (
            "Connection information · PgUp/PgDn scroll".to_owned(),
            pretty(&value),
        )
    } else {
        (
            "Connection information".to_owned(),
            "Select a connection to inspect its details.".to_owned(),
        )
    };
    text_panel(
        frame,
        rows[2],
        &title,
        Text::from(clean(&body)),
        dialog.scroll,
    );
}

fn render_form(frame: &mut Frame, area: Rect, dialog: &McpWorkbench, draft: &McpDraft) {
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    let workspace = dialog
        .catalog
        .as_ref()
        .map(|c| c.workspace_root.as_str())
        .unwrap_or("Loading server workspace…");
    frame.render_widget(
        Paragraph::new(clean(&format!(
            "{} · {}\nServer workspace: {workspace}",
            if draft.editing.is_some() {
                "Edit connection"
            } else {
                "New connection"
            },
            scope_label(&draft.scope)
        )))
        .style(theme::muted_style()),
        rows[0],
    );
    let fields = draft.fields();
    let items: Vec<_> = fields
        .iter()
        .map(|field| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{}  ", field.label()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(clean(&draft.field_value(*field)), theme::muted_style()),
            ]))
        })
        .collect();
    let title = if draft.dirty {
        "Configuration · unsaved changes"
    } else {
        "Configuration"
    };
    if rows[1].width >= 88 {
        let columns = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(rows[1]);
        render_list_panel(
            frame,
            columns[0],
            &ListPanelSpec::new(
                Some(title.into()),
                &items,
                Some(draft.selected),
                panel_highlight_style(true),
                "› ".into(),
            ),
        );
        text_panel(
            frame,
            columns[1],
            draft.field().label(),
            Text::from(format!(
                "{}\n\nEnter edits or switches the selected field. Ctrl+S saves. O opens native JSON for advanced settings and repairs.",
                draft.field().hint()
            )),
            0,
        );
    } else {
        render_list_panel(
            frame,
            rows[1],
            &ListPanelSpec::new(
                Some(title.into()),
                &items,
                Some(draft.selected),
                panel_highlight_style(true),
                "› ".into(),
            ),
        );
    }
}

fn render_map(frame: &mut Frame, area: Rect, dialog: &McpWorkbench) {
    let Some(map) = &dialog.map_editor else {
        return;
    };
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    frame.render_widget(Paragraph::new("Values are hidden. Enter replaces a value; D removes it.\nChanges apply only after the connection is saved.").style(theme::muted_style()).wrap(Wrap{trim:false}),rows[0]);
    let mut items: Vec<_> = dialog
        .map_keys()
        .into_iter()
        .map(|key| {
            ListItem::new(Line::from(vec![
                Span::raw(clean(&key)),
                Span::styled("  Configured · hidden", theme::muted_style()),
            ]))
        })
        .collect();
    items.push(ListItem::new(Span::styled(
        "+ Add a key",
        Style::default().fg(theme::success_color()),
    )));
    render_list_panel(
        frame,
        rows[1],
        &ListPanelSpec::new(
            Some(map.field.label().into()),
            &items,
            Some(map.selected.min(items.len().saturating_sub(1))),
            panel_highlight_style(true),
            "› ".into(),
        ),
    );
}

fn render_import(frame: &mut Frame, area: Rect, import: &McpImport) {
    let columns = if area.width >= 88 {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area)
    } else {
        Layout::vertical([Constraint::Percentage(48), Constraint::Percentage(52)]).split(area)
    };
    let items: Vec<_> = import
        .preview
        .candidates
        .iter()
        .map(|candidate| {
            let status = if !candidate.valid {
                "Invalid"
            } else if candidate.conflict {
                "Exists"
            } else {
                "Ready"
            };
            build_two_line_list_item(TwoLineListItemSpec {
                label: format!(
                    "{} {}",
                    if import.selected.contains(&candidate.name) {
                        "[x]"
                    } else {
                        "[ ]"
                    },
                    clean(&candidate.name)
                )
                .into(),
                value: Some(status.into()),
                detail: Some(
                    candidate
                        .config
                        .get("transport")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown transport")
                        .to_owned()
                        .into(),
                ),
                label_style: Style::default().add_modifier(Modifier::BOLD),
                value_style: if candidate.valid {
                    theme::muted_style()
                } else {
                    Style::default().fg(theme::danger_color())
                },
                detail_style: theme::muted_style(),
                separator: "  ".into(),
            })
        })
        .collect();
    render_list_panel(
        frame,
        columns[0],
        &ListPanelSpec::new(
            Some(format!("Candidates · {} selected", import.selected.len()).into()),
            &items,
            (!items.is_empty()).then_some(import.cursor),
            panel_highlight_style(true),
            "› ".into(),
        ),
    );
    let mut lines = vec![
        format!("Scope: {}", scope_label(&import.scope)),
        format!(
            "Replace existing: {} · O toggles",
            if import.replace { "Allowed" } else { "Off" }
        ),
        String::new(),
    ];
    lines.extend(import.preview.errors.iter().cloned());
    if let Some(candidate) = import.preview.candidates.get(import.cursor) {
        lines.extend(candidate.errors.iter().cloned());
        lines.push(pretty(&candidate.config));
    }
    text_panel(
        frame,
        columns[1],
        "Import preview · credentials redacted",
        Text::from(clean(&lines.join("\n"))),
        0,
    );
}

fn text_panel(frame: &mut Frame, area: Rect, title: &str, body: Text<'_>, scroll: u16) {
    render_text_panel(
        frame,
        area,
        &TextPanelSpec {
            title: Some(title.to_owned().into()),
            body: &body,
            wrap: true,
            scroll: Some((scroll, 0)),
            alignment: None,
        },
    );
}
fn capability_name(item: &Value) -> String {
    item.get("name")
        .or_else(|| item.get("title"))
        .or_else(|| item.get("uri"))
        .or_else(|| item.get("uriTemplate"))
        .and_then(Value::as_str)
        .unwrap_or("Unnamed capability")
        .to_owned()
}
fn state_style(state: &str) -> Style {
    Style::default().fg(match state {
        "connected" => theme::success_color(),
        "invalid" => theme::danger_color(),
        "authentication_required" | "disconnected" | "pending_reload" => theme::warning_color(),
        _ => theme::muted_color(),
    })
}
fn pretty(value: &Value) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let mut result: String = text.chars().take(96 * 1024).collect();
    if result.len() < text.len() {
        result.push_str("\n\nPreview truncated after 96 Ki characters.");
    }
    result
}
fn clean(text: &str) -> String {
    crate::sanitize_terminal_text(text)
}

#[cfg(test)]
mod tests {
    use super::super::state::{EditAction, FormField};
    use super::*;
    use agena_api::mcp_connections::{
        McpCapabilityPage, McpConnectionCatalog, McpConnectionDetail,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn populated() -> McpWorkbench {
        let mut state = McpWorkbench::new();
        let detail:McpConnectionDetail=serde_json::from_value(json!({
            "id":"connection-one","name":"long-external-research-service-with-many-tools","scope":"workspace","transport":"http","enabled":true,"effective":true,"state":"connected","tool_count":1,"revision":"version-one","error":null,
            "config":{"transport":"http","endpoint":{"url":"https://example.test/mcp","headers":{}},"auth":{"kind":"bearer","token":"[REDACTED]"}},"secret_paths":["/auth/token"],"protocol_version":"2025-11-25","server_info":{"name":"Test service","version":"1"},"capabilities":{"tools":{}},"oauth_health":null
        })).unwrap();
        state.catalog = Some(McpConnectionCatalog {
            connections: vec![detail.summary.clone()],
            workspace_root: "/server/project-with-a-long-directory-name".to_owned(),
            global_path: "/server/global.json".to_owned(),
            workspace_path: "/server/project/config.json".to_owned(),
            bridge_enabled: true,
        });
        state.detail = Some(detail);
        state.pane = McpPane::Capabilities;
        state.capabilities = Some(McpCapabilityPage {
            kind: "tools".to_owned(),
            items: vec![
                json!({"name":"read_inventory","description":"Read the selected inventory without modifying it.","inputSchema":{"type":"object"}}),
            ],
            next_cursor: None,
        });
        state
    }
    fn rendered(state: &McpWorkbench, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                draw(frame, area, state);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn connection_state_and_capability_remain_visible_at_wide_and_narrow_sizes() {
        let state = populated();
        for (width, height) in [(140, 38), (80, 24), (45, 15)] {
            let text = rendered(&state, width, height);
            for expected in ["MCP connections", "connected", "read_inventory"] {
                assert!(
                    text.contains(expected),
                    "{width}x{height} must show {expected}:\n{text}"
                );
            }
        }
    }
    #[test]
    fn connection_editor_and_masked_overlay_never_render_token_values() {
        let mut state = populated();
        let mut draft = McpDraft::from_detail(state.detail.as_ref().unwrap());
        draft.set("/auth/token", json!("never-render-this-token"));
        draft.selected = draft
            .fields()
            .iter()
            .position(|field| *field == FormField::Token)
            .unwrap();
        state.draft = Some(draft);
        for (width, height) in [(140, 38), (80, 24), (45, 15)] {
            let text = rendered(&state, width, height);
            assert!(!text.contains("never-render-this-token"));
            assert!(text.contains("hidden"), "{text}");
        }
        state.open_editor(
            "Bearer token",
            "Replace the token",
            false,
            "never-render-this-token".to_owned(),
            EditAction::Field(FormField::Token),
        );
        let text = rendered(&state, 80, 24);
        assert!(!text.contains("never-render-this-token"));
        assert!(text.contains('•'));
    }
    #[test]
    fn oauth_client_secrets_and_sensitive_endpoints_are_hidden_in_forms_and_dialogs() {
        let mut state = populated();
        let mut draft = McpDraft::from_detail(state.detail.as_ref().unwrap());
        draft.set("/auth",json!({"kind":"o_auth","client_id":"client-id","client_secret":"never-render-client-secret"}));
        draft.set(
            "/endpoint/url",
            json!("https://example.test/mcp?key=never-render-url-secret"),
        );
        for field in [FormField::ClientSecret, FormField::Url] {
            draft.selected = draft
                .fields()
                .iter()
                .position(|item| *item == field)
                .unwrap();
            state.draft = Some(draft.clone());
            for (width, height) in [(140, 38), (80, 24), (45, 15)] {
                let text = rendered(&state, width, height);
                assert!(!text.contains("never-render-client-secret"));
                assert!(!text.contains("never-render-url-secret"));
                assert!(text.contains("hidden"), "{text}");
            }
            let text = draft
                .config
                .pointer(field.path())
                .and_then(Value::as_str)
                .unwrap()
                .to_owned();
            state.open_editor(
                field.label(),
                field.hint(),
                false,
                text,
                EditAction::Field(field),
            );
            let text = rendered(&state, 80, 24);
            assert!(!text.contains("never-render-client-secret"));
            assert!(!text.contains("never-render-url-secret"));
            assert!(text.contains('•'));
            state.editor = None;
        }
    }
    #[test]
    fn pending_reload_hides_stale_live_capabilities_and_keeps_test_available() {
        let mut state = populated();
        state.catalog.as_mut().unwrap().connections[0].state = "pending_reload".to_owned();
        // Keep the previous connected detail and capability result to model
        // a catalog refresh racing their asynchronous responses.
        for (width, height) in [(140, 38), (80, 24), (45, 15)] {
            let text = rendered(&state, width, height);
            assert!(text.contains("pending reload"), "{text}");
            assert!(text.contains("R refresh"), "{text}");
            assert!(text.contains("T test"), "{text}");
            assert!(!text.contains("C reconnect"));
            assert!(!text.contains("read_inventory"));
            assert!(!text.contains("2025-11-25"));
            assert!(!text.contains("Test service"));
        }
        state.preview = Some((
            "Connection test".to_owned(),
            json!({"ok":true,"tool_count":3}),
        ));
        assert!(rendered(&state, 140, 38).contains("Connection test"));
    }
}
