//! Session hub route: current project, project tree, or all sessions.
//!
//! The display projection and rendering live in `agena_tui_session::session_hub`;
//! this module owns the overview request/response plumbing and the session-open
//! / create / session-list effects that only the App can perform.

use agena_tui_session::session_hub::{HubPageTarget, HubRow, HubView, SessionHubDirectory};

use super::{App, AppMessage, HubState, KeyEvent, Route};
use crate::{SessionHubItem, SessionHubSection, SessionHubSectionKind, SessionResource, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

impl App {
    /// Opens the session hub as the current route and kicks off the overview
    /// load. Used as the bootstrap landing view and from the hub itself.
    pub(crate) fn open_hub(&mut self) {
        if let Some(task) = self.active_subscription.take() {
            task.abort();
        }
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        let client = self.application.client().clone();
        let tx = self.tx.clone();
        self.active_subscription = Some(tokio::spawn(async move {
            loop {
                if let Ok(mut stream) = client.stream_changes(agena_api::Scope::Global).await {
                    if tx.send(AppMessage::CatalogInvalidated).await.is_err() {
                        return;
                    }
                    while let Some(event) = tokio::select! {
                        _ = tx.closed() => None,
                        event = stream.recv() => event,
                    } {
                        use agena_api::live::SessionChangeResource as Change;
                        use agena_client::SubscriptionEvent as Event;
                        let relevant = !matches!(&event, Ok(Event::SessionChanged(Change::PartUpdated { part, .. })) if !part.is_message_boundary());
                        if relevant && tx.send(AppMessage::CatalogInvalidated).await.is_err() {
                            return;
                        }
                        if event.is_err() {
                            break;
                        }
                    }
                }
                tokio::select! {
                    _ = tx.closed() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        }));
        let mut state = HubState::new();
        self.seed_hub_project(&mut state);
        self.spawn_hub_overview_request(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Hub(state);
    }

    fn seed_hub_project(&self, state: &mut HubState) {
        if state.presentation.view() == HubView::AllSessions {
            return;
        }
        let workspace_id = self.application.workspace_id();
        let path = self.application.workspace_root().display().to_string();
        state
            .presentation
            .set_directories(vec![SessionHubDirectory {
                workspace_id,
                new_session: self.hub_new_session_item(workspace_id, &path),
                path,
                sections: Vec::new(),
                items: Vec::new(),
                expanded: true,
                visible_limit: 20,
            }]);
        state.presentation.select_new_session(workspace_id);
    }

    fn switch_hub_view(&mut self, state: &mut HubState, view: HubView) {
        if state.presentation.view() == view {
            return;
        }
        // Clear the old scope immediately. The new request ID also rejects an
        // already queued response from the previous view/search/page.
        state.presentation = crate::SessionHubPresentation::empty();
        state.presentation.set_view(view);
        state.pages.clear();
        state.pending_page = None;
        self.seed_hub_project(state);
        state.presentation.set_query(&state.query);
        self.spawn_hub_overview_request(state);
    }

    pub(crate) fn spawn_hub_overview_request(&mut self, state: &mut HubState) {
        if !self.session_mark_writes.is_idle() {
            self.defer_hub_catalog_for_marks(state);
            return;
        }
        if let Some(task) = state.request_task.take() {
            task.abort();
        }
        state.search_changed_at = None;
        state.dirty = false;
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        state.request_id = self.next_hub_request_id;
        state.loading = true;
        state.error = None;
        state.refreshed_at = std::time::Instant::now();
        let request_id = state.request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        let expanded = state.presentation.expanded_directories();
        let query = crate::app_backend::session_hub::HubCatalogQuery {
            view: state.presentation.view(),
            pages: state.pages.clone(),
            expanded,
            search: state.query.clone(),
        };
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                application.session_hub_catalog(query),
            )
            .await
            .map_err(|_| crate::UiFailure::internal("Session center request timed out"))
            .and_then(|result| result.map_err(crate::UiFailure::from_backend));
            let _ = tx
                .send(AppMessage::HubCatalogLoaded { request_id, result })
                .await;
        });
        state.request_task = Some(task.abort_handle());
    }

    pub(crate) fn refresh_hub_if_due(&mut self) {
        if !self.session_mark_writes.is_idle() {
            return;
        }
        let due = matches!(&self.current_route, Route::Hub(state)
            if state.search_changed_at.is_some_and(|at| at.elapsed().as_millis() >= 250)
                || (!state.loading && state.dirty && state.refreshed_at.elapsed().as_millis() >= 500)
                || (state.search_changed_at.is_none() && state.refreshed_at.elapsed().as_secs() >= if state.loading { 30 } else { 10 }));
        if !due {
            return;
        }
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
        self.spawn_hub_overview_request(&mut state);
        self.current_route = Route::Hub(state);
    }

    pub(crate) fn refresh_hub_after_mutation(&mut self) {
        if !self.session_mark_writes.is_idle() || !matches!(&self.current_route, Route::Hub(_)) {
            return;
        }
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
        self.spawn_hub_overview_request(&mut state);
        self.current_route = Route::Hub(state);
    }

    fn defer_hub_catalog_for_marks(&mut self, state: &mut HubState) {
        if let Some(task) = state.request_task.take() {
            task.abort();
        }
        // Invalidate even a completed request whose response is still queued.
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        state.request_id = self.next_hub_request_id;
        state.loading = false;
        state.dirty = true;
    }

    fn schedule_hub_search(&mut self, state: &mut HubState) {
        if let Some(task) = state.request_task.take() {
            task.abort();
        }
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        state.request_id = self.next_hub_request_id;
        state.search_changed_at = Some(std::time::Instant::now());
        state.pages.clear();
        state.pending_page = None;
        state.loading = false;
    }

    pub(crate) fn handle_hub_catalog_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<crate::app_backend::session_hub::HubCatalog>,
    ) {
        if !matches!(&self.current_route, Route::Hub(state) if state.request_id == request_id) {
            return;
        }
        if !self.session_mark_writes.is_idle() {
            return;
        }
        let catalog = match result {
            Ok(catalog) => catalog,
            Err(error) => {
                if let Route::Hub(state) = &mut self.current_route {
                    state.loading = false;
                    state.error = Some(error.to_string());
                    state.pending_page = None;
                }
                return;
            }
        };
        let mut sections = [SessionHubSectionKind::All]
            .into_iter()
            .filter_map(|kind| {
                let ids = catalog.members.get(&HubPageTarget::Section(kind))?;
                Some(SessionHubSection::new(
                    kind,
                    catalog
                        .sessions
                        .iter()
                        .filter(|session| ids.contains(&session.id))
                        .map(|session| self.hub_session_item(session))
                        .collect(),
                ))
            })
            .collect::<Vec<_>>();
        let mut directories = catalog
            .workspaces
            .iter()
            .map(|workspace| SessionHubDirectory {
                workspace_id: workspace.id,
                path: workspace.path.clone(),
                new_session: self.hub_new_session_item(workspace.id, &workspace.path),
                sections: [
                    SessionHubSectionKind::Pinned,
                    SessionHubSectionKind::Favorites,
                ]
                .into_iter()
                .map(|kind| {
                    let ids = catalog
                        .members
                        .get(&HubPageTarget::DirectorySection(workspace.id, kind));
                    SessionHubSection::new(
                        kind,
                        catalog
                            .sessions
                            .iter()
                            .filter(|session| {
                                session.workspace_id == workspace.id
                                    && ids.is_some_and(|ids| ids.contains(&session.id))
                            })
                            .map(|session| self.hub_session_item(session))
                            .collect(),
                    )
                })
                .collect(),
                items: catalog
                    .sessions
                    .iter()
                    .filter(|session| {
                        session.workspace_id == workspace.id
                            && (catalog.pages.is_empty()
                                || catalog
                                    .members
                                    .get(&HubPageTarget::Directory(workspace.id))
                                    .is_some_and(|ids| ids.contains(&session.id)))
                    })
                    .map(|session| self.hub_session_item(session))
                    .collect(),
                expanded: workspace.id == self.application.workspace_id(),
                visible_limit: 20,
            })
            .collect::<Vec<_>>();
        directories.sort_by(|a, b| {
            (a.workspace_id != self.application.workspace_id())
                .cmp(&(b.workspace_id != self.application.workspace_id()))
                .then_with(|| a.path.cmp(&b.path))
        });
        // Quick-access duplicates include their directory, so identically
        // titled sessions from different workspaces remain distinguishable.
        for item in sections.iter_mut().flat_map(|section| &mut section.items) {
            if let Some(workspace) = catalog
                .workspaces
                .iter()
                .find(|workspace| workspace.id == item.workspace_id)
            {
                item.detail = format!("{} | {}", workspace.path, item.detail);
            }
        }
        if let Route::Hub(state) = &mut self.current_route {
            state.loading = false;
            state.error = None;
            state.pages = catalog
                .pages
                .iter()
                .map(|(target, page)| (*target, page.page))
                .collect();
            state.presentation.set_catalog(directories, sections);
            state.presentation.set_pagination(catalog.pages);
            if let Some((target, last)) = state.pending_page.take() {
                state.presentation.select_page_target(target, last);
            }
            state.request_task = None;
        }
    }

    fn create_hub_session(&mut self, workspace_id: i64) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        let title = ui_text::default_session_title(&self.i18n);
        tokio::spawn(async move {
            let result = application
                .create_workspace_session(workspace_id, title)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionCreated {
                    submit_draft: None,
                    model_stack: None,
                    result,
                })
                .await;
        });
    }

    fn activate_hub_row(&mut self, state: &mut HubState) -> bool {
        match state.presentation.selected_row().cloned() {
            Some(HubRow::Directory { workspace_id, .. }) => {
                state.presentation.toggle_directory(workspace_id, None);
                if state
                    .presentation
                    .expanded_directories()
                    .contains(&workspace_id)
                {
                    self.spawn_hub_overview_request(state);
                }
            }
            Some(HubRow::Page { target, page, .. }) => {
                if !state.loading {
                    state.pages.insert(target, page);
                    state.pending_page = Some((target, false));
                    self.spawn_hub_overview_request(state);
                }
            }
            Some(HubRow::More { workspace_id, .. }) => state.presentation.show_more(workspace_id),
            Some(HubRow::Item(item)) => {
                if let Some(task) = state.request_task.take() {
                    task.abort();
                }
                if item.is_new_session {
                    self.create_hub_session(item.workspace_id);
                } else {
                    self.open_session(item.session_id, item.title);
                    self.focus = Focus::Composer;
                }
                return true;
            }
            _ => {}
        }
        false
    }

    pub(crate) fn handle_hub_key(&mut self, key: KeyEvent, state: &mut HubState) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        // Printable search input is handled before commands, including CJK;
        // arrow keys and Ctrl shortcuts retain their usual meaning.
        if state.search_active
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char(c) => {
                    state.query.push(c);
                    state.presentation.set_query(&state.query);
                    self.schedule_hub_search(state);
                    return false;
                }
                KeyCode::Backspace => {
                    state.query.pop();
                    state.presentation.set_query(&state.query);
                    self.schedule_hub_search(state);
                    return false;
                }
                _ => {}
            }
        }
        if key.modifiers.is_empty() && key.code == KeyCode::Char('/') {
            state.search_active = true;
            return false;
        }
        match resolve_tui_key(KeyContext::Hub, key) {
            Some(action @ (KeyAction::MoveLeft | KeyAction::MoveRight)) => {
                if state
                    .presentation
                    .move_selection_project(if action == KeyAction::MoveRight {
                        1
                    } else {
                        -1
                    })
                    .is_some()
                {
                    self.spawn_hub_overview_request(state);
                }
            }
            Some(action @ (KeyAction::HubPreviousPage | KeyAction::HubNextPage)) => {
                let forward = action == KeyAction::HubNextPage;
                if !state.loading
                    && let Some((target, page)) = state.presentation.page_request(forward)
                {
                    state.pages.insert(target, page);
                    state.pending_page = Some((target, !forward));
                    self.spawn_hub_overview_request(state);
                }
            }
            Some(KeyAction::HubTogglePinned) => {
                if let Some(item) = state
                    .presentation
                    .selected_item()
                    .filter(|item| !item.is_new_session)
                {
                    let session_id = item.session_id;
                    let pinned = self.toggle_session_pin(session_id, item.pinned);
                    state
                        .presentation
                        .set_pending_marks(session_id, Some(pinned), None);
                    self.defer_hub_catalog_for_marks(state);
                }
            }
            Some(KeyAction::Close) => {
                // The Hub is a root route, not a dismissible overlay. Escape
                // only cancels its search; it must not reveal an empty Main.
                if state.search_active || !state.query.is_empty() {
                    state.search_active = false;
                    state.query.clear();
                    state.presentation.set_query("");
                    self.schedule_hub_search(state);
                }
            }
            Some(KeyAction::HubCreateSession) => {
                self.create_hub_session(
                    state
                        .presentation
                        .selected_workspace()
                        .unwrap_or(self.application.workspace_id()),
                );
                return true;
            }
            Some(KeyAction::Refresh) => {
                if !state.loading || state.refreshed_at.elapsed().as_secs() >= 30 {
                    self.spawn_hub_overview_request(state);
                }
            }
            Some(KeyAction::ToggleFavorite) => {
                if let Some(item) = state
                    .presentation
                    .selected_item()
                    .filter(|item| !item.is_new_session)
                {
                    let session_id = item.session_id;
                    let favorite = self.toggle_session_favorite(session_id, item.favorite);
                    state
                        .presentation
                        .set_pending_marks(session_id, None, Some(favorite));
                    self.defer_hub_catalog_for_marks(state);
                }
            }
            Some(KeyAction::SearchForward) => state.search_active = true,
            Some(KeyAction::MoveUp) => {
                state.search_active = false;
                state.presentation.move_selection(-1);
            }
            Some(KeyAction::MoveDown) => {
                state.search_active = false;
                state.presentation.move_selection(1);
            }
            Some(KeyAction::PageUp) => state
                .presentation
                .move_selection_page(-1, state.presentation.viewport_page_size()),
            Some(KeyAction::PageDown) => state
                .presentation
                .move_selection_page(1, state.presentation.viewport_page_size()),
            Some(KeyAction::Home) => state.presentation.move_selection_home(),
            Some(KeyAction::End) => state.presentation.move_selection_end(),
            Some(action @ (KeyAction::NextTab | KeyAction::PreviousTab)) => {
                let delta = if action == KeyAction::NextTab { 1 } else { -1 };
                self.switch_hub_view(state, state.presentation.view().next(delta));
            }
            Some(KeyAction::Open) => return self.activate_hub_row(state),
            // Search input belongs only to the explicitly opened search box.
            _ => {}
        }
        false
    }

    pub(crate) fn handle_hub_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(view) = agena_tui_session::session_hub::hub_view_hit_test(
                self.layout.overlay_area,
                state.error.is_some(),
                mouse.column,
                mouse.row,
                &self.i18n,
            )
        {
            self.switch_hub_view(&mut state, view);
            self.current_route = Route::Hub(state);
            return;
        }
        let hit = agena_tui_session::session_hub::hub_hit_test(
            self.layout.overlay_area,
            &state.presentation,
            state.error.is_some(),
            mouse.column,
            mouse.row,
            &self.i18n,
        );
        let mut close = false;
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = hit {
                    state.presentation.select_row(hit.row);
                    if hit.activate {
                        close = self.activate_hub_row(&mut state);
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // Scroll the region under the pointer; subsequent motion uses
                // the same selection/viewport model as the keyboard.
                if let Some(hit) = hit
                    && !state
                        .presentation
                        .same_region(hit.row, state.presentation.selection())
                {
                    state.presentation.select_row(hit.row);
                }
                state
                    .presentation
                    .scroll_region(if mouse.kind == MouseEventKind::ScrollUp {
                        -2
                    } else {
                        2
                    });
            }
            _ => {}
        }
        if !close && self.current_route_is_main() {
            self.current_route = Route::Hub(state);
        }
    }

    /// Builds the display projection of one session row. Detail mirrors the
    /// session-search picker so the hub reads consistently with the rest of
    /// the TUI.
    pub(crate) fn hub_session_item(&self, session: &SessionResource) -> SessionHubItem {
        let mut detail_parts = vec![
            ui_text::session_state_label(&self.i18n, &session.state),
            ui_text::session_meta(
                &self.i18n,
                session.id,
                session.message_count,
                session.updated_at,
            ),
        ];
        if self.transcript.session_id == Some(session.id) {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-current"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &agena_tui::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &agena_tui::fl_args!("count" => session.child_session_count as i64),
            ));
        }
        SessionHubItem {
            session_id: session.id,
            workspace_id: session.workspace_id,
            pinned: session.pinned,
            title: session.title.clone(),
            favorite: session.favorite,
            label: session.title.clone(),
            status: match session.state {
                agena_api::resource::SessionState::Ready { .. } => {
                    self.i18n.text("session-work-ready")
                }
                agena_api::resource::SessionState::AwaitingInteraction { .. } => {
                    self.i18n.text("session-work-status-waiting")
                }
                _ => ui_text::session_state_label(&self.i18n, &session.state),
            },
            updated: chrono::DateTime::<chrono::Local>::from(session.updated_at)
                .format("%m-%d %H:%M")
                .to_string(),
            detail: detail_parts.join(" | "),
            is_new_session: false,
        }
    }

    /// A project-local create action, available before metadata has loaded.
    fn hub_new_session_item(&self, workspace_id: i64, path: &str) -> SessionHubItem {
        SessionHubItem {
            session_id: 0,
            workspace_id,
            pinned: false,
            title: String::new(),
            favorite: false,
            label: self.i18n.text("hub-item-new"),
            status: String::new(),
            updated: String::new(),
            detail: format!("{} | {}", path, self.i18n.text("hub-item-new-detail")),
            is_new_session: true,
        }
    }
}
