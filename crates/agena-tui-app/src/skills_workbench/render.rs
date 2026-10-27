use agena_tui::help::{HelpOverlay, HelpOverlayKind};
use agena_tui_components::{
    HelpDialogEntry, HelpDialogSection, ListPanelSpec, ScrollState, SurfaceMode, TextPanelSpec,
    TwoLineListItemSpec, WorkbenchFrameSpec, build_two_line_list_item, panel_highlight_style,
    render_confirm_dialog, render_list_panel, render_text_panel, render_workbench_editor_dialog,
    render_workbench_frame, theme,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{ListItem, Paragraph, Wrap},
};

use super::state::{SkillInstallation, scope_label, status_label};
use super::{App, SkillPane, SkillsWorkbench};

impl App {
    pub(crate) fn render_skills_workbench(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SkillsWorkbench,
    ) {
        draw_skills_workbench(frame, area, dialog);
    }

    pub(crate) fn help_for_skills_workbench(&self, dialog: &SkillsWorkbench) -> HelpOverlay {
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
        HelpOverlay {
            kind: HelpOverlayKind::ContextHelp,
            modal_title: self.i18n.text("context-help-title"),
            eyebrow: self.i18n.text("context-help-eyebrow"),
            footer: self.i18n.text("context-help-footer"),
            context: "Skills".to_owned(),
            summary: "Manage standard SKILL.md packages and their supporting files.".to_owned(),
            sections: if dialog.installation.is_some() {
                vec![section("Installation preview", &[
                    ("↑ / ↓", "Inspect a candidate and its validation results"),
                    ("Space", "Select or deselect a valid skill"),
                    ("A", "Select or deselect all valid skills"),
                    ("O", "Allow or disallow replacing existing installations"),
                    ("Enter", "Review and confirm the selected installation"),
                    ("R", "Refresh the source and preview"),
                    ("Esc", "Return to the skill library"),
                ])]
            } else {
                vec![
                    section("Library", &[
                        ("↑ / ↓, Home / End", "Select a skill or resource"),
                        ("Tab", "Switch between the library and resource list"),
                        ("/", "Search names, descriptions, and paths"),
                        ("G", "Filter all, workspace, or global skills"),
                        ("S", "Choose the workspace or global target for new skills"),
                        ("R / Ctrl+R", "Refresh the library"),
                        ("PgUp / PgDn", "Scroll the resource preview"),
                    ]),
                    section("Manage", &[
                        ("N", "Create a skill with standard YAML frontmatter"),
                        ("E", "Edit SKILL.md or the selected text resource"),
                        ("F", "Add a text resource to the selected skill"),
                        ("Space", "Enable or disable the selected skill"),
                        ("D / Delete", "Remove the selected skill or resource, with confirmation"),
                        ("I", "Install from GitHub, an archive, or a folder"),
                        ("C", "Create a managed copy of the selected skill"),
                        ("U", "Preview an update from the recorded installation source"),
                        ("Esc", "Return to settings"),
                    ]),
                ]
            },
            tips: vec![
                "Installation paths refer to the server. Prefix a local folder or archive with @ to upload from this computer.".to_owned(),
                "Workspace skills take precedence over same-name global skills. Disabled and invalid skills remain visible for management.".to_owned(),
                "Ctrl+S saves multiline edits. Esc asks before discarding unsaved changes.".to_owned(),
            ],
            scroll: ScrollState::default(),
            max_scroll: 0,
        }
    }
}

