use serde_json::Value;

pub fn merge_openai_chat_reasoning_details(target: &mut Option<Value>, incoming: &Value) {
    if !reasoning_detail_value_is_meaningful(incoming) {
        return;
    }
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
        if !reasoning_detail_value_is_meaningful(value) {
            continue;
        }
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

fn reasoning_detail_value_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::merge_openai_chat_reasoning_details;

    #[test]
    fn empty_reasoning_detail_update_does_not_erase_prior_state() {
        let original = serde_json::json!([{
            "type": "reasoning.text",
            "index": 0,
            "text": "kept",
            "signature": "signed"
        }]);
        let mut target = Some(original.clone());
        merge_openai_chat_reasoning_details(&mut target, &serde_json::Value::Null);
        merge_openai_chat_reasoning_details(&mut target, &serde_json::json!([]));
        merge_openai_chat_reasoning_details(
            &mut target,
            &serde_json::json!([{
                "type": "reasoning.text",
                "index": 0,
                "text": "   ",
                "signature": null
            }]),
        );
        assert_eq!(target, Some(original));
    }
}
