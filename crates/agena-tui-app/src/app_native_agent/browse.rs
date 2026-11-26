use agena_api::{
    commands::CreateSessionParams,
    queries::{ListWorkspacesParams, Query, QueryResult},
};
use agena_domain::{
    ExternalAgentIndexCapabilities, ExternalAgentWorkspaceAction as WorkspaceAction,
    ExternalAgentWorkspaceResult as WorkspaceResult,
};
use anyhow::bail;

use super::{
    ArchiveFilter, ExternalAgent, ExternalAgentIndexedSession, NativeAgentChoice, NativeAgentPage,
    menu_row,
};
use crate::{App, Overlay, ui_text};

const WORKSPACE_PAGE_SIZE: u64 = 30;

impl App {
    pub(crate) fn open_native_agent_browser(&mut self) {
        self.load_native_workspaces(0, None);
    }

    pub(super) fn load_native_workspaces(&mut self, offset: u64, fallback: Option<Overlay>) {
        self.native_agent_request(
            "external-agent-browser",
            fallback,
            move |backend| async move {
                let result = backend
                    .client()
                    .query_revalidated(Query::ListWorkspaces(ListWorkspacesParams {
                        limit: Some(WORKSPACE_PAGE_SIZE),
                        offset,
                        ..Default::default()
                    }))
                    .await?;
                let QueryResult::Workspaces(page) = result else {
                    bail!("expected a workspace page")
                };
                Ok(page)
            },
            move |app, workspaces| {
                let current = app.application.workspace_id();
                let mut rows = workspaces
                    .items
                    .into_iter()
                    .map(|workspace| {
                        let mut row = menu_row(
                            workspace.path.clone(),
                            String::new(),
                            NativeAgentChoice::Workspace {
                                id: workspace.id,
                                path: workspace.path,
                            },
                        );
                        row.0.current = workspace.id == current;
                        row
                    })
                    .collect::<Vec<_>>();
                if offset == 0 && !rows.iter().any(|(item, _)| item.current) {
                    let path = app.application.workspace_root().display().to_string();
                    rows.insert(
                        0,
                        menu_row(
                            path.clone(),
                            String::new(),
                            NativeAgentChoice::Workspace { id: current, path },
                        ),
                    );
                    rows[0].0.current = true;
                }
                if offset > 0 {
                    rows.push(app.native_agent_row(
                        "external-agent-previous-page",
                        NativeAgentChoice::Workspaces(offset.saturating_sub(WORKSPACE_PAGE_SIZE)),
                    ));
                }
                if workspaces.page.has_more {
                    rows.push(app.native_agent_row(
                        "external-agent-next-page",
                        NativeAgentChoice::Workspaces(offset.saturating_add(WORKSPACE_PAGE_SIZE)),
                    ));
                }
                app.open_native_agent_menu(
                    "external-agent-browser",
                    app.i18n.text("external-agent-choose-workspace"),
                    rows,
                    None,
                );
            },
        );
    }

    pub(super) fn choose_native_agent(&mut self, workspace_id: i64, path: String) {
        let rows = [ExternalAgent::ClaudeCode, ExternalAgent::Antigravity]
            .into_iter()
            .flat_map(|agent| {
                let page = NativeAgentPage {
                    workspace_id,
                    workspace_path: path.clone(),
                    agent,
                    archived: ArchiveFilter::Unarchived,
                    cursors: Vec::new(),
                };
                [
                    menu_row(
                        agent.label().into(),
                        self.i18n.text("external-agent-current-sessions"),
                        NativeAgentChoice::Agent(page.clone()),
                    ),
                    menu_row(
                        self.i18n.text_args(
                            "external-agent-new-with",
                            &agena_tui::fl_args!("agent" => agent.label()),
                        ),
                        String::new(),
                        NativeAgentChoice::Create(page),
                    ),
                ]
            })
            .collect();
        self.open_native_agent_menu("external-agent-choose-agent", path, rows, None);
    }

    pub(super) fn load_native_page(&mut self, page: NativeAgentPage, fallback: Option<Overlay>) {
        let workspace = page.workspace_id;
        let agent = page.agent;
        let cursor = page.cursors.last().cloned();
        let archived = page.archived;
        self.native_agent_request(
            "external-agent-browser",
            fallback,
            move |backend| async move {
                let result = backend
                    .client()
                    .external_agent_workspace(
                        workspace,
                        WorkspaceAction::IndexList {
                            agent,
                            cursor,
                            archived,
                        },
                    )
                    .await?;
                let WorkspaceResult::IndexPage {
                    sessions,
                    next_cursor,
                    capabilities,
                } = result
                else {
                    bail!("expected a native ACP session index page")
                };
                Ok((sessions, next_cursor, capabilities))
            },
            move |app, (sessions, next_cursor, capabilities)| {
                app.show_native_page(page, sessions, next_cursor, capabilities)
            },
        );
    }