fn draw_skills_workbench(frame: &mut Frame, area: Rect, dialog: &SkillsWorkbench) {
    let footer = if dialog.installation.is_some() {
        "↑/↓ inspect · Space select · A all · O replace · Enter install · R refresh · Esc back"
    } else if area.width < 100 {
        "↑/↓ select · Tab pane · / search · E edit · N new · I install · F1 help · Esc back"
    } else {
        "↑/↓ select · Tab pane · / search · E edit · N new · I install · Space enable · D delete\nF new file · C copy · U update · G filter · S target · R refresh · PgUp/PgDn scroll · F1 help · Esc back"
    };
    let summary = if dialog.busy {
        "Working…".to_owned()
    } else if dialog.loading {
        "Loading…".to_owned()
    } else {
        format!(
            "{} skills · new skills: {}",
            dialog.catalog.skills.len(),
            scope_label(dialog.target_scope)
        )
    };
    let surface = render_workbench_frame(
        frame,
        area,
        SurfaceMode::Route,
        &WorkbenchFrameSpec::new("Skills".into(), footer.into(), 144, 38)
            .summary(Some(summary.into())),
    );
    let body = if let Some(error) = &dialog.error {
        let rows =
            Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(surface.body);
        frame.render_widget(
            Paragraph::new(clean(error))
                .style(Style::default().fg(theme::danger_color()))
                .wrap(Wrap { trim: false }),
            rows[0],
        );
        rows[1]
    } else {
        surface.body
    };

    if let Some(installation) = &dialog.installation {
        render_installation(frame, body, installation);
    } else {
        let compact = body.width < 92;
        dialog.compact_layout.set(compact);
        if compact {
            match dialog.pane {
                SkillPane::Library => render_library(frame, body, dialog),
                SkillPane::Files => render_skill_detail(frame, body, dialog),
            }
        } else {
            let columns =
                Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)])
                    .split(body);
            render_library(frame, columns[0], dialog);
            render_skill_detail(frame, columns[1], dialog);
        }
    }
    if let Some(editor) = &dialog.editor {
        let prompt = if let Some(error) = &dialog.error {
            format!("Error: {error}\n\n{}", editor.prompt)
        } else {
            editor.prompt.clone()
        };
        render_workbench_editor_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            clean(&editor.title).into(),
            clean(&prompt).into(),
            if dialog.busy {
                "Working…".into()
            } else {
                editor.footer.clone().into()
            },
            editor.multiline,
            &editor.input,
        );
    }
    if let Some(confirm) = &dialog.confirm {
        render_confirm_dialog(frame, area, confirm, clean);
    }
}

fn render_library(frame: &mut Frame, area: Rect, dialog: &SkillsWorkbench) {
    let filter = dialog.scope_filter.map(scope_label).unwrap_or("All scopes");
    let visible = dialog.visible_skills();
    let title = format!("Library · {filter} · {}", visible.len());
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let query = if dialog.query.is_empty() {
        "Press / to search".to_owned()
    } else {
        format!("Search: {}", clean(&dialog.query))
    };
    frame.render_widget(Paragraph::new(query).style(theme::muted_style()), rows[0]);
    if visible.is_empty() {
        let mut lines = vec![if dialog.loading {
            "Loading skill directories…".to_owned()
        } else if !dialog.query.is_empty() || dialog.scope_filter.is_some() {
            "No skills match this filter. Press / to change the search or G to change scopes."
                .to_owned()
        } else {
            "No skills installed. Press N to create one or I to install from GitHub, an archive, or a folder.".to_owned()
        }];
        for diagnostic in &dialog.catalog.diagnostics {
            lines.push(format!("{}: {}", diagnostic.path, diagnostic.message));
        }
        text_panel(
            frame,
            rows[1],
            &title,
            Text::from(clean(&lines.join("\n\n"))),
            0,
        );
        return;
    }
    let items: Vec<ListItem<'_>> = visible
        .iter()
        .map(|skill| {
            let color = if !skill.valid {
                theme::danger_color()
            } else if !skill.enabled || skill.shadowed {
                theme::muted_color()
            } else {
                theme::success_color()
            };
            build_two_line_list_item(TwoLineListItemSpec {
                label: clean(&skill.name).into(),
                value: Some(
                    format!("{} · {}", scope_label(skill.scope), status_label(skill)).into(),
                ),
                detail: Some(clean(&skill.description).into()),
                label_style: Style::default().add_modifier(Modifier::BOLD),
                value_style: Style::default().fg(color),
                detail_style: theme::muted_style(),
                separator: "  ".into(),
            })
        })
        .collect();
    render_list_panel(
        frame,
        rows[1],
        &ListPanelSpec::new(
            Some(title.into()),
            &items,
            Some(dialog.selected),
            panel_highlight_style(dialog.pane == SkillPane::Library),
            "› ".into(),
        ),
    );
}

