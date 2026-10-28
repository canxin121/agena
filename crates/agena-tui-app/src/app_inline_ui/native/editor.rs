use super::*;

pub(super) fn apply_modal_effect(dialog: &mut InlineUiDialog, effect: Effect) -> InlineKeyEffect {
    let mut outcome = InlineKeyEffect {
        changed: effect.changed,
        copy: effect.copy,
        ..Default::default()
    };
    dialog.error = effect.error;
    if let Some(edit) = effect.edit {
        dialog.edit_cursor = Some(edit.len());
        dialog.editing = Some(edit);
    }
    if effect.picker {
        dialog.picker = true;
        dialog.choice = None;
    }
    if dialog.editor_only && dialog.editing.is_none() && !dialog.picker && dialog.error.is_none() {
        outcome.close = true;
    }
    dialog.reveal_native.set(true);
    outcome
}

pub(crate) fn handle_modal_key(
    dialog: &mut InlineUiDialog,
    read: &mut InlineUiRead,
    key: KeyEvent,
    i18n: &agena_tui::i18n::I18n,
) -> InlineKeyEffect {
    use unicode_segmentation::UnicodeSegmentation;
    let mut outcome = InlineKeyEffect::default();
    if key.code == KeyCode::Esc {
        if (dialog.editing.is_some() || dialog.picker) && !dialog.editor_only {
            dialog.editing = None;
            dialog.picker = false;
            dialog.error = None;
        } else {
            outcome.close = true;
        }
        return outcome;
    }
    if dialog.picker {
        let Some(target) = dialog.native_focus.clone() else {
            return outcome;
        };
        let Some((program, view, node)) = resolve(read, &target) else {
            dialog.error = Some(i18n.text("inline-ui-control-changed"));
            return outcome;
        };
        let options = node
            .field
            .as_ref()
            .map(|field| field.options.as_slice())
            .unwrap_or(&[]);
        if options.is_empty() {
            return outcome;
        }
        let index = options
            .iter()
            .position(|option| {
                dialog
                    .choice
                    .as_ref()
                    .map_or(node.value.as_str() == Some(&option.value), |choice| {
                        choice == &option.value
                    })
            })
            .unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                dialog.choice = Some(options[(index + 1) % options.len()].value.clone())
            }
            KeyCode::Up | KeyCode::Left | KeyCode::BackTab => {
                dialog.choice = Some(
                    options[(index + options.len() - 1) % options.len()]
                        .value
                        .clone(),
                )
            }
            KeyCode::Home => dialog.choice = Some(options[0].value.clone()),
            KeyCode::End => dialog.choice = Some(options[options.len() - 1].value.clone()),
            KeyCode::Enter | KeyCode::Char(' ') => {
                match program.input(&view.state, &node.id, json!(options[index].value)) {
                    Ok(result) => {
                        read.states.insert(target.block.clone(), result.state);
                        outcome.changed = true;
                        dialog.picker = false;
                        outcome.close = dialog.editor_only;
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
            _ => {}
        }
        dialog.reveal_native.set(true);
        return outcome;
    }
    if let Some(buffer) = &mut dialog.editing {
        let mut cursor = dialog.edit_cursor.unwrap_or(buffer.len()).min(buffer.len());
        if !buffer.is_char_boundary(cursor) {
            cursor = buffer.len();
        }
        let previous = buffer[..cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        let next = buffer[cursor..]
            .graphemes(true)
            .next()
            .map_or(cursor, |grapheme| cursor + grapheme.len());
        match key.code {
            KeyCode::Left => cursor = previous,
            KeyCode::Right => cursor = next,
            KeyCode::Home => cursor = 0,
            KeyCode::End => cursor = buffer.len(),
            KeyCode::Backspace => {
                buffer.replace_range(previous..cursor, "");
                cursor = previous;
            }
            KeyCode::Delete => {
                buffer.replace_range(cursor..next, "");
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                buffer.clear();
                cursor = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
                    && !character.is_control()
                    && buffer.chars().count() < 6000 =>
            {
                buffer.insert(cursor, character);
                cursor += character.len_utf8();
            }
            KeyCode::Enter => {
                let Some(target) = &dialog.native_focus else {
                    return outcome;
                };
                let Some((program, view, node)) = resolve(read, target) else {
                    dialog.error = Some(i18n.text("inline-ui-control-changed"));
                    return outcome;
                };
                let value = if matches!(node.kind, UiKind::Number | UiKind::Slider) {
                    match buffer.trim().parse::<f64>() {
                        Ok(number) if number.is_finite() => json!(number),
                        _ => {
                            dialog.error = Some(i18n.text("inline-ui-invalid-number"));
                            return outcome;
                        }
                    }
                } else {
                    json!(buffer)
                };
                match program.input(&view.state, &node.id, value) {
                    Ok(result) => {
                        read.states.insert(target.block.clone(), result.state);
                        outcome.changed = true;
                        dialog.editing = None;
                        dialog.error = None;
                        outcome.close = dialog.editor_only;
                        dialog.reveal_native.set(true);
                        return outcome;
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
            _ => {}
        }
        dialog.edit_cursor = Some(cursor);
        return outcome;
    }
    let (rows, current_scroll) = {
        let layout = dialog.native_layout.borrow();
        (layout.rows.clone(), layout.scroll)
    };
    if matches!(key.code, KeyCode::PageDown | KeyCode::PageUp) {
        dialog.scroll = if key.code == KeyCode::PageDown {
            current_scroll
                .saturating_add(8)
                .min(dialog.scroll_limit.get())
        } else {
            current_scroll.saturating_sub(8)
        };
        return outcome;
    }
    let controls = rendered_controls(&rows);
    let Some((_, target)) = focus_target(dialog, &rows) else {
        return outcome;
    };
    if key.code == KeyCode::Char('r') && key.modifiers == KeyModifiers::CONTROL {
        read.states.remove(&target.block);
        read.error = None;
        outcome.changed = true;
        return outcome;
    }
    if matches!(
        key.code,
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down
    ) {
        let current = controls
            .iter()
            .position(|(_, control)| {
                control.action == TranscriptControlAction::InlineUi(target.clone())
            })
            .unwrap_or(0);
        let previous = matches!(key.code, KeyCode::BackTab | KeyCode::Up)
            || key.modifiers.contains(KeyModifiers::SHIFT);
        let next = if previous {
            (current + controls.len() - 1) % controls.len()
        } else {
            (current + 1) % controls.len()
        };
        if let TranscriptControlAction::InlineUi(target) = &controls[next].1.action {
            dialog.native_focus = Some(target.clone());
            dialog.focused = target.node.clone();
        }
        dialog.scroll = current_scroll;
        dialog.reveal_native.set(true);
        return outcome;
    }
    if matches!(
        key.code,
        KeyCode::Enter
            | KeyCode::Char(' ')
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
    ) {
        dialog.native_focus = Some(target.clone());
        let effect = activate(read, &target, key.code, None);
        dialog.native_focus = Some(updated_target(read, &target));
        return apply_modal_effect(dialog, effect);
    }
    outcome
}

pub(crate) fn paste_editor(dialog: &mut InlineUiDialog, text: &str) {
    let Some(buffer) = &mut dialog.editing else {
        return;
    };
    let mut cursor = dialog.edit_cursor.unwrap_or(buffer.len()).min(buffer.len());
    if !buffer.is_char_boundary(cursor) {
        cursor = buffer.len();
    }
    let remaining = 6000usize.saturating_sub(buffer.chars().count());
    let insertion = text
        .chars()
        .filter(|c| !c.is_control())
        .take(remaining)
        .collect::<String>();
    buffer.insert_str(cursor, &insertion);
    dialog.edit_cursor = Some(cursor + insertion.len());
}
