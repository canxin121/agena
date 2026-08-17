impl App {
    pub(crate) fn current_route_is_main(&self) -> bool {
        matches!(self.current_route, Route::Main)
    }

    pub(crate) fn mouse_capture_active(&self) -> bool {
        self.current_route_is_main() && self.overlay.is_none() && self.context_help.is_none()
    }

    /// Resolve a possibly-relative path against the workspace root.
    pub(crate) fn resolve_workspace_path(&self, path: &std::path::Path) -> std::path::PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.application.workspace_root().join(path)
        }
    }

    pub fn new_with_backend(
        application: crate::TuiBackend,
        mut launch: LaunchOptions,
        i18n: I18n,
    ) -> Self {
        apply_math_graphics_appearance(&mut launch);
        let math_render_context = agena_tui_media::MathRenderContext::new(
            launch.math_graphics.as_ref(),
            application.media_workspace_root(),
        );
        let (tx, rx) = tokio::sync::mpsc::channel(APP_MESSAGE_QUEUE_CAPACITY);
        let (command_tx, command_rx) = tokio::sync::mpsc::channel(UI_COMMAND_QUEUE_CAPACITY);
        let draft_store_path = default_draft_store_path();
        let (draft_store, pending_draft_store_error) = match DraftStore::load(&draft_store_path) {
            Ok(store) => (store, None),
            Err(error) => (DraftStore::default(), Some(error)),
        };
        let prompt_history_path = default_prompt_history_path();
        let (prompt_history, pending_prompt_history_error) =
            match PromptHistory::load(&prompt_history_path) {
                Ok(history) => (history, None),
                Err(error) => (PromptHistory::default(), Some(error)),
            };
        let keybindings = launch.tui_config.keybindings.clone();
        let status_line = StatusLinePresentation::from_config(&launch.tui_config.status_line);
        let double_esc_window = Duration::from_millis(launch.tui_config.double_esc_window_ms);
        let plugin_theme = launch.tui_config.theme.as_ref().and_then(|theme_id| {
            crate::app_backend::plugin_effects::plugin_theme_palettes(&application)
                .into_iter()
                .find(|palette| palette.id == *theme_id)
        });
        let base_palette = launch.tui_config.palette(launch.terminal_background);
        let tui_palette = tui_palette_with_plugin(base_palette, plugin_theme.as_ref());
        agena_tui_components::theme::set_active_palette(tui_palette);
        let mut transcript = TranscriptState::new(
            i18n.clone(),
            TranscriptDetailDefaults {
                activity_default_expanded: launch.tui_config.transcript.activity_default_expanded,
                kind_defaults: launch.tui_config.transcript.activity_kinds.clone(),
            },
        );
        transcript.set_math_render_context(math_render_context.clone());
        let mut app = Self {
            application,
            i18n: i18n.clone(),
            tx,
            rx,
            command_tx,
            command_rx: Some(command_rx),
            command_actor: None,
            settings_runtime_snapshot_summary: None,
            settings_session_permission: None,
            session_selection_revision: 0,
            launch: launch.clone(),
            math_renderer: launch
                .math_graphics
                .clone()
                .and_then(MathGraphicsRenderer::new),
            math_render_context,
            should_quit: false,
            focus: Focus::Transcript,
            current_route: Route::Main,
            route_stack: Vec::new(),
            overlay: None,
            overlay_stack: Vec::new(),
            context_help: None,
            seen_permission_request_ids: BTreeSet::new(),
            seen_user_input_request_ids: BTreeSet::new(),
            revealed_user_input_request_ids: BTreeSet::new(),
            user_input_interactions: BTreeMap::new(),
            interaction_editing: None,
            notifications: crate::notifications::NotificationStore::new(),
            seen_failure_ids: HashSet::new(),
            sessions: SessionListPresentation::new(
                launch.initial_session_search.unwrap_or_default(),
            ),
            session_load: SessionListLoadState::default(),
            session_composer: SessionComposerState::default(),
            session_controller: agena_tui_session::SessionController::default(),
            transcript,
            transcript_cache: HashMap::new(),
            run_options: RunOptionsState::default(),
            composer: Editor::default(),
            composer_items: Vec::new(),
            slash_command_suggestions: None,
            slash_command_suggestion_actions: BTreeMap::new(),
            dismissed_slash_command_suggestions_for: None,
            file_mention_suggestions: None,
            file_mention_suggestion_actions: BTreeMap::new(),
            dismissed_file_mention_suggestions_for: None,
            prompt_history_search: None,
            composer_item_selection: Default::default(),
            draft_store,
            draft_store_path,
            draft_store_dirty: false,
            draft_store_last_persist_at: Instant::now()
                .checked_sub(Duration::from_millis(DRAFT_PERSIST_INTERVAL_MS))
                .unwrap_or_else(Instant::now),
            draft_store_reported_error: None,
            pending_draft_store_error,
            prompt_history,
            prompt_history_path,
            prompt_history_reported_error: None,
            pending_prompt_history_error,
            run_activity: RunActivityTracker::default(),
            next_pending_user_message_id: 1,
            layout: LayoutCache::default(),
            surface_layout: crate::SurfaceLayout::default(),
            surface_selection: None,
            transcript_scrollbar_drag: None,
            transcript_pointer_gesture: None,
            last_transcript_click: None,
            mouse_events_seen: 0,
            last_mouse_event: None,
            bootstrap_done: false,
            last_refresh_at: Instant::now()
                .checked_sub(Duration::from_millis(REFRESH_INTERVAL_MS))
                .unwrap_or_else(Instant::now),
            pending_refresh: None,
            pending_ui_action: None,
            current_lineage: None,
            side_sessions: HashMap::new(),
            next_usage_request_id: 0,
            next_hub_request_id: 0,
            active_subscription: None,
            queue: ComposerQueue::new(),
            status_line,
            plugin_theme,
            keybindings,
            transcript_motion_prefix: None,
            transcript_yank_pending: false,
            transcript_yank_origin: None,
            transcript_goto_pending: false,
            transcript_viewport_pending: false,
            transcript_find_pending: None,
            transcript_last_find: None,
            transcript_text_object_pending: None,
            transcript_search_forward: true,
            last_ctrl_c_at: None,
            plan_display_refresh: None,
            double_esc_window,
            terminal_integration: TerminalIntegrationState::default(),
        };
        if let Some(draft) = app.draft_store.get(DraftSlot::NewSession).cloned() {
            app.restore_composer_draft(draft);
        }
        app
    }

    pub(crate) fn refresh_tui_palette_from_runtime(&mut self) {
        self.launch.tui_config = crate::tui_config_from_preferences(
            &crate::app_backend::config::ui_configuration(&self.application),
        );
        self.plugin_theme = self.launch.tui_config.theme.as_ref().and_then(|theme_id| {
            crate::app_backend::plugin_effects::plugin_theme_palettes(&self.application)
                .into_iter()
                .find(|palette| palette.id == *theme_id)
        });
        self.apply_current_tui_appearance();
    }

    fn sync_terminal_appearance(&mut self, terminal: &TerminalRuntime) {
        let context = terminal.context();
        let already_current = self
            .launch
            .terminal_context
            .as_ref()
            .is_some_and(|current| {
                current.color_generation == context.color_generation
                    && current.color == context.color
            });
        if already_current {
            return;
        }

        self.launch.terminal_background = terminal.background();
        self.launch.terminal_context = Some(context.clone());
        self.apply_current_tui_appearance();
    }

    fn apply_current_tui_appearance(&mut self) {
        let diagnostics_scroll = self
            .context_help
            .as_ref()
            .filter(|help| help.kind == crate::HelpOverlayKind::Diagnostics)
            .map(|help| help.scroll);
        let base_palette = self
            .launch
            .tui_config
            .palette(self.launch.terminal_background);
        let palette = tui_palette_with_plugin(base_palette, self.plugin_theme.as_ref());
        agena_tui_components::theme::set_active_palette(palette);
        // Formula glyph rasters, SVG `currentColor`, and rich transcript styles
        // all contain resolved appearance colors. Refresh them together so a
        // live light/dark switch cannot mix both themes.
        apply_math_graphics_appearance(&mut self.launch);
        self.math_render_context = agena_tui_media::MathRenderContext::new(
            self.launch.math_graphics.as_ref(),
            self.application.media_workspace_root(),
        );
        self.transcript
            .set_math_render_context(self.math_render_context.clone());
        self.math_renderer = self
            .launch
            .math_graphics
            .clone()
            .and_then(MathGraphicsRenderer::new);
        if let Some(scroll) = diagnostics_scroll {
            self.open_terminal_diagnostics();
            if let Some(help) = self.context_help.as_mut() {
                help.scroll = scroll;
            }
        }
    }

    pub async fn run(&mut self, terminal: &mut TerminalRuntime) -> Result<()> {
        self.start_command_actor();
        self.bootstrap();

        let mut ticker = interval(Duration::from_millis(UI_TICK_MS));

        loop {
            // TerminalRuntime may have observed a new background after focus
            // regain or terminal resume. Apply it before drawing so the text
            // palette, cached formula rasters, SVG currentColor, and native
            // image compositor all advance in the same frame.
            self.sync_terminal_appearance(terminal);
            if let Some(renderer) = self.math_renderer.as_mut() {
                renderer.sync_generation(terminal.generation());
            }
            let mouse_capture_active = self.mouse_capture_active();
            if !mouse_capture_active {
                self.cancel_active_pointer_gesture();
            }
            terminal.set_mouse_capture_active(mouse_capture_active)?;
            let math_render_context = self.math_render_context.clone();
            terminal.draw(|frame| {
                agena_tui_media::with_math_render_context(&math_render_context, || {
                    self.draw(frame);
                });
            })?;
            terminal.set_text_input_active(self.has_active_text_input());
            crate::sync_terminal_title(self, terminal)?;
            crate::sync_terminal_progress(self, terminal)?;
            crate::drain_terminal_notification(self, terminal)?;

            tokio::select! {
                maybe_event = terminal.next_event() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if matches!(event, Event::FocusGained) {
                                terminal.refresh_color_on_focus();
                            }
                            self.handle_terminal_event(event);
                        }
                        Some(Err(error)) => self.flash_error(self.i18n.text_args(
                            "flash-terminal-event-error",
                            &agena_tui::fl_args!("error" => error.to_string()),
                        )),
                        None => self.should_quit = true,
                    }
                }
                maybe_message = self.rx.recv() => {
                    if let Some(message) = maybe_message {
                        self.handle_message(message);
                    } else {
                        self.should_quit = true;
                    }
                }
                _ = ticker.tick() => {
                    self.on_tick();
                }
            }

            if let Some(action) = self.pending_ui_action.take() {
                self.run_ui_action(action, terminal)?;
            }

            if self.should_quit {
                break;
            }
        }

        self.rx.close();
        if let Some(subscription) = self.active_subscription.take() {
            subscription.abort();
        }
        if let Some(actor) = self.command_actor.take() {
            actor.abort();
        }

        Ok(())
    }

    pub(crate) fn bootstrap(&mut self) {
        if self.bootstrap_done {
            return;
        }

        self.bootstrap_done = true;
        self.request_sessions(false);

        if let Some(session_id) = self.launch.initial_session_id {
            self.open_session(
                session_id,
                ui_text::session_fallback_title(&self.i18n, session_id),
            );
        } else if let Some(query) = self.launch.initial_session_search.clone() {
            // An explicit --search launch keeps opening the resume picker so
            // the requested session list is visible immediately.
            self.open_resume_session_picker_with_query(query.as_str());
        } else {
            // The server owns sessions independently from this client. Land
            // on the session hub home screen (attention / running / recent,
            // create-new-session) instead of opening straight into a session
            // list, so reconnecting to an existing server never looks like a
            // brand-new empty session and the user can pick where to go.
            self.open_hub();
        }
    }

    pub(crate) fn on_tick(&mut self) {
        let now = Instant::now();
        self.refresh_input_derived_state();
        self.refresh_status_line_if_due(now);
        self.poll_provider_studio_auth_if_due(now);
        self.refresh_activities_panel_if_due(now);
        self.heal_plan_display_refresh();
        if let Some(error) = self.pending_draft_store_error.take() {
            self.report_draft_store_error(error);
        }
        if let Some(error) = self.pending_prompt_history_error.take() {
            self.report_prompt_history_error(error);
        }

        self.notifications
            .prune_expired(crate::notifications::now_ms());

        // A refresh or session-state-load response can be lost (spawned task
        // panicked, message dropped, or a backend call that never resolved).
        // The in-flight flag would then stay set and block every later
        // refresh, freezing the transcript at a stale snapshot — including
        // stuck \"working\" indicators and a reply that never converges.
        // Recover the wedge so the periodic refresh resumes.
        self.transcript
            .recover_stalled_requests(Duration::from_millis(REFRESH_STALL_TIMEOUT_MS));
        // The session list has the same in-flight contract: a lost response
        // would leave `loading` set and coalesce every later request, freezing
        // the list at stale rows. Recover it alongside the transcript.
        self.session_load
            .recover_stalled_request(Duration::from_millis(REFRESH_STALL_TIMEOUT_MS));

        // An event-driven refresh request (a streaming `PartUpdated` arrived)
        // is merged into the same interval gate: flushing once per event
        // would spawn a full-snapshot refresh for every coalesced stream
        // flush, saturating the TUI with ~100+ refreshes/s and keeping the
        // transcript permanently behind a running reply. The periodic path
        // below repaints streamed parts at most every `REFRESH_INTERVAL_MS`,
        // which is what makes reasoning/tool-call deltas appear live. A
        // parked force refresh (bus lag, terminal safety net) rides the same
        // gate — the terminal state converges a fraction of a second later.
        if let Some(session_id) = self.transcript.session_id
            && !self.transcript.refreshing
            && !self.transcript.state_loading
            && self.last_refresh_at.elapsed() >= Duration::from_millis(REFRESH_INTERVAL_MS)
        {
            self.last_refresh_at = Instant::now();
            let force = self.pending_refresh.take().is_some();
            // While a run is executing, storage commits streamed parts only
            // at part completion (end-only flush), so the in-memory overlay
            // advances without bumping the durable watermark. A changed-gated
            // reload would see "no new events" and never repaint; force the
            // reload so reasoning/tool deltas stream live into the transcript.
            let streaming_live = self
                .transcript
                .execution
                .as_ref()
                .is_some_and(|execution| execution.session.state.active_execution().is_some());
            self.request_refresh(session_id, force || streaming_live);
        }

        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(false);
    }

    pub(crate) fn poll_provider_studio_auth_if_due(&mut self, now: Instant) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            return;
        };

        if dialog.pending_auth_key.is_none()
            && let Some(interval) = provider_studio_auth_poll_interval(&dialog)
        {
            match dialog.next_auth_poll_at {
                Some(deadline) if now >= deadline => {
                    self.request_provider_studio_continue_auth(&mut dialog);
                }
                Some(_) => {}
                None => {
                    dialog.next_auth_poll_at = now.checked_add(interval).or(Some(now));
                }
            }
        }

        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(crate) fn handle_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                self.transcript_scrollbar_drag = None;
                self.last_transcript_click = None;
                if self
                    .transcript_pointer_gesture
                    .take()
                    .is_some_and(|gesture| gesture.dragged)
                {
                    self.transcript.cancel_text_selection(
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                self.handle_key_event(key);
            }
            Event::Paste(text) => {
                self.transcript_scrollbar_drag = None;
                self.transcript_pointer_gesture = None;
                self.last_transcript_click = None;
                self.transcript.cancel_text_selection(
                    self.layout.transcript_body.width,
                    self.layout.transcript_body.height,
                );
                self.handle_paste(text);
            }
            Event::Resize(_, _) => {
                self.transcript_scrollbar_drag = None;
                self.transcript_pointer_gesture = None;
                self.last_transcript_click = None;
                self.transcript.cancel_text_selection(
                    self.layout.transcript_body.width,
                    self.layout.transcript_body.height,
                );
                self.transcript.invalidate_render();
            }
            Event::Mouse(mouse) => {
                self.mouse_events_seen = self.mouse_events_seen.saturating_add(1);
                self.last_mouse_event = Some(format!(
                    "{:?} @ {},{} modifiers={:?}",
                    mouse.kind, mouse.column, mouse.row, mouse.modifiers
                ));
                if self.mouse_events_seen == 1 {
                    tracing::info!(
                        kind = ?mouse.kind,
                        column = mouse.column,
                        row = mouse.row,
                        modifiers = ?mouse.modifiers,
                        "received first terminal mouse event"
                    );
                }
                self.handle_mouse_event(mouse);
            }
            Event::FocusGained => {}
            Event::FocusLost => {
                self.transcript_scrollbar_drag = None;
                self.last_transcript_click = None;
                if self
                    .transcript_pointer_gesture
                    .take()
                    .is_some_and(|gesture| gesture.dragged)
                {
                    self.transcript.cancel_text_selection(
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
            }
            // TerminalRuntime consumes these before dispatch. Keep the app
            // boundary total in case a synthetic event is supplied by a test.
            Event::TerminalResponse(_) => {}
        }
    }
}

