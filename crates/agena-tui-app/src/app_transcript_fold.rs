use crate::{App, Editor, I18n, Overlay, TranscriptNodeKey, TranscriptState};
use agena_tui::input::ComposerAction;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_components::{InputDialogKeyResult, InputDialogState, drive_input_dialog_key};
use agena_tui_transcript::{
    TranscriptActivityContent, TranscriptContentId, TranscriptEntryId, TranscriptFold,
    TranscriptPartContent,
};
use crossterm::event::KeyEvent;

pub(crate) type FoldLoadDialog = InputDialogState<()>;
const MAX_LOAD_COUNT: usize = 50;

enum FoldLoadEffect {
    Continue,
    Cancel,
    Submit(usize),
}

fn count_prompt(i18n: &I18n, invalid: bool) -> String {
    i18n.text_args(
        if invalid {
            "transcript-load-count-invalid"
        } else {
            "transcript-load-count-prompt"
        },
        &agena_tui::fl_args!("max" => MAX_LOAD_COUNT as i64),
    )
}

fn count_dialog(count: usize, i18n: &I18n) -> FoldLoadDialog {
    InputDialogState::new(
        i18n.text("transcript-load-count-title"),
        count_prompt(i18n, false),
        Editor::from_text(count.to_string()),
        (),
    )
}

fn reduce_count_key(dialog: &mut FoldLoadDialog, key: KeyEvent, i18n: &I18n) -> FoldLoadEffect {
    let previous = dialog.input.text().to_owned();
    match drive_input_dialog_key(dialog, key) {
        InputDialogKeyResult::Close => FoldLoadEffect::Cancel,
        InputDialogKeyResult::Continue => {
            if dialog.input.text() != previous {
                dialog.prompt = count_prompt(i18n, false);
            }
            FoldLoadEffect::Continue
        }
        InputDialogKeyResult::Submit((), input) => {
            let input = input.trim();
            let count = input
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| input.parse::<usize>().ok())
                .flatten()
                .filter(|count| (1..=MAX_LOAD_COUNT).contains(count));
            match count {
                Some(count) => FoldLoadEffect::Submit(count),
                None => {
                    dialog.prompt = count_prompt(i18n, true);
                    FoldLoadEffect::Continue
                }
            }
        }
    }
}

impl TranscriptState {
    pub(crate) fn cursor_is_on_transcript_fold_marker(&mut self, width: u16) -> bool {
        self.current_cursor_node_cloned(width).is_some_and(|node| {
            matches!(
                node.key,
                TranscriptNodeKey::ActivitySummary { .. }
                    | TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::TranscriptFold { .. },
                        ..
                    }
            )
        })
    }

    pub(crate) fn fold_hidden_part_count(&mut self, width: u16, key: &TranscriptNodeKey) -> usize {
        self.rendered(width);
        self.projected_entries
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == key.entry_id()))
            .map_or(0, |entry| {
                agena_tui_transcript::entry_hidden_part_count(
                    entry,
                    &self.node_expansions,
                    &self.activity_summary_visible_counts,
                )
            })
    }

    /// Reveal a contiguous older slice of this reply. Missing server parts
    /// occupy positions in the same window as already loaded activities.
    pub(crate) fn reveal_older_reply_parts(
        &mut self,
        width: u16,
        height: u16,
        entry_id: TranscriptEntryId,
        count: usize,
    ) -> Vec<(TranscriptFold, usize)> {
        self.rendered(width);
        let Some(entry) = self
            .projected_entries
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == entry_id))
            .cloned()
        else {
            return Vec::new();
        };
        let loading = entry.parts.iter().any(|part| {
            matches!(part.id, TranscriptContentId::TranscriptFold { run_id, .. }
                if self.transcript_fold_loads.keys().any(|(id, _)| *id == run_id))
        });
        if loading {
            return Vec::new();
        }
        let hidden = agena_tui_transcript::entry_hidden_part_count(
            &entry,
            &self.node_expansions,
            &self.activity_summary_visible_counts,
        );
        if hidden == 0 {
            return Vec::new();
        }
        let visible = agena_tui_transcript::entry_visible_activity_count(
            &entry,
            &self.node_expansions,
            &self.activity_summary_visible_counts,
        );
        let desired = visible.saturating_add(count.clamp(1, MAX_LOAD_COUNT).min(hidden));
        self.allow_reply_part_reveal(entry_id);
        self.activity_summary_visible_counts
            .retain(|key, _| key.entry_id() != entry_id);
        self.node_expansions.retain(|key, _| {
            key.entry_id() != entry_id || !matches!(key, TranscriptNodeKey::ActivitySummary { .. })
        });
        for key in agena_tui_transcript::entry_activity_summary_keys(&entry) {
            self.activity_summary_visible_counts.insert(key, desired);
        }
        self.invalidate_render();
        self.ensure_visual_focus(width, height);
        self.fold_requests_in_window(width, entry_id)
    }

    pub(crate) fn fold_entry_id(&mut self, width: u16, run_id: i64) -> Option<TranscriptEntryId> {
        self.rendered(width);
        self.projected_entries.as_ref()?.iter().find(|entry| entry.parts.iter().any(|part| {
            matches!(part.id, TranscriptContentId::TranscriptFold { run_id: id, .. } if id == run_id)
        })).map(|entry| entry.id)
    }

    pub(crate) fn fold_requests_in_window(
        &mut self,
        width: u16,
        entry_id: TranscriptEntryId,
    ) -> Vec<(TranscriptFold, usize)> {
        if self.activity_summary_collapsed_entries.contains(&entry_id) {
            return Vec::new();
        }
        self.rendered(width);
        let Some(entry) = self
            .projected_entries
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == entry_id))
        else {
            return Vec::new();
        };
        let mut budget = self
            .activity_summary_visible_counts
            .iter()
            .filter(|(key, _)| key.entry_id() == entry_id)
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(agena_tui_transcript::COLLAPSED_ACTIVITY_VISIBLE_COUNT);
        let mut requests = Vec::new();
        for part in entry.parts.iter().rev() {
            match &part.content {
                TranscriptPartContent::Activity(TranscriptActivityContent::Fold {
                    hidden_count,
                }) => {
                    let count = budget.min(*hidden_count).min(MAX_LOAD_COUNT);
                    if count > 0
                        && let Some(fold) =
                            self.transcript_fold_for_node(&TranscriptNodeKey::Activity {
                                entry_id,
                                content_id: part.id,
                            })
                    {
                        requests.push((fold, count));
                    }
                    budget = budget.saturating_sub(*hidden_count);
                }
                TranscriptPartContent::Activity(_) => budget = budget.saturating_sub(1),
                _ => {}
            }
        }
        requests
    }
}

