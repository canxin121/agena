use super::*;

impl App {
    pub(super) fn handle_native_modal_mouse(&mut self, mouse: MouseEvent) -> bool {
        let Some(Overlay::InlineUi(mut dialog)) = self.overlay.take() else {
            return false;
        };
        let layout = dialog.native_layout.borrow().clone();
        let mut outcome = InlineKeyEffect::default();
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            dialog.scroll = if mouse.kind == MouseEventKind::ScrollUp {
                layout.scroll.saturating_sub(3)
            } else {
                layout
                    .scroll
                    .saturating_add(3)
                    .min(dialog.scroll_limit.get())
            };
        } else if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && contains(layout.body, mouse.column, mouse.row)
        {
            let line = usize::from(mouse.row - layout.body.y) + usize::from(layout.scroll);
            let column = usize::from(mouse.column - layout.body.x);
            if let Some(control) = layout.rows.get(line).and_then(|row| {
                row.controls
                    .iter()
                    .find(|control| control.enabled && control.columns.contains(&column))
            }) && let TranscriptControlAction::InlineUi(target) = &control.action
            {
                dialog.native_focus = Some(target.clone());
                dialog.focused = target.node.clone();
                if let Some(read) = self.transcript.inline_reads.get_mut(&dialog.reference) {
                    if dialog.picker {
                        if let InlineEvent::Choose(value) = &target.event {
                            dialog.choice = Some(value.clone());
                            outcome = handle_modal_key(
                                &mut dialog,
                                read,
                                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                                &self.i18n,
                            );
                        }
                    } else if dialog.editing.is_none() {
                        let ratio = (target.event == InlineEvent::Slider).then(|| {
                            column.saturating_sub(control.columns.start) as f64
                                / control.columns.len().saturating_sub(1).max(1) as f64
                        });
                        outcome = apply_modal_effect(
                            &mut dialog,
                            activate(read, target, KeyCode::Enter, ratio),
                        );
                        if target.event == InlineEvent::Slider {
                            dialog.pointer = Some(InlinePointer {
                                target: target.clone(),
                                columns: control.columns.clone(),
                            });
                        }
                    }
                }
            }
        } else if matches!(
            mouse.kind,
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        ) && let Some(pointer) = dialog.pointer.clone()
        {
            if let Some(read) = self.transcript.inline_reads.get_mut(&dialog.reference) {
                let column = mouse.column.saturating_sub(layout.body.x) as usize;
                let ratio = column.saturating_sub(pointer.columns.start) as f64
                    / pointer.columns.len().saturating_sub(1).max(1) as f64;
                outcome = apply_modal_effect(
                    &mut dialog,
                    activate(read, &pointer.target, KeyCode::Enter, Some(ratio)),
                );
            }
            if matches!(mouse.kind, MouseEventKind::Up(_)) {
                dialog.pointer = None;
            }
        }
        if outcome.changed {
            self.transcript.invalidate_render();
        }
        if let Some(text) = outcome.copy {
            self.request_clipboard_copy(text, self.i18n.text("inline-ui-copied"));
        }
        if !outcome.close {
            self.overlay = Some(Overlay::InlineUi(dialog));
        }
        true
    }
}
