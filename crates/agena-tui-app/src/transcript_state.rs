#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptRevealPolicy {
    /// Keep the current viewport while the cursor remains visible; otherwise
    /// move only far enough to reveal it.
    Minimal,
    /// Keep the cursor at the same terminal row while content flows past it.
    PreserveScreenRow(usize),
    /// Explicit location jumps such as search results deserve centering.
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranscriptViewportRow {
    Top,
    Middle,
    Bottom,
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
            #[cfg(test)]
            messages: Vec::new(),
            snapshot: agena_domain::TranscriptSnapshot::default(),
            reply_failures: BTreeMap::new(),
            pending_user_messages: Vec::new(),
            refreshing: false,
            state_loading: false,
            refresh_in_flight_since: None,
            state_load_in_flight_since: None,
            viewport: TranscriptViewport::default(),
            interaction: TranscriptInteraction::default(),
            search_query: String::new(),
            search_match_index: None,
            jump_history: Vec::new(),
            jump_history_index: 0,
            execution: None,
            last_event_seq: None,
            detail_expanded_by_default,
            node_expansions: BTreeMap::new(),
            expanded_operation_activity_ids: BTreeSet::new(),
            v2_activities: BTreeMap::new(),
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
        #[cfg(test)]
        self.messages.clear();
        self.snapshot = agena_domain::TranscriptSnapshot {
            session_id,
            ..Default::default()
        };
        self.reply_failures.clear();
        self.pending_user_messages.clear();
        self.refreshing = false;
        self.state_loading = false;
        self.viewport.reduce(TranscriptAction::Reset);
        self.interaction = TranscriptInteraction::default();
        self.execution = None;
        self.last_event_seq = None;
        self.search_query.clear();
        self.search_match_index = None;
        self.jump_history.clear();
        self.jump_history_index = 0;
        self.node_expansions.clear();
        self.v2_activities.clear();
        self.invalidate_render();
    }

    pub(crate) fn apply_execution(&mut self, execution: SessionExecutionResource) {
        self.session_title = execution.session.title.clone();
        self.last_event_seq = execution.latest_event_seq;
        self.merge_snapshot(execution.transcript.clone());
        self.execution = Some(execution);
        self.invalidate_render();
    }

    /// Clear in-flight request flags that have exceeded `timeout` so the
    /// periodic refresh resumes even when a response was lost (spawned task
    /// panicked, message dropped, or a backend call that never resolved).
    /// Returns true when anything was recovered.
    pub(crate) fn recover_stalled_requests(&mut self, timeout: Duration) -> bool {
        let mut recovered = false;
        if self
            .refresh_in_flight_since
            .is_some_and(|since| since.elapsed() >= timeout)
        {
            self.refreshing = false;
            self.refresh_in_flight_since = None;
            recovered = true;
        }
        if self
            .state_load_in_flight_since
            .is_some_and(|since| since.elapsed() >= timeout)
        {
            self.state_loading = false;
            self.state_load_in_flight_since = None;
            recovered = true;
        }
        recovered
    }

    /// True when any assistant reply in the snapshot has not reached a
    /// terminal status. A terminal execution must never leave such a reply
    /// behind; the caller uses this as a safety net to force a refresh.
    pub(crate) fn has_non_terminal_replies(&self) -> bool {
        self.snapshot
            .turns
            .iter()
            .any(|turn| !turn.reply.status.is_terminal())
    }

    pub(crate) fn merge_snapshot(&mut self, snapshot: agena_domain::TranscriptSnapshot) {
        self.apply_snapshot_change(|current| current.merge(snapshot));
        self.invalidate_render();
    }

    /// Apply one canonical transcript mutation and reconcile optimistic user
    /// entries against user inputs that became visible because of it.
    ///
    /// A turn can arrive first as an empty execution envelope and acquire its
    /// input document in a later durable snapshot or live patch. Comparing
    /// only newly seen turn ids leaves the optimistic entry behind in that
    /// case. Visibility is the actual transcript boundary: a stable turn id
    /// crossing from absent/empty to non-empty replaces exactly one pending
    /// user entry, while permission continuations with empty input replace
    /// none.
    fn apply_snapshot_change(
        &mut self,
        change: impl FnOnce(&mut agena_domain::TranscriptSnapshot),
    ) {
        let visible_before = self
            .snapshot
            .turns
            .iter()
            .filter(|turn| !turn.input.is_empty())
            .map(|turn| (self.snapshot.session_id, turn.id))
            .collect::<BTreeSet<_>>();

        change(&mut self.snapshot);
        self.record_reply_failures();

        let newly_visible_user_inputs = self
            .snapshot
            .turns
            .iter()
            .filter(|turn| !turn.input.is_empty())
            .filter(|turn| !visible_before.contains(&(self.snapshot.session_id, turn.id)))
            .count();
        let reconciled = newly_visible_user_inputs.min(self.pending_user_messages.len());
        self.pending_user_messages.drain(..reconciled);
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

    /// Remember the latest structured failure observed per assistant reply so
    /// a later continuation that recovers the reply keeps the failure visible
    /// in the chat. The runtime clears its failure projection when the reply
    /// completes, but the chat is a user-facing history: the failure stays.
    fn record_reply_failures(&mut self) {
        for turn in &self.snapshot.turns {
            if let Some(failure) = turn.reply.failure.clone() {
                self.reply_failures.insert(turn.reply.id, failure);
            }
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
            agena_runtime::RuntimePresentationEventKind::PartPatch(_) => true,
            agena_runtime::RuntimePresentationEventKind::TranscriptPatch(patch) => {
                if transcript_patch_can_materialize_user_input(patch) {
                    self.apply_snapshot_change(|snapshot| snapshot.apply((**patch).clone()));
                } else {
                    self.snapshot.apply((**patch).clone());
                    self.record_reply_failures();
                }
                self.invalidate_render();
                false
            }
            agena_runtime::RuntimePresentationEventKind::Refresh { .. } => true,
            agena_runtime::RuntimePresentationEventKind::ActivityChanged { .. } => true,
            agena_runtime::RuntimePresentationEventKind::ActivityV2(_) => {
                // Unified live wire event (07 §5.2): merge into the in-memory
                // v2 overlay. Never persisted and never part of the snapshot.
                self.apply_activity_v2_event(event);
                false
            }
        };

        // The local watermark must only count events the server's durable
        // `latest_event_seq` also counts. Live-only events (ActivityV2,
        // streamed text upserts, retry notices) consume global sequence
        // numbers but are never written to the durable log; counting them
        // here would push the watermark ahead of the durable log and make
        // every later refresh look stale, dropping the terminal execution
        // and leaving the final reply stuck as an InProgress "Text" card.
        if !refresh_needed && event.durable {
            self.last_event_seq = Some(event.meta.seq_global);
        }

        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }

        refresh_needed
    }

    /// Apply a live activity-v2 wire event to the in-memory overlay.
    ///
    /// Detail deltas are only merged for expanded Activities (same budget
    /// rule as the legacy Operation path); collapsed Activities keep only
    /// their headline. Title/state/upsert/removal events update the overlay
    /// entry's stable fields. Nothing here touches the persisted snapshot.
    fn apply_activity_v2_event(&mut self, event: &agena_runtime::RuntimePresentationEvent) {
        let agena_runtime::RuntimePresentationEventKind::ActivityV2(activity) = &event.kind else {
            return;
        };
        match activity.as_ref() {
            agena_runtime::session::activity::ActivityLiveEvent::DetailDelta {
                activity_id,
                delta,
            } => {
                if !self.expanded_operation_activity_ids.contains(activity_id) {
                    return;
                }
                let entry =
                    self.v2_activities
                        .entry(*activity_id)
                        .or_insert_with(|| V2LiveActivity {
                            activity_id: *activity_id,
                            title: String::new(),
                            state: agena_domain::ActivityState::InProgress,
                            live_blocks: Vec::new(),
                        });
                merge_v2_delta(&mut entry.live_blocks, delta);
            }
            agena_runtime::session::activity::ActivityLiveEvent::TitleChanged {
                activity_id,
                title,
            } => {
                if let Some(entry) = self.v2_activities.get_mut(activity_id) {
                    entry.title = title.clone();
                }
            }
            agena_runtime::session::activity::ActivityLiveEvent::SummaryChanged { .. } => {}
            agena_runtime::session::activity::ActivityLiveEvent::StateChanged {
                activity_id,
                state,
            } => {
                if let Some(entry) = self.v2_activities.get_mut(activity_id) {
                    entry.state = *state;
                }
            }
            agena_runtime::session::activity::ActivityLiveEvent::Upserted { node } => {
                let live_blocks = self
                    .v2_activities
                    .get(&node.activity_id)
                    .map(|entry| entry.live_blocks.clone())
                    .unwrap_or_default();
                self.v2_activities.insert(
                    node.activity_id,
                    V2LiveActivity {
                        activity_id: node.activity_id,
                        title: node.title.clone(),
                        state: node.state,
                        live_blocks,
                    },
                );
            }
            agena_runtime::session::activity::ActivityLiveEvent::Removed { activity_id } => {
                self.v2_activities.remove(activity_id);
            }
        }
        self.invalidate_render();
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

        self.push_jump_mark(width);
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    /// Move across terminal grapheme cells without crossing a rendered row.
    /// This is the normal-mode `h`/`l` motion: it leaves semantic block
    /// selection behind and never turns a horizontal key press into a message
    /// jump.
    pub(crate) fn move_cursor_horizontally(
        &mut self,
        width: u16,
        height: u16,
        forward: bool,
        count: usize,
    ) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let line = cursor.line;
        let mut column = cursor.column;
        let mut moved = false;
        {
            let rendered = self.rendered(width);
            let Some(rendered_line) = rendered.lines.get(line) else {
                return;
            };
            let graphemes = transcript_cursor_grapheme_ranges(rendered_line);
            let Some(mut index) = transcript_cursor_grapheme_index(graphemes.as_slice(), column)
            else {
                return;
            };
            for _ in 0..count.max(1) {
                let next = if forward {
                    index.checked_add(1).filter(|next| *next < graphemes.len())
                } else {
                    index.checked_sub(1)
                };
                let Some(next) = next else {
                    break;
                };
                index = next;
                moved = true;
            }
            column = graphemes[index].start;
        }

        // A horizontal Vim motion exits an explicit block selection even at a
        // row boundary. The cursor still denotes one grapheme rather than the
        // entire selected line or message.
        if moved || cursor.block_cursor.is_some() {
            self.set_cursor_position_with_reveal(
                width,
                height,
                line,
                column,
                column,
                TranscriptRevealPolicy::Minimal,
            );
        }
    }

    /// Vim's `0` and `^`: move to the first displayable grapheme or the
    /// first non-whitespace grapheme on the current rendered row.
    pub(crate) fn move_cursor_to_line_start(
        &mut self,
        width: u16,
        height: u16,
        first_non_blank: bool,
    ) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let column = {
            let rendered = self.rendered(width);
            let Some(line) = rendered.lines.get(cursor.line) else {
                return;
            };
            let graphemes = transcript_cursor_graphemes(line);
            graphemes
                .iter()
                .find(|(_, grapheme)| {
                    !first_non_blank || !grapheme.chars().all(char::is_whitespace)
                })
                .map(|(range, _)| range.start)
                .unwrap_or_default()
        };
        self.set_cursor_position_with_reveal(
            width,
            height,
            cursor.line,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Vim's `$`: move to the final grapheme on the current rendered row.
    pub(crate) fn move_cursor_to_line_end(&mut self, width: u16, height: u16) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let column = {
            let rendered = self.rendered(width);
            rendered
                .lines
                .get(cursor.line)
                .and_then(|line| {
                    transcript_cursor_graphemes(line)
                        .last()
                        .map(|(range, _)| range.start)
                })
                .unwrap_or_default()
        };
        self.set_cursor_position_with_reveal(
            width,
            height,
            cursor.line,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Back up one cursor grapheme across focusable rendered rows. This is
    /// used for exclusive operator motions such as `yw`: the normal `w`
    /// cursor lands on the next word, while the yanked range ends immediately
    /// before that destination.
    pub(crate) fn move_cursor_to_previous_grapheme(&mut self, width: u16, height: u16) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let target = {
            let rendered = self.rendered(width);
            let same_line = rendered.lines.get(cursor.line).and_then(|line| {
                let graphemes = transcript_cursor_graphemes(line);
                let current = graphemes
                    .iter()
                    .position(|(range, _)| range.contains(&cursor.column))
                    .or_else(|| {
                        graphemes
                            .iter()
                            .rposition(|(range, _)| range.start <= cursor.column)
                    })?;
                current
                    .checked_sub(1)
                    .map(|previous| (cursor.line, graphemes[previous].0.start))
            });
            same_line.or_else(|| {
                (0..cursor.line).rev().find_map(|line| {
                    transcript_rendered_line_is_focusable(rendered, line).then(|| {
                        transcript_cursor_graphemes(&rendered.lines[line])
                            .last()
                            .map(|(range, _)| (line, range.start))
                    })?
                })
            })
        };
        let Some((line, column)) = target else {
            return;
        };
        self.set_cursor_position_with_reveal(
            width,
            height,
            line,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Implements Vim's `w`, `b`, `e`, `ge` and their WORD variants over
    /// the clean, focusable transcript rows. A word is Unicode
    /// alphanumeric/underscore; a WORD is any non-whitespace run.
    pub(crate) fn move_cursor_by_words(
        &mut self,
        width: u16,
        height: u16,
        forward: bool,
        to_end: bool,
        big_word: bool,
        count: usize,
    ) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let (positions, current_index) = {
            let rendered = self.rendered(width);
            let positions = rendered
                .lines
                .iter()
                .enumerate()
                .filter(|(line, _)| transcript_rendered_line_is_focusable(rendered, *line))
                .flat_map(|(line, rendered_line)| {
                    transcript_cursor_graphemes(rendered_line).into_iter().map(
                        move |(range, grapheme)| TranscriptGraphemePosition {
                            position: TranscriptTextPosition {
                                line,
                                column: range.start,
                            },
                            grapheme,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let current_index = positions.iter().rposition(|position| {
                position.position.line == cursor.line && position.position.column <= cursor.column
            });
            (positions, current_index)
        };
        let Some(mut index) = current_index else {
            return;
        };
        let mut moved = false;
        for _ in 0..count.max(1) {
            let Some(next) = transcript_word_motion_target(
                positions.as_slice(),
                index,
                forward,
                to_end,
                big_word,
            ) else {
                break;
            };
            moved |= next != index;
            index = next;
        }
        if moved {
            let target = positions[index].position;
            self.set_cursor_position_with_reveal(
                width,
                height,
                target.line,
                target.column,
                target.column,
                TranscriptRevealPolicy::Minimal,
            );
        }
    }

    /// Complete a Vim `f`/`F`/`t`/`T` motion on the current rendered row.
    /// `t` stops immediately before its target and `T` immediately after it.
    pub(crate) fn move_cursor_to_find(
        &mut self,
        width: u16,
        height: u16,
        forward: bool,
        till: bool,
        target: char,
        count: usize,
    ) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let graphemes = {
            let rendered = self.rendered(width);
            let Some(line) = rendered.lines.get(cursor.line) else {
                return;
            };
            transcript_cursor_graphemes(line)
        };
        let Some(current) = graphemes
            .iter()
            .position(|(range, _)| range.contains(&cursor.column))
            .or_else(|| {
                graphemes
                    .iter()
                    .rposition(|(range, _)| range.start <= cursor.column)
            })
        else {
            return;
        };
        let mut found = None;
        let mut start = current;
        for _ in 0..count.max(1) {
            let candidate = if forward {
                (start.saturating_add(1)..graphemes.len()).find(|index| {
                    graphemes[*index]
                        .1
                        .chars()
                        .any(|character| character == target)
                })
            } else {
                (0..start).rev().find(|index| {
                    graphemes[*index]
                        .1
                        .chars()
                        .any(|character| character == target)
                })
            };
            let Some(candidate) = candidate else {
                return;
            };
            found = Some(candidate);
            start = candidate;
        }
        let Some(found) = found else {
            return;
        };
        let target_index = if till && forward {
            found.saturating_sub(1)
        } else if till {
            found
                .saturating_add(1)
                .min(graphemes.len().saturating_sub(1))
        } else {
            found
        };
        let column = graphemes[target_index].0.start;
        self.set_cursor_position_with_reveal(
            width,
            height,
            cursor.line,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Move by rendered rows while retaining the target terminal column. This
    /// deliberately bypasses the older semantic hierarchy used by mouse and
    /// block selection: `j`/`k` behave like Vim's visual-line movement.
    pub(crate) fn move_cursor_by_visual_lines(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let target = self.focusable_line_after_steps(width, cursor.line, direction, count.max(1));
        self.set_cursor_position_with_reveal(
            width,
            height,
            target,
            cursor.preferred_column,
            cursor.preferred_column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Jump directly to the adjacent message. This is intentionally separate
    /// from block navigation so `Ctrl+K`/`Ctrl+J` always skip exactly one
    /// message, independent of the number of Markdown or activity blocks it
    /// contains.
    pub(crate) fn move_cursor_by_messages(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        self.push_jump_mark(width);
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };

        let target_line = {
            let rendered = self.rendered(width);
            let message_nodes = rendered
                .nodes
                .iter()
                .filter(|node| node.key.is_entry_container())
                .collect::<Vec<_>>();
            let current_message_id = cursor
                .block_cursor
                .as_ref()
                .map(|block| block.key.entry_id())
                .or_else(|| {
                    message_nodes
                        .iter()
                        .find(|node| cursor.line >= node.start_line && cursor.line < node.end_line)
                        .map(|node| node.key.entry_id())
                });
            let Some(current_index) = current_message_id.and_then(|message_id| {
                message_nodes
                    .iter()
                    .position(|node| node.key.entry_id() == message_id)
            }) else {
                return;
            };
            let mut target_index = current_index;
            for _ in 0..count.max(1) {
                let next = match direction {
                    TranscriptMoveDirection::Up => target_index.checked_sub(1),
                    TranscriptMoveDirection::Down => target_index
                        .checked_add(1)
                        .filter(|next| *next < message_nodes.len()),
                };
                let Some(next) = next else {
                    break;
                };
                target_index = next;
            }
            (target_index != current_index).then(|| {
                let node = message_nodes[target_index];
                (node.start_line..node.end_line)
                    .find(|line| transcript_rendered_line_is_focusable(rendered, *line))
                    .unwrap_or(node.start_line)
            })
        };

        if let Some(target_line) = target_line {
            self.set_cursor_position_with_reveal(
                width,
                height,
                target_line,
                0,
                0,
                TranscriptRevealPolicy::Minimal,
            );
        }
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
            self.interaction.visual_selection = None;
            self.interaction.visual_anchor = None;
            self.interaction.last_visual_selection = None;
        }

        let mut lines = Vec::new();
        let mut nodes = Vec::new();
        let mut line_nodes = Vec::new();
        let mut presentation_snapshot = self.snapshot.clone();
        for turn in &mut presentation_snapshot.turns {
            if let Some(failure) = self.reply_failures.get(&turn.reply.id) {
                turn.reply.failure = Some(failure.clone());
            }
        }
        let snapshot_entries = transcript_entries(&presentation_snapshot);
        #[cfg(not(test))]
        let entries = snapshot_entries;
        #[cfg(test)]
        let entries = if self.session_id == Some(self.snapshot.session_id) {
            snapshot_entries
        } else {
            self.messages
                .iter()
                .map(agena_tui_transcript::TranscriptEntry::from)
                .collect::<Vec<_>>()
        };
        let entries = weave_pending_user_entries(
            &self.snapshot,
            entries,
            self.pending_user_messages.as_slice(),
        );
        if entries.is_empty() && self.pending_user_messages.is_empty() && self.session_id.is_some()
        {
            lines.push(
                RenderedLine::dim(ui_text::t(&self.i18n, "transcript-empty-session"))
                    .with_copy_projection(String::new(), 0),
            );
            line_nodes.push(None);
        }

        for entry in &entries {
            let rendered = render_entry_detailed(
                entry,
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
            let body_copy_text = nodes[base_node..]
                .iter()
                .filter(|node| node.contributes_to_aggregate_copy())
                .map(|node| node.copy_text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let header_copy_text = if entry.role.is_some() {
                lines[base_line..]
                    .first()
                    .map(|line| line.copy_text.as_str())
                    .unwrap_or_default()
            } else {
                ""
            };
            let message_copy_text = [header_copy_text, body_copy_text.as_str()]
                .into_iter()
                .filter(|text| !text.is_empty())
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
            if entry.role.is_some() {
                nodes.push(RenderedTranscriptNode {
                    key: TranscriptNodeKey::Entry { entry_id: entry.id },
                    kind: TranscriptNodeKind::Message,
                    start_line: base_line,
                    end_line: lines.len(),
                    copy_text: message_copy_text,
                    atomic: false,
                    toggleable: false,
                    expanded: true,
                });
            }
        }

        // Live activity-v2 overlays render as a compact trailing section: one
        // headline per activity plus the merged ViewBlocks when expanded.
        for activity in self.v2_activities.values() {
            let state_label = format!("{:?}", activity.state).to_ascii_lowercase();
            let headline = format!("\u{25b8} {} \u{00b7} {state_label}", activity.title);
            lines.push(RenderedLine::dim(headline.clone()).with_copy_projection(headline, 0));
            line_nodes.push(None);
            if self
                .expanded_operation_activity_ids
                .contains(&activity.activity_id)
            {
                for block in &activity.live_blocks {
                    for block_line in render_activity_block(block) {
                        lines.push(
                            RenderedLine::plain(
                                block_line.clone(),
                                ratatui::style::Style::default(),
                            )
                            .with_copy_projection(block_line.clone(), 0),
                        );
                        line_nodes.push(None);
                    }
                }
            }
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
            if let Some(cursor) = self.interaction.cursor.clone()
                && cursor.line == last_line
            {
                self.install_cursor_at_column(
                    width,
                    height,
                    last_line,
                    cursor.column,
                    cursor.preferred_column,
                    cursor.block_cursor,
                    false,
                );
            } else {
                self.install_cursor(width, height, last_line, None, false);
            }
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

    pub(crate) fn cursor_text_position(&mut self, width: u16) -> Option<TranscriptTextPosition> {
        let cursor = self.interaction.cursor.clone()?;
        Some(self.cursor_character_start(width, &cursor))
    }

    /// Operator commands such as `yw` and `Y` preserve the normal-mode
    /// cursor. This restores the original grapheme after copying while the
    /// copied Visual geometry remains available to `gv`.
    pub(crate) fn restore_cursor_text_position(
        &mut self,
        width: u16,
        height: u16,
        position: TranscriptTextPosition,
    ) {
        self.set_cursor_position_with_reveal(
            width,
            height,
            position.line,
            position.column,
            position.column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    pub(crate) fn navigation_cursor_line(&self) -> Option<usize> {
        self.interaction_line()
    }

    /// Terminal-cell range occupied by the one grapheme under the normal-mode
    /// cursor. Explicit block selection remains a separate interaction and
    /// therefore deliberately has no character range.
    pub(crate) fn cursor_cell_range(&mut self, width: u16) -> Option<(usize, Range<usize>)> {
        let cursor = self.interaction.cursor.as_ref()?;
        if cursor.block_cursor.is_some() {
            return None;
        }
        let line = cursor.line;
        let column = cursor.column;
        let rendered = self.rendered(width);
        let rendered_line = rendered.lines.get(line)?;
        Some((line, transcript_cursor_cell_range(rendered_line, column)))
    }

    fn cursor_character_start(
        &mut self,
        width: u16,
        cursor: &TranscriptCursor,
    ) -> TranscriptTextPosition {
        let column = self
            .rendered(width)
            .lines
            .get(cursor.line)
            .map(|line| transcript_cursor_column_for_line(line, cursor.column))
            .unwrap_or_default();
        TranscriptTextPosition {
            line: cursor.line,
            column,
        }
    }

    fn refresh_visual_selection(&mut self, width: u16) {
        let Some(mode) = self.interaction.visual_selection else {
            return;
        };
        let Some(anchor) = self.interaction.visual_anchor else {
            self.interaction.text_selection = None;
            self.interaction.visual_selection = None;
            return;
        };
        let Some(cursor) = self.interaction.cursor.clone() else {
            self.interaction.text_selection = None;
            self.interaction.visual_selection = None;
            self.interaction.visual_anchor = None;
            return;
        };
        let selection = {
            let rendered = self.rendered(width);
            let cursor_range = rendered
                .lines
                .get(cursor.line)
                .map(|line| transcript_cursor_cell_range(line, cursor.column))
                .unwrap_or(0..1);
            match mode {
                TranscriptVisualSelectionMode::Character => TranscriptTextSelection {
                    anchor,
                    head: TranscriptTextPosition {
                        line: cursor.line,
                        column: cursor_range.end.saturating_sub(1),
                    },
                },
                TranscriptVisualSelectionMode::Line if cursor.line >= anchor.line => {
                    TranscriptTextSelection {
                        anchor: TranscriptTextPosition {
                            line: anchor.line,
                            column: 0,
                        },
                        head: TranscriptTextPosition {
                            line: cursor.line,
                            column: usize::MAX,
                        },
                    }
                }
                TranscriptVisualSelectionMode::Line => TranscriptTextSelection {
                    anchor: TranscriptTextPosition {
                        line: anchor.line,
                        column: usize::MAX,
                    },
                    head: TranscriptTextPosition {
                        line: cursor.line,
                        column: 0,
                    },
                },
                TranscriptVisualSelectionMode::Block => {
                    self.interaction.text_selection = None;
                    self.viewport.follow_tail = false;
                    return;
                }
            }
        };
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
    }

    fn visual_selection_state(
        &self,
    ) -> Option<(TranscriptVisualSelectionMode, TranscriptTextPosition)> {
        self.interaction
            .visual_selection
            .zip(self.interaction.visual_anchor)
    }

    fn restore_visual_selection(
        &mut self,
        width: u16,
        visual: Option<(TranscriptVisualSelectionMode, TranscriptTextPosition)>,
    ) {
        if let Some((mode, anchor)) = visual {
            self.interaction.visual_selection = Some(mode);
            self.interaction.visual_anchor = Some(anchor);
            self.refresh_visual_selection(width);
        }
    }

    fn remember_visual_selection(&mut self, width: u16) {
        let Some(mode) = self.interaction.visual_selection else {
            return;
        };
        let Some(anchor) = self.interaction.visual_anchor else {
            return;
        };
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        self.interaction.last_visual_selection = Some(TranscriptVisualSelectionSnapshot {
            mode,
            anchor,
            head: self.cursor_character_start(width, &cursor),
        });
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

    pub(crate) fn has_active_text_selection(&self) -> bool {
        self.text_selection().is_some() || self.has_visual_selection()
    }

    /// Cell ranges for the active pointer or Visual selection. A block Visual
    /// selection is rectangular, so it cannot be represented by the linear
    /// `TranscriptTextSelection` used for pointer ranges.
    pub(crate) fn selection_cell_ranges(&mut self, width: u16) -> Vec<Option<Range<usize>>> {
        let line_count = self.rendered(width).lines.len();
        if self.interaction.visual_selection != Some(TranscriptVisualSelectionMode::Block) {
            return (0..line_count)
                .map(|line| {
                    self.interaction
                        .text_selection
                        .and_then(|selection| selection.cell_range_for_line(line))
                })
                .collect();
        }
        let Some(anchor) = self.interaction.visual_anchor else {
            return vec![None; line_count];
        };
        let Some(cursor) = self.interaction.cursor.clone() else {
            return vec![None; line_count];
        };
        let cursor_range = self
            .rendered(width)
            .lines
            .get(cursor.line)
            .map(|line| transcript_cursor_cell_range(line, cursor.column))
            .unwrap_or(0..1);
        let start_line = anchor.line.min(cursor.line);
        let end_line = anchor
            .line
            .max(cursor.line)
            .min(line_count.saturating_sub(1));
        let start_column = anchor.column.min(cursor_range.start);
        let end_column = anchor
            .column
            .max(cursor_range.end.saturating_sub(1))
            .saturating_add(1);
        (0..line_count)
            .map(|line| {
                (line >= start_line && line <= end_line).then_some(start_column..end_column)
            })
            .collect()
    }

    pub(crate) fn selected_text(&mut self, width: u16, spinner: &str) -> Option<String> {
        if self.interaction.visual_selection == Some(TranscriptVisualSelectionMode::Block) {
            let ranges = self.selection_cell_ranges(width);
            let rendered = self.rendered(width);
            let fragments = ranges
                .into_iter()
                .enumerate()
                .filter_map(|(line, range)| {
                    let range = range?;
                    let selection = TranscriptTextSelection {
                        anchor: TranscriptTextPosition {
                            line,
                            column: range.start,
                        },
                        head: TranscriptTextPosition {
                            line,
                            column: range.end.saturating_sub(1),
                        },
                    };
                    Some(transcript_text_selection_text(
                        rendered.lines.as_slice(),
                        rendered.nodes.as_slice(),
                        rendered.line_nodes.as_slice(),
                        selection,
                        spinner,
                    ))
                })
                .collect::<Vec<_>>();
            return (!fragments.is_empty()).then(|| fragments.join("\n"));
        }

        let selection = self.text_selection()?;
        let rendered = self.rendered(width);
        Some(transcript_text_selection_text(
            rendered.lines.as_slice(),
            rendered.nodes.as_slice(),
            rendered.line_nodes.as_slice(),
            selection,
            spinner,
        ))
    }

    pub(crate) fn has_visual_selection(&self) -> bool {
        self.interaction.visual_selection.is_some()
    }

    /// Enter, switch, or leave Vim-style visual selection. Pointer ranges are
    /// intentionally replaced here: `v` and `V` always begin from the live
    /// keyboard cursor rather than from an earlier drag endpoint.
    pub(crate) fn toggle_visual_selection(
        &mut self,
        width: u16,
        height: u16,
        mode: TranscriptVisualSelectionMode,
    ) {
        self.ensure_visual_focus(width, height);
        if self.interaction.visual_selection == Some(mode) {
            self.cancel_text_selection(width, height);
            return;
        }

        if self
            .interaction
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.block_cursor.is_some())
        {
            let cursor = self
                .interaction
                .cursor
                .clone()
                .expect("block cursor was checked above");
            self.install_cursor_at_column(
                width,
                height,
                cursor.line,
                cursor.column,
                cursor.preferred_column,
                None,
                true,
            );
        }

        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let anchor = self
            .interaction
            .visual_anchor
            .unwrap_or_else(|| self.cursor_character_start(width, &cursor));
        self.interaction.visual_selection = Some(mode);
        self.interaction.visual_anchor = Some(anchor);
        self.refresh_visual_selection(width);
    }

    /// Vim's `o`: exchange the fixed and active ends of the current Visual
    /// selection so subsequent motions grow from the other side.
    pub(crate) fn swap_visual_selection_endpoint(&mut self, width: u16, height: u16) {
        let Some((mode, anchor)) = self.visual_selection_state() else {
            return;
        };
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let head = self.cursor_character_start(width, &cursor);
        self.install_cursor_at_column(
            width,
            height,
            anchor.line,
            anchor.column,
            anchor.column,
            None,
            false,
        );
        self.interaction.visual_selection = Some(mode);
        self.interaction.visual_anchor = Some(head);
        self.refresh_visual_selection(width);
    }

    /// Vim's block-Visual `O`: retain the same rectangle while moving the
    /// active cursor to the other corner.
    pub(crate) fn swap_visual_block_corner(&mut self, width: u16, height: u16) {
        if self.interaction.visual_selection != Some(TranscriptVisualSelectionMode::Block) {
            self.swap_visual_selection_endpoint(width, height);
            return;
        }
        let Some(anchor) = self.interaction.visual_anchor else {
            return;
        };
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let head = self.cursor_character_start(width, &cursor);
        self.install_cursor_at_column(
            width,
            height,
            head.line,
            anchor.column,
            anchor.column,
            None,
            false,
        );
        self.interaction.visual_anchor = Some(TranscriptTextPosition {
            line: anchor.line,
            column: head.column,
        });
        self.refresh_visual_selection(width);
    }

    /// Restore the last exited keyboard Visual selection (`gv`), including a
    /// rectangular block selection.
    pub(crate) fn reselect_last_visual_selection(&mut self, width: u16, height: u16) {
        let Some(snapshot) = self.interaction.last_visual_selection else {
            return;
        };
        self.ensure_visual_focus(width, height);
        self.install_cursor_at_column(
            width,
            height,
            snapshot.head.line,
            snapshot.head.column,
            snapshot.head.column,
            None,
            true,
        );
        self.interaction.visual_selection = Some(snapshot.mode);
        self.interaction.visual_anchor = Some(snapshot.anchor);
        self.refresh_visual_selection(width);
    }

    /// Select the semantic Markdown block or whole message underneath the
    /// cursor. These are Transcript-local Vim text objects used by `vam`,
    /// `vim`, `yam`, `yim`, `vaM`, `viM`, `yaM`, and `yiM`.
    pub(crate) fn select_current_text_object(
        &mut self,
        width: u16,
        height: u16,
        message: bool,
    ) -> bool {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return false;
        };
        let range = {
            let rendered = self.rendered(width);
            if message {
                let message_id = rendered
                    .line_nodes
                    .get(cursor.line)
                    .and_then(|index| *index)
                    .and_then(|index| rendered.nodes.get(index))
                    .map(|node| node.key.entry_id())
                    .or_else(|| {
                        rendered
                            .nodes
                            .iter()
                            .find(|node| {
                                node.key.is_entry_container()
                                    && cursor.line >= node.start_line
                                    && cursor.line < node.end_line
                            })
                            .map(|node| node.key.entry_id())
                    });
                message_id.and_then(|message_id| {
                    rendered
                        .nodes
                        .iter()
                        .find(|node| {
                            node.key.is_entry_container() && node.key.entry_id() == message_id
                        })
                        .and_then(|node| {
                            transcript_node_highlight_range(rendered.nodes.as_slice(), &node.key)
                        })
                })
            } else {
                rendered
                    .line_nodes
                    .get(cursor.line)
                    .and_then(|index| *index)
                    .and_then(|index| rendered.nodes.get(index))
                    .filter(|node| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
                    .map(|node| node.start_line..node.end_line)
            }
        };
        let Some(range) = range.filter(|range| range.start < range.end) else {
            return false;
        };
        let last_line = range.end.saturating_sub(1);
        self.install_cursor_at_column(width, height, last_line, usize::MAX, usize::MAX, None, true);
        self.interaction.visual_selection = Some(TranscriptVisualSelectionMode::Line);
        self.interaction.visual_anchor = Some(TranscriptTextPosition {
            line: range.start,
            column: 0,
        });
        self.refresh_visual_selection(width);
        true
    }

    /// Vim's `iw`/`aw` on the current rendered text row. `aw` includes the
    /// following whitespace when present, otherwise the preceding whitespace.
    pub(crate) fn select_current_word_text_object(
        &mut self,
        width: u16,
        height: u16,
        around: bool,
    ) -> bool {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return false;
        };
        let range = {
            let rendered = self.rendered(width);
            let Some(line) = rendered.lines.get(cursor.line) else {
                return false;
            };
            let graphemes = transcript_cursor_graphemes(line);
            let Some(mut current) = graphemes
                .iter()
                .position(|(range, _)| range.contains(&cursor.column))
                .or_else(|| {
                    graphemes
                        .iter()
                        .rposition(|(range, _)| range.start <= cursor.column)
                })
            else {
                return false;
            };
            while current < graphemes.len()
                && transcript_word_class(graphemes[current].1.as_str(), false)
                    == TranscriptWordClass::Whitespace
            {
                current = current.saturating_add(1);
            }
            if current == graphemes.len() {
                return false;
            }
            let class = transcript_word_class(graphemes[current].1.as_str(), false);
            let mut start = current;
            while start > 0
                && transcript_word_class(graphemes[start.saturating_sub(1)].1.as_str(), false)
                    == class
            {
                start = start.saturating_sub(1);
            }
            let mut end = current;
            while end.saturating_add(1) < graphemes.len()
                && transcript_word_class(graphemes[end.saturating_add(1)].1.as_str(), false)
                    == class
            {
                end = end.saturating_add(1);
            }
            if around {
                let word_end = end;
                let mut after = end.saturating_add(1);
                while after < graphemes.len()
                    && transcript_word_class(graphemes[after].1.as_str(), false)
                        == TranscriptWordClass::Whitespace
                {
                    end = after;
                    after = after.saturating_add(1);
                }
                if end == word_end {
                    while start > 0
                        && transcript_word_class(
                            graphemes[start.saturating_sub(1)].1.as_str(),
                            false,
                        ) == TranscriptWordClass::Whitespace
                    {
                        start = start.saturating_sub(1);
                    }
                }
            }
            (graphemes[start].0.start, graphemes[end].0.start)
        };
        self.install_cursor_at_column(width, height, cursor.line, range.1, range.1, None, true);
        self.interaction.visual_selection = Some(TranscriptVisualSelectionMode::Character);
        self.interaction.visual_anchor = Some(TranscriptTextPosition {
            line: cursor.line,
            column: range.0,
        });
        self.refresh_visual_selection(width);
        true
    }

    /// Vim's `ip`/`ap` is projected onto the enclosing Markdown paragraph.
    /// Transcript blocks already omit card chrome and blank layout rows, so
    /// the inner and around clipboard projections are intentionally equal.
    pub(crate) fn select_current_paragraph_text_object(
        &mut self,
        width: u16,
        height: u16,
        _around: bool,
    ) -> bool {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return false;
        };
        let is_paragraph = {
            let rendered = self.rendered(width);
            rendered
                .line_nodes
                .get(cursor.line)
                .and_then(|index| *index)
                .and_then(|index| rendered.nodes.get(index))
                .is_some_and(|node| node.kind == TranscriptNodeKind::MarkdownParagraph)
        };
        is_paragraph && self.select_current_text_object(width, height, false)
    }

    fn navigation_parts(&self) -> Option<(usize, Option<TranscriptBlockCursor>)> {
        self.interaction
            .cursor
            .as_ref()
            .map(|cursor| (cursor.line, cursor.block_cursor.clone()))
    }

    pub(crate) fn clamp_scroll(&mut self, width: u16, height: u16) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            self.viewport.top = 0;
            self.viewport.follow_tail = true;
            return;
        }
        // While the user is following the tail, content that arrived through a
        // full-state merge (periodic refresh, session load, turn submission)
        // must keep the viewport pinned to the bottom. The navigation cursor
        // can be stale relative to the newly appended lines, so running
        // sync_follow_tail on the old cursor below would cancel follow mode
        // before ensure_visual_focus gets a chance to scroll.
        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
            return;
        }
        self.viewport.top = self.viewport.top.min(self.max_scroll(width, height));
        self.reconcile_cursor_anchor(width, height);
        let last_line = total_lines.saturating_sub(1);
        if let Some(selection) = &mut self.interaction.text_selection {
            selection.anchor.line = selection.anchor.line.min(last_line);
            selection.head.line = selection.head.line.min(last_line);
        }
        if let Some(anchor) = &mut self.interaction.visual_anchor {
            anchor.line = anchor.line.min(last_line);
        }
        if let Some(selection) = &mut self.interaction.last_visual_selection {
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
        self.push_jump_mark(width);
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            self.viewport.top = 0;
            self.viewport.follow_tail = true;
            return;
        }
        let visual = self.visual_selection_state();
        let target = self.focusable_line_near(width, total_lines.saturating_sub(1), false);
        self.install_cursor(width, height, target, None, true);
        self.viewport.top = self.max_scroll(width, height);
        self.refresh_cursor_screen_row(height);
        self.restore_visual_selection(width, visual);
        self.viewport.follow_tail = true;
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.push_jump_mark(width);
        if self.rendered(width).lines.is_empty() {
            self.interaction = TranscriptInteraction::default();
            return;
        }
        let visual = self.visual_selection_state();
        let target = self.focusable_line_near(width, 0, true);
        self.install_cursor(width, height, target, None, true);
        self.viewport.top = 0;
        self.refresh_cursor_screen_row(height);
        self.restore_visual_selection(width, visual);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn move_cursor_by_wheel(&mut self, width: u16, height: u16, delta: isize) {
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
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
        self.set_cursor_position_with_reveal(
            width,
            height,
            target,
            cursor.preferred_column,
            cursor.preferred_column,
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

    /// Record the current cursor location as the newest entry in the Vim-style
    /// jump list. Called immediately before large navigation jumps (pages,
    /// halves, message jumps, search hits, `gg`/`G`) so `Ctrl+O` can step back
    /// to the origin. A new jump discards any redo tail, matching Vim.
    pub(crate) fn push_jump_mark(&mut self, width: u16) {
        const MAX_JUMP_HISTORY: usize = 100;
        let Some(position) = self.cursor_text_position(width) else {
            return;
        };
        if self.jump_history_index.saturating_add(1) < self.jump_history.len() {
            self.jump_history.truncate(self.jump_history_index + 1);
        }
        if self
            .jump_history
            .last()
            .is_some_and(|last| *last == position)
        {
            return;
        }
        self.jump_history.push(position);
        if self.jump_history.len() > MAX_JUMP_HISTORY {
            self.jump_history.remove(0);
        }
        self.jump_history_index = self.jump_history.len().saturating_sub(1);
    }

    /// Vim's `Ctrl+O`: step backward through the transcript jump list. If the
    /// user moved manually since the last jump, the current position is folded
    /// into the list first so the first press still goes somewhere useful.
    pub(crate) fn jump_backward(&mut self, width: u16, height: u16) {
        if self.jump_history.is_empty() {
            return;
        }
        let Some(position) = self.cursor_text_position(width) else {
            return;
        };
        if self.jump_history[self.jump_history_index] != position {
            self.push_jump_mark(width);
        }
        if self.jump_history_index == 0 {
            return;
        }
        self.jump_history_index -= 1;
        self.jump_to_position(width, height, self.jump_history[self.jump_history_index]);
    }

    /// Vim's `Ctrl+I`: step forward through the transcript jump list.
    pub(crate) fn jump_forward(&mut self, width: u16, height: u16) {
        if self.jump_history.is_empty() {
            return;
        }
        let Some(position) = self.cursor_text_position(width) else {
            return;
        };
        if self.jump_history[self.jump_history_index] != position {
            self.push_jump_mark(width);
            return;
        }
        if self.jump_history_index.saturating_add(1) >= self.jump_history.len() {
            return;
        }
        self.jump_history_index += 1;
        self.jump_to_position(width, height, self.jump_history[self.jump_history_index]);
    }

    fn jump_to_position(&mut self, width: u16, height: u16, target: TranscriptTextPosition) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            return;
        }
        self.set_cursor_position_with_reveal(
            width,
            height,
            target.line.min(total_lines.saturating_sub(1)),
            target.column,
            target.column,
            TranscriptRevealPolicy::Center,
        );
    }

    /// Vim's `Ctrl+E` / `Ctrl+Y`: scroll the viewport one rendered row while
    /// keeping the cursor as still as possible. When the scroll pushes the
    /// cursor outside the new visible window, the cursor is relocated to the
    /// nearest visible focusable row (Vim's top/bottom edge behavior).
    pub(crate) fn scroll_viewport_by_lines(
        &mut self,
        width: u16,
        height: u16,
        forward: bool,
        count: usize,
    ) {
        self.ensure_visual_focus(width, height);
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            return;
        }
        let count = count.max(1);
        let max_scroll = self.max_scroll(width, height);
        let new_top = if forward {
            self.viewport.top.saturating_add(count).min(max_scroll)
        } else {
            self.viewport.top.saturating_sub(count)
        };
        if new_top == self.viewport.top {
            return;
        }
        self.viewport.top = new_top;
        self.viewport.follow_tail = false;
        let visible = usize::from(height.max(1));
        let last_visible = new_top
            .saturating_add(visible)
            .saturating_sub(1)
            .min(total_lines.saturating_sub(1));
        if let Some(cursor) = self.interaction.cursor.clone() {
            let target = if cursor.line < new_top {
                Some(self.focusable_line_near(width, new_top, true))
            } else if cursor.line > last_visible {
                Some(self.focusable_line_near(width, last_visible, false))
            } else {
                None
            };
            if let Some(target) = target {
                self.install_cursor_at_column(
                    width,
                    height,
                    target,
                    cursor.column,
                    cursor.preferred_column,
                    None,
                    false,
                );
            } else {
                self.refresh_cursor_screen_row(height);
            }
        }
        self.sync_follow_tail(width, height);
    }

    /// Vim's `gg`/`[count]G` destination. Transcript rows without selectable
    /// content are skipped, just as the rest of navigation skips borders and
    /// layout-only rows.
    pub(crate) fn move_cursor_to_visual_line_number(
        &mut self,
        width: u16,
        height: u16,
        line_number: Option<usize>,
    ) {
        self.push_jump_mark(width);
        self.ensure_visual_focus(width, height);
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            return;
        }
        let requested = line_number
            .unwrap_or(1)
            .saturating_sub(1)
            .min(total_lines.saturating_sub(1));
        let target = self.focusable_line_near(width, requested, true);
        self.set_cursor_position_with_reveal(
            width,
            height,
            target,
            0,
            0,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Vim's `H`, `M`, and `L`: move to the first nonblank character of the
    /// top, middle, or bottom selectable row in the current viewport.
    pub(crate) fn move_cursor_to_viewport_row(
        &mut self,
        width: u16,
        height: u16,
        row: TranscriptViewportRow,
    ) {
        self.ensure_visual_focus(width, height);
        let visible = usize::from(height.max(1));
        let target = match row {
            TranscriptViewportRow::Top => self.viewport.top,
            TranscriptViewportRow::Middle => self.viewport.top.saturating_add(visible / 2),
            TranscriptViewportRow::Bottom => {
                self.viewport.top.saturating_add(visible.saturating_sub(1))
            }
        };
        let direction = match row {
            TranscriptViewportRow::Bottom => TranscriptMoveDirection::Up,
            TranscriptViewportRow::Top | TranscriptViewportRow::Middle => {
                TranscriptMoveDirection::Down
            }
        };
        let target =
            self.focusable_line_in_viewport(width, self.viewport.top, visible, target, direction);
        let column = {
            let rendered = self.rendered(width);
            rendered
                .lines
                .get(target)
                .and_then(|line| {
                    transcript_cursor_graphemes(line)
                        .into_iter()
                        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
                        .map(|(range, _)| range.start)
                })
                .unwrap_or_default()
        };
        self.set_cursor_position_with_reveal(
            width,
            height,
            target,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
    }

    /// Vim's `zt`, `zz`, and `zb`. Unlike page motions these retain the
    /// current cursor target and merely change its placement in the viewport.
    pub(crate) fn place_cursor_in_viewport(
        &mut self,
        width: u16,
        height: u16,
        row: TranscriptViewportRow,
    ) {
        let Some(cursor) = self.interaction.cursor.as_ref() else {
            return;
        };
        let visible = usize::from(height.max(1));
        let offset = match row {
            TranscriptViewportRow::Top => 0,
            TranscriptViewportRow::Middle => visible / 2,
            TranscriptViewportRow::Bottom => visible.saturating_sub(1),
        };
        self.viewport.top = cursor
            .line
            .saturating_sub(offset)
            .min(self.max_scroll(width, height));
        self.refresh_cursor_screen_row(height);
        self.sync_follow_tail(width, height);
    }

    /// Move the viewport by `distance` rendered rows and place the cursor on
    /// the directional edge of the new viewport.
    ///
    /// The page is anchored to the viewport top rather than the cursor line.
    /// Anchoring to the cursor makes the jump size depend on the cursor's
    /// screen row: paging from the bottom of the window skips up to two
    /// pages per keypress, which is especially visible when an expanded
    /// Activity fills the viewport and the cursor sits deep inside it.
    fn move_cursor_by_distance(&mut self, width: u16, height: u16, distance: usize, forward: bool) {
        self.push_jump_mark(width);
        self.ensure_visual_focus(width, height);
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            return;
        }
        let visible = usize::from(height.max(1));
        let max_scroll = total_lines.saturating_sub(visible);
        let direction = if forward {
            TranscriptMoveDirection::Down
        } else {
            TranscriptMoveDirection::Up
        };
        let new_top = if forward {
            self.viewport.top.saturating_add(distance).min(max_scroll)
        } else {
            self.viewport.top.saturating_sub(distance)
        };

        let line = if new_top != self.viewport.top {
            // Pick the directional edge of the new page. Landing on the final
            // (or first) focusable row when the page reaches the end (or start)
            // resumes tail-following instead of stranding the cursor at a
            // partially filled edge page.
            if forward && new_top == max_scroll {
                self.focusable_line_near(width, total_lines.saturating_sub(1), false)
            } else if !forward && new_top == 0 {
                self.focusable_line_near(width, 0, true)
            } else {
                let inset = usize::from(visible > 2);
                let raw_line = match direction {
                    TranscriptMoveDirection::Down => new_top.saturating_add(inset),
                    TranscriptMoveDirection::Up => new_top
                        .saturating_add(visible.saturating_sub(1).saturating_sub(inset))
                        .min(total_lines.saturating_sub(1)),
                };
                self.focusable_line_in_viewport(width, new_top, visible, raw_line, direction)
            }
        } else {
            // The viewport cannot move: either the whole transcript fits in
            // the window (few messages, short replies) or the document edge
            // is already reached. Still move the cursor by the page distance
            // so paging works on short transcripts, clamping to the document
            // boundary instead of freezing the cursor.
            let current = self
                .interaction
                .cursor
                .as_ref()
                .map(|cursor| cursor.line)
                .unwrap_or(0);
            let raw_line = if forward {
                current
                    .saturating_add(distance)
                    .min(total_lines.saturating_sub(1))
            } else {
                current.saturating_sub(distance)
            };
            self.focusable_line_near(width, raw_line, forward)
        };
        let column = self
            .interaction
            .cursor
            .as_ref()
            .map(|cursor| cursor.preferred_column)
            .unwrap_or(0);

        let visual = self.visual_selection_state();
        self.install_cursor_at_column(width, height, line, column, column, None, true);
        self.viewport.top = new_top;
        self.refresh_cursor_screen_row(height);
        self.restore_visual_selection(width, visual);
        self.sync_follow_tail(width, height);
    }

    pub(crate) fn max_scroll(&mut self, width: u16, height: u16) -> usize {
        let visible = height.max(1) as usize;
        self.rendered(width).lines.len().saturating_sub(visible)
    }

    #[cfg(test)]
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
        let visual = self.visual_selection_state();
        self.install_cursor(
            width,
            height,
            target.min(total_lines.saturating_sub(1)),
            None,
            true,
        );
        self.reveal_current_cursor(width, height, reveal);
        self.restore_visual_selection(width, visual);
        self.sync_follow_tail(width, height);
    }

    fn set_cursor_position_with_reveal(
        &mut self,
        width: u16,
        height: u16,
        target: usize,
        column: usize,
        preferred_column: usize,
        reveal: TranscriptRevealPolicy,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction = TranscriptInteraction::default();
            return;
        }
        let visual = self.visual_selection_state();
        self.install_cursor_at_column(
            width,
            height,
            target.min(total_lines.saturating_sub(1)),
            column,
            preferred_column,
            None,
            true,
        );
        self.reveal_current_cursor(width, height, reveal);
        self.restore_visual_selection(width, visual);
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
        self.interaction.visual_selection = None;
        self.interaction.visual_anchor = None;
        self.viewport.follow_tail = false;
        selection
    }

    pub(crate) fn cancel_text_selection(&mut self, width: u16, height: u16) {
        self.remember_visual_selection(width);
        self.interaction.text_selection = None;
        self.interaction.visual_selection = None;
        self.interaction.visual_anchor = None;
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
        self.install_cursor_at_column(
            width,
            height,
            line,
            position.column,
            position.column,
            None,
            true,
        );
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
                        (node.key.is_entry_container()
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
        self.install_cursor_at_column(
            width,
            height,
            target,
            0,
            0,
            block_cursor,
            clear_text_selection,
        );
    }

    fn install_cursor_at_column(
        &mut self,
        width: u16,
        height: u16,
        target: usize,
        requested_column: usize,
        preferred_column: usize,
        block_cursor: Option<TranscriptBlockCursor>,
        clear_text_selection: bool,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.interaction.cursor = None;
            return;
        }
        let line = target.min(total_lines.saturating_sub(1));
        let (anchor, column) = {
            let rendered = self.rendered(width);
            let anchor = rendered
                .line_nodes
                .get(line)
                .and_then(|index| *index)
                .and_then(|index| rendered.nodes.get(index))
                .map(|node| TranscriptCursorAnchor {
                    key: node.key.clone(),
                    line_offset: line.saturating_sub(node.start_line),
                });
            let column = rendered
                .lines
                .get(line)
                .map(|line| transcript_cursor_column_for_line(line, requested_column))
                .unwrap_or_default();
            (anchor, column)
        };
        let preferred_screen_row = line
            .saturating_sub(self.viewport.top)
            .min(usize::from(height.max(1)).saturating_sub(1));
        self.interaction.cursor = Some(TranscriptCursor {
            line,
            column,
            preferred_column,
            anchor,
            block_cursor,
            preferred_screen_row,
        });
        if clear_text_selection {
            self.interaction.text_selection = None;
            self.interaction.visual_selection = None;
            self.interaction.visual_anchor = None;
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
        self.install_cursor_at_column(
            width,
            height,
            line,
            cursor.column,
            cursor.preferred_column,
            block_cursor,
            false,
        );
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
        let view = agena_tui_transcript::project_view(
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
        // Track which Operation Activities have their detail expanded. Live
        // streaming deltas are only applied to these; collapsing stops detail
        // computation and transfer for that Activity.
        if let TranscriptNodeKey::Activity { content_id, .. } = &node.key
            && let TranscriptContentId::Activity(activity_id) = content_id
        {
            if expanded {
                self.expanded_operation_activity_ids.insert(*activity_id);
            } else {
                self.expanded_operation_activity_ids.remove(activity_id);
            }
        }
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
        self.viewport.follow_tail = !self.has_active_text_selection()
            && self.navigation_cursor_line() == Some(last_line)
            && self.viewport.top >= self.max_scroll(width, height);
    }
}

fn merge_v2_delta(
    live_blocks: &mut Vec<agena_domain::ViewBlock>,
    delta: &agena_domain::RenderDelta,
) {
    let block_id = delta.block_id.as_deref().or_else(|| delta.view.block_id());
    match delta.mode {
        agena_domain::DeltaMode::New => live_blocks.push(delta.view.clone()),
        agena_domain::DeltaMode::Append => {
            if let Some(id) = block_id
                && let Some(block) = live_blocks.iter_mut().find(|b| b.block_id() == Some(id))
            {
                append_to_v2_block(block, &delta.view);
                return;
            }
            live_blocks.push(delta.view.clone());
        }
        agena_domain::DeltaMode::Replace => {
            if let Some(id) = block_id
                && let Some(block) = live_blocks.iter_mut().find(|b| b.block_id() == Some(id))
            {
                *block = delta.view.clone();
                return;
            }
            live_blocks.push(delta.view.clone());
        }
    }
}

fn append_to_v2_block(target: &mut agena_domain::ViewBlock, incoming: &agena_domain::ViewBlock) {
    let append_text = match incoming {
        agena_domain::ViewBlock::Text { text, .. }
        | agena_domain::ViewBlock::Markdown { text, .. }
        | agena_domain::ViewBlock::Log { text, .. } => Some(text.as_str()),
        _ => None,
    };
    if let Some(text) = append_text
        && let agena_domain::ViewBlock::Text { text: t, .. }
        | agena_domain::ViewBlock::Markdown { text: t, .. }
        | agena_domain::ViewBlock::Log { text: t, .. } = target
    {
        t.push_str(text);
    }
}

/// Render one unified activity-v2 `ViewBlock` into display lines. This is the
/// single rendering entry point for every ViewBlock variant (07 §5.2): the
/// terminal handles all 11 variants here, mirroring the wire shape Web
/// receives.
pub(crate) fn render_activity_block(block: &agena_domain::ViewBlock) -> Vec<String> {
    match block {
        agena_domain::ViewBlock::Text { text, .. } => vec![text.clone()],
        agena_domain::ViewBlock::Markdown { text, .. } => vec![text.clone()],
        agena_domain::ViewBlock::Json { value, .. } => {
            vec![serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())]
        }
        agena_domain::ViewBlock::Table { columns, rows, .. } => {
            let mut lines = vec![columns.join(" | ")];
            lines.extend(rows.iter().map(|row| {
                row.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ")
            }));
            lines
        }
        agena_domain::ViewBlock::Log { stream, text, .. } => {
            let prefix = match stream {
                agena_domain::CommandOutputStream::Stdout => "",
                agena_domain::CommandOutputStream::Stderr => "[stderr] ",
            };
            text.lines().map(|line| format!("{prefix}{line}")).collect()
        }
        agena_domain::ViewBlock::Command {
            command,
            cwd,
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            let mut lines = Vec::new();
            let cwd_suffix = cwd
                .as_deref()
                .map(|c| format!("  (cwd: {c})"))
                .unwrap_or_default();
            lines.push(format!("$ {command}{cwd_suffix}"));
            lines.extend(stdout.lines().map(str::to_owned));
            lines.extend(stderr.lines().map(|line| format!("[stderr] {line}")));
            if let Some(code) = exit_code {
                lines.push(format!("exit code: {code}"));
            }
            lines
        }
        agena_domain::ViewBlock::FileChanges { changes, .. } => {
            changes.iter().map(|change| format!("{change:?}")).collect()
        }
        agena_domain::ViewBlock::Diff { diff, .. } => diff.lines().map(str::to_owned).collect(),
        agena_domain::ViewBlock::SearchResults { items, total, .. } => {
            let mut lines = Vec::new();
            if let Some(total) = total {
                lines.push(format!("{total} results"));
            }
            for item in items {
                lines.push(item.title.clone());
                lines.push(format!("  {}", item.url));
                if let Some(snippet) = &item.snippet {
                    lines.push(format!("    {snippet}"));
                }
            }
            lines
        }
        agena_domain::ViewBlock::Media { artifact, .. } => {
            vec![format!("media: {}", artifact.uri)]
        }
        agena_domain::ViewBlock::Custom {
            kind, presentation, ..
        } => {
            if presentation.is_empty() {
                vec![format!("[{kind}]")]
            } else {
                presentation.values().cloned().collect()
            }
        }
    }
}

fn weave_pending_user_entries<'a>(
    snapshot: &'a agena_domain::TranscriptSnapshot,
    canonical_entries: Vec<agena_tui_transcript::TranscriptEntry<'a>>,
    pending_messages: &'a [PendingUserMessage],
) -> Vec<agena_tui_transcript::TranscriptEntry<'a>> {
    let empty_active_replies = snapshot
        .turns
        .iter()
        .filter(|turn| turn.input.is_empty() && !turn.reply.status.is_terminal())
        .map(|turn| turn.reply.id)
        .collect::<BTreeSet<_>>();
    let mut pending = pending_messages.iter();
    let mut entries = Vec::with_capacity(
        canonical_entries
            .len()
            .saturating_add(pending_messages.len()),
    );
    for entry in canonical_entries {
        if matches!(
            entry.id,
            agena_tui_transcript::TranscriptEntryId::AssistantReply(reply_id)
                if empty_active_replies.contains(&reply_id)
        ) && let Some(message) = pending.next()
        {
            entries.push(agena_tui_transcript::pending_user_entry(
                message.id,
                message.confirmed,
                &message.document,
            ));
        }
        entries.push(entry);
    }
    entries.extend(pending.map(|message| {
        agena_tui_transcript::pending_user_entry(message.id, message.confirmed, &message.document)
    }));
    entries
}

fn transcript_patch_can_materialize_user_input(patch: &agena_domain::TranscriptPatch) -> bool {
    match patch {
        agena_domain::TranscriptPatch::TurnOpened { turn, .. } => !turn.input.is_empty(),
        agena_domain::TranscriptPatch::ContentUpserted { owner, .. } => {
            matches!(owner, agena_domain::ActivityOwner::TurnInput { .. })
        }
        agena_domain::TranscriptPatch::AssistantReplyUpdated { .. } => false,
        agena_domain::TranscriptPatch::ContentRemoved { .. } => false,
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

/// Return the display-cell span for every cursorable grapheme in a rendered
/// row. Zero-width marks remain part of their grapheme cluster and do not
/// produce a second cursor stop of their own.
fn transcript_cursor_grapheme_ranges(line: &RenderedLine) -> Vec<Range<usize>> {
    let mut column = 0_usize;
    line.text
        .graphemes(true)
        .filter_map(|grapheme| {
            let width = UnicodeWidthStr::width(grapheme);
            let start = column;
            column = column.saturating_add(width);
            (width > 0 && start >= line.copy_column).then_some(start..column)
        })
        .collect()
}

fn transcript_cursor_graphemes(line: &RenderedLine) -> Vec<(Range<usize>, String)> {
    let mut column = 0_usize;
    line.text
        .graphemes(true)
        .filter_map(|grapheme| {
            let width = UnicodeWidthStr::width(grapheme);
            let start = column;
            column = column.saturating_add(width);
            (width > 0 && start >= line.copy_column).then(|| (start..column, grapheme.to_owned()))
        })
        .collect()
}

#[derive(Debug, Clone)]
struct TranscriptGraphemePosition {
    position: TranscriptTextPosition,
    grapheme: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptWordClass {
    Whitespace,
    Keyword,
    Punctuation,
}

fn transcript_word_class(grapheme: &str, big_word: bool) -> TranscriptWordClass {
    if grapheme.chars().all(char::is_whitespace) {
        TranscriptWordClass::Whitespace
    } else if big_word
        || grapheme
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        TranscriptWordClass::Keyword
    } else {
        TranscriptWordClass::Punctuation
    }
}

fn transcript_word_motion_target(
    positions: &[TranscriptGraphemePosition],
    current: usize,
    forward: bool,
    to_end: bool,
    big_word: bool,
) -> Option<usize> {
    let class_at =
        |index: usize| transcript_word_class(positions[index].grapheme.as_str(), big_word);
    let len = positions.len();
    (current < len).then_some(())?;

    if forward && !to_end {
        let current_class = class_at(current);
        let mut index = current.saturating_add(1);
        if current_class != TranscriptWordClass::Whitespace {
            while index < len && class_at(index) == current_class {
                index = index.saturating_add(1);
            }
        }
        while index < len && class_at(index) == TranscriptWordClass::Whitespace {
            index = index.saturating_add(1);
        }
        return (index < len).then_some(index);
    }

    if forward {
        let mut index = current;
        if class_at(index) == TranscriptWordClass::Whitespace {
            while index < len && class_at(index) == TranscriptWordClass::Whitespace {
                index = index.saturating_add(1);
            }
            if index == len {
                return None;
            }
        }
        let class = class_at(index);
        while index.saturating_add(1) < len && class_at(index.saturating_add(1)) == class {
            index = index.saturating_add(1);
        }
        return Some(index);
    }

    if !to_end {
        let current_class = class_at(current);
        if current_class != TranscriptWordClass::Whitespace
            && current > 0
            && class_at(current.saturating_sub(1)) == current_class
        {
            let mut index = current;
            while index > 0 && class_at(index.saturating_sub(1)) == current_class {
                index = index.saturating_sub(1);
            }
            return Some(index);
        }
        let mut index = current.checked_sub(1)?;
        while class_at(index) == TranscriptWordClass::Whitespace {
            index = index.checked_sub(1)?;
        }
        let class = class_at(index);
        while index > 0 && class_at(index.saturating_sub(1)) == class {
            index = index.saturating_sub(1);
        }
        return Some(index);
    }

    let mut index = current.checked_sub(1)?;
    while class_at(index) == TranscriptWordClass::Whitespace {
        index = index.checked_sub(1)?;
    }
    let class = class_at(index);
    while index.saturating_add(1) < len && class_at(index.saturating_add(1)) == class {
        index = index.saturating_add(1);
    }
    Some(index)
}

fn transcript_cursor_grapheme_index(graphemes: &[Range<usize>], column: usize) -> Option<usize> {
    graphemes
        .iter()
        .position(|range| range.contains(&column))
        .or_else(|| graphemes.iter().rposition(|range| range.start <= column))
        .or_else(|| (!graphemes.is_empty()).then_some(0))
}

fn transcript_cursor_column_for_line(line: &RenderedLine, requested_column: usize) -> usize {
    let graphemes = transcript_cursor_grapheme_ranges(line);
    transcript_cursor_grapheme_index(graphemes.as_slice(), requested_column)
        .map(|index| graphemes[index].start)
        .unwrap_or_default()
}

fn transcript_cursor_cell_range(line: &RenderedLine, column: usize) -> Range<usize> {
    let graphemes = transcript_cursor_grapheme_ranges(line);
    transcript_cursor_grapheme_index(graphemes.as_slice(), column)
        .map(|index| graphemes[index].clone())
        // An empty rendered row still has a visible Vim-style cursor cell.
        .unwrap_or(0..1)
}

use super::TranscriptAction;
use crate::{
    BTreeMap, BTreeSet, Duration, I18n, PendingUserMessage, Range, RenderedLine,
    RenderedTranscript, RenderedTranscriptNode, SessionExecutionResource, TranscriptBlockCursor,
    TranscriptBlockSelectionMode, TranscriptContentId, TranscriptCursor, TranscriptCursorAnchor,
    TranscriptDetailDefaults, TranscriptInteraction, TranscriptMoveDirection, TranscriptNodeKey,
    TranscriptNodeKind, TranscriptState, TranscriptTextPosition, TranscriptTextSelection,
    TranscriptViewport, TranscriptVisualSelectionMode, TranscriptVisualSelectionSnapshot,
    V2LiveActivity, contains_case_insensitive, initial_search_match_index, min,
    normalize_transcript_text_selection, render_entry_detailed, transcript_entries,
    transcript_node_highlight_range, transcript_selection_scroll_position,
    transcript_text_selection_text, ui_text,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
use crate::{
    TranscriptVerticalNavigationStep, transcript_message_navigation_target,
    transcript_semantic_line_range, transcript_should_fall_back_to_message_navigation,
    transcript_vertical_line_navigation_step, transcript_vertical_navigation_step,
};

#[cfg(test)]
mod stall_recovery_tests {
    use super::*;
    use std::time::Instant;

    fn state() -> TranscriptState {
        TranscriptState::new(
            I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: false,
            },
        )
    }

    #[test]
    fn elapsed_in_flight_requests_are_force_cleared() {
        let mut state = state();
        state.refreshing = true;
        state.state_loading = true;
        state.refresh_in_flight_since = Some(Instant::now());
        state.state_load_in_flight_since = Some(Instant::now());

        // A zero timeout has already elapsed for both requests: the wedge
        // must be broken so the periodic refresh can resume.
        assert!(state.recover_stalled_requests(Duration::ZERO));
        assert!(!state.refreshing);
        assert!(!state.state_loading);
        assert!(state.refresh_in_flight_since.is_none());
        assert!(state.state_load_in_flight_since.is_none());
    }

    #[test]
    fn fresh_in_flight_requests_survive_the_recovery_pass() {
        let mut state = state();
        state.refreshing = true;
        state.state_loading = true;
        state.refresh_in_flight_since = Some(Instant::now());
        state.state_load_in_flight_since = Some(Instant::now());

        assert!(!state.recover_stalled_requests(Duration::from_secs(3600)));
        assert!(state.refreshing);
        assert!(state.state_loading);
    }

    #[test]
    fn in_progress_reply_counts_as_non_terminal_until_completed() {
        let mut state = state();
        let turn_id = agena_domain::TurnId::new();
        state.snapshot.turns.push(agena_domain::TurnSnapshot {
            id: turn_id,
            session_id: 1,
            sequence: 1,
            input: agena_domain::ContentDocument::default(),
            reply: agena_domain::AssistantReplySnapshot {
                id: agena_domain::AssistantReplyId::new(),
                turn_id,
                status: agena_domain::AssistantReplyStatus::InProgress,
                content: agena_domain::ContentDocument::default(),
                revision_seq: 1,
                created_at_ms: 0,
                finished_at_ms: None,
                failure: None,
            },
            created_at_ms: 0,
        });

        assert!(state.has_non_terminal_replies());

        state.snapshot.turns[0].reply.status = agena_domain::AssistantReplyStatus::Completed;
        assert!(!state.has_non_terminal_replies());
    }
}

#[cfg(test)]
mod activity_v2_tests {
    use super::*;
    use agena_domain::{ActivityId, ActivityState, CommandOutputStream, RenderDelta, ViewBlock};
    use agena_runtime::{
        RuntimePresentationEvent, RuntimePresentationEventKind, RuntimePresentationEventMeta,
        session::activity::{ActivityKind, ActivityLiveEvent, ActivityStateNode},
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn state() -> TranscriptState {
        TranscriptState::new(
            I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: false,
            },
        )
    }

    fn v2_event(activity: ActivityLiveEvent, seq: i64) -> RuntimePresentationEvent {
        RuntimePresentationEvent {
            meta: RuntimePresentationEventMeta {
                id: Uuid::new_v4(),
                seq_global: seq,
                seq_session: Some(seq),
                session_id: Some(7),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection: false,
            // ActivityV2 is live-only: it must never advance the durable
            // watermark used for staleness comparisons.
            durable: false,
            kind: RuntimePresentationEventKind::ActivityV2(Box::new(activity)),
        }
    }

    #[test]
    fn render_activity_block_handles_all_variants() {
        let cases: Vec<(ViewBlock, &str)> = vec![
            (
                ViewBlock::Text {
                    id: None,
                    text: "hello".into(),
                },
                "hello",
            ),
            (
                ViewBlock::Markdown {
                    id: None,
                    text: "# hi".into(),
                },
                "# hi",
            ),
            (
                ViewBlock::Json {
                    id: None,
                    value: serde_json::json!({ "a": 1 }),
                },
                "{",
            ),
            (
                ViewBlock::Table {
                    id: None,
                    columns: vec!["name".into()],
                    rows: vec![vec![serde_json::json!("v")]],
                },
                "name",
            ),
            (
                ViewBlock::Log {
                    id: None,
                    stream: CommandOutputStream::Stderr,
                    text: "boom\nnext".into(),
                },
                "[stderr] boom",
            ),
            (
                ViewBlock::Command {
                    id: None,
                    command: "cargo test".into(),
                    cwd: Some("/tmp".into()),
                    exit_code: Some(0),
                    stdout: "ok".into(),
                    stderr: String::new(),
                },
                "$ cargo test  (cwd: /tmp)",
            ),
            (
                ViewBlock::FileChanges {
                    id: None,
                    changes: vec![agena_domain::FileChangeRecord {
                        path: "a.txt".into(),
                        kind: agena_domain::FileChangeKind::Updated,
                        from_path: None,
                    }],
                },
                "a.txt",
            ),
            (
                ViewBlock::Diff {
                    id: None,
                    diff: "-a\n+b".into(),
                    language: None,
                },
                "-a",
            ),
            (
                ViewBlock::SearchResults {
                    id: None,
                    items: vec![agena_domain::WebSearchResult {
                        title: "doc".into(),
                        url: "https://example.test".into(),
                        snippet: None,
                    }],
                    total: Some(1),
                },
                "1 results",
            ),
            (
                ViewBlock::Media {
                    id: None,
                    artifact: agena_domain::ArtifactRef {
                        uri: "file:///tmp/x.png".into(),
                        mime: "image/png".into(),
                        name: None,
                        size_bytes: None,
                        sha256: None,
                    },
                },
                "media: file:///tmp/x.png",
            ),
            (
                ViewBlock::Custom {
                    id: None,
                    kind: "badge".into(),
                    schema: serde_json::Value::Null,
                    presentation: std::collections::BTreeMap::new(),
                },
                "[badge]",
            ),
        ];
        for (block, expected) in cases {
            let lines = render_activity_block(&block);
            assert!(
                lines.iter().any(|line| line.contains(expected)),
                "variant {block:?} rendered {lines:?} without {expected}"
            );
        }
    }

    #[test]
    fn activity_v2_events_drive_the_live_overlay() {
        let activity_id = ActivityId::new();
        let mut transcript = state();
        transcript
            .expanded_operation_activity_ids
            .insert(activity_id);

        // New + Append deltas merge into live blocks when expanded.
        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::DetailDelta {
                    activity_id,
                    delta: RenderDelta::new(ViewBlock::log(
                        "out",
                        CommandOutputStream::Stdout,
                        "a\n",
                    )),
                },
                1,
            ),
            80,
            20,
        );
        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::DetailDelta {
                    activity_id,
                    delta: RenderDelta::append(
                        "out",
                        ViewBlock::log("out", CommandOutputStream::Stdout, "b\n"),
                    ),
                },
                2,
            ),
            80,
            20,
        );
        let entry = transcript
            .v2_activities
            .get(&activity_id)
            .expect("overlay entry");
        assert_eq!(entry.live_blocks.len(), 1);
        match &entry.live_blocks[0] {
            ViewBlock::Log { text, .. } => assert_eq!(text, "a\nb\n"),
            other => panic!("expected log block, got {other:?}"),
        }

        // Collapsed activities ignore detail deltas.
        transcript.expanded_operation_activity_ids.clear();
        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::DetailDelta {
                    activity_id,
                    delta: RenderDelta::append(
                        "out",
                        ViewBlock::log("out", CommandOutputStream::Stdout, "c\n"),
                    ),
                },
                3,
            ),
            80,
            20,
        );
        assert_eq!(
            transcript
                .v2_activities
                .get(&activity_id)
                .unwrap()
                .live_blocks[0],
            ViewBlock::log("out", CommandOutputStream::Stdout, "a\nb\n")
        );

        // Title/state/upsert/removed lifecycle.
        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::TitleChanged {
                    activity_id,
                    title: "cargo test".into(),
                },
                4,
            ),
            80,
            20,
        );
        assert_eq!(
            transcript.v2_activities.get(&activity_id).unwrap().title,
            "cargo test"
        );

        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::StateChanged {
                    activity_id,
                    state: ActivityState::Completed,
                },
                5,
            ),
            80,
            20,
        );
        assert_eq!(
            transcript.v2_activities.get(&activity_id).unwrap().state,
            ActivityState::Completed
        );

        let node = ActivityStateNode {
            activity_id,
            kind: ActivityKind::Operation,
            title: "shell.run".into(),
            summary: String::new(),
            state: ActivityState::Completed,
            raw_output: None,
            sections: Vec::new(),
        };
        transcript.apply_presentation_event(
            &v2_event(
                ActivityLiveEvent::Upserted {
                    node: Box::new(node),
                },
                6,
            ),
            80,
            20,
        );
        let entry = transcript
            .v2_activities
            .get(&activity_id)
            .expect("upserted");
        assert_eq!(entry.title, "shell.run");
        assert_eq!(entry.state, ActivityState::Completed);
        assert_eq!(entry.live_blocks.len(), 1, "upsert keeps merged blocks");

        transcript.apply_presentation_event(
            &v2_event(ActivityLiveEvent::Removed { activity_id }, 7),
            80,
            20,
        );
        assert!(!transcript.v2_activities.contains_key(&activity_id));
    }
}
