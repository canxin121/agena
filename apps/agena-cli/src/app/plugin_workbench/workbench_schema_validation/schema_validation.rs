use super::super::{
    BTreeSet, ConfigDiagnostic, ConfigPath, DiagnosticSeverity, JsonMap, JsonValue, PathSegment,
    array_item_schema, object_property_schema, resolve_schema, title_for_property,
    title_for_schema_or_key, value_matches_schema_type,
};
use super::{format_is_valid, pattern_matches, validate_regex_pattern};

pub(in crate::app) fn validate_schema_at(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
    title: &str,
) {
    let schema = resolve_schema(root, schema);
    if matches!(schema, JsonValue::Bool(true)) {
        return;
    }
    if matches!(schema, JsonValue::Bool(false)) {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "schema rejects this value",
        );
        return;
    }
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            validate_schema_at(diagnostics, root, branch, value, path, title);
        }
    }
    if let Some(any_of) = object.get("anyOf").and_then(JsonValue::as_array)
        && !any_of
            .iter()
            .any(|branch| schema_matches(root, branch, value))
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value must match at least one allowed shape",
        );
    }
    if let Some(one_of) = object.get("oneOf").and_then(JsonValue::as_array) {
        let count = one_of
            .iter()
            .filter(|branch| schema_matches(root, branch, value))
            .count();
        if count != 1 {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                title,
                "value must match exactly one allowed shape",
            );
        }
    }

    if let Some(expected) = object.get("const")
        && expected != value
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value does not match the required constant",
        );
    }
    if let Some(variants) = object.get("enum").and_then(JsonValue::as_array)
        && !variants.iter().any(|variant| variant == value)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value is not one of the allowed options",
        );
    }
    if let Some(schema_type) = object.get("type")
        && !value_matches_schema_type(value, schema_type)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            "value does not match declared type",
        );
        return;
    }
    if let Some(if_schema) = object.get("if") {
        let target = if schema_matches(root, if_schema, value) {
            object.get("then")
        } else {
            object.get("else")
        };
        if let Some(target_schema) = target {
            validate_schema_at(diagnostics, root, target_schema, value, path, title);
        }
    }
    if object.get("deprecated").and_then(JsonValue::as_bool) == Some(true) {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Warning,
            path,
            title,
            "field is deprecated",
        );
    }
    if let Some(object_value) = value.as_object() {
        validate_object_schema(diagnostics, root, &schema, object, object_value, path);
    }
    if let Some(array) = value.as_array() {
        validate_array_schema(diagnostics, root, &schema, object, array, path);
    }
    if let Some(text) = value.as_str() {
        validate_string_schema(diagnostics, object, text, path, title);
    }
    if let Some(number) = value.as_f64() {
        validate_number_schema(diagnostics, object, number, path, title);
    }
}

pub(in crate::app) fn validate_object_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &JsonMap<String, JsonValue>,
    path: &ConfigPath,
) {
    if let Some(patterns) = schema_object
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for pattern in patterns.keys() {
            if let Err(error) = validate_regex_pattern(pattern) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    &title_for_schema_or_key(schema, "Object"),
                    format!("invalid patternProperties regex `{pattern}`: {error}").as_str(),
                );
            }
        }
    }
    if let Some(min_properties) = schema_object
        .get("minProperties")
        .and_then(JsonValue::as_u64)
        && value.len() < min_properties as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Object"),
            format!("object must contain at least {min_properties} field(s)").as_str(),
        );
    }
    if let Some(max_properties) = schema_object
        .get("maxProperties")
        .and_then(JsonValue::as_u64)
        && value.len() > max_properties as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Object"),
            format!("object must contain at most {max_properties} field(s)").as_str(),
        );
    }
    let required = schema_object
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<BTreeSet<_>>();
    for field in required {
        if !value.contains_key(field) {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Key(field.to_owned()));
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                &child_path,
                &title_for_property(root, schema, field),
                "required field is missing",
            );
        }
    }
    if let Some(property_names_schema) = schema_object.get("propertyNames") {
        for key in value.keys() {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Key(key.clone()));
            validate_schema_at(
                diagnostics,
                root,
                property_names_schema,
                &JsonValue::String(key.clone()),
                &child_path,
                format!("{key} name").as_str(),
            );
        }
    }
    let properties = schema_object
        .get("properties")
        .and_then(JsonValue::as_object);
    for (key, child_value) in value {
        let mut child_path = path.clone();
        child_path.push(PathSegment::Key(key.clone()));
        if let Some(child_schema) = object_property_schema(root, schema, key) {
            validate_schema_at(
                diagnostics,
                root,
                &child_schema,
                child_value,
                &child_path,
                &title_for_schema_or_key(&child_schema, key),
            );
        } else if schema_object.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
            if properties.is_none_or(|properties| !properties.contains_key(key)) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    &child_path,
                    key,
                    "unexpected property",
                );
            }
        } else if let Some(additional) = schema_object.get("additionalProperties")
            && !matches!(additional, JsonValue::Bool(true))
        {
            validate_schema_at(diagnostics, root, additional, child_value, &child_path, key);
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentRequired")
        .and_then(JsonValue::as_object)
    {
        for (trigger, required_fields) in dependencies {
            if !value.contains_key(trigger) {
                continue;
            }
            for required in required_fields
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
            {
                if value.contains_key(required) {
                    continue;
                }
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(required.to_owned()));
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    &child_path,
                    &title_for_property(root, schema, required),
                    format!("required because `{trigger}` is set").as_str(),
                );
            }
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentSchemas")
        .and_then(JsonValue::as_object)
    {
        for (trigger, dependency_schema) in dependencies {
            if value.contains_key(trigger) {
                validate_schema_at(
                    diagnostics,
                    root,
                    dependency_schema,
                    &JsonValue::Object(value.clone()),
                    path,
                    &title_for_schema_or_key(schema, trigger),
                );
            }
        }
    }
}

