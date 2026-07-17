pub(crate) fn merge_value(current: &mut serde_json::Value, patch: &serde_json::Value) {
    match (current, patch) {
        (serde_json::Value::Object(current), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match current.get_mut(key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        current.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (current, patch) => *current = patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::merge_value;

    #[test]
    fn merges_json_objects_recursively() {
        let mut current = json!({"nested": {"left": 1}, "replace": true});
        merge_value(
            &mut current,
            &json!({"nested": {"right": 2}, "replace": false}),
        );
        assert_eq!(
            current,
            json!({"nested": {"left": 1, "right": 2}, "replace": false})
        );
    }
}
