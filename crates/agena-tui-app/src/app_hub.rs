//! Session hub route: home screen listing favorites, sessions needing
//! attention, running sessions, and recent sessions, with a create-new-session
//! action.
//!
//! The display projection and rendering live in `agena_tui_session::session_hub`;
//! this module owns the overview request/response plumbing and the session-open
//! / create / session-list effects that only the App can perform.

use agena_api::resource::SessionOverviewResource;
use agena_tui_session::session_hub::{HubPageTarget, HubRow, SessionHubDirectory};

use super::{App, AppMessage, HubState, KeyEvent, Route};
use crate::{SessionHubItem, SessionHubSection, SessionHubSectionKind, SessionResource, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

/// Number of most-recently-used sessions in the quick-access region.
const HUB_RECENT_LIMIT: u64 = 20;

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
                        let relevant = !matches!(&event, Ok(Event::SessionChanged(Change::PartUpdated { part, .. })) if part.kind != "run");
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
        state.presentation.set_sections(vec![SessionHubSection::new(
            SessionHubSectionKind::New,
            vec![self.hub_new_session_item()],
        )]);
        self.spawn_hub_overview_request(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Hub(state);
    }

    pub(crate) fn spawn_hub_overview_request(&mut self, state: &mut HubState) {
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
        let mut expanded = state.presentation.expanded_directories();
        if state.pages.is_empty() {
            expanded.insert(self.application.workspace_id());
        }
        let query = crate::app_backend::session_hub::HubCatalogQuery {
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
        if !matches!(&self.current_route, Route::Hub(state) if state.loading) {
            return;
        }
        // Reject an overview requested before the mutation was committed.
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
        self.spawn_hub_overview_request(&mut state);
        self.current_route = Route::Hub(state);
    }

    fn schedule_hub_search(&mut self, state: &mut HubState) {
        if let Some(task) = state.request_task.take() {
            task.abort();
        }
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        state.request_id = self.next_hub_request_id;
        state.search_changed_at = Some(std::time::Instant::now());
        state.pages.clear();
        state.loading = false;
    }

    fn hub_sections(
        &self,
        overview: SessionOverviewResource,
        pinned: Vec<SessionHubItem>,
    ) -> Vec<SessionHubSection> {
        vec![
            SessionHubSection::new(
                SessionHubSectionKind::New,
                vec![self.hub_new_session_item()],
            ),
            SessionHubSection::new(
                SessionHubSectionKind::Running,
                overview
                    .running
                    .iter()
                    .map(|session| self.hub_session_item(session))
                    .collect(),
            ),
            SessionHubSection::new(
                SessionHubSectionKind::Attention,
                overview
                    .attention
                    .iter()
                    .map(|session| self.hub_session_item(session))
                    .collect(),
            ),
            SessionHubSection::new(SessionHubSectionKind::Pinned, pinned),
            SessionHubSection::new(
                SessionHubSectionKind::Favorites,
                overview
                    .favorites
                    .iter()
                    .map(|session| self.hub_session_item(session))
                    .collect(),
            ),
            SessionHubSection::new(
                SessionHubSectionKind::Recent,
                overview
                    .recent
                    .iter()
                    .map(|session| self.hub_session_item(session))
                    .collect(),
            ),
        ]
    }

    pub(crate) fn handle_hub_catalog_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<crate::app_backend::session_hub::HubCatalog>,
    ) {
        if !matches!(&self.current_route, Route::Hub(state) if state.request_id == request_id) {
            return;
        }
        let catalog = match result {
            Ok(catalog) => catalog,
            Err(error) => {
                if let Route::Hub(state) = &mut self.current_route {
                    state.loading = false;
                    state.error = Some(error.to_string());
                }
                return;
            }
        };
        let mut overview = SessionOverviewResource {
            favorites: Vec::new(),
            attention: Vec::new(),
            running: Vec::new(),
            recent: Vec::new(),
            generated_at: chrono::Utc::now(),
        };
        let mut pinned = Vec::new();
        for session in &catalog.sessions {
            if session.pinned {
                pinned.push(self.hub_session_item(session));
            }
            if session.favorite {
                overview.favorites.push(session.clone());
            }
            if session.state.is_attention() {
                overview.attention.push(session.clone());
            } else if session.state.is_running()
                || matches!(session.state, agena_api::resource::SessionState::Creating)
            {
                overview.running.push(session.clone());
            } else {
                overview.recent.push(session.clone());
            }
        }
        if catalog.pages.is_empty() {
            overview.recent.truncate(HUB_RECENT_LIMIT as usize);
        }
        let mut sections = self.hub_sections(overview, pinned);
        if !catalog.members.is_empty() {
            for section in &mut sections {
                if section.kind == SessionHubSectionKind::New {
                    continue;
                }
                let members = catalog.members.get(&HubPageTarget::Section(section.kind));
                section
                    .items
                    .retain(|item| members.is_some_and(|ids| ids.contains(&item.session_id)));
            }
            if let Some(ids) = catalog
                .members
                .get(&HubPageTarget::Section(SessionHubSectionKind::Search))
            {
                sections.push(SessionHubSection::new(
                    SessionHubSectionKind::Search,
                    catalog
                        .sessions
                        .iter()
                        .filter(|session| ids.contains(&session.id))
                        .map(|session| self.hub_session_item(session))
                        .collect(),
                ));
            }
        }
        let mut directories = catalog
            .workspaces
            .iter()
            .map(|workspace| SessionHubDirectory {
                workspace_id: workspace.id,
                path: workspace.path.clone(),
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
                    pending_message_id: None,
                    model_stack: None,
                    result,
                })
                .await;
        });
    }

    fn request_session_pin(&mut self, session_id: i64, pinned: bool) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .set_session_pinned(session_id, pinned)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionPinnedUpdated { session_id, result })
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
                if let Some(id) = state.presentation.selected_workspace() {
                    state
                        .presentation
                        .toggle_directory(id, Some(action == KeyAction::MoveRight));
                    if action == KeyAction::MoveRight && !state.loading {
                        self.spawn_hub_overview_request(state);
                    }
                }
            }
            Some(KeyAction::HubTogglePinned) => {
                if let Some(item) = state
                    .presentation
                    .selected_item()
                    .filter(|item| !item.is_new_session)
                {
                    self.request_session_pin(item.session_id, !item.pinned);
                }
            }
            Some(KeyAction::Close) => {
                if state.search_active || !state.query.is_empty() {
                    state.search_active = false;
                    state.query.clear();
                    state.presentation.set_query("");
                    self.schedule_hub_search(state);
                } else {
                    if let Some(task) = state.request_task.take() {
                        task.abort();
                    }
                    return true;
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
            Some(KeyAction::HubOpenSessionList) => {
                self.open_resume_session_picker();
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
                    self.request_session_favorite(item.session_id, !item.favorite);
                }
            }
            Some(KeyAction::MoveUp) => state.presentation.move_selection(-1),
            Some(KeyAction::MoveDown) => state.presentation.move_selection(1),
            Some(KeyAction::PageUp) => state
                .presentation
                .move_selection_page(-1, (self.layout.overlay_area.height as usize / 4).max(1)),
            Some(KeyAction::PageDown) => state
                .presentation
                .move_selection_page(1, (self.layout.overlay_area.height as usize / 4).max(1)),
            Some(KeyAction::Home) => state.presentation.move_selection_home(),
            Some(KeyAction::End) => state.presentation.move_selection_end(),
            Some(KeyAction::NextTab) => state.presentation.move_selection_section(1),
            Some(KeyAction::PreviousTab) => state.presentation.move_selection_section(-1),
            Some(KeyAction::Open) => return self.activate_hub_row(state),
            _ => {
                if let KeyCode::Char(c) = key.code
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    state.search_active = true;
                    state.query.push(c);
                    state.presentation.set_query(&state.query);
                    self.schedule_hub_search(state);
                }
            }
        }
        false
    }

    pub(crate) fn handle_hub_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let Route::Hub(mut state) = std::mem::replace(&mut self.current_route, Route::Main) else {
            return;
        };
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
            detail: detail_parts.join(" | "),
            is_new_session: false,
        }
    }

    /// The synthetic first row of the hub: Entering on it creates a fresh
    /// session, so the hub supports "Enter → new session" directly.
    fn hub_new_session_item(&self) -> SessionHubItem {
        SessionHubItem {
            session_id: 0,
            workspace_id: self.application.workspace_id(),
            pinned: false,
            title: String::new(),
            favorite: false,
            label: self.i18n.text("hub-item-new"),
            detail: self.i18n.text("hub-item-new-detail"),
            is_new_session: true,
        }
    }
}