impl App {
    /// The transcript remains visible while the Composer has focus. Keep its
    /// fold actions available there too, while preserving Enter queueing and
    /// Ctrl+Enter submission whenever there is a draft to send.
    pub(crate) fn handle_composer_transcript_fold_shortcut(&mut self, key: KeyEvent) -> bool {
        let Some(action) = resolve_tui_key(KeyContext::Transcript, key) else {
            return false;
        };
        let width = self.layout.transcript_body.width;
        let hidden_parts_selected = self
            .transcript
            .current_cursor_node_cloned(width)
            .is_some_and(|node| self.transcript.fold_hidden_part_count(width, &node.key) > 0);
        let composer_action = self.keybindings.match_action(&key);
        let composer_is_empty = self.composer.text().is_empty() && self.composer_items.is_empty();

        match action {
            KeyAction::LoadOlderParts
                if hidden_parts_selected
                    && self.transcript.cursor_is_on_transcript_fold_marker(width)
                    && composer_action == Some(ComposerAction::Queue)
                    && composer_is_empty =>
            {
                self.clear_transcript_pending_command();
                self.load_older_transcript_parts();
                true
            }
            KeyAction::EditFoldCount
                if hidden_parts_selected
                    && (composer_action.is_none()
                        || (composer_action == Some(ComposerAction::Submit)
                            && composer_is_empty)) =>
            {
                self.clear_transcript_pending_command();
                self.open_transcript_fold_load();
                true
            }
            KeyAction::ExpandAll if composer_action.is_none() => {
                self.clear_transcript_pending_command();
                self.expand_all_transcript_parts_from_shortcut();
                true
            }
            _ => false,
        }
    }