pub(in crate::app) fn validate_array_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &[JsonValue],
    path: &ConfigPath,
) {
    if let Some(min_items) = schema_object.get("minItems").and_then(JsonValue::as_u64)
        && value.len() < min_items as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Array"),
            format!("array must contain at least {min_items} item(s)").as_str(),
        );
    }
    if let Some(max_items) = schema_object.get("maxItems").and_then(JsonValue::as_u64)
        && value.len() > max_items as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            &title_for_schema_or_key(schema, "Array"),
            format!("array must contain at most {max_items} item(s)").as_str(),
        );
    }
    if schema_object
        .get("uniqueItems")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let mut seen = BTreeSet::new();
        for item in value {
            if !seen.insert(item.to_string()) {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    &title_for_schema_or_key(schema, "Array"),
                    "array contains duplicate items",
                );
                break;
            }
        }
    }
    if let Some(contains_schema) = schema_object.get("contains") {
        let matches = value
            .iter()
            .filter(|item| schema_matches(root, contains_schema, item))
            .count();
        let min_contains = schema_object
            .get("minContains")
            .and_then(JsonValue::as_u64)
            .unwrap_or(1);
        let max_contains = schema_object.get("maxContains").and_then(JsonValue::as_u64);
        if matches < min_contains as usize {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                &title_for_schema_or_key(schema, "Array"),
                format!("array must contain at least {min_contains} matching item(s)").as_str(),
            );
        }
        if let Some(max_contains) = max_contains
            && matches > max_contains as usize
        {
            push_diag(
                diagnostics,
                DiagnosticSeverity::Error,
                path,
                &title_for_schema_or_key(schema, "Array"),
                format!("array must contain at most {max_contains} matching item(s)").as_str(),
            );
        }
    }
    for (index, item) in value.iter().enumerate() {
        if let Some(item_schema) = array_item_schema(root, schema, index) {
            let mut child_path = path.clone();
            child_path.push(PathSegment::Index(index));
            validate_schema_at(
                diagnostics,
                root,
                &item_schema,
                item,
                &child_path,
                format!("Item {index}").as_str(),
            );
        }
    }
}

pub(in crate::app) fn validate_string_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    schema_object: &JsonMap<String, JsonValue>,
    text: &str,
    path: &ConfigPath,
    title: &str,
) {
    if let Some(min_length) = schema_object.get("minLength").and_then(JsonValue::as_u64)
        && text.chars().count() < min_length as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be at least {min_length} characters").as_str(),
        );
    }
    if let Some(max_length) = schema_object.get("maxLength").and_then(JsonValue::as_u64)
        && text.chars().count() > max_length as usize
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be at most {max_length} characters").as_str(),
        );
    }
    if let Some(format) = schema_object.get("format").and_then(JsonValue::as_str)
        && !format_is_valid(format, text)
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must match format: {format}").as_str(),
        );
    }
    if let Some(pattern) = schema_object.get("pattern").and_then(JsonValue::as_str) {
        match pattern_matches(pattern, text) {
            Ok(true) => {}
            Ok(false) => {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    title,
                    format!("must match pattern: {pattern}").as_str(),
                );
            }
            Err(error) => {
                push_diag(
                    diagnostics,
                    DiagnosticSeverity::Error,
                    path,
                    title,
                    format!("invalid regex pattern `{pattern}`: {error}").as_str(),
                );
            }
        }
    }
}

pub(in crate::app) fn validate_number_schema(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    schema_object: &JsonMap<String, JsonValue>,
    number: f64,
    path: &ConfigPath,
    title: &str,
) {
    if let Some(minimum) = schema_object.get("minimum").and_then(JsonValue::as_f64)
        && number < minimum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be >= {minimum}").as_str(),
        );
    }
    if let Some(maximum) = schema_object.get("maximum").and_then(JsonValue::as_f64)
        && number > maximum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be <= {maximum}").as_str(),
        );
    }
    if let Some(minimum) = schema_object
        .get("exclusiveMinimum")
        .and_then(JsonValue::as_f64)
        && number <= minimum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be > {minimum}").as_str(),
        );
    }
    if let Some(maximum) = schema_object
        .get("exclusiveMaximum")
        .and_then(JsonValue::as_f64)
        && number >= maximum
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be < {maximum}").as_str(),
        );
    }
    if let Some(multiple_of) = schema_object.get("multipleOf").and_then(JsonValue::as_f64)
        && multiple_of > 0.0
        && (number / multiple_of).fract().abs() > f64::EPSILON
    {
        push_diag(
            diagnostics,
            DiagnosticSeverity::Error,
            path,
            title,
            format!("must be a multiple of {multiple_of}").as_str(),
        );
    }
}

pub(in crate::app) fn schema_matches(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
) -> bool {
    let mut diagnostics = Vec::new();
    validate_schema_at(&mut diagnostics, root, schema, value, &Vec::new(), "Value");
    diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error)
}

pub(in crate::app) fn push_diag(
    diagnostics: &mut Vec<ConfigDiagnostic>,
    severity: DiagnosticSeverity,
    path: &ConfigPath,
    field: &str,
    message: &str,
) {
    diagnostics.push(ConfigDiagnostic {
        severity,
        source: "config".to_owned(),
        path: path.clone(),
        field: field.to_owned(),
        message: message.to_owned(),
    });
}
