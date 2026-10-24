use crate::{
    App, Editor, I18n, Overlay, TranscriptNodeKey, TranscriptState, TranscriptTextPosition,
};
use agena_tui_components::{InputDialogKeyResult, InputDialogState, drive_input_dialog_key};
use crossterm::event::KeyEvent;

pub(crate) type FoldLoadDialog = InputDialogState<FoldLoadTarget>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoldLoadTarget {
    session_id: i64,
    key: TranscriptNodeKey,
    maximum: usize,
}

enum FoldLoadEffect {
    Continue,
    Cancel,
    Submit(FoldLoadTarget, usize),
}

fn count_prompt(i18n: &I18n, maximum: usize, invalid: bool) -> String {
    i18n.text_args(
        if invalid {
            "transcript-load-count-invalid"
        } else {
            "transcript-load-count-prompt"
        },
        &agena_tui::fl_args!("max" => maximum as i64),
    )
}

fn count_dialog(target: FoldLoadTarget, i18n: &I18n) -> FoldLoadDialog {
    InputDialogState::new(
        i18n.text("transcript-load-count-title"),
        count_prompt(i18n, target.maximum, false),
        Editor::from_text(
            target
                .maximum
                .min(agena_tui_transcript::COLLAPSED_ACTIVITY_VISIBLE_COUNT)
                .to_string(),
        ),
        target,
    )
}

fn reduce_count_key(dialog: &mut FoldLoadDialog, key: KeyEvent, i18n: &I18n) -> FoldLoadEffect {
    let previous = dialog.input.text().to_owned();
    match drive_input_dialog_key(dialog, key) {
        InputDialogKeyResult::Close => FoldLoadEffect::Cancel,
        InputDialogKeyResult::Continue => {
            if dialog.input.text() != previous {
                dialog.prompt = count_prompt(i18n, dialog.action.maximum, false);
            }
            FoldLoadEffect::Continue
        }
        InputDialogKeyResult::Submit(target, input) => {
            let input = input.trim();
            let count = input
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| input.parse::<usize>().ok())
                .flatten()
                .filter(|count| (1..=target.maximum).contains(count));
            match count {
                Some(count) => FoldLoadEffect::Submit(target, count),
                None => {
                    dialog.prompt = count_prompt(i18n, target.maximum, true);
                    FoldLoadEffect::Continue
                }
            }
        }
    }
}

impl TranscriptState {
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
}

impl App {
    pub(crate) fn open_transcript_fold_load(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        let width = self.layout.transcript_body.width;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        if self.transcript.interaction_line() != Some(node.start_line) {
            return;
        }
        let maximum = self
            .transcript
            .fold_hidden_part_count(width, &node.key)
            .min(50);
        if maximum == 0 {
            return;
        }
        self.overlay = Some(Overlay::TranscriptFoldLoad(count_dialog(
            FoldLoadTarget {
                session_id,
                key: node.key,
                maximum,
            },
            &self.i18n,
        )));
    }

