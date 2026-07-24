#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptRevealPolicy {
    /// Keep the current viewport while the cursor remains visible; otherwise
    /// move only far enough to reveal it.
    Minimal,
    /// Keep the cursor at the same terminal row while content flows past it.
    PreserveScreenRow(usize),
    /// Large directional jumps expose the content ahead of the cursor.
    DirectionalEdge(TranscriptMoveDirection),
    /// Explicit location jumps such as search results deserve centering.
    Center,
}

impl Default for TranscriptState {
    fn default() -> Self {
        Self::new(
            I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: false,
            },
        )
    }
}

impl TranscriptState {
    pub(crate) fn new(i18n: I18n, detail_expanded_by_default: TranscriptDetailDefaults) -> Self {
        Self {
            i18n,
            math_render_context: agena_tui_media::MathRenderContext::default(),
            session_id: None,
            session_title: String::new(),
            messages: Vec::new(),
            pending_user_messages: Vec::new(),
            older_cursor: None,
            has_more_older: false,
            loading_initial: false,
            loading_older: false,
            refreshing: false,
            state_loading: false,
            pending_restore_draft: None,
            viewport: TranscriptViewport::default(),
            interaction: TranscriptInteraction::default(),
            search_query: String::new(),
            search_match_index: None,
            execution: None,
            last_event_seq: None,
            detail_expanded_by_default,
            node_expansions: BTreeMap::new(),
            rendered: None,
        }
    }

    pub(crate) fn set_math_render_context(&mut self, context: agena_tui_media::MathRenderContext) {
        self.math_render_context = context;
        self.invalidate_render();
    }

    pub(crate) fn reset(&mut self, session_id: i64, title: String) {
        self.session_id = Some(session_id);
        self.session_title = title;
        self.messages.clear();
        self.pending_user_messages.clear();
        self.older_cursor = None;
        self.has_more_older = false;
        self.loading_initial = false;
        self.loading_older = false;
        self.refreshing = false;
        self.state_loading = false;
        self.pending_restore_draft = None;
        self.viewport.reduce(TranscriptAction::Reset);
        self.interaction = TranscriptInteraction::default();
        self.execution = None;
        self.last_event_seq = None;
        self.search_query.clear();
        self.search_match_index = None;
        self.node_expansions.clear();
        self.invalidate_render();
    }

    pub(crate) fn apply_execution(&mut self, execution: SessionExecutionResource) {
        self.session_title = execution.session.title.clone();
        self.last_event_seq = execution.latest_event_seq;
        self.execution = Some(execution);
        self.invalidate_render();
    }

    pub(crate) fn add_pending_user_message(&mut self, message: PendingUserMessage) {
        self.pending_user_messages.push(message);
        self.viewport.reduce(TranscriptAction::FollowTail);
        self.invalidate_render();
    }

    pub(crate) fn remove_pending_user_message(&mut self, id: u64) {
        self.pending_user_messages
            .retain(|message| message.id != id);
        self.invalidate_render();
    }

    pub(crate) fn confirm_pending_user_message(&mut self, id: u64) {
        if let Some(message) = self
            .pending_user_messages
            .iter_mut()
            .find(|message| message.id == id)
        {
            message.confirmed = true;
            self.invalidate_render();
        }
    }

    fn acknowledge_next_pending_user_message(&mut self, persisted_message_id: i64) {
        let Some(index) = self
            .pending_user_messages
            .iter()
            .position(|message| message.persisted_message_id.is_none())
        else {
            return;
        };

        if self
            .messages
            .iter()
            .any(|message| message.id == persisted_message_id)
        {
            self.pending_user_messages.remove(index);
        } else {
            let message = &mut self.pending_user_messages[index];
            message.confirmed = true;
            message.persisted_message_id = Some(persisted_message_id);
        }
        self.invalidate_render();
    }

    fn reconcile_pending_user_messages(&mut self, incoming: &[MessageResource]) {
        let incoming_ids = incoming
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let current_ids = self
            .messages
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let mut unassigned_new_user_messages = incoming
            .iter()
            .filter(|message| {
                message.role == MessageRole::User && !current_ids.contains(&message.id)
            })
            .count();

        self.pending_user_messages.retain(|message| {
            if message
                .persisted_message_id
                .is_some_and(|id| incoming_ids.contains(&id))
            {
                return false;
            }
            if message.confirmed
                && message.persisted_message_id.is_none()
                && unassigned_new_user_messages > 0
            {
                unassigned_new_user_messages -= 1;
                return false;
            }
            true
        });
    }