fn apply_math_graphics_appearance(launch: &mut LaunchOptions) {
    let background = launch
        .tui_config
        .graphics_background(launch.terminal_background);
    if let Some(graphics) = launch.math_graphics.as_mut() {
        graphics.apply_terminal_appearance(background);
    }
}

fn tui_palette_with_plugin(
    base: agena_tui_components::ThemePalette,
    plugin_theme: Option<&agena_plugin_host::HostThemePalette>,
) -> agena_tui_components::ThemePalette {
    let Some(theme) = plugin_theme else {
        return base;
    };
    base.with_overrides(agena_tui_components::ThemeOverrides {
        muted: tui_plugin_color(theme.colors.muted.as_ref()),
        accent: tui_plugin_color(theme.colors.accent.as_ref()),
        info: tui_plugin_color(theme.colors.info.as_ref()),
        success: tui_plugin_color(theme.colors.success.as_ref()),
        warning: tui_plugin_color(theme.colors.warning.as_ref()),
        danger: tui_plugin_color(theme.colors.danger.as_ref()),
        special: tui_plugin_color(theme.colors.special.as_ref()),
        selection_fg: tui_plugin_color(theme.colors.selection_fg.as_ref()),
        selection_bg: tui_plugin_color(theme.colors.selection_bg.as_ref()),
    })
}