    pub(crate) fn handle_transcript_fold_load_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut FoldLoadDialog,
    ) -> bool {
        match reduce_count_key(dialog, key, &self.i18n) {
            FoldLoadEffect::Continue => false,
            FoldLoadEffect::Cancel => true,
            FoldLoadEffect::Submit(target, count) => {
                // Streaming or a session change can replace a fold while its
                // editor is open. Never apply its count to another row.
                if self.transcript.session_id != Some(target.session_id) {
                    return true;
                }
                let width = self.layout.transcript_body.width;
                let height = self.layout.transcript_body.height;
                let available = self.transcript.fold_hidden_part_count(width, &target.key);
                let line = self
                    .transcript
                    .rendered(width)
                    .nodes
                    .iter()
                    .find(|node| node.key == target.key)
                    .map(|node| node.start_line);
                if available > 0
                    && let Some(line) = line
                {
                    self.transcript.select_pointer_line(
                        width,
                        height,
                        TranscriptTextPosition { line, column: 0 },
                    );
                    self.toggle_transcript_cursor_node(Some(count.min(available)));
                }
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::part::PartResource;
    use agena_tui_transcript::{TranscriptContentId, TranscriptEntryId, TranscriptFold};
    use crossterm::event::{KeyCode, KeyModifiers};
    use serde_json::json;

    fn target(maximum: usize) -> FoldLoadTarget {
        FoldLoadTarget {
            session_id: 1,
            key: TranscriptNodeKey::ActivitySummary {
                entry_id: TranscriptEntryId::StoredMessage(1),
                first_content_id: TranscriptContentId::StoredPart(1),
            },
            maximum,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn count_editor_uses_remaining_count_and_supports_backspace_and_cancel() {
        let i18n = I18n::english();
        let expected = target(2);
        let mut dialog = count_dialog(expected.clone(), &i18n);
        assert_eq!(dialog.input.text(), "2");
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Backspace), &i18n),
            FoldLoadEffect::Continue
        ));
        reduce_count_key(&mut dialog, key(KeyCode::Char('1')), &i18n);
        assert_eq!(dialog.input.text(), "1");
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Enter), &i18n),
            FoldLoadEffect::Submit(actual, 1) if actual == expected
        ));
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Esc), &i18n),
            FoldLoadEffect::Cancel
        ));
        assert_eq!(count_dialog(target(50), &i18n).input.text(), "5");
    }

    #[test]
    fn invalid_counts_keep_the_editor_open_without_loading_a_default() {
        let i18n = I18n::english();
        let mut dialog = count_dialog(target(2), &i18n);
        for input in [
            "",
            "0",
            "3",
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
            assert_eq!(dialog.prompt, count_prompt(&i18n, 2, true));
        }
        dialog.input.set_text(String::new());
        reduce_count_key(&mut dialog, key(KeyCode::Char('2')), &i18n);
        assert_eq!(dialog.prompt, count_prompt(&i18n, 2, false));
        assert!(matches!(
            reduce_count_key(&mut dialog, key(KeyCode::Enter), &i18n),
            FoldLoadEffect::Submit(_, 2)
        ));
    }

    fn folded_transcript() -> TranscriptState {
        let mut state = TranscriptState::default();
        state.session_id = Some(1);
        state.merge_parts(
            (1..=7)
                .map(|part_id| PartResource {
                    part_id,
                    kind: "tool_call".into(),
                    message_boundary: part_id == 1,
                    role: "assistant".into(),
                    state: "completed".into(),
                    run_id: Some(1),
                    message_id: Some(1),
                    content: json!({
                        "name": "shell.open", "input": {}, "call_id": part_id,
                        "state": "completed", "lifecycle": {"start_ms": 0}
                    }),
                    ..Default::default()
                })
                .collect(),
        );
        state
    }

    #[test]
    fn edited_quantity_reveals_that_many_parts_and_marker_matches_remaining_count() {
        let mut state = folded_transcript();
        let marker = state
            .rendered(120)
            .nodes
            .iter()
            .find(|node| matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
            .unwrap()
            .clone();
        assert_eq!(state.fold_hidden_part_count(120, &marker.key), 2);
        let text = &state.rendered(120).lines[marker.start_line].text;
        assert!(text.starts_with("  … 2 older parts hidden"), "{text}");
        assert!(!text.contains("n Enter") && !text.contains("load 5"));
        state.select_pointer_line(
            120,
            30,
            TranscriptTextPosition {
                line: marker.start_line,
                column: 0,
            },
        );
        state
            .toggle_cursor_node_expansion_by(120, 30, Some(1))
            .unwrap();
        assert_eq!(state.fold_hidden_part_count(120, &marker.key), 1);
        state
            .toggle_cursor_node_expansion_by(120, 30, Some(1))
            .unwrap();
        assert_eq!(state.fold_hidden_part_count(120, &marker.key), 0);
        assert!(
            !state
                .rendered(120)
                .nodes
                .iter()
                .any(|node| { matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }) })
        );
    }

    #[test]
    fn input_limit_counts_both_remote_and_local_hidden_parts() {
        let mut state = folded_transcript();
        state.transcript_folds.push(TranscriptFold {
            run_id: 1,
            run_ids: vec![1],
            anchor_part_id: 1,
            hidden_count: 3,
            next_cursor: Some("older".into()),
        });
        let marker = state
            .rendered(120)
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::TranscriptFold { .. },
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        assert_eq!(state.fold_hidden_part_count(120, &marker.key), 5);
        assert!(
            state.rendered(120).lines[marker.start_line]
                .text
                .contains("5 older parts hidden")
        );
    }
}
