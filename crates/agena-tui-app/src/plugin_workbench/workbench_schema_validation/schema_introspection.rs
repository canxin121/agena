use super::super::{
    JsonValue, array_item_schema, effective_schema_kind, preview_value, schema_type_choices,
};

pub(crate) fn schema_contains_keyword(schema: &JsonValue, key: &str) -> bool {
    if schema
        .as_object()
        .is_some_and(|object| object.contains_key(key))
    {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_contains_keyword(branch, key))
        })
}

pub(crate) fn schema_bool_keyword_any(schema: &JsonValue, key: &str) -> bool {
    if schema.get(key).and_then(JsonValue::as_bool) == Some(true) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_bool_keyword_any(branch, key))
        })
}

pub(crate) fn schema_first_string_keyword<'a>(schema: &'a JsonValue, key: &str) -> Option<&'a str> {
    if let Some(value) = schema.get(key).and_then(JsonValue::as_str) {
        return Some(value);
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .find_map(|branch| schema_first_string_keyword(branch, key))
        })
}

pub(crate) fn schema_const_value(schema: &JsonValue) -> Option<JsonValue> {
    if let Some(constant) = schema.get("const") {
        return Some(constant.clone());
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .and_then(|branches| branches.iter().find_map(schema_const_value))
}

pub(crate) fn schema_enum_values(schema: &JsonValue) -> Option<Vec<JsonValue>> {
    let mut combined = schema
        .get("const")
        .map(|constant| vec![constant.clone()])
        .or_else(|| {
            schema
                .get("enum")
                .and_then(JsonValue::as_array)
                .cloned()
                .filter(|variants| !variants.is_empty())
        });
    if let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            let Some(branch_values) = schema_enum_values(branch) else {
                continue;
            };
            combined = Some(match combined.take() {
                Some(current) => current
                    .into_iter()
                    .filter(|value| branch_values.iter().any(|candidate| candidate == value))
                    .collect(),
                None => branch_values,
            });
        }
    }
    combined.filter(|variants| !variants.is_empty())
}

pub(crate) fn schema_description_text(schema: &JsonValue) -> Option<String> {
    schema
        .get("description")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| {
            schema
                .get("allOf")
                .and_then(JsonValue::as_array)
                .and_then(|branches| branches.iter().find_map(schema_description_text))
        })
}

pub(crate) fn schema_examples(schema: &JsonValue) -> Option<Vec<JsonValue>> {
    schema
        .get("examples")
        .and_then(JsonValue::as_array)
        .cloned()
        .filter(|examples| !examples.is_empty())
        .or_else(|| {
            schema
                .get("allOf")
                .and_then(JsonValue::as_array)
                .and_then(|branches| branches.iter().find_map(schema_examples))
        })
}

pub(crate) fn array_enum_variants(root: &JsonValue, schema: &JsonValue) -> Option<Vec<JsonValue>> {
    if effective_schema_kind(schema).as_deref() != Some("array") {
        return None;
    }
    if schema_contains_keyword(schema, "prefixItems") {
        return None;
    }
    if !schema_bool_keyword_any(schema, "uniqueItems") {
        return None;
    }
    let item_schema = array_item_schema(root, schema, 0)?;
    if matches!(item_schema, JsonValue::Bool(false)) {
        return None;
    }
    let variants = schema_enum_values(&item_schema)?;
    (!variants.is_empty()).then_some(variants)
}

pub(crate) fn schema_uses_nullable_string_editor(schema: &JsonValue) -> bool {
    let type_choices = schema_type_choices(schema);
    !type_choices.is_empty()
        && type_choices.iter().any(|kind| kind == "null")
        && type_choices.iter().any(|kind| kind == "string")
        && type_choices
            .iter()
            .all(|kind| matches!(kind.as_str(), "null" | "string"))
}

pub(crate) fn format_multi_enum_value_with_selector(values: &[JsonValue]) -> String {
    if values.is_empty() {
        "[ None ▾ ]".to_owned()
    } else {
        format!(
            "[ {} ▾ ]",
            values
                .iter()
                .map(preview_value)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub(crate) fn format_multi_enum_default_value(values: &[JsonValue]) -> String {
    if values.is_empty() {
        "None".to_owned()
    } else {
        values
            .iter()
            .map(preview_value)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