fn tui_plugin_color(color: Option<&agena_plugin_sdk::PluginTuiColor>) -> Option<Color> {
    color.map(|color| {
        agena_tui_components::theme::parse_color(color.as_str())
            .expect("PluginTuiColor guarantees the canonical TUI color grammar")
    })
}

use crate::Result;
use crate::{
    APP_MESSAGE_QUEUE_CAPACITY, App, BTreeMap, BTreeSet, Color, ComposerQueue,
    DRAFT_PERSIST_INTERVAL_MS, DraftSlot, DraftStore, Duration, Editor, Event, HashMap, HashSet,
    I18n, Instant, LaunchOptions, LayoutCache, PromptHistory, REFRESH_INTERVAL_MS,
    REFRESH_STALL_TIMEOUT_MS, Route, RunActivityTracker, RunOptionsState, SessionComposerState,
    SessionListLoadState, TerminalIntegrationState, TerminalRuntime, TranscriptDetailDefaults,
    TranscriptState, UI_COMMAND_QUEUE_CAPACITY, UI_TICK_MS, default_draft_store_path,
    default_prompt_history_path, interval, provider_studio_auth_poll_interval, ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui::status_line::StatusLinePresentation;
use agena_tui_media::MathGraphicsRenderer;
use agena_tui_session::session_list::SessionListPresentation;
#[cfg(test)]
mod tests {
    use agena_tui::presentation_config::ColorSchemePreference;
    use agena_tui_components::TerminalRgb;

    use super::*;

    #[test]
    fn formula_glyph_appearance_follows_live_scheme_instead_of_stale_detection() {
        let detected_dark = TerminalRgb::new(18, 18, 20);
        let mut launch = LaunchOptions {
            terminal_background: Some(detected_dark),
            math_graphics: Some(agena_tui_media::MathGraphicsConfig::query(
                Some(detected_dark),
                false,
                false,
                None,
            )),
            ..LaunchOptions::default()
        };

        launch.tui_config.color_scheme = ColorSchemePreference::Light;
        apply_math_graphics_appearance(&mut launch);
        let light = agena_tui_media::MathRenderContext::new(
            launch.math_graphics.as_ref(),
            std::path::Path::new("."),
        );
        agena_tui_media::with_math_render_context(&light, || {
            let layout = agena_tui_media::layout_config();
            assert_eq!(layout.foreground, [28, 28, 28]);
        });

        launch.tui_config.color_scheme = ColorSchemePreference::Dark;
        apply_math_graphics_appearance(&mut launch);
        let dark = agena_tui_media::MathRenderContext::new(
            launch.math_graphics.as_ref(),
            std::path::Path::new("."),
        );
        agena_tui_media::with_math_render_context(&dark, || {
            let layout = agena_tui_media::layout_config();
            assert_eq!(layout.foreground, [235, 235, 235]);
        });
    }
}