    pub(crate) fn replace_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        self.reconcile_pending_user_messages(page.items.as_slice());
        self.messages = page.items;
        self.older_cursor = page.page.next_cursor;
        self.has_more_older = page.page.has_more;
        self.invalidate_render();
        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    pub(crate) fn prepend_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        let old_total = self.rendered(width).lines.len();
        let mut merged = page.items;
        merged.extend(self.messages.clone());
        merged.sort_by_key(message_sort_key);
        merged.dedup_by_key(|message| message.id);
        self.messages = merged;
        self.older_cursor = page.page.next_cursor;
        self.has_more_older = page.page.has_more;
        self.invalidate_render();
        let new_total = self.rendered(width).lines.len();
        let added_lines = new_total.saturating_sub(old_total);
        self.viewport.top = self.viewport.top.saturating_add(added_lines);
        self.shift_interaction_lines(added_lines);
        self.clamp_scroll(width, height);
    }

    pub(crate) fn merge_latest_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        self.reconcile_pending_user_messages(page.items.as_slice());
        let latest_ids = page
            .items
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let mut merged = self
            .messages
            .iter()
            .filter(|message| !latest_ids.contains(&message.id))
            .cloned()
            .collect::<Vec<_>>();
        for incoming in page.items {
            if let Some(existing) = self
                .messages
                .iter()
                .find(|message| message.id == incoming.id)
            {
                merged.push(merge_message_resources(existing, &incoming));
            } else {
                merged.push(incoming);
            }
        }
        merged.sort_by_key(message_sort_key);
        merged.dedup_by_key(|message| message.id);
        self.messages = merged;
        self.invalidate_render();
        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    /// Apply the Runtime-owned live projection used by the terminal backend.
    /// The terminal never reconstructs a concrete event envelope from JSON.
    pub(crate) fn apply_presentation_event(
        &mut self,
        event: &agena_runtime::RuntimePresentationEvent,
        width: u16,
        height: u16,
    ) -> bool {
        let refresh_needed = match &event.kind {
            agena_runtime::RuntimePresentationEventKind::UserMessageAppended { message_id } => {
                self.acknowledge_next_pending_user_message(*message_id);
                true
            }
            agena_runtime::RuntimePresentationEventKind::MessagePartCheckpointed(update) => {
                if update.message_role == agena_domain::Role::User {
                    if update.part.part_index == 0 {
                        self.acknowledge_next_pending_user_message(update.message_id);
                    }
                    true
                } else {
                    self.apply_runtime_message_part_checkpointed(update);
                    false
                }
            }
            agena_runtime::RuntimePresentationEventKind::MessagePartDelta(delta) => {
                self.apply_message_part_delta(delta).is_err()
            }
            agena_runtime::RuntimePresentationEventKind::AssistantMessageFinished
            | agena_runtime::RuntimePresentationEventKind::Refresh { .. } => true,
        };

        if !refresh_needed && let Some(seq) = event.meta.seq_session {
            self.last_event_seq = Some(seq);
        }

        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }

        refresh_needed
    }

    fn apply_runtime_message_part_checkpointed(
        &mut self,
        update: &agena_runtime::RuntimeMessagePartCheckpoint,
    ) {
        let incoming_metadata = message_metadata_from_runtime(&update.message_metadata);
        let target = self
            .live_message_target(update.message_id, Some(update.part.id))
            .or_else(|| {
                update.message_metadata.turn_id.and_then(|turn_id| {
                    self.messages.iter().position(|message| {
                        message.role == MessageRole::Assistant
                            && message.metadata.turn_id == Some(turn_id)
                            && same_assistant_model_route(&message.metadata, &incoming_metadata)
                    })
                })
            });

        let index = match target {
            Some(index) => index,
            None => {
                self.messages.push(MessageResource {
                    id: update.message_id,
                    session_id: update.session_id,
                    role: message_role_from_domain(update.message_role),
                    state: message_status_from_domain(update.message_state),
                    created_at: update.message_created_at,
                    updated_at: timestamp_ms_or(update.ts_ms, update.message_created_at),
                    metadata: incoming_metadata,
                    usage: None,
                    part_count: 0,
                    parts: Some(Vec::new()),
                });
                self.messages.sort_by_key(message_sort_key);
                self.messages
                    .iter()
                    .position(|message| message.id == update.message_id)
                    .expect("new live message should remain in the transcript")
            }
        };

        let message = &mut self.messages[index];
        message.state = message_status_from_domain(update.message_state);
        message.updated_at = timestamp_ms_or(update.ts_ms, message.updated_at);
        let visible_message_id = message.id;
        let parts = message.parts.get_or_insert_with(Vec::new);
        if let Some(existing) = parts.iter_mut().find(|part| part.id == update.part.id) {
            let visible_part_index = existing.part_index;
            *existing = message_part_resource_from_runtime(
                &update.part,
                agena_api::resource::PartLoadMode::Full,
            );
            existing.message_id = visible_message_id;
            existing.part_index = visible_part_index;
        } else {
            let mut part = message_part_resource_from_runtime(
                &update.part,
                agena_api::resource::PartLoadMode::Full,
            );
            part.message_id = message.id;
            part.part_index = parts.len() as i32;
            parts.push(part);
        }
        message.part_count = parts.len() as u64;
        self.invalidate_render();
    }

    fn apply_message_part_delta(
        &mut self,
        delta: &agena_domain::MessagePartDeltaEvent,
    ) -> Result<(), ()> {
        let Some(message) = self.messages.iter_mut().find(|message| {
            message
                .parts
                .as_ref()
                .is_some_and(|parts| parts.iter().any(|part| part.id == delta.part_id))
        }) else {
            return Err(());
        };
        let Some(parts) = message.parts.as_mut() else {
            return Err(());
        };
        let Some(part) = parts.iter_mut().find(|part| part.id == delta.part_id) else {
            return Err(());
        };

        if part.status == PartExecutionStatusResource::Pending {
            part.status = PartExecutionStatusResource::InProgress;
        }
        if message.state == MessageStatus::Pending {
            message.state = MessageStatus::InProgress;
        }
        message.updated_at = timestamp_ms_or(delta.ts_ms, message.updated_at);

        let updated = match &delta.field {
            agena_domain::PartDeltaField::Text => match part.content.as_mut() {
                Some(MessagePartDetailResource::Text(text)) => {
                    text.text.push_str(&delta.delta);
                    true
                }
                _ => false,
            },
            agena_domain::PartDeltaField::ReasoningSummary => match part.content.as_mut() {
                Some(MessagePartDetailResource::Reasoning(reasoning)) => {
                    reasoning.summary.push(delta.delta.clone());
                    true
                }
                _ => false,
            },
            agena_domain::PartDeltaField::ReasoningRawContent => match part.content.as_mut() {
                Some(MessagePartDetailResource::Reasoning(reasoning)) => {
                    reasoning.raw_content.push(delta.delta.clone());
                    true
                }
                _ => false,
            },
            agena_domain::PartDeltaField::CommandStdout
            | agena_domain::PartDeltaField::CommandStderr => {
                append_operation_output_delta(part, delta.delta.as_str())
            }
            agena_domain::PartDeltaField::ToolOutputText => {
                append_operation_output_delta(part, delta.delta.as_str())
            }
            agena_domain::PartDeltaField::Custom { .. } => false,
        };
        if !updated {
            return Err(());
        }
        self.invalidate_render();
        Ok(())
    }

    /// Resolve a live event by its durable provider-message or part identity.
    /// Conversation-turn aggregation is handled separately from this lookup.
    fn live_message_target(&self, message_id: i64, part_id: Option<i64>) -> Option<usize> {
        if let Some(part_id) = part_id
            && let Some(index) = self.messages.iter().position(|message| {
                message
                    .parts
                    .as_ref()
                    .is_some_and(|parts| parts.iter().any(|part| part.id == part_id))
            })
        {
            return Some(index);
        }
        if let Some(index) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        {
            return Some(index);
        }
        None
    }

    pub(crate) fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.search_match_index = None;
        self.invalidate_render();
    }

    pub(crate) fn current_search_match_count(&self) -> usize {
        self.rendered
            .as_ref()
            .map(|rendered| rendered.search_matches.len())
            .unwrap_or(0)
    }

    pub(crate) fn current_search_match_number(&self) -> usize {
        match (self.search_match_index, self.current_search_match_count()) {
            (Some(index), count) if count > 0 => min(index + 1, count),
            _ => 0,
        }
    }

    pub(crate) fn jump_search_match(&mut self, width: u16, height: u16, forward: bool) {
        let matches = self.rendered(width).search_matches.clone();
        if matches.is_empty() {
            self.search_match_index = None;
            return;
        }

        let anchor_line = self.interaction_line().unwrap_or(self.viewport.top);
        let next_index = match (self.search_match_index, forward) {
            (None, direction) => {
                initial_search_match_index(matches.as_slice(), anchor_line, direction)
            }
            (Some(index), true) => (index + 1) % matches.len(),
            (Some(0), false) => matches.len().saturating_sub(1),
            (Some(index), false) => index.saturating_sub(1),
        };

        self.search_match_index = Some(next_index);
        let line = matches[next_index];
        self.set_cursor_line_with_reveal(width, height, line, TranscriptRevealPolicy::Center);
    }

    pub(crate) fn jump_to_message(&mut self, width: u16, height: u16, message_id: i64) {
        let rendered = self.rendered(width);
        let Some((_, line)) = rendered
            .message_line_starts
            .iter()
            .find(|(candidate_id, _)| *candidate_id == message_id)
            .copied()
        else {
            return;
        };
        self.set_cursor_line_with_reveal(width, height, line, TranscriptRevealPolicy::Center);
    }

    pub(crate) fn highlighted_block_key(&self) -> Option<TranscriptNodeKey> {
        self.interaction
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.block_cursor.as_ref())
            .map(|cursor| cursor.key.clone())
    }

    pub(crate) fn highlighted_block_range(&mut self, width: u16) -> Option<Range<usize>> {
        let key = self.highlighted_block_key()?;
        let rendered = self.rendered(width);
        transcript_node_highlight_range(rendered.nodes.as_slice(), &key)
    }

    pub(crate) fn highlighted_line_range(&mut self, width: u16) -> Option<Range<usize>> {
        let cursor = self.interaction.cursor.as_ref()?;
        if cursor.block_cursor.is_some() {
            return None;
        }
        let line = cursor.line;
        let rendered = self.rendered(width);
        let node = rendered
            .line_nodes
            .get(line)
            .and_then(|index| *index)
            .and_then(|index| rendered.nodes.get(index))?;
        transcript_semantic_line_range(rendered.lines.as_slice(), node, line)
    }

    pub(crate) fn move_cursor_one_line(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
    ) {
        let Some((cursor_line, selected_cursor)) = self.navigation_parts() else {
            return;
        };
        let step = {
            let rendered = self.rendered(width);
            if selected_cursor.is_some() {
                transcript_vertical_navigation_step(
                    rendered.nodes.as_slice(),
                    rendered.lines.as_slice(),
                    cursor_line,
                    selected_cursor.as_ref(),
                    direction,
                )
            } else {
                transcript_vertical_line_navigation_step(
                    rendered.nodes.as_slice(),
                    rendered.lines.as_slice(),
                    cursor_line,
                    direction,
                )
                .or_else(|| {
                    transcript_should_fall_back_to_message_navigation(
                        rendered.nodes.as_slice(),
                        cursor_line,
                    )
                    .then(|| {
                        transcript_vertical_navigation_step(
                            rendered.nodes.as_slice(),
                            rendered.lines.as_slice(),
                            cursor_line,
                            None,
                            direction,
                        )
                    })
                    .flatten()
                })
            }
        };
        match step {
            Some(TranscriptVerticalNavigationStep::SelectNode { node_index, mode }) => {
                self.set_block_cursor_with_mode(width, height, node_index, direction, mode);
            }
            Some(TranscriptVerticalNavigationStep::MoveToLine(line)) => {
                self.set_cursor_line(width, height, line);
            }
            None => {}
        }
    }

    pub(crate) fn step_block(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
    ) {
        let Some((cursor_line, selected_cursor)) = self.navigation_parts() else {
            return;
        };
        let selected_key = selected_cursor.map(|cursor| cursor.key);
        let target_node = {
            let rendered = self.rendered(width);
            transcript_message_navigation_target(
                rendered.nodes.as_slice(),
                cursor_line,
                selected_key.as_ref(),
                direction,
            )
        };
        if let Some(target_node) = target_node {
            self.set_block_cursor(width, height, target_node, direction);
        }
    }

    pub(crate) fn move_by_blocks(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        for _ in 0..count.max(1) {
            self.step_block(width, height, direction);
        }
    }

    pub(crate) fn move_cursor_by_lines(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        for _ in 0..count.max(1) {
            self.move_cursor_one_line(width, height, direction);
        }
    }

    pub(crate) fn should_load_older(&self) -> bool {
        self.session_id.is_some()
            && self
                .viewport
                .history_effect(
                    self.has_more_older,
                    self.loading_initial || self.loading_older,
                )
                .request_older_messages
    }

    pub(crate) fn rendered(&mut self, width: u16) -> &RenderedTranscript {
        let context = self.math_render_context.clone();
        agena_tui_media::with_math_render_context(&context, || self.rendered_inner(width))
    }

    fn rendered_inner(&mut self, width: u16) -> &RenderedTranscript {
        let palette = agena_tui_components::theme::active_palette();
        let remote_image_generation = agena_tui_media::remote_image_generation();
        if self.rendered.as_ref().is_some_and(|rendered| {
            rendered.width == width
                && rendered.palette == palette
                && rendered.remote_image_generation == remote_image_generation
        }) {
            return self.rendered.as_ref().expect("render cache should exist");
        }

        // Terminal-cell endpoints cannot be carried safely through Markdown
        // reflow. Keep the semantic cursor anchor, but retire the transient
        // range instead of highlighting unrelated cells after a resize.
        if self
            .rendered
            .as_ref()
            .is_some_and(|rendered| rendered.width != width)
        {
            self.interaction.text_selection = None;
        }

        let mut lines = Vec::new();
        let mut message_line_starts = Vec::new();
        let mut nodes = Vec::new();
        let mut line_nodes = Vec::new();
        if self.session_id.is_some() {
            if self.loading_older {
                lines.push(
                    RenderedLine::dim(ui_text::t(&self.i18n, "transcript-loading-older"))
                        .with_copy_projection(String::new(), 0),
                );
                line_nodes.push(None);
            } else if self.has_more_older {
                lines.push(
                    RenderedLine::dim(ui_text::t(&self.i18n, "transcript-more-older"))
                        .with_copy_projection(String::new(), 0),
                );
                line_nodes.push(None);
            }
        }

        if self.messages.is_empty()
            && self.pending_user_messages.is_empty()
            && self.session_id.is_some()
            && !self.loading_initial
        {
            lines.push(
                RenderedLine::dim(ui_text::t(&self.i18n, "transcript-empty-session"))
                    .with_copy_projection(String::new(), 0),
            );
            line_nodes.push(None);
        }

        for message in &self.messages {
            message_line_starts.push((message.id, lines.len()));
            let rendered = render_message_detailed(
                message,
                width,
                &self.i18n,
                self.detail_expanded_by_default,
                &self.node_expansions,
            );
            let base_line = lines.len();
            let base_node = nodes.len();
            lines.extend(rendered.lines);
            nodes.extend(
                rendered
                    .nodes
                    .into_iter()
                    .map(|node| RenderedTranscriptNode {
                        start_line: node.start_line.saturating_add(base_line),
                        end_line: node.end_line.saturating_add(base_line),
                        ..node
                    }),
            );
            let added_lines = lines.len().saturating_sub(base_line);
            let message_copy_text = nodes[base_node..]
                .iter()
                .filter(|node| node.contributes_to_aggregate_copy())
                .map(|node| node.copy_text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            line_nodes.extend((0..added_lines).map(|offset| {
                nodes
                    .iter()
                    .enumerate()
                    .skip(base_node)
                    .find(|(_, node)| {
                        let line_index = base_line.saturating_add(offset);
                        line_index >= node.start_line && line_index < node.end_line
                    })
                    .map(|(index, _)| index)
            }));
            // Keep a message-level parent after its leaf nodes. `line_nodes`
            // deliberately continues to point at leaves, so line navigation
            // stays precise while h/l can first select the whole reply.
            nodes.push(RenderedTranscriptNode {
                key: TranscriptNodeKey::Message {
                    message_id: message.id,
                },
                kind: TranscriptNodeKind::Message,
                start_line: base_line,
                end_line: lines.len(),
                copy_text: message_copy_text,
                atomic: false,
                toggleable: false,
                expanded: true,
            });
        }

        for message in &self.pending_user_messages {
            let status = if message.confirmed {
                String::new()
            } else {
                format!(" {}", transcript_spinner_placeholder())
            };
            lines.push(
                RenderedLine::plain(
                    format!(
                        "{}{status}",
                        ui_text::role_label(&self.i18n, MessageRole::User)
                    ),
                    style_for_role(MessageRole::User).add_modifier(Modifier::BOLD),
                )
                .with_copy_projection(String::new(), 0),
            );
            for block in markdown_blocks(message.text.as_str()) {
                render_markdown_block(&mut lines, "  ", &block, width);
            }
            line_nodes.extend(std::iter::repeat_n(
                None,
                lines.len().saturating_sub(line_nodes.len()),
            ));
        }

        let search_matches = if self.search_query.trim().is_empty() {
            Vec::new()
        } else {
            lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    contains_case_insensitive(&line.text, self.search_query.as_str())
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };

        let math = lines
            .iter()
            .enumerate()
            .flat_map(|(line, rendered)| {
                rendered.math.iter().map(move |placement| {
                    agena_tui_media::TranscriptMathPlacement {
                        line,
                        column: placement.column,
                        artifact: std::sync::Arc::clone(&placement.artifact),
                        size: placement.size,
                    }
                })
            })
            .collect();

        self.rendered = Some(RenderedTranscript {
            width,
            palette,
            remote_image_generation,
            lines,
            search_matches,
            message_line_starts,
            nodes,
            line_nodes,
            math,
        });
        self.rendered.as_ref().expect("render cache should exist")
    }

    pub(crate) fn invalidate_render(&mut self) {
        self.rendered = None;
    }

    pub(crate) fn viewport_top(&self) -> usize {
        self.viewport.top
    }

    pub(crate) fn ensure_visual_focus(&mut self, width: u16, height: u16) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            self.viewport.top = 0;
            self.viewport.follow_tail = true;
            return;
        }
        if self.viewport.follow_tail {
            let last_line = self.focusable_line_near(width, total_lines.saturating_sub(1), false);
            self.viewport.top = self.max_scroll(width, height);
            self.install_cursor(width, height, last_line, None, false);
            self.viewport.follow_tail = true;
            return;
        }

        self.viewport.top = self.viewport.top.min(self.max_scroll(width, height));
        self.reconcile_cursor_anchor(width, height);
        if self.interaction.cursor.is_none() {
            let target = self.focusable_line_near(width, self.viewport.top, true);
            self.install_cursor(width, height, target, None, false);
        }
        let preferred_row = self
            .interaction
            .cursor
            .as_ref()
            .map(|cursor| cursor.preferred_screen_row)
            .unwrap_or(0);
        self.reveal_current_cursor(
            width,
            height,
            TranscriptRevealPolicy::PreserveScreenRow(preferred_row),
        );
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn interaction_line(&self) -> Option<usize> {
        self.interaction.cursor.as_ref().map(|cursor| cursor.line)
    }

    pub(crate) fn navigation_cursor_line(&self) -> Option<usize> {
        self.interaction_line()
    }

    pub(crate) fn current_selected_line_text(&mut self, width: u16) -> Option<String> {
        let cursor = self.interaction.cursor.as_ref()?;
        if cursor.block_cursor.is_some() {
            return None;
        }
        let cursor_line = cursor.line;
        let rendered = self.rendered(width);
        if rendered
            .line_nodes
            .get(cursor_line)
            .and_then(|index| *index)
            .and_then(|index| rendered.nodes.get(index))
            .is_some_and(|node| node.atomic)
        {
            return None;
        }
        let line = rendered.lines.get(cursor_line)?;
        if line.navigation_unit.is_some() {
            return Some(line.navigation_copy_text.clone());
        }
        (!line.copy_text.is_empty()).then(|| line.copy_text.clone())
    }

    pub(crate) fn has_navigation_target(&self) -> bool {
        self.interaction.cursor.is_some()
    }

    pub(crate) fn text_selection(&self) -> Option<TranscriptTextSelection> {
        self.interaction.text_selection
    }

    fn navigation_parts(&self) -> Option<(usize, Option<TranscriptBlockCursor>)> {
        self.interaction
            .cursor
            .as_ref()
            .map(|cursor| (cursor.line, cursor.block_cursor.clone()))
    }

    fn shift_interaction_lines(&mut self, added_lines: usize) {
        if let Some(cursor) = &mut self.interaction.cursor {
            cursor.line = cursor.line.saturating_add(added_lines);
        }
        if let Some(selection) = &mut self.interaction.text_selection {
            selection.anchor.line = selection.anchor.line.saturating_add(added_lines);
            selection.head.line = selection.head.line.saturating_add(added_lines);
        }
    }

    pub(crate) fn clamp_scroll(&mut self, width: u16, height: u16) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            self.viewport.top = 0;
            self.viewport.follow_tail = true;
            return;
        }
        self.viewport.top = self.viewport.top.min(self.max_scroll(width, height));
        self.reconcile_cursor_anchor(width, height);
        let last_line = total_lines.saturating_sub(1);
        if let Some(selection) = &mut self.interaction.text_selection {
            selection.anchor.line = selection.anchor.line.min(last_line);
            selection.head.line = selection.head.line.min(last_line);
        }
        if self.interaction.cursor.is_none() {
            let target = self.focusable_line_near(width, self.viewport.top, true);
            self.install_cursor(width, height, target, None, false);
        }
        let preferred_row = self
            .interaction
            .cursor
            .as_ref()
            .map(|cursor| cursor.preferred_screen_row)
            .unwrap_or(0);
        self.reveal_current_cursor(
            width,
            height,
            TranscriptRevealPolicy::PreserveScreenRow(preferred_row),
        );
        self.sync_follow_tail(width.max(1), height.max(1));
    }

    pub(crate) fn scroll_to_bottom(&mut self, width: u16, height: u16) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            self.viewport.top = 0;
            self.viewport.follow_tail = true;
            return;
        }
        let target = self.focusable_line_near(width, total_lines.saturating_sub(1), false);
        self.install_cursor(width, height, target, None, true);
        self.viewport.top = self.max_scroll(width, height);
        self.refresh_cursor_screen_row(height);
        self.viewport.follow_tail = true;
    }

    pub(crate) fn scroll_to_top(&mut self, width: u16, height: u16) {
        if self.rendered(width).lines.is_empty() {
            self.interaction = TranscriptInteraction::default();
            return;
        }
        let target = self.focusable_line_near(width, 0, true);
        self.install_cursor(width, height, target, None, true);
        self.viewport.top = 0;
        self.refresh_cursor_screen_row(height);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn move_cursor_by_wheel(&mut self, width: u16, height: u16, delta: isize) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.as_ref() else {
            return;
        };
        let cursor_line = cursor.line;
        let screen_row = cursor_line.saturating_sub(self.viewport.top);
        let direction = if delta.is_negative() {
            TranscriptMoveDirection::Up
        } else {
            TranscriptMoveDirection::Down
        };
        let target =
            self.focusable_line_after_steps(width, cursor_line, direction, delta.unsigned_abs());
        self.set_cursor_line_with_reveal(
            width,
            height,
            target,
            TranscriptRevealPolicy::PreserveScreenRow(screen_row),
        );
    }

    pub(crate) fn relocate_cursor_from_scrollbar(
        &mut self,
        width: u16,
        height: u16,
        requested_top: usize,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            return;
        }
        let visible = usize::from(height.max(1));
        let target_top = requested_top.min(total_lines.saturating_sub(visible));
        let direction = if target_top < self.viewport.top {
            TranscriptMoveDirection::Up
        } else {
            TranscriptMoveDirection::Down
        };
        let inset = usize::from(visible > 2);
        let raw_line = match direction {
            TranscriptMoveDirection::Down => target_top.saturating_add(inset),
            TranscriptMoveDirection::Up => target_top
                .saturating_add(visible.saturating_sub(1).saturating_sub(inset))
                .min(total_lines.saturating_sub(1)),
        };
        let line = self.focusable_line_in_viewport(width, target_top, visible, raw_line, direction);
        self.install_cursor(width, height, line, None, true);
        self.viewport.reduce(TranscriptAction::ScrollTo(target_top));
        self.refresh_cursor_screen_row(height);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn scroll_text_selection_viewport_by(
        &mut self,
        width: u16,
        height: u16,
        delta: isize,
    ) {
        let max_scroll = self.max_scroll(width, height);
        let top = if delta.is_negative() {
            self.viewport.top.saturating_sub(delta.unsigned_abs())
        } else {
            self.viewport
                .top
                .saturating_add(delta as usize)
                .min(max_scroll)
        };
        self.viewport.reduce(TranscriptAction::ScrollTo(top));
    }

    pub(crate) fn move_cursor_by_page(&mut self, width: u16, height: u16, forward: bool) {
        self.move_cursor_by_distance(width, height, usize::from(height.max(1)), forward);
    }

    pub(crate) fn move_cursor_by_half_page(&mut self, width: u16, height: u16, forward: bool) {
        let distance = usize::from(height.max(1)).saturating_add(1) / 2;
        self.move_cursor_by_distance(width, height, distance, forward);
    }

    fn move_cursor_by_distance(&mut self, width: u16, height: u16, distance: usize, forward: bool) {
        self.ensure_visual_focus(width, height);
        let Some(line) = self.navigation_cursor_line() else {
            return;
        };
        let direction = if forward {
            TranscriptMoveDirection::Down
        } else {
            TranscriptMoveDirection::Up
        };
        let target = if forward {
            line.saturating_add(distance)
        } else {
            line.saturating_sub(distance)
        };
        let target = self.focusable_line_near(width, target, forward);
        self.set_cursor_line_with_reveal(
            width,
            height,
            target,
            TranscriptRevealPolicy::DirectionalEdge(direction),
        );
    }

    pub(crate) fn max_scroll(&mut self, width: u16, height: u16) -> usize {
        let visible = height.max(1) as usize;
        self.rendered(width).lines.len().saturating_sub(visible)
    }

    pub(crate) fn set_cursor_line(&mut self, width: u16, height: u16, target: usize) {
        self.set_cursor_line_with_reveal(width, height, target, TranscriptRevealPolicy::Minimal);
    }

    fn set_cursor_line_with_reveal(
        &mut self,
        width: u16,
        height: u16,
        target: usize,
        reveal: TranscriptRevealPolicy,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            return;
        }
        self.install_cursor(
            width,
            height,
            target.min(total_lines.saturating_sub(1)),
            None,
            true,
        );
        self.reveal_current_cursor(width, height, reveal);
        self.sync_follow_tail(width, height);
    }

    /// Commit a raw terminal-cell range without changing the permanent
    /// navigation cursor. Only graphical/atomic endpoints are expanded;
    /// keyboard semantic rows such as code and table rows remain character
    /// selectable.
    pub(crate) fn set_text_selection(
        &mut self,
        width: u16,
        selection: TranscriptTextSelection,
    ) -> TranscriptTextSelection {
        let selection = {
            let rendered = self.rendered(width);
            normalize_transcript_text_selection(
                selection,
                rendered.lines.as_slice(),
                rendered.nodes.as_slice(),
                rendered.line_nodes.as_slice(),
            )
        };
        self.interaction.text_selection = Some(selection);
        self.viewport.follow_tail = false;
        selection
    }

    pub(crate) fn cancel_text_selection(&mut self, width: u16, height: u16) {
        self.interaction.text_selection = None;
        self.reveal_current_cursor(width.max(1), height.max(1), TranscriptRevealPolicy::Minimal);
        self.sync_follow_tail(width.max(1), height.max(1));
    }

    pub(crate) fn select_pointer_line(
        &mut self,
        width: u16,
        height: u16,
        position: TranscriptTextPosition,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if position.line >= total_lines {
            return;
        }
        let atomic_node = {
            let rendered = self.rendered(width);
            rendered
                .line_nodes
                .get(position.line)
                .and_then(|index| *index)
                .filter(|index| rendered.nodes.get(*index).is_some_and(|node| node.atomic))
        };
        if let Some(node_index) = atomic_node {
            self.set_block_cursor(width, height, node_index, TranscriptMoveDirection::Down);
            return;
        }
        let line_is_focusable = {
            let rendered = self.rendered(width);
            transcript_rendered_line_is_focusable(rendered, position.line)
        };
        let line = if line_is_focusable {
            position.line
        } else {
            self.focusable_line_near(width, position.line, true)
        };
        self.install_cursor(width, height, line, None, true);
        self.reveal_current_cursor(width, height, TranscriptRevealPolicy::Minimal);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn select_pointer_block(
        &mut self,
        width: u16,
        height: u16,
        position: TranscriptTextPosition,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if position.line >= total_lines {
            return;
        }
        let node_index = {
            let rendered = self.rendered(width);
            rendered
                .line_nodes
                .get(position.line)
                .and_then(|value| *value)
                .or_else(|| {
                    rendered.nodes.iter().enumerate().find_map(|(index, node)| {
                        (node.key.is_message_container()
                            && position.line >= node.start_line
                            && position.line < node.end_line)
                            .then_some(index)
                    })
                })
        };
        let block_cursor = node_index.and_then(|index| {
            self.rendered(width)
                .nodes
                .get(index)
                .map(|node| TranscriptBlockCursor {
                    key: node.key.clone(),
                    direction: TranscriptMoveDirection::Down,
                    mode: TranscriptBlockSelectionMode::Direct,
                })
        });
        let line_is_focusable = {
            let rendered = self.rendered(width);
            transcript_rendered_line_is_focusable(rendered, position.line)
        };
        let line = if line_is_focusable {
            position.line
        } else {
            self.focusable_line_near(width, position.line, true)
        };
        self.install_cursor(width, height, line, block_cursor, true);
        self.reveal_current_cursor(width, height, TranscriptRevealPolicy::Minimal);
        self.sync_follow_tail(width, height);
    }

    fn install_cursor(
        &mut self,
        width: u16,
        height: u16,
        target: usize,
        block_cursor: Option<TranscriptBlockCursor>,
        clear_text_selection: bool,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction.cursor = None;
            return;
        }
        let line = target.min(total_lines.saturating_sub(1));
        let anchor = {
            let rendered = self.rendered(width);
            rendered
                .line_nodes
                .get(line)
                .and_then(|index| *index)
                .and_then(|index| rendered.nodes.get(index))
                .map(|node| TranscriptCursorAnchor {
                    key: node.key.clone(),
                    line_offset: line.saturating_sub(node.start_line),
                })
        };
        let preferred_screen_row = line
            .saturating_sub(self.viewport.top)
            .min(usize::from(height.max(1)).saturating_sub(1));
        self.interaction.cursor = Some(TranscriptCursor {
            line,
            anchor,
            block_cursor,
            preferred_screen_row,
        });
        if clear_text_selection {
            self.interaction.text_selection = None;
        }
    }

    fn reconcile_cursor_anchor(&mut self, width: u16, height: u16) {
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction.cursor = None;
            return;
        }
        let (line, block_cursor) = {
            let rendered = self.rendered(width);
            let line = cursor
                .anchor
                .as_ref()
                .and_then(|anchor| {
                    rendered
                        .nodes
                        .iter()
                        .find(|node| node.key == anchor.key)
                        .map(|node| {
                            node.start_line.saturating_add(
                                anchor.line_offset.min(
                                    node.end_line
                                        .saturating_sub(node.start_line)
                                        .saturating_sub(1),
                                ),
                            )
                        })
                })
                .unwrap_or(cursor.line.min(total_lines.saturating_sub(1)));
            let block_cursor = cursor
                .block_cursor
                .filter(|block| rendered.nodes.iter().any(|node| node.key == block.key));
            (line, block_cursor)
        };
        self.install_cursor(width, height, line, block_cursor, false);
        if let Some(current) = &mut self.interaction.cursor {
            current.preferred_screen_row = cursor
                .preferred_screen_row
                .min(usize::from(height.max(1)).saturating_sub(1));
        }
    }

    fn reveal_current_cursor(&mut self, width: u16, height: u16, policy: TranscriptRevealPolicy) {
        let Some(cursor) = self.interaction.cursor.as_ref() else {
            return;
        };
        let line = cursor.line;
        let visible = usize::from(height.max(1));
        let max_scroll = self.max_scroll(width, height);
        let target_top = match policy {
            TranscriptRevealPolicy::Minimal if line < self.viewport.top => line,
            TranscriptRevealPolicy::Minimal
                if line >= self.viewport.top.saturating_add(visible) =>
            {
                line.saturating_add(1).saturating_sub(visible)
            }
            TranscriptRevealPolicy::Minimal => self.viewport.top,
            TranscriptRevealPolicy::PreserveScreenRow(row) => line.saturating_sub(row),
            TranscriptRevealPolicy::DirectionalEdge(direction) => {
                let inset = usize::from(visible > 2);
                match direction {
                    TranscriptMoveDirection::Down => line.saturating_sub(inset),
                    TranscriptMoveDirection::Up => {
                        line.saturating_sub(visible.saturating_sub(1).saturating_sub(inset))
                    }
                }
            }
            TranscriptRevealPolicy::Center => line.saturating_sub(visible / 2),
        };
        self.viewport.top = target_top.min(max_scroll);
        self.refresh_cursor_screen_row(height);
    }

    fn refresh_cursor_screen_row(&mut self, height: u16) {
        if let Some(cursor) = &mut self.interaction.cursor {
            cursor.preferred_screen_row = cursor
                .line
                .saturating_sub(self.viewport.top)
                .min(usize::from(height.max(1)).saturating_sub(1));
        }
    }

    fn focusable_line_near(&mut self, width: u16, target: usize, prefer_after: bool) -> usize {
        let rendered = self.rendered(width);
        let last = rendered.lines.len().saturating_sub(1);
        let target = target.min(last);
        if prefer_after {
            (target..rendered.lines.len())
                .find(|line| transcript_rendered_line_is_focusable(rendered, *line))
                .or_else(|| {
                    (0..target)
                        .rev()
                        .find(|line| transcript_rendered_line_is_focusable(rendered, *line))
                })
        } else {
            (0..=target)
                .rev()
                .find(|line| transcript_rendered_line_is_focusable(rendered, *line))
                .or_else(|| {
                    (target.saturating_add(1)..rendered.lines.len())
                        .find(|line| transcript_rendered_line_is_focusable(rendered, *line))
                })
        }
        .unwrap_or(target)
    }

    fn focusable_line_after_steps(
        &mut self,
        width: u16,
        start: usize,
        direction: TranscriptMoveDirection,
        count: usize,
    ) -> usize {
        let rendered = self.rendered(width);
        let mut line = start.min(rendered.lines.len().saturating_sub(1));
        for _ in 0..count {
            let next = match direction {
                TranscriptMoveDirection::Up => (0..line)
                    .rev()
                    .find(|candidate| transcript_rendered_line_is_focusable(rendered, *candidate)),
                TranscriptMoveDirection::Down => (line.saturating_add(1)..rendered.lines.len())
                    .find(|candidate| transcript_rendered_line_is_focusable(rendered, *candidate)),
            };
            let Some(next) = next else {
                break;
            };
            line = next;
        }
        line
    }

    fn focusable_line_in_viewport(
        &mut self,
        width: u16,
        top: usize,
        visible: usize,
        target: usize,
        direction: TranscriptMoveDirection,
    ) -> usize {
        let follow_tail = self.viewport.follow_tail;
        let rendered = self.rendered(width);
        let view = agena_tui::transcript::project_view(
            &TranscriptViewport { top, follow_tail },
            rendered.lines.len(),
            visible,
        );
        view.visible
            .into_iter()
            .filter(|line| transcript_rendered_line_is_focusable(rendered, *line))
            .min_by_key(|line| {
                (
                    line.abs_diff(target),
                    usize::from(match direction {
                        TranscriptMoveDirection::Down => *line < target,
                        TranscriptMoveDirection::Up => *line > target,
                    }),
                )
            })
            .unwrap_or(target.min(rendered.lines.len().saturating_sub(1)))
    }

    pub(crate) fn current_cursor_node(&mut self, width: u16) -> Option<&RenderedTranscriptNode> {
        let node_index = self.current_highlighted_node_index(width)?;
        let rendered = self.rendered(width);
        rendered.nodes.get(node_index)
    }

    pub(crate) fn current_cursor_node_cloned(
        &mut self,
        width: u16,
    ) -> Option<RenderedTranscriptNode> {
        self.current_cursor_node(width).cloned()
    }

    /// Toggle the activity under the cursor and keep the cursor attached to it.
    ///
    /// A cursor can be several rendered lines into an expanded activity. Once
    /// that activity is collapsed, retaining the old absolute line would point
    /// at an unrelated node later in the transcript. Preserve the cursor's
    /// relative row when possible and clamp it to the node's new range.
    pub(crate) fn toggle_cursor_node_expansion(
        &mut self,
        width: u16,
        height: u16,
    ) -> Option<(TranscriptNodeKind, bool)> {
        let node = self.current_cursor_node_cloned(width)?;
        if !node.toggleable {
            return None;
        }

        let (cursor_line, block_cursor) = self.navigation_parts()?;
        let cursor_offset = cursor_line.saturating_sub(node.start_line).min(
            node.end_line
                .saturating_sub(node.start_line)
                .saturating_sub(1),
        );
        let block_cursor = block_cursor.filter(|cursor| cursor.key == node.key);
        let selection_direction = block_cursor
            .as_ref()
            .map(|cursor| cursor.direction)
            .unwrap_or(TranscriptMoveDirection::Down);
        let expanded = !node.expanded;
        self.node_expansions.insert(node.key.clone(), expanded);
        self.invalidate_render();

        let (start_line, end_line) = {
            let rendered = self.rendered(width);
            let rerendered = rendered.nodes.iter().find(|item| item.key == node.key)?;
            (rerendered.start_line, rerendered.end_line)
        };
        let target_line = start_line.saturating_add(
            cursor_offset.min(end_line.saturating_sub(start_line).saturating_sub(1)),
        );
        let total_lines = self.rendered(width).lines.len();
        self.viewport.top = transcript_selection_scroll_position(
            total_lines,
            start_line,
            end_line,
            height.max(1) as usize,
            self.viewport.top,
            selection_direction,
        );
        self.install_cursor(width, height, target_line, block_cursor, true);
        self.refresh_cursor_screen_row(height);
        self.sync_follow_tail(width, height);

        Some((node.kind, expanded))
    }

    pub(crate) fn current_highlighted_node_index(&mut self, width: u16) -> Option<usize> {
        let (cursor_line, block_cursor) = self.navigation_parts()?;
        if let Some(block_cursor) = block_cursor {
            let highlighted_key = block_cursor.key;
            let block_index = {
                let rendered = self.rendered(width);
                rendered
                    .nodes
                    .iter()
                    .position(|node| node.key == highlighted_key)
            };
            if let Some(index) = block_index {
                return Some(index);
            }
            if let Some(cursor) = &mut self.interaction.cursor {
                cursor.block_cursor = None;
            }
        }
        let rendered = self.rendered(width);
        rendered
            .line_nodes
            .get(cursor_line)
            .and_then(|value| *value)
    }

    pub(crate) fn set_block_cursor(
        &mut self,
        width: u16,
        height: u16,
        node_index: usize,
        direction: TranscriptMoveDirection,
    ) {
        self.set_block_cursor_with_mode(
            width,
            height,
            node_index,
            direction,
            TranscriptBlockSelectionMode::Entering,
        );
    }

    pub(crate) fn set_block_cursor_with_mode(
        &mut self,
        width: u16,
        height: u16,
        node_index: usize,
        direction: TranscriptMoveDirection,
        mode: TranscriptBlockSelectionMode,
    ) {
        let (start_line, end_line, target_line, key) = {
            let rendered = self.rendered(width);
            let Some(node) = rendered.nodes.get(node_index) else {
                return;
            };
            let target_line = match direction {
                TranscriptMoveDirection::Up => node.end_line.saturating_sub(1),
                TranscriptMoveDirection::Down => node.start_line,
            };
            (
                node.start_line,
                node.end_line,
                target_line,
                node.key.clone(),
            )
        };
        let total_lines = self.rendered(width).lines.len();
        self.viewport.top = transcript_selection_scroll_position(
            total_lines,
            start_line,
            end_line,
            height.max(1) as usize,
            self.viewport.top,
            direction,
        );
        self.install_cursor(
            width,
            height,
            target_line,
            Some(TranscriptBlockCursor {
                key,
                direction,
                mode,
            }),
            true,
        );
        self.refresh_cursor_screen_row(height);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn sync_follow_tail(&mut self, width: u16, height: u16) {
        let total_lines = self.rendered(width).lines.len();
        let last_line = self.focusable_line_near(width, total_lines.saturating_sub(1), false);
        self.viewport.follow_tail = self.text_selection().is_none()
            && self.navigation_cursor_line() == Some(last_line)
            && self.viewport.top >= self.max_scroll(width, height);
    }
}