    pub(super) fn show_native_page(
        &mut self,
        page: NativeAgentPage,
        sessions: Vec<ExternalAgentIndexedSession>,
        next_cursor: Option<String>,
        capabilities: ExternalAgentIndexCapabilities,
    ) {
        let listing = if page.archived == ArchiveFilter::Archived {
            "external-agent-archived-sessions"
        } else {
            "external-agent-current-sessions"
        };
        let mut description = format!(
            "{} · {}\n{}",
            page.agent.label(),
            self.i18n.text(listing),
            page.workspace_path
        );
        if sessions.is_empty() {
            description.push('\n');
            description.push_str(&self.i18n.text("external-agent-no-sessions"));
        }
        if !capabilities.archive {
            description.push('\n');
            description.push_str(&self.i18n.text("external-agent-archive-unavailable"));
        }
        let mut rows = sessions
            .into_iter()
            .map(|entry| {
                let title = entry
                    .session
                    .title
                    .clone()
                    .unwrap_or_else(|| entry.session.native_id.clone());
                let detail = [
                    entry.model.as_deref(),
                    entry.state.as_deref(),
                    entry.session.updated_at.as_deref(),
                    Some(&entry.session.native_id),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                menu_row(
                    title,
                    detail,
                    NativeAgentChoice::Conversation {
                        page: page.clone(),
                        entry,
                        archive_supported: capabilities.archive,
                    },
                )
            })
            .collect::<Vec<_>>();
        if let Some(next) = page.next(next_cursor) {
            rows.push(
                self.native_agent_row("external-agent-next-page", NativeAgentChoice::Browse(next)),
            );
        }
        if let Some(previous) = page.previous() {
            rows.push(self.native_agent_row(
                "external-agent-previous-page",
                NativeAgentChoice::Browse(previous),
            ));
        }
        rows.push(self.native_agent_row(
            "external-agent-new-session",
            NativeAgentChoice::Create(page.clone()),
        ));
        if capabilities.archive {
            let mut alternate = page.clone();
            alternate.cursors.clear();
            let filter = if page.archived == ArchiveFilter::Archived {
                alternate.archived = ArchiveFilter::Unarchived;
                "external-agent-show-current"
            } else {
                alternate.archived = ArchiveFilter::Archived;
                "external-agent-show-archived"
            };
            rows.push(self.native_agent_row(filter, NativeAgentChoice::Browse(alternate)));
        }
        rows.push(self.native_agent_row("external-agent-refresh", NativeAgentChoice::Browse(page)));
        self.open_native_agent_menu("external-agent-browser", description, rows, None);
    }

    pub(super) fn show_native_conversation(
        &mut self,
        page: NativeAgentPage,
        entry: ExternalAgentIndexedSession,
        archive_supported: bool,
    ) {
        let name = entry
            .session
            .title
            .as_deref()
            .unwrap_or(&entry.session.native_id)
            .to_owned();
        let mut rows = vec![self.native_agent_row(
            "external-agent-open-session",
            NativeAgentChoice::Open {
                page: page.clone(),
                entry: entry.clone(),
            },
        )];
        if archive_supported
            && let Some(archive) = entry.archived.or(match page.archived {
                ArchiveFilter::Unarchived => Some(false),
                ArchiveFilter::Archived => Some(true),
                ArchiveFilter::All => None,
            })
        {
            rows.push(self.native_agent_row(
                if archive {
                    "external-agent-unarchive"
                } else {
                    "external-agent-archive"
                },
                NativeAgentChoice::Archive {
                    page,
                    native_id: entry.session.native_id,
                    archived: !archive,
                },
            ));
        }
        self.open_native_agent_menu("external-agent-browser", name, rows, None);
    }

    pub(super) fn open_native_conversation(
        &mut self,
        page: NativeAgentPage,
        entry: ExternalAgentIndexedSession,
        fallback: Option<Overlay>,
    ) {
        let native_id = entry.session.native_id;
        let title = entry.session.title.unwrap_or_else(|| native_id.clone());
        self.native_agent_request(
            "external-agent-browser",
            fallback,
            move |backend| async move {
                let result = backend
                    .client()
                    .external_agent_workspace(
                        page.workspace_id,
                        WorkspaceAction::Open {
                            agent: page.agent,
                            native_id,
                        },
                    )
                    .await?;
                let WorkspaceResult::Opened { session_id } = result else {
                    bail!("expected an opened native ACP session")
                };
                Ok(session_id)
            },
            move |app, session_id| app.enter_native_view(session_id, title),
        );
    }

    pub(super) fn create_native_conversation(
        &mut self,
        page: NativeAgentPage,
        fallback: Option<Overlay>,
    ) {
        let title = ui_text::default_session_title(&self.i18n);
        self.native_agent_request(
            "external-agent-browser",
            fallback,
            move |backend| async move {
                Ok(backend
                    .client()
                    .create_session_configured(CreateSessionParams {
                        workspace_id: page.workspace_id,
                        title,
                        parent_id: None,
                        auto_title: true,
                        profile: None,
                        prompts: None,
                        external_agent: Some(page.agent),
                    })
                    .await?)
            },
            |app, session| app.enter_native_view(session.id, session.title),
        );
    }

    pub(super) fn set_native_conversation_archived(
        &mut self,
        page: NativeAgentPage,
        native_id: String,
        archived: bool,
        fallback: Option<Overlay>,
    ) {
        let (workspace, agent) = (page.workspace_id, page.agent);
        self.native_agent_request(
            "external-agent-browser",
            fallback,
            move |backend| async move {
                let result = backend
                    .client()
                    .external_agent_workspace(
                        workspace,
                        WorkspaceAction::SetArchived {
                            agent,
                            native_id,
                            archived,
                        },
                    )
                    .await?;
                let WorkspaceResult::Archive { archived, .. } = result else {
                    bail!("expected an ACP archive result")
                };
                Ok(archived)
            },
            move |app, archived| {
                let fallback = app.overlay_stack.pop();
                app.flash_info(app.i18n.text(if archived {
                    "external-agent-archived"
                } else {
                    "external-agent-unarchived"
                }));
                app.load_native_page(page, fallback);
            },
        );
    }
}