    pub(crate) fn expand_all_transcript_parts_from_shortcut(&mut self) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let folds = self.transcript.transcript_folds.clone();
        self.transcript.expand_all_transcript_parts(width, height);
        for fold in folds {
            self.request_transcript_fold_parts(fold, true, 50);
        }
    }

    pub(crate) fn open_transcript_fold_load(&mut self) {
        self.overlay = Some(Overlay::TranscriptFoldLoad(count_dialog(
            self.transcript.fold_load_count,
            &self.i18n,
        )));
    }

    pub(crate) fn load_older_transcript_parts(&mut self) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        let count = self.transcript.fold_load_count;
        let hidden = self.transcript.fold_hidden_part_count(width, &node.key);
        if hidden == 0 {
            return;
        }
        let requests =
            self.transcript
                .reveal_older_reply_parts(width, height, node.key.entry_id(), count);
        for (fold, count) in requests {
            self.request_transcript_fold_parts(fold, false, count);
        }
    }

    pub(crate) fn handle_transcript_fold_load_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut FoldLoadDialog,
    ) -> bool {
        match reduce_count_key(dialog, key, &self.i18n) {
            FoldLoadEffect::Continue => false,
            FoldLoadEffect::Cancel => true,
            FoldLoadEffect::Submit(count) => {
                self.transcript.fold_load_count = count;
                self.transcript.invalidate_render();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TranscriptTextPosition;
    use agena_api::part::PartResource;
    use crossterm::event::{KeyCode, KeyModifiers};
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn batch_editor_changes_a_setting_without_loading_parts() {
        let i18n = I18n::english();
        let mut dialog = count_dialog(5, &i18n);
        assert_eq!(dialog.input.text(), "5");
        reduce_count_key(&mut dialog, key(KeyCode::Backspace), &i18n);
        reduce_count_key(&mut dialog, key(KeyCode::Char('2')), &i18n);
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Enter), &i18n),
            FoldLoadEffect::Submit(2)
        ));
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Esc), &i18n),
            FoldLoadEffect::Cancel
        ));
        assert_eq!(count_dialog(25, &i18n).input.text(), "25");
    }

    #[test]
    fn invalid_counts_keep_the_editor_open_without_changing_the_setting() {
        let i18n = I18n::english();
        let mut dialog = count_dialog(5, &i18n);
        for input in [
            "",
            "0",
            "51",
            "-1",
            "+1",
            "1.5",
            "two",
            "9999999999999999999999",
        ] {
            dialog.input.set_text(input.to_owned());
            assert!(matches!(
                reduce_count_key(&mut dialog, key(KeyCode::Enter), &i18n),
                FoldLoadEffect::Continue
            ));
            assert_eq!(dialog.input.text(), input);
            assert_eq!(dialog.prompt, count_prompt(&i18n, true));
        }
        dialog.input.set_text(String::new());
        reduce_count_key(&mut dialog, key(KeyCode::Char('2')), &i18n);
        assert_eq!(dialog.prompt, count_prompt(&i18n, false));
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Enter), &i18n),
            FoldLoadEffect::Submit(2)
        ));
    }

    fn part(id: i64) -> PartResource {
        PartResource {
            part_id: id,
            kind: "tool_call".into(),
            message_boundary: id == 1,
            role: "assistant".into(),
            state: "completed".into(),
            run_id: Some(1),
            message_id: Some(1),
            content: json!({"name": "shell.open", "input": {}, "call_id": id,
                "state": "completed", "lifecycle": {"start_ms": 0}}),
            ..Default::default()
        }
    }

    fn local_state() -> TranscriptState {
        let mut state = TranscriptState {
            session_id: Some(1),
            ..Default::default()
        };
        state.merge_parts((1..=7).map(part).collect());
        state
    }

    fn remote_state() -> TranscriptState {
        let mut state = TranscriptState {
            session_id: Some(1),
            ..Default::default()
        };
        state.merge_parts([1, 36, 37, 38, 39, 40].into_iter().map(part).collect());
        state.transcript_folds.push(TranscriptFold {
            run_id: 1,
            run_ids: vec![1],
            anchor_part_id: 36,
            hidden_count: 34,
            next_cursor: Some("older-36".into()),
        });
        state
    }

    fn visible(state: &mut TranscriptState) -> Vec<i64> {
        state
            .rendered(120)
            .nodes
            .iter()
            .filter_map(|node| match node.key {
                TranscriptNodeKey::Activity {
                    content_id: TranscriptContentId::StoredPart(id),
                    ..
                } => Some(id),
                _ => None,
            })
            .collect()
    }

    fn entry_id() -> TranscriptEntryId {
        TranscriptEntryId::StoredMessage(1)
    }

    #[test]
    fn only_the_latest_parts_survive_gaps_and_streaming_updates() {
        let mut state = remote_state();
        assert_eq!(visible(&mut state), [36, 37, 38, 39, 40]);
        let key = TranscriptNodeKey::Entry {
            entry_id: entry_id(),
        };
        assert_eq!(state.fold_hidden_part_count(120, &key), 35);
        let text = state
            .rendered(120)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Enter: load")
                && text.contains("Ctrl+Enter: count")
                && text.contains("Ctrl+Shift+Enter: all"),
            "{text}"
        );
        assert!(!text.contains("choose count"));
        let mut updated = state.parts.clone();
        updated.extend((41..=43).map(part));
        state.merge_parts(updated);
        assert_eq!(visible(&mut state), [39, 40, 41, 42, 43]);
    }

    #[test]
    fn local_reveal_is_exact_and_marker_is_informational() {
        let mut state = local_state();
        assert_eq!(visible(&mut state), [3, 4, 5, 6, 7]);
        let marker = state
            .rendered(120)
            .nodes
            .iter()
            .find(|node| matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
            .unwrap()
            .clone();
        assert!(!marker.toggleable);
        state.select_pointer_line(
            120,
            30,
            TranscriptTextPosition {
                line: marker.start_line,
                column: 0,
            },
        );
        assert!(
            state
                .toggle_cursor_node_expansion_by(120, 30, None)
                .is_none()
        );
        let details = state.node_expansions.clone();
        assert!(
            state
                .reveal_older_reply_parts(120, 30, entry_id(), 1)
                .is_empty()
        );
        assert_eq!(visible(&mut state), [2, 3, 4, 5, 6, 7]);
        assert_eq!(state.node_expansions, details);
        state.reveal_older_reply_parts(120, 30, entry_id(), 1);
        assert_eq!(visible(&mut state), [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn remote_reveal_loads_the_nearest_older_parts_without_exposing_the_anchor() {
        let mut state = remote_state();
        let requests = state.reveal_older_reply_parts(120, 30, entry_id(), 2);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, 2);
        assert_eq!(visible(&mut state), [36, 37, 38, 39, 40]);
        assert!(state.merge_fold_parts(
            1,
            36,
            vec![part(34), part(35)],
            Some("older-34".into()),
            true
        ));
        assert_eq!(visible(&mut state), [34, 35, 36, 37, 38, 39, 40]);
        assert!(state.fold_requests_in_window(120, entry_id()).is_empty());
        let key = TranscriptNodeKey::Entry {
            entry_id: entry_id(),
        };
        assert_eq!(state.fold_hidden_part_count(120, &key), 33);
        let requests = state.reveal_older_reply_parts(120, 30, entry_id(), 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, 1);
        assert!(state.merge_fold_parts(1, 34, vec![part(33)], Some("older-33".into()), true));
        assert_eq!(visible(&mut state), [33, 34, 35, 36, 37, 38, 39, 40]);
    }

    #[test]
    fn retained_owner_boundary_is_not_subtracted_from_missing_history() {
        let mut state = remote_state();
        let original = state.transcript_folds[0].clone();
        state.merge_transcript_folds(vec![original.clone()]);
        assert_eq!(state.transcript_folds[0].hidden_count, 34);
        let requests = state.reveal_older_reply_parts(120, 30, entry_id(), 2);
        assert_eq!(requests[0].1, 2);
        state.merge_fold_parts(
            1,
            36,
            vec![part(34), part(35)],
            Some("older-34".into()),
            true,
        );
        state.merge_transcript_folds(vec![original]);
        assert_eq!(state.transcript_folds[0].hidden_count, 32);
        assert_eq!(state.transcript_folds[0].anchor_part_id, 34);
        let requests = state.reveal_older_reply_parts(120, 30, entry_id(), 50);
        assert_eq!(requests[0].1, 32);
        state.merge_fold_parts(1, 34, (2..=33).map(part).collect(), None, false);
        assert_eq!(visible(&mut state), (1..=40).collect::<Vec<_>>());
        assert!(state.transcript_folds.is_empty());
    }

    #[test]
    fn a_gap_inside_the_default_window_does_not_reveal_an_old_boundary() {
        let mut state = remote_state();
        state.parts.retain(|part| part.part_id != 36);
        state.transcript_folds[0].anchor_part_id = 37;
        state.transcript_folds[0].hidden_count = 35;
        state.invalidate_render();
        assert_eq!(visible(&mut state), [37, 38, 39, 40]);
        let requests = state.reveal_older_reply_parts(120, 30, entry_id(), 2);
        assert_eq!(requests[0].1, 2);
        state.merge_fold_parts(
            1,
            37,
            vec![part(35), part(36)],
            Some("older-35".into()),
            true,
        );
        assert_eq!(visible(&mut state), [35, 36, 37, 38, 39, 40]);
    }

    #[test]
    fn collapsing_while_a_page_is_loading_keeps_the_latest_window_and_cached_content() {
        let mut state = remote_state();
        state.reveal_older_reply_parts(120, 30, entry_id(), 2);
        state.collapse_transcript_parts(120, 30, true);
        state.merge_fold_parts(
            1,
            36,
            vec![part(34), part(35)],
            Some("older-34".into()),
            true,
        );
        assert_eq!(visible(&mut state), [36, 37, 38, 39, 40]);
        assert!(state.parts.iter().any(|part| part.part_id == 34));
        state.expand_all_transcript_parts(120, 30);
        assert_eq!(visible(&mut state), [1, 34, 35, 36, 37, 38, 39, 40]);
    }
}