fn same_assistant_model_route(existing: &MessageMetadata, next: &MessageMetadata) -> bool {
    existing.model_provider_id == next.model_provider_id
        && existing.model_adapter_id == next.model_adapter_id
        && existing.model_id == next.model_id
}

fn timestamp_ms_or(
    timestamp_ms: i64,
    fallback: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms).unwrap_or(fallback)
}

const fn message_role_from_domain(role: agena_domain::Role) -> MessageRole {
    match role {
        agena_domain::Role::User => MessageRole::User,
        agena_domain::Role::Assistant => MessageRole::Assistant,
        agena_domain::Role::System => MessageRole::System,
        agena_domain::Role::Tool => MessageRole::Tool,
    }
}

const fn message_status_from_domain(status: agena_domain::ExecutionStatus) -> MessageStatus {
    match status {
        agena_domain::ExecutionStatus::Pending => MessageStatus::Pending,
        agena_domain::ExecutionStatus::InProgress => MessageStatus::InProgress,
        agena_domain::ExecutionStatus::Completed => MessageStatus::Completed,
        agena_domain::ExecutionStatus::Failed => MessageStatus::Failed,
        agena_domain::ExecutionStatus::Cancelled => MessageStatus::Cancelled,
    }
}

fn message_metadata_from_runtime(value: &agena_runtime::RuntimeMessageMetadata) -> MessageMetadata {
    MessageMetadata {
        source: match value.source {
            agena_domain::MessageSource::User => MessageSource::User,
            agena_domain::MessageSource::Assistant => MessageSource::Assistant,
            agena_domain::MessageSource::System => MessageSource::System,
        },
        turn_id: value.turn_id,
        parent_message_id: value.parent_message_id,
        generated_by_call_id: value.generated_by_call_id,
        model_provider_id: value.model_provider_id.clone(),
        model_adapter_id: value.model_adapter_id.clone(),
        model_id: value.model_id.clone(),
        model_thinking_mode: value.model_thinking_mode.clone(),
        model_speed_mode: value.model_speed_mode.clone(),
    }
}

