//! Browse and manage native ACP conversations from the command palette.

mod browse;
mod history;
mod manage;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, future::Future};

use agena_domain::{
    ExternalAgent, ExternalAgentArchiveFilter as ArchiveFilter,
    ExternalAgentAuthAction as AuthAction, ExternalAgentForkPoint, ExternalAgentIndexedSession,
    ExternalAgentSessionAction as SessionAction,
};
use agena_tui::{choice::ChoicePresentationStyle, main_focus::Focus};
use uuid::Uuid;

pub(crate) use history::NativeAgentHistory;

use crate::{
    App, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, ConfirmAction, ConfirmOverlay, Overlay,
    TuiBackend, UiFailure,
};

#[derive(Clone, Debug)]
pub(crate) struct NativeAgentMenu {
    loading: Option<Uuid>,
    actions: BTreeMap<String, NativeAgentChoice>,
}

#[derive(Clone, Debug)]
enum NativeAgentChoice {
    Workspaces(u64),
    Workspace {
        id: i64,
        path: String,
    },
    Agent(NativeAgentPage),
    Browse(NativeAgentPage),
    Conversation {
        page: NativeAgentPage,
        entry: ExternalAgentIndexedSession,
        archive_supported: bool,
    },
    Open {
        page: NativeAgentPage,
        entry: ExternalAgentIndexedSession,
    },
    Preview {
        page: NativeAgentPage,
        entry: ExternalAgentIndexedSession,
    },
    Create(NativeAgentPage),
    Archive {
        page: NativeAgentPage,
        native_id: String,
        archived: bool,
    },
    Manage(i64),
    Edit {
        session_id: i64,
        kind: NativeAgentEditKind,
        initial: String,
    },
    Change {
        session_id: i64,
        action: SessionAction,
    },
    Positions {
        session_id: i64,
        points: Vec<ExternalAgentForkPoint>,
        operation: NativeAgentPosition,
    },
    Confirm {
        session_id: i64,
        action: SessionAction,
        title: &'static str,
        prompt: &'static str,
        label: Option<String>,
    },
    Auth {
        session_id: i64,
        action: AuthAction,
    },
    Providers(i64),
}

#[derive(Clone, Debug)]
pub(crate) struct NativeAgentEdit {
    session_id: i64,
    kind: NativeAgentEditKind,
}

#[derive(Clone, Copy, Debug)]
enum NativeAgentEditKind {
    Title,
    Directories,
}

#[derive(Clone, Copy, Debug)]
enum NativeAgentPosition {
    Fork,
    Rewind,
}

impl NativeAgentPosition {
    fn title(self) -> &'static str {
        match self {
            Self::Fork => "external-agent-fork-before",
            Self::Rewind => "external-agent-rewind-before",
        }
    }
}

#[derive(Clone, Debug)]
struct NativeAgentPage {
    workspace_id: i64,
    workspace_path: String,
    agent: ExternalAgent,
    agent_name: String,
    archived: ArchiveFilter,
    // The first page has no cursor. An advertised empty cursor is valid.
    cursors: Vec<String>,
}

impl NativeAgentPage {
    fn next(&self, cursor: Option<String>) -> Option<Self> {
        let cursor = cursor.filter(|cursor| !self.cursors.contains(cursor))?;
        let mut next = self.clone();
        next.cursors.push(cursor);
        Some(next)
    }

    fn previous(&self) -> Option<Self> {
        let mut previous = self.clone();
        previous.cursors.pop()?;
        Some(previous)
    }
}

type MenuRow = (ChoiceItem, NativeAgentChoice);