fn render_skill_detail(frame: &mut Frame, area: Rect, dialog: &SkillsWorkbench) {
    let Some(summary) = dialog.selected_skill() else {
        let roots = dialog
            .catalog
            .roots
            .iter()
            .map(|root| format!("{}: {}", scope_label(root.scope), root.path))
            .collect::<Vec<_>>();
        text_panel(
            frame,
            area,
            "Skill details",
            Text::from(clean(&format!(
                "Select a skill to inspect its metadata and files.\n\nSkill directories\n{}",
                roots.join("\n")
            ))),
            0,
        );
        return;
    };
    let mut metadata = vec![
        Line::from(format!(
            "{} · {} · {}",
            scope_label(summary.scope),
            status_label(summary),
            if summary.editable {
                "Managed"
            } else {
                "Read-only"
            }
        )),
        Line::from(Span::styled(
            clean(&summary.description),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(clean(&summary.path)),
    ];
    if let Some(source) = &summary.source {
        metadata.push(Line::from(Span::styled(
            clean(&format!("Source: {} · {}", source.kind, source.location)),
            theme::muted_style(),
        )));
    }
    for diagnostic in &summary.diagnostics {
        metadata.push(Line::from(Span::styled(
            clean(diagnostic),
            Style::default().fg(theme::warning_color()),
        )));
    }
    let paths = dialog.file_paths();
    let header_height = (metadata.len() as u16 + 2).clamp(5, 9);
    let file_height = (paths.len() as u16 + 2).clamp(3, 7);
    let rows = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Length(file_height),
        Constraint::Min(3),
    ])
    .split(area);
    text_panel(
        frame,
        rows[0],
        &clean(&summary.name),
        Text::from(metadata),
        0,
    );
    if dialog.detail_loading {
        text_panel(
            frame,
            rows[1],
            "Files",
            Text::from("Loading skill contents…"),
            0,
        );
    } else {
        let items: Vec<ListItem<'_>> = paths
            .iter()
            .map(|path| {
                let size = dialog
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.files.iter().find(|file| file.path == *path))
                    .map(|file| format!("  {}", format_size(file.size)))
                    .unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::raw(clean(path)),
                    Span::styled(size, theme::muted_style()),
                ]))
            })
            .collect();
        render_list_panel(
            frame,
            rows[1],
            &ListPanelSpec::new(
                Some(format!("Files · {}", paths.len()).into()),
                &items,
                (!items.is_empty()).then_some(dialog.file_index),
                panel_highlight_style(dialog.pane == SkillPane::Files),
                "› ".into(),
            ),
        );
    }
    let title = dialog
        .resource
        .as_ref()
        .map(|resource| format!("{} · PgUp/PgDn scroll", clean(&resource.path)))
        .unwrap_or_else(|| "Resource preview".to_owned());
    let content = if dialog.resource_loading {
        "Loading resource…".to_owned()
    } else if let Some(resource) = &dialog.resource {
        clean(&resource.content)
    } else {
        "Select a text resource and press Enter to read it.".to_owned()
    };
    text_panel(frame, rows[2], &title, Text::from(content), dialog.scroll);
}