fn transcript_rendered_line_is_focusable(rendered: &RenderedTranscript, line: usize) -> bool {
    let rendered_line = rendered.lines.get(line);
    rendered
        .line_nodes
        .get(line)
        .and_then(|index| *index)
        .and_then(|index| rendered.nodes.get(index))
        .is_some_and(|node| node.atomic)
        || rendered_line
            .is_some_and(|line| line.navigation_unit.is_some() || !line.copy_text.is_empty())
}

fn append_operation_output_delta(part: &mut crate::MessagePartResource, delta: &str) -> bool {
    let Some(crate::MessagePartDetailResource::Operation(operation)) = part.content.as_mut() else {
        return false;
    };
    operation.model_output.text.push_str(delta);
    operation.result.state = agena_api::message_part::ToolResultStateResource::Running;
    operation.result.model_preview.text.push_str(delta);
    if operation.summary.is_empty() {
        operation.summary.push_str(delta);
    }
    if operation.result.display.summary.is_empty() {
        operation.result.display.summary.push_str(delta);
    }
    append_text_operation_block(&mut operation.blocks, delta);
    append_text_operation_block(&mut operation.result.content, delta);
    true
}

fn append_text_operation_block(blocks: &mut Vec<crate::OperationBlockResource>, delta: &str) {
    match blocks.last_mut() {
        Some(crate::OperationBlockResource::Text { text }) => text.push_str(delta),
        _ => blocks.push(crate::OperationBlockResource::Text {
            text: delta.to_owned(),
        }),
    }
}

