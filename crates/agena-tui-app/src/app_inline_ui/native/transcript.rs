use super::*;

impl App {
    fn apply_inline_control(
        &mut self,
        target: InlineControl,
        key: KeyCode,
        ratio: Option<f64>,
    ) -> bool {
        if target.event == InlineEvent::Open {
            self.transcript.move_inline_focus(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
                false,
                Some(&target.reference),
            );
            return true;
        }
        let Some(read) = self.transcript.inline_reads.get_mut(&target.reference) else {
            return false;
        };
        let effect = activate(read, &target, key, ratio);
        let target = updated_target(read, &target);
        if effect.changed {
            self.transcript.invalidate_render();
        }
        if let Some(error) = effect.error {
            self.flash_warning(error);
        }
        if let Some(text) = effect.copy {
            self.request_clipboard_copy(text, self.i18n.text("inline-ui-copied"));
        }
        if effect.edit.is_some() || effect.picker {
            self.overlay = Some(Overlay::InlineUi(InlineUiDialog {
                reference: target.reference.clone(),
                focused: target.node.clone(),
                editing: effect.edit,
                editor_only: true,
                picker: effect.picker,
                native_focus: Some(target),
                ..Default::default()
            }));
        } else {
            self.transcript.restore_inline_focus(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
                &target,
            );
        }
        true
    }
    pub(crate) fn handle_inline_transcript_key(&mut self, key: KeyEvent) -> bool {
        if self.overlay.is_some()
            || self.focus != agena_tui::main_focus::Focus::Transcript
            || !self.current_route_is_main()
            || self.transcript.has_visual_selection()
        {
            return false;
        }
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            return self.transcript.move_inline_focus(
                width,
                height,
                key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                None,
            );
        }
        let Some(control) = self.transcript.control_at_cursor(width) else {
            return false;
        };
        let TranscriptControlAction::InlineUi(target) = control.action else {
            return false;
        };
        if key.code == KeyCode::Char('r') && key.modifiers == KeyModifiers::CONTROL {
            if let Some(read) = self.transcript.inline_reads.get_mut(&target.reference) {
                read.states.remove(&target.block);
                read.error = None;
                self.transcript.invalidate_render();
            }
            return true;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
            return self.apply_inline_control(target, key.code, None);
        }
        if matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
        ) {
            let adjustable = self
                .transcript
                .inline_reads
                .get(&target.reference)
                .and_then(|read| resolve(read, &target))
                .is_some_and(|(_, _, node)| {
                    matches!(
                        node.kind,
                        UiKind::Slider | UiKind::Number | UiKind::Tabs | UiKind::Select
                    )
                });
            if adjustable {
                return self.apply_inline_control(target, key.code, None);
            }
        }
        false
    }
    pub(crate) fn handle_native_inline_mouse(&mut self, mouse: MouseEvent) -> bool {
        if matches!(self.overlay, Some(Overlay::InlineUi(_))) {
            return self.handle_native_modal_mouse(mouse);
        }
        if self.overlay.is_some() || !self.current_route_is_main() {
            return false;
        }
        let body = self.layout.transcript_body;
        if mouse
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            || matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            self.transcript.inline_pointer = None;
            return false;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !contains(body, mouse.column, mouse.row) {
                    return false;
                }
                let line =
                    self.transcript.viewport_top() + usize::from(mouse.row.saturating_sub(body.y));
                let column = usize::from(mouse.column.saturating_sub(body.x));
                let hit = self
                    .transcript
                    .rendered(body.width)
                    .lines
                    .get(line)
                    .and_then(|row| {
                        row.controls.iter().find(|c| {
                            c.enabled
                                && c.columns.contains(&column)
                                && matches!(c.action, TranscriptControlAction::InlineUi(_))
                        })
                    })
                    .cloned();
                let Some(hit) = hit else {
                    return false;
                };
                let TranscriptControlAction::InlineUi(target) = hit.action else {
                    return false;
                };
                self.cancel_active_pointer_gesture();
                self.cancel_surface_selection();
                self.focus = agena_tui::main_focus::Focus::Transcript;
                self.transcript.focus_pointer_control(
                    body.width,
                    body.height,
                    TranscriptTextPosition { line, column },
                );
                self.transcript.inline_pointer = Some(InlinePointer {
                    target: target.clone(),
                    columns: hit.columns.clone(),
                });
                if target.event == InlineEvent::Slider {
                    let ratio = column.saturating_sub(hit.columns.start) as f64
                        / hit.columns.len().saturating_sub(1).max(1) as f64;
                    self.apply_inline_control(target, KeyCode::Enter, Some(ratio));
                }
                true
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                let Some(pointer) = self.transcript.inline_pointer.clone() else {
                    return false;
                };
                let release = matches!(mouse.kind, MouseEventKind::Up(_));
                if release {
                    self.transcript.inline_pointer = None;
                }
                if pointer.target.event == InlineEvent::Slider {
                    let column = mouse.column.saturating_sub(body.x) as usize;
                    let ratio = column.saturating_sub(pointer.columns.start) as f64
                        / pointer.columns.len().saturating_sub(1).max(1) as f64;
                    self.apply_inline_control(pointer.target, KeyCode::Enter, Some(ratio));
                } else if release && contains(body, mouse.column, mouse.row) {
                    let line = self.transcript.viewport_top()
                        + usize::from(mouse.row.saturating_sub(body.y));
                    let column = usize::from(mouse.column.saturating_sub(body.x));
                    let same = self
                        .transcript
                        .rendered(body.width)
                        .lines
                        .get(line)
                        .is_some_and(|row| {
                            row.controls.iter().any(|ctrl| {
                                ctrl.enabled
                                    && ctrl.columns.contains(&column)
                                    && ctrl.action
                                        == TranscriptControlAction::InlineUi(pointer.target.clone())
                            })
                        });
                    if same {
                        self.apply_inline_control(pointer.target, KeyCode::Enter, None);
                    }
                }
                true
            }
            _ => false,
        }
    }
}