fn render_installation(frame: &mut Frame, area: Rect, installation: &SkillInstallation) {
    let columns = if area.width >= 88 {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area)
    } else {
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area)
    };
    let items: Vec<ListItem<'_>> = installation
        .preview
        .candidates
        .iter()
        .map(|candidate| {
            let marker = if installation.selected.contains(&candidate.path) {
                "[x]"
            } else {
                "[ ]"
            };
            let status = if !candidate.valid {
                "Invalid"
            } else if candidate.conflict {
                "Already installed"
            } else {
                "Ready"
            };
            build_two_line_list_item(TwoLineListItemSpec {
                label: format!("{marker} {}", clean(&candidate.name)).into(),
                value: Some(status.into()),
                detail: Some(clean(&candidate.path).into()),
                label_style: Style::default().add_modifier(Modifier::BOLD),
                value_style: Style::default().fg(if !candidate.valid {
                    theme::danger_color()
                } else if candidate.conflict {
                    theme::warning_color()
                } else {
                    theme::success_color()
                }),
                detail_style: theme::muted_style(),
                separator: "  ".into(),
            })
        })
        .collect();
    render_list_panel(
        frame,
        columns[0],
        &ListPanelSpec::new(
            Some(format!("Candidates · {} selected", installation.selected.len()).into()),
            &items,
            (!items.is_empty()).then_some(installation.cursor),
            panel_highlight_style(true),
            "› ".into(),
        ),
    );
    let mut lines = vec![
        format!("Install into: {}", scope_label(installation.scope)),
        format!(
            "Replace existing: {} (O toggles)",
            if installation.replace {
                "Allowed"
            } else {
                "Off"
            }
        ),
        String::new(),
    ];
    if let Some(candidate) = installation.preview.candidates.get(installation.cursor) {
        lines.push(candidate.description.clone());
        lines.push(String::new());
        lines.push(format!("Directory: {}", candidate.path));
        if candidate.diagnostics.is_empty() {
            lines.push("Standard validation passed.".to_owned());
        } else {
            lines.extend(candidate.diagnostics.clone());
        }
    }
    text_panel(
        frame,
        columns[1],
        "Installation preview",
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

fn clean(value: &str) -> String {
    crate::sanitize_terminal_text(value)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn long_skill_details_keep_availability_and_resource_preview_visible() {
        use agena_api::skills::{
            SkillDetail, SkillFile, SkillMetadata, SkillResource, SkillScope, SkillSummary,
        };

        let name = "summarize-technical-research-and-verify-references-with-evidence";
        let description =
            "Review supplied sources, preserve citations, and explain uncertainty. ".repeat(12);
        let summary = SkillSummary {
            id: "long-skill".to_owned(),
            name: name.to_owned(),
            description: description.clone(),
            scope: SkillScope::Global,
            path: format!(
                "/home/example/projects/a-long-project-directory/.agents/skills/{name}/SKILL.md"
            ),
            editable: false,
            enabled: false,
            valid: true,
            shadowed: false,
            revision: "package-revision".to_owned(),
            source: None,
            diagnostics: Vec::new(),
        };
        let mut state = SkillsWorkbench::new(1);
        state.install_detail(SkillDetail {
            summary,
            document: format!(
                "---\nname: {name}\ndescription: {description}\n---\n\nInstructions.\n"
            ),
            metadata: Some(SkillMetadata {
                name: name.to_owned(),
                description,
                ..Default::default()
            }),
            files: vec![SkillFile {
                path: "references/notes.txt".to_owned(),
                size: 25,
            }],
        });
        state.pane = SkillPane::Files;
        state.file_index = 1;
        state.resource = Some(SkillResource {
            path: "references/notes.txt".to_owned(),
            content: "Preview remains readable.".to_owned(),
            revision: "resource-revision".to_owned(),
        });

        for (width, height) in [(140, 38), (80, 24), (45, 15)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    draw_skills_workbench(frame, area, &state);
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            for expected in [
                "Skills",
                "summarize-technical-research",
                "Read-only",
                "Disabled",
                "references/notes.txt",
                "Preview remains readable.",
            ] {
                assert!(
                    text.contains(expected),
                    "{width}x{height} must show {expected:?}:\n{text}"
                );
            }
            assert_eq!(text.contains("Library"), width == 140, "{text}");
        }
    }

    #[test]
    fn empty_workbench_renders_with_shared_chrome_at_narrow_and_wide_sizes() {
        for (width, height) in [(140, 38), (80, 24), (45, 15)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let state = SkillsWorkbench::new(1);
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    draw_skills_workbench(frame, area, &state);
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("Skills"));
            assert!(text.contains("Library"));
            assert!(text.contains("No skills installed"));
        }
    }
}