use super::TranscriptAction;
use super::transcript_view::style_for_role;
use crate::{
    BTreeMap, HashSet, I18n, MessagePartDetailResource, MessageResource, MessageRole,
    MessageStatus, Modifier, PaginatedResponse, PartExecutionStatusResource, PendingUserMessage,
    Range, RenderedLine, RenderedTranscript, RenderedTranscriptNode, SessionExecutionResource,
    TranscriptBlockCursor, TranscriptBlockSelectionMode, TranscriptCursor, TranscriptCursorAnchor,
    TranscriptDetailDefaults, TranscriptInteraction, TranscriptMoveDirection, TranscriptNodeKey,
    TranscriptNodeKind, TranscriptState, TranscriptTextPosition, TranscriptTextSelection,
    TranscriptVerticalNavigationStep, TranscriptViewport, contains_case_insensitive,
    initial_search_match_index, markdown_blocks, merge_message_resources, message_sort_key, min,
    normalize_transcript_text_selection, render_markdown_block, render_message_detailed,
    transcript_message_navigation_target, transcript_node_highlight_range,
    transcript_selection_scroll_position, transcript_semantic_line_range,
    transcript_should_fall_back_to_message_navigation, transcript_spinner_placeholder,
    transcript_vertical_line_navigation_step, transcript_vertical_navigation_step, ui_text,
};
use agena_api::resource::{MessageMetadata, MessageSource};
use agena_application::message_part_resource_from_runtime;
