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
    pub(in crate::app) fn new(
        i18n: I18n,
        detail_expanded_by_default: TranscriptDetailDefaults,
    ) -> Self {
        Self {
            i18n,
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
            submitting: false,
            pending_restore_draft: None,
            follow_tail: true,
            scroll: 0,
            cursor_line: 0,
            block_cursor: None,
            search_query: String::new(),
            search_match_index: None,
            execution: None,
            last_event_seq: None,
            detail_expanded_by_default,
            node_expansions: BTreeMap::new(),
            rendered: None,
        }
    }

    pub(in crate::app) fn reset(&mut self, session_id: i64, title: String) {
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
        self.submitting = false;
        self.pending_restore_draft = None;
        self.follow_tail = true;
        self.scroll = 0;
        self.cursor_line = 0;
        self.block_cursor = None;
        self.execution = None;
        self.last_event_seq = None;
        self.search_query.clear();
        self.search_match_index = None;
        self.node_expansions.clear();
        self.invalidate_render();
    }

    pub(in crate::app) fn apply_execution(&mut self, execution: SessionExecutionResource) {
        self.session_title = execution.session.title.clone();
        self.last_event_seq = execution.latest_event_seq;
        self.execution = Some(execution);
        self.invalidate_render();
    }

    pub(in crate::app) fn add_pending_user_message(&mut self, message: PendingUserMessage) {
        self.pending_user_messages.push(message);
        self.follow_tail = true;
        self.invalidate_render();
    }

    pub(in crate::app) fn remove_pending_user_message(&mut self, id: u64) {
        self.pending_user_messages
            .retain(|message| message.id != id);
        self.invalidate_render();
    }

    pub(in crate::app) fn confirm_pending_user_message(&mut self, id: u64) {
        if let Some(message) = self
            .pending_user_messages
            .iter_mut()
            .find(|message| message.id == id)
        {
            message.confirmed = true;
            self.invalidate_render();
        }
    }

    fn confirm_next_pending_user_message(&mut self) {
        if let Some(message) = self
            .pending_user_messages
            .iter_mut()
            .find(|message| !message.confirmed)
        {
            message.confirmed = true;
            self.invalidate_render();
        }
    }

    pub(in crate::app) fn replace_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        self.pending_user_messages
            .retain(|message| !message.confirmed);
        self.messages = page.items;
        self.older_cursor = page.page.next_cursor;
        self.has_more_older = page.page.has_more;
        self.invalidate_render();
        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    pub(in crate::app) fn prepend_messages(
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
        self.scroll = self
            .scroll
            .saturating_add(new_total.saturating_sub(old_total));
        self.cursor_line = self
            .cursor_line
            .saturating_add(new_total.saturating_sub(old_total));
        self.clamp_scroll(width, height);
    }

    pub(in crate::app) fn merge_latest_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        self.pending_user_messages
            .retain(|message| !message.confirmed);
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
        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    pub(in crate::app) fn apply_live_event(
        &mut self,
        event: &DomainEvent,
        width: u16,
        height: u16,
    ) -> bool {
        let refresh_needed = match &event.kind {
            // The transcript now comes from the server-side collapsed
            // conversation projection. Raw live message events can describe
            // intermediate assistant passes that are intentionally hidden
            // from the user-visible transcript, so we always re-fetch the
            // latest projection instead of mutating the local message list.
            AgenaSessionEvent::UserMessageAppended(_) => {
                // Submission waits for the whole agent run, but the durable
                // user-message event is emitted as soon as the prompt is
                // stored. A refresh can therefore load the real message
                // while its optimistic copy is still marked pending. Retire
                // that copy at the durable boundary so the transcript never
                // renders both versions of the same prompt.
                self.confirm_next_pending_user_message();
                true
            }
            AgenaSessionEvent::MessagePartUpdated(_)
            | AgenaSessionEvent::MessagePartDelta(_)
            | AgenaSessionEvent::AssistantMessageCompleted(_) => true,
            _ => false,
        };

        if !refresh_needed && let Some(seq) = event.meta.seq_session {
            self.last_event_seq = Some(seq);
        }

        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }

        refresh_needed
    }

    pub(in crate::app) fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.search_match_index = None;
        self.invalidate_render();
    }

    pub(in crate::app) fn current_search_match_count(&self) -> usize {
        self.rendered
            .as_ref()
            .map(|rendered| rendered.search_matches.len())
            .unwrap_or(0)
    }

    pub(in crate::app) fn current_search_match_number(&self) -> usize {
        match (self.search_match_index, self.current_search_match_count()) {
            (Some(index), count) if count > 0 => min(index + 1, count),
            _ => 0,
        }
    }

    pub(in crate::app) fn jump_search_match(&mut self, width: u16, height: u16, forward: bool) {
        let matches = self.rendered(width).search_matches.clone();
        if matches.is_empty() {
            self.search_match_index = None;
            return;
        }

        let next_index = match (self.search_match_index, forward) {
            (None, direction) => {
                initial_search_match_index(matches.as_slice(), self.cursor_line, direction)
            }
            (Some(index), true) => (index + 1) % matches.len(),
            (Some(0), false) => matches.len().saturating_sub(1),
            (Some(index), false) => index.saturating_sub(1),
        };

        self.search_match_index = Some(next_index);
        let line = matches[next_index];
        self.set_cursor_line(width, height, line);
    }

    pub(in crate::app) fn jump_to_message(&mut self, width: u16, height: u16, message_id: i64) {
        let rendered = self.rendered(width);
        let Some((_, line)) = rendered
            .message_line_starts
            .iter()
            .find(|(candidate_id, _)| *candidate_id == message_id)
            .copied()
        else {
            return;
        };
        self.set_cursor_line(width, height, line);
    }

    pub(in crate::app) fn highlighted_block_key(&self) -> Option<TranscriptNodeKey> {
        self.block_cursor.as_ref().map(|cursor| cursor.key.clone())
    }

    pub(in crate::app) fn highlighted_block_range(&mut self, width: u16) -> Option<Range<usize>> {
        let key = self.highlighted_block_key()?;
        let rendered = self.rendered(width);
        transcript_node_highlight_range(rendered.nodes.as_slice(), &key)
    }

    pub(in crate::app) fn step_line_with_block_selection(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
    ) {
        let selected_cursor = self.block_cursor.clone();
        let cursor_line = self.cursor_line;
        let step = {
            let rendered = self.rendered(width);
            if selected_cursor.is_some() {
                transcript_vertical_navigation_step(
                    rendered.nodes.as_slice(),
                    cursor_line,
                    selected_cursor.as_ref(),
                    direction,
                )
            } else {
                transcript_vertical_line_navigation_step(
                    rendered.nodes.as_slice(),
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

    pub(in crate::app) fn step_block(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
    ) {
        let selected_key = self.block_cursor.as_ref().map(|cursor| cursor.key.clone());
        let cursor_line = self.cursor_line;
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

    pub(in crate::app) fn move_by_blocks(
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

    pub(in crate::app) fn scroll_by_lines_with_blocks(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        for _ in 0..count.max(1) {
            self.step_line_with_block_selection(width, height, direction);
        }
    }

    pub(in crate::app) fn should_load_older(&self) -> bool {
        self.session_id.is_some()
            && self.has_more_older
            && !self.loading_initial
            && !self.loading_older
            && self.scroll <= 2
    }

    pub(in crate::app) fn rendered(&mut self, width: u16) -> &RenderedTranscript {
        if self
            .rendered
            .as_ref()
            .is_some_and(|rendered| rendered.width == width)
        {
            return self.rendered.as_ref().expect("render cache should exist");
        }

        let mut lines = Vec::new();
        let mut message_line_starts = Vec::new();
        let mut nodes = Vec::new();
        let mut line_nodes = Vec::new();
        if self.session_id.is_some() {
            if self.loading_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-loading-older",
                )));
                line_nodes.push(None);
            } else if self.has_more_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-more-older",
                )));
                line_nodes.push(None);
            }
        }

        if self.messages.is_empty()
            && self.pending_user_messages.is_empty()
            && self.session_id.is_some()
            && !self.loading_initial
        {
            lines.push(RenderedLine::dim(ui_text::t(
                &self.i18n,
                "transcript-empty-session",
            )));
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
                .map(|node| node.copy_text.as_str())
                .filter(|text| !text.trim().is_empty())
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
                toggleable: false,
                expanded: true,
            });
        }

        for message in self
            .pending_user_messages
            .iter()
            .filter(|message| !message.confirmed)
        {
            let status = format!(" {}", spinner_frame(current_spinner_millis()));
            lines.push(RenderedLine::plain(
                format!(
                    "{}{status}",
                    ui_text::role_label(&self.i18n, MessageRole::User)
                ),
                style_for_role(MessageRole::User).add_modifier(Modifier::BOLD),
            ));
            push_markdown(&mut lines, "  ", message.text.as_str(), width);
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

        self.rendered = Some(RenderedTranscript {
            width,
            lines,
            search_matches,
            message_line_starts,
            nodes,
            line_nodes,
        });
        self.rendered.as_ref().expect("render cache should exist")
    }

    pub(in crate::app) fn invalidate_render(&mut self) {
        self.rendered = None;
    }

    pub(in crate::app) fn has_animated_activity(&self) -> bool {
        self.pending_user_messages
            .iter()
            .any(|message| !message.confirmed)
            || self.messages.iter().any(|message| {
                message.parts.as_deref().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part.status == ExecutionStatus::InProgress
                            && matches!(
                                part.content.as_ref(),
                                Some(PartContent::Reasoning(_) | PartContent::Operation(_))
                            )
                    })
                })
            })
    }

    pub(in crate::app) fn clamp_scroll(&mut self, width: u16, height: u16) {
        let max_scroll = self.max_scroll(width, height);
        self.scroll = min(self.scroll, max_scroll);
        self.cursor_line = min(
            self.cursor_line,
            self.rendered(width).lines.len().saturating_sub(1),
        );
        if self.current_highlighted_node_index(width).is_none() {
            self.block_cursor = None;
        }
    }

    pub(in crate::app) fn scroll_to_bottom(&mut self, width: u16, height: u16) {
        self.scroll = self.max_scroll(width, height);
        self.follow_tail = true;
        self.cursor_line = self.rendered(width).lines.len().saturating_sub(1);
        self.block_cursor = None;
    }

    pub(in crate::app) fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.scroll = 0;
        self.cursor_line = 0;
        self.block_cursor = None;
        self.recompute_follow_tail(width, height);
    }

    pub(in crate::app) fn scroll_by_lines(&mut self, width: u16, height: u16, delta: isize) {
        self.follow_tail = false;
        self.block_cursor = None;
        let next = if delta.is_negative() {
            self.cursor_line.saturating_sub(delta.unsigned_abs())
        } else {
            self.cursor_line.saturating_add(delta as usize)
        };
        self.set_cursor_line(width, height, next);
    }

    pub(in crate::app) fn scroll_by_page(&mut self, width: u16, height: u16, forward: bool) {
        let page = height.max(1) as usize;
        self.scroll_by_lines(
            width,
            height,
            if forward {
                page as isize
            } else {
                -(page as isize)
            },
        );
    }

    pub(in crate::app) fn scroll_by_half_page(&mut self, width: u16, height: u16, forward: bool) {
        let half_page = (height.max(1) as usize).saturating_add(1) / 2;
        self.scroll_by_lines(
            width,
            height,
            if forward {
                half_page as isize
            } else {
                -(half_page as isize)
            },
        );
    }

    pub(in crate::app) fn max_scroll(&mut self, width: u16, height: u16) -> usize {
        let visible = height.max(1) as usize;
        self.rendered(width).lines.len().saturating_sub(visible)
    }

    pub(in crate::app) fn is_at_bottom(&mut self, width: u16, height: u16) -> bool {
        self.scroll >= self.max_scroll(width, height)
    }

    pub(in crate::app) fn set_cursor_line(&mut self, width: u16, height: u16, target: usize) {
        let total_lines = self.rendered(width).lines.len();
        self.cursor_line = if total_lines == 0 {
            0
        } else {
            min(target, total_lines.saturating_sub(1))
        };
        self.block_cursor = None;
        self.follow_tail = false;
        let visible = height.max(1) as usize;
        if self.cursor_line < self.scroll {
            self.scroll = self.cursor_line;
        } else if self.cursor_line >= self.scroll.saturating_add(visible) {
            self.scroll = self.cursor_line.saturating_add(1).saturating_sub(visible);
        }
        self.clamp_scroll(width, height);
        self.recompute_follow_tail(width, height);
    }

    pub(in crate::app) fn current_cursor_node(
        &mut self,
        width: u16,
    ) -> Option<&RenderedTranscriptNode> {
        let node_index = self.current_highlighted_node_index(width)?;
        let rendered = self.rendered(width);
        rendered.nodes.get(node_index)
    }

    pub(in crate::app) fn current_cursor_node_cloned(
        &mut self,
        width: u16,
    ) -> Option<RenderedTranscriptNode> {
        self.current_cursor_node(width).cloned()
    }

    pub(in crate::app) fn current_highlighted_node_index(&mut self, width: u16) -> Option<usize> {
        if let Some(block_cursor) = self.block_cursor.as_ref() {
            let highlighted_key = block_cursor.key.clone();
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
            self.block_cursor = None;
        }
        let cursor_line = self.cursor_line;
        let rendered = self.rendered(width);
        rendered
            .line_nodes
            .get(cursor_line)
            .and_then(|value| *value)
    }

    pub(in crate::app) fn set_block_cursor(
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

    pub(in crate::app) fn set_block_cursor_with_mode(
        &mut self,
        width: u16,
        height: u16,
        node_index: usize,
        direction: TranscriptMoveDirection,
        mode: TranscriptBlockSelectionMode,
    ) {
        let (start_line, end_line, target_line) = {
            let rendered = self.rendered(width);
            let Some(node) = rendered.nodes.get(node_index) else {
                return;
            };
            let target_line = match direction {
                TranscriptMoveDirection::Up => node.end_line.saturating_sub(1),
                TranscriptMoveDirection::Down => node.start_line,
            };
            (node.start_line, node.end_line, target_line)
        };
        self.set_cursor_line(width, height, target_line);
        let total_lines = self.rendered(width).lines.len();
        self.scroll = transcript_selection_scroll_position(
            total_lines,
            start_line,
            end_line,
            height.max(1) as usize,
            self.scroll,
            direction,
        );
        self.recompute_follow_tail(width, height);
        let key = {
            let rendered = self.rendered(width);
            rendered.nodes.get(node_index).map(|node| node.key.clone())
        };
        self.block_cursor = key.map(|key| TranscriptBlockCursor {
            key,
            direction,
            mode,
        });
    }

    pub(in crate::app) fn recompute_follow_tail(&mut self, width: u16, height: u16) {
        let line_count = self.rendered(width).lines.len();
        self.follow_tail = transcript_should_follow_tail(
            self.cursor_line,
            line_count,
            self.is_at_bottom(width, height),
        );
    }
}
use crate::app::{
    AgenaSessionEvent, BTreeMap, DomainEvent, ExecutionStatus, HashSet, I18n, MessageResource,
    MessageRole, Modifier, PaginatedResponse, PartContent, PendingUserMessage, Range, RenderedLine,
    RenderedTranscript, RenderedTranscriptNode, SessionExecutionResource, TranscriptBlockCursor,
    TranscriptBlockSelectionMode, TranscriptDetailDefaults, TranscriptMoveDirection,
    TranscriptNodeKey, TranscriptNodeKind, TranscriptState, TranscriptVerticalNavigationStep,
    contains_case_insensitive, current_spinner_millis, initial_search_match_index,
    merge_message_resources, message_sort_key, min, push_markdown, render_message_detailed,
    spinner_frame, style_for_role, transcript_message_navigation_target,
    transcript_node_highlight_range, transcript_selection_scroll_position,
    transcript_should_fall_back_to_message_navigation, transcript_should_follow_tail,
    transcript_vertical_line_navigation_step, transcript_vertical_navigation_step, ui_text,
};
