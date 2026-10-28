use super::*;

pub(super) fn resolve(
    read: &InlineUiRead,
    target: &InlineControl,
) -> Option<(UiProgram, UiView, UiResolvedNode)> {
    let document = read.document()?;
    let program = document.blocks.iter().find_map(|block| match block {
        InlineBlock::Panel { id, program } if id == &target.block => Some(program),
        _ => None,
    })?;
    let view = program
        .render(read.states.get(&target.block).unwrap_or(&UiState::new()))
        .ok()?;
    let node = view.root.find(&target.node)?.clone();
    if node.props.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some((program.clone(), view, node))
}
pub(super) fn activate(
    read: &mut InlineUiRead,
    target: &InlineControl,
    key: KeyCode,
    ratio: Option<f64>,
) -> Effect {
    let mut result = Effect::default();
    let pressed = matches!(key, KeyCode::Enter | KeyCode::Char(' '));
    if target.event == InlineEvent::Reset {
        if pressed
            && read.document().is_some_and(|document| {
                document
                    .blocks
                    .iter()
                    .any(|block| matches!(block,InlineBlock::Panel{id,..} if id==&target.block))
            })
        {
            result.changed = read.states.remove(&target.block).is_some();
            read.error = None;
        }
        return result;
    }
    if !pressed
        && matches!(
            target.event,
            InlineEvent::Details
                | InlineEvent::Data
                | InlineEvent::Series(_)
                | InlineEvent::Task(_)
        )
    {
        return result;
    }
    if matches!(
        target.event,
        InlineEvent::Details | InlineEvent::Data | InlineEvent::Series(_)
    ) {
        // Presentation state is local and still checked against the current tree.
        let valid = resolve(read, target).is_some_and(|(_, _, node)| match target.event {
            InlineEvent::Details => node.kind == UiKind::Details,
            InlineEvent::Data => node.kind == UiKind::Chart,
            InlineEvent::Series(index) => {
                node.kind == UiKind::Chart
                    && node
                        .props
                        .get("series")
                        .and_then(Value::as_array)
                        .is_some_and(|series| index < series.len())
            }
            _ => false,
        }) || read.document().is_some_and(|doc| {
            doc.blocks.iter().any(|block| {
                block.id() == target.block
                    && block.id() == target.node
                    && match (&target.event, block) {
                        (InlineEvent::Details, InlineBlock::Details { .. })
                        | (InlineEvent::Data, InlineBlock::Chart { .. }) => true,
                        (InlineEvent::Series(index), InlineBlock::Chart { series, .. }) => {
                            *index < series.len()
                        }
                        _ => false,
                    }
            })
        });
        if !valid {
            return result;
        }
        let presentation = read.presentation.entry(target.block.clone()).or_default();
        let set = match target.event {
            InlineEvent::Details => &mut presentation.open,
            InlineEvent::Data => &mut presentation.data,
            InlineEvent::Series(index) => {
                let set = presentation
                    .hidden_series
                    .entry(target.node.clone())
                    .or_default();
                if !set.remove(&index) {
                    set.insert(index);
                }
                result.changed = true;
                return result;
            }
            _ => unreachable!(),
        };
        if !set.remove(&target.node) {
            set.insert(target.node.clone());
        }
        result.changed = true;
        return result;
    }
    let Some((program, view, node)) = resolve(read, target) else {
        return result;
    };
    let mut input = None;
    match (&target.event, node.kind) {
        (InlineEvent::Choose(value), UiKind::Tabs)
            if matches!(key, KeyCode::Enter | KeyCode::Char(' ')) =>
        {
            input = Some(json!(value))
        }
        (InlineEvent::Task(item), UiKind::Tasks) => {
            if !node
                .props
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|entry| entry.get("id").and_then(Value::as_str) == Some(item))
                })
            {
                return result;
            }
            let mut values = node.value.as_array().cloned().unwrap_or_default();
            let value = json!(item);
            if values.contains(&value) {
                values.retain(|existing| existing != &value);
            } else {
                values.push(value);
            }
            input = Some(json!(values));
        }
        (_, UiKind::Checkbox | UiKind::Switch) if pressed => {
            input = Some(json!(!node.value.as_bool().unwrap_or(false)))
        }
        (_, UiKind::Tabs | UiKind::Select) => {
            if node.kind == UiKind::Select && matches!(key, KeyCode::Enter | KeyCode::Char(' ')) {
                result.picker = true;
                return result;
            }
            let options = node
                .field
                .as_ref()
                .map(|field| field.options.as_slice())
                .unwrap_or(&[]);
            if options.is_empty() {
                return result;
            }
            let current = options
                .iter()
                .position(|option| node.value.as_str() == Some(&option.value))
                .unwrap_or(0);
            let next = if matches!(key, KeyCode::Left | KeyCode::Up) {
                (current + options.len() - 1) % options.len()
            } else {
                (current + 1) % options.len()
            };
            input = Some(json!(options[next].value));
        }
        (_, UiKind::Slider | UiKind::Number) => {
            let Some(field) = &node.field else {
                return result;
            };
            let min = field
                .min
                .as_ref()
                .and_then(|number| number.as_f64())
                .unwrap_or(if node.kind == UiKind::Number {
                    -1e100
                } else {
                    0.0
                });
            let max = field
                .max
                .as_ref()
                .and_then(|number| number.as_f64())
                .unwrap_or(if node.kind == UiKind::Number {
                    1e100
                } else {
                    100.0
                });
            let step = field
                .step
                .as_ref()
                .and_then(|number| number.as_f64())
                .unwrap_or(1.0)
                .max(f64::EPSILON);
            let current = node.value.as_f64().unwrap_or(min);
            let value = if let Some(ratio) = ratio {
                min + (((max - min) * ratio.clamp(0.0, 1.0)) / step).round() * step
            } else {
                match key {
                    KeyCode::Left => current - step,
                    KeyCode::Right => current + step,
                    KeyCode::Home if field.min.is_some() || node.kind == UiKind::Slider => min,
                    KeyCode::End if field.max.is_some() || node.kind == UiKind::Slider => max,
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        result.edit = Some(value_text(&node.value));
                        return result;
                    }
                    _ => return result,
                }
            };
            input = Some(json!(value.clamp(min, max)));
        }
        (_, UiKind::Input) if pressed => {
            result.edit = Some(node.value.as_str().unwrap_or("").to_owned());
            return result;
        }
        (_, UiKind::Button) if pressed => match program.activate(&view.state, &node.id) {
            Ok(next) => {
                result.changed = next.state != view.state;
                read.states.insert(target.block.clone(), next.state);
                result.copy = next.copy;
            }
            Err(error) => result.error = Some(error),
        },
        _ => {}
    }
    if let Some(value) = input {
        match program.input(&view.state, &node.id, value) {
            Ok(next) => {
                result.changed = next.state != view.state;
                read.states.insert(target.block.clone(), next.state);
            }
            Err(error) => result.error = Some(error),
        }
    }
    result
}

pub(super) fn updated_target(read: &InlineUiRead, target: &InlineControl) -> InlineControl {
    let mut target = target.clone();
    if let Some((_, _, node)) = resolve(read, &target)
        && node.kind == UiKind::Tabs
        && let Some(value) = node.value.as_str()
    {
        target.event = InlineEvent::Choose(value.to_owned());
    }
    target
}
