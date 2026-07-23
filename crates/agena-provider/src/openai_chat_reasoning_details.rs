use serde_json::Value;

pub fn merge_openai_chat_reasoning_details(target: &mut Option<Value>, incoming: &Value) {
    let Some(current) = target.as_mut() else {
        *target = Some(incoming.clone());
        return;
    };
    let (Some(current_items), Some(incoming_items)) = (current.as_array_mut(), incoming.as_array())
    else {
        *current = incoming.clone();
        return;
    };
    for (position, incoming_item) in incoming_items.iter().enumerate() {
        let merge_with_tail = position == 0
            && current_items.last().is_some_and(|current| {
                reasoning_detail_key(current) == reasoning_detail_key(incoming_item)
                    && reasoning_detail_key(current)
                        .is_some_and(|(kind, _)| kind == "reasoning.text")
            });
        if merge_with_tail {
            merge_reasoning_detail(
                current_items
                    .last_mut()
                    .expect("merge candidate checked above"),
                incoming_item,
            );
        } else {
            current_items.push(incoming_item.clone());
        }
    }
}

fn reasoning_detail_key(value: &Value) -> Option<(String, u64)> {
    let object = value.as_object()?;
    Some((
        object.get("type")?.as_str()?.to_owned(),
        object
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    ))
}

fn merge_reasoning_detail(current: &mut Value, incoming: &Value) {
    if !current.is_object() || !incoming.is_object() {
        *current = incoming.clone();
        return;
    }
    let current = current.as_object_mut().expect("object checked above");
    let incoming = incoming.as_object().expect("object checked above");
    for (key, value) in incoming {
        if key == "text"
            && let Some(next_text) = value.as_str()
            && let Some(existing_text) = current.get(key).and_then(Value::as_str)
        {
            let merged = if next_text == existing_text || existing_text.ends_with(next_text) {
                existing_text.to_owned()
            } else if next_text.starts_with(existing_text) {
                next_text.to_owned()
            } else {
                format!("{existing_text}{next_text}")
            };
            current.insert(key.clone(), Value::String(merged));
        } else {
            current.insert(key.clone(), value.clone());
        }
    }
}
