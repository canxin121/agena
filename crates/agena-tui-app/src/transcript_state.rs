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
            pending_user_messages: Vec::new(),
            refreshing: false,
            state_loading: false,
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
        self.invalidate_render();
    }

    pub(crate) fn apply_execution(&mut self, execution: SessionExecutionResource) {
        self.session_title = execution.session.title.clone();
        self.last_event_seq = execution.latest_event_seq;
        self.merge_snapshot(execution.transcript.clone());
        self.execution = Some(execution);
        self.invalidate_render();
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

    /// Apply the Runtime-owned live projection used by the terminal backend.
    /// The terminal never reconstructs a concrete event envelope from JSON.
    pub(crate) fn apply_presentation_event(
        &mut self,
        event: &agena_runtime::RuntimePresentationEvent,
        width: u16,
        height: u16,
    ) -> bool {
        let refresh_needed = match &event.kind {
            agena_runtime::RuntimePresentationEventKind::TranscriptPatch(patch) => {
                if transcript_patch_can_materialize_user_input(patch) {
                    self.apply_snapshot_change(|snapshot| snapshot.apply((**patch).clone()));
                } else {
                    self.snapshot.apply((**patch).clone());
                }
                self.invalidate_render();
                false
            }
            agena_runtime::RuntimePresentationEventKind::OperationDetailDelta {
                activity_id,
                delta,
            } => {
                // A live slice of a streaming tool's output. Append it to the
                // matching Operation's derived detail so the expanded Activity
                // renders the growing output in real time. Collapsed Activities
                // are unaffected — their detail stays unreferenced.
                self.append_live_operation_detail(*activity_id, delta);
                self.invalidate_render();
                false
            }
            agena_runtime::RuntimePresentationEventKind::Refresh { .. } => true,
        };

        if !refresh_needed {
            self.last_event_seq = Some(event.meta.seq_global);
        }

        if self.viewport.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }

        refresh_needed
    }

    /// Append a live streaming-output delta to an Operation Activity's detail.
    ///
    /// The detail is derived at render time; a streaming tool appends raw
    /// output to the compact result's derived Markdown so the expanded
    /// Activity shows live progress. Nothing here is persisted — this is pure
    /// in-memory presentation state.
    fn append_live_operation_detail(
        &mut self,
        activity_id: agena_domain::ActivityId,
        delta: &str,
    ) {
        // Only expanded Operations receive live detail. Collapsed Activities
        // (or non-Operations) are untouched — this is what lets expanding
        // start and collapsing stop detail computation and transfer.
        if !self.expanded_operation_activity_ids.contains(&activity_id) {
            return;
        }
        let append = |activity: &mut agena_domain::ActivityNode| {
            if activity.id != activity_id {
                return false;
            }
            if let agena_domain::ActivityPayload::Operation(operation) = &mut activity.payload {
                operation.markdown.push_str(delta);
                return true;
            }
            false
        };
        let mut matched = false;
        for turn in &mut self.snapshot.turns {
            for node in &mut turn.input.0 {
                if let agena_domain::ContentNode::Activity { activity } = node {
                    matched |= append(activity);
                }
            }
            for node in &mut turn.reply.content.0 {
                if let agena_domain::ContentNode::Activity { activity } = node {
                    matched |= append(activity);
                }
            }
        }
        for activity in &mut self.snapshot.session_activities {
            matched |= append(activity);
        }
        let _ = matched;
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
        let snapshot_entries = transcript_entries(&self.snapshot);
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

    fn move_cursor_by_distance(&mut self, width: u16, height: u16, distance: usize, forward: bool) {
        self.push_jump_mark(width);
        self.ensure_visual_focus(width, height);
        let Some(cursor) = self.interaction.cursor.clone() else {
            return;
        };
        let line = cursor.line;
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
        self.set_cursor_position_with_reveal(
            width,
            height,
            target,
            cursor.preferred_column,
            cursor.preferred_column,
            TranscriptRevealPolicy::DirectionalEdge(direction),
        );
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
    BTreeMap, BTreeSet, I18n, PendingUserMessage, Range, RenderedLine, RenderedTranscript,
    RenderedTranscriptNode, SessionExecutionResource, TranscriptBlockCursor,
    TranscriptBlockSelectionMode, TranscriptCursor, TranscriptCursorAnchor,
    TranscriptContentId, TranscriptDetailDefaults, TranscriptInteraction, TranscriptMoveDirection,
    TranscriptNodeKey, TranscriptNodeKind, TranscriptState, TranscriptTextPosition,
    TranscriptTextSelection,
    TranscriptViewport, TranscriptVisualSelectionMode, TranscriptVisualSelectionSnapshot,
    contains_case_insensitive, initial_search_match_index, min,
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