impl App {
    pub(crate) fn commit_native_agent_choice(
        &mut self,
        menu: NativeAgentMenu,
        key: &str,
        dialog: &ChoiceOverlay,
    ) -> bool {
        let Some(action) = menu.actions.get(key).cloned() else {
            return false;
        };
        let fallback = || Some(Overlay::Choice(dialog.clone()));
        match action {
            NativeAgentChoice::Workspaces(offset) => {
                self.load_native_workspaces(offset, fallback())
            }
            NativeAgentChoice::Workspace { id, path } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.choose_native_agent(id, path);
            }
            NativeAgentChoice::Agent(page) => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.load_native_page(page, None);
            }
            NativeAgentChoice::Browse(page) => self.load_native_page(page, fallback()),
            NativeAgentChoice::Conversation {
                page,
                entry,
                archive_supported,
            } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.show_native_conversation(page, entry, archive_supported);
            }
            NativeAgentChoice::Open { page, entry } => {
                self.open_native_conversation(page, entry, fallback())
            }
            NativeAgentChoice::Preview { page, entry } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.open_native_history(page, entry.session);
            }
            NativeAgentChoice::Create(page) => self.create_native_conversation(page, fallback()),
            NativeAgentChoice::Archive {
                page,
                native_id,
                archived,
            } => self.set_native_conversation_archived(page, native_id, archived, fallback()),
            NativeAgentChoice::Manage(session_id) => self.open_native_agent_management(session_id),
            NativeAgentChoice::Edit {
                session_id,
                kind,
                initial,
            } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.edit_native_management(session_id, kind, initial);
            }
            NativeAgentChoice::Change { session_id, action } => {
                self.change_native_management(session_id, action, fallback())
            }
            NativeAgentChoice::Positions {
                session_id,
                points,
                operation,
            } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                let rows = points
                    .into_iter()
                    .map(|point| {
                        let action = match operation {
                            NativeAgentPosition::Fork => NativeAgentChoice::Change {
                                session_id,
                                action: SessionAction::Fork {
                                    title: None,
                                    before_message_id: Some(point.message_id),
                                },
                            },
                            NativeAgentPosition::Rewind => NativeAgentChoice::Confirm {
                                session_id,
                                action: SessionAction::Rewind {
                                    before_message_id: point.message_id,
                                },
                                title: operation.title(),
                                prompt: "external-agent-rewind-confirm",
                                label: Some(point.label.clone()),
                            },
                        };
                        menu_row(point.label, String::new(), action)
                    })
                    .collect();
                self.open_native_agent_menu(operation.title(), String::new(), rows, None);
            }
            NativeAgentChoice::Confirm {
                session_id,
                action,
                title,
                prompt,
                label,
            } => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                let lines = label
                    .into_iter()
                    .chain(std::iter::once(self.i18n.text(prompt)))
                    .collect();
                self.overlay = Some(Overlay::Confirm(ConfirmOverlay::new(
                    self.i18n.text(title),
                    lines,
                    self.i18n.text("overlay-confirm-footer"),
                    ConfirmAction::NativeAgentChange { session_id, action },
                )));
            }
            NativeAgentChoice::Auth { session_id, action } => {
                self.request_native_auth(session_id, action)
            }
            NativeAgentChoice::Providers(session_id) => {
                self.overlay_stack.push(Overlay::Choice(dialog.clone()));
                self.open_native_agent_providers(session_id);
            }
        }
        true
    }

    fn native_agent_request<T, Op, Fut, Complete>(
        &mut self,
        title: &'static str,
        fallback: Option<Overlay>,
        operation: Op,
        complete: Complete,
    ) where
        T: Send + 'static,
        Op: FnOnce(TuiBackend) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        Complete: FnOnce(&mut Self, T) + Send + 'static,
    {
        let request_id = self.begin_native_agent_request(title);
        self.dispatch_backend_operation(operation, move |app, result| {
            if !app.finish_native_agent_request(request_id) {
                return;
            }
            match result {
                Ok(value) => complete(app, value),
                Err(error) => app.restore_native_agent_menu(fallback, error),
            }
        });
    }

    fn begin_native_agent_request(&mut self, title: &str) -> Uuid {
        let request = Uuid::new_v4();
        self.open_native_agent_menu(
            title,
            self.i18n.text("overlay-picker-loading"),
            Vec::new(),
            Some(request),
        );
        request
    }

    fn finish_native_agent_request(&mut self, request: Uuid) -> bool {
        let current = matches!(&self.overlay, Some(Overlay::Choice(choice)) if matches!(&choice.action, ChoiceOverlayAction::NativeAgent(menu) if menu.loading == Some(request)));
        if current {
            self.overlay = None;
        }
        current
    }

    fn restore_native_agent_menu(&mut self, fallback: Option<Overlay>, error: UiFailure) {
        self.flash_error(error);
        if let Some(parent) = fallback.or_else(|| self.overlay_stack.pop()) {
            self.overlay = Some(self.refresh_restored_overlay(parent));
        }
    }

    fn open_native_agent_menu(
        &mut self,
        title: &str,
        prompt: String,
        rows: Vec<MenuRow>,
        loading: Option<Uuid>,
    ) {
        let mut actions = BTreeMap::new();
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, (mut item, action))| {
                item.value = index.to_string();
                actions.insert(item.value.clone(), action);
                item
            })
            .collect();
        self.open_choice_overlay(self.build_choice_overlay(
            self.i18n.text(title),
            prompt,
            None,
            rows,
            ChoiceOverlayAction::NativeAgent(Box::new(NativeAgentMenu { loading, actions })),
            false,
            ChoicePresentationStyle::SearchableSelect,
        ));
    }

    fn native_agent_row(&self, label: &str, action: NativeAgentChoice) -> MenuRow {
        menu_row(self.i18n.text(label), String::new(), action)
    }

    fn enter_native_view(&mut self, session_id: i64, title: String) {
        self.clear_native_agent_overlays();
        self.open_session(session_id, title);
        self.focus = Focus::Composer;
    }

    fn clear_native_agent_overlays(&mut self) {
        self.overlay_stack.retain(|layer| !matches!(layer, Overlay::Choice(choice) if matches!(choice.action, ChoiceOverlayAction::NativeAgent(_))));
    }
}

fn menu_row(label: String, detail: String, action: NativeAgentChoice) -> MenuRow {
    (
        ChoiceItem {
            search_text: format!("{label} {detail}"),
            label,
            detail,
            value: String::new(),
            current: false,
        },
        action,
    )
}
