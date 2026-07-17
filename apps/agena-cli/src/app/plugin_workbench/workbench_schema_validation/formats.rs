use super::super::JsonValue;

pub(in crate::app) use agena::plugin::sdk::schema_validation::{
    format_is_valid, pattern_matches, validate_regex_pattern,
};

pub(in crate::app) fn merge_multi_enum_selection(
    current: &[JsonValue],
    selected: &[JsonValue],
) -> Vec<JsonValue> {
    let mut values = current
        .iter()
        .filter(|value| {
            selected
                .iter()
                .any(|selected_value| selected_value == *value)
        })
        .cloned()
        .collect::<Vec<_>>();
    for selected_value in selected {
        if !values.iter().any(|value| value == selected_value) {
            values.push(selected_value.clone());
        }
    }
    values
}
