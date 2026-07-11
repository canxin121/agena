pub(in crate::app) fn diff_config_values(
    before: &JsonValue,
    after: &JsonValue,
) -> Vec<ConfigDiffRow> {
    let mut rows = Vec::new();
    collect_diff_rows(&mut rows, before, after, &Vec::new());
    rows
}

pub(in crate::app) fn collect_diff_rows(
    rows: &mut Vec<ConfigDiffRow>,
    before: &JsonValue,
    after: &JsonValue,
    path: &ConfigPath,
) {
    if before == after {
        return;
    }
    match (before, after) {
        (JsonValue::Object(left), JsonValue::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Key(key.clone()));
                collect_diff_rows(
                    rows,
                    left.get(key.as_str()).unwrap_or(&JsonValue::Null),
                    right.get(key.as_str()).unwrap_or(&JsonValue::Null),
                    &child_path,
                );
            }
        }
        (JsonValue::Array(left), JsonValue::Array(right)) => {
            let max = left.len().max(right.len());
            for index in 0..max {
                let mut child_path = path.clone();
                child_path.push(PathSegment::Index(index));
                collect_diff_rows(
                    rows,
                    left.get(index).unwrap_or(&JsonValue::Null),
                    right.get(index).unwrap_or(&JsonValue::Null),
                    &child_path,
                );
            }
        }
        _ => rows.push(ConfigDiffRow {
            path: path.clone(),
            before: diff_preview(before),
            after: diff_preview(after),
            summary: diff_summary(before, after),
        }),
    }
}

pub(in crate::app) fn schema_has_direct_defaulted_object_fields(
    schema: &JsonValue,
    root: &JsonValue,
) -> bool {
    let schema = resolve_schema(root, schema);
    schema_declared_property_keys(schema)
        .into_iter()
        .any(|key| {
            object_property_schema(root, schema, key.as_str())
                .as_ref()
                .is_some_and(|child_schema| child_schema.get("default").is_some())
        })
}

pub(in crate::app) fn insert_schema_defaults(
    value: &mut JsonValue,
    schema: &JsonValue,
    root: &JsonValue,
) {
    let schema = active_schema_for_value(root, schema, value);
    if value.is_null() {
        if let Some(default) = schema.get("default") {
            *value = default.clone();
        } else if effective_schema_kind(&schema).as_deref() == Some("object")
            && schema_has_direct_defaulted_object_fields(&schema, root)
        {
            *value = JsonValue::Object(JsonMap::new());
        } else {
            return;
        }
    }
    let kind = effective_schema_kind(&schema);
    if kind.as_deref() == Some("object") {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        for key in schema_declared_property_keys(&schema) {
            let Some(child_schema) = object_property_schema(root, &schema, key.as_str()) else {
                continue;
            };
            if let Some(default) = child_schema.get("default")
                && !object.contains_key(key.as_str())
            {
                object.insert(key.clone(), default.clone());
            }
            if let Some(child) = object.get_mut(key.as_str()) {
                insert_schema_defaults(child, &child_schema, root);
            }
        }
    } else if kind.as_deref() == Some("array")
        && let Some(array) = value.as_array_mut()
    {
        for (index, item) in array.iter_mut().enumerate() {
            if let Some(item_schema) = array_item_schema(root, &schema, index) {
                insert_schema_defaults(item, &item_schema, root);
            }
        }
    }
}

pub(in crate::app) fn default_value_for_schema(schema: &JsonValue, root: &JsonValue) -> JsonValue {
    materialized_value_for_schema(schema, root)
}

pub(in crate::app) fn merge_default_value(target: &mut JsonValue, patch: JsonValue) {
    match (target, patch) {
        (JsonValue::Object(target), JsonValue::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(key.as_str()) {
                    Some(existing) => merge_default_value(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, value) if target.is_null() => *target = value,
        _ => {}
    }
}

pub(in crate::app) fn default_value_for_type(kind: &str, schema: Option<&JsonValue>) -> JsonValue {
    match kind {
        "object" => {
            let mut value = JsonValue::Object(JsonMap::new());
            if let Some(schema) = schema {
                insert_schema_defaults(&mut value, schema, schema);
            }
            value
        }
        "array" => JsonValue::Array(Vec::new()),
        "string" => JsonValue::String(String::new()),
        "integer" => JsonValue::Number(JsonNumber::from(0)),
        "number" => JsonValue::Number(JsonNumber::from(0)),
        "boolean" => JsonValue::Bool(false),
        "null" => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

pub(in crate::app) fn schema_for_path(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
) -> Option<JsonValue> {
    let mut current_schema = schema.clone();
    let mut current_value = value;
    for segment in path {
        current_schema = active_schema_for_value(root, &current_schema, current_value);
        match segment {
            PathSegment::Key(key) => {
                current_schema = object_property_schema(root, &current_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(root, &current_schema, *index)?;
                current_value = current_value.get(*index).unwrap_or(&JsonValue::Null);
            }
        }
    }
    Some(active_schema_for_value(
        root,
        &current_schema,
        current_value,
    ))
}

pub(in crate::app) fn declared_schema_for_path(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    path: &ConfigPath,
) -> Option<JsonValue> {
    let mut current_schema = schema.clone();
    let mut current_value = value;
    for segment in path {
        let parent_schema = active_schema_for_value(root, &current_schema, current_value);
        match segment {
            PathSegment::Key(key) => {
                current_schema = object_property_schema(root, &parent_schema, key)?;
                current_value = current_value.get(key).unwrap_or(&JsonValue::Null);
            }
            PathSegment::Index(index) => {
                current_schema = array_item_schema(root, &parent_schema, *index)?;
                current_value = current_value.get(*index).unwrap_or(&JsonValue::Null);
            }
        }
    }
    Some(resolve_schema(root, &current_schema).clone())
}

pub(in crate::app) fn schema_base_without_applicators(
    schema_object: &JsonMap<String, JsonValue>,
) -> Option<JsonValue> {
    let mut base = schema_object.clone();
    for key in ["allOf", "anyOf", "oneOf", "if", "then", "else"] {
        base.remove(key);
    }
    (!base.is_empty()).then_some(JsonValue::Object(base))
}

pub(in crate::app) fn first_matching_branch<'a>(
    root: &JsonValue,
    branches: &'a [JsonValue],
    value: &JsonValue,
) -> Option<&'a JsonValue> {
    branches
        .iter()
        .find(|branch| schema_matches(root, branch, value))
        .or_else(|| branches.first())
}

pub(in crate::app) fn collect_applicable_schema_fragments(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
    fragments: &mut Vec<JsonValue>,
) {
    let schema = resolve_schema(root, schema);
    match schema {
        JsonValue::Bool(_) => fragments.push(schema.clone()),
        JsonValue::Object(object) => {
            if let Some(base) = schema_base_without_applicators(object) {
                fragments.push(base);
            }
            if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
                for branch in all_of {
                    collect_applicable_schema_fragments(root, branch, value, fragments);
                }
            }
            for key in ["oneOf", "anyOf"] {
                if let Some(branches) = object.get(key).and_then(JsonValue::as_array)
                    && let Some(branch) = first_matching_branch(root, branches, value)
                {
                    collect_applicable_schema_fragments(root, branch, value, fragments);
                }
            }
            if let Some(if_schema) = object.get("if") {
                let target = if schema_matches(root, if_schema, value) {
                    object.get("then")
                } else {
                    object.get("else")
                };
                if let Some(target_schema) = target {
                    collect_applicable_schema_fragments(root, target_schema, value, fragments);
                }
            }
        }
        _ => fragments.push(schema.clone()),
    }
}

pub(in crate::app) fn compose_schema_fragments(mut fragments: Vec<JsonValue>) -> JsonValue {
    fragments.retain(|fragment| !matches!(fragment, JsonValue::Bool(true)));
    if fragments
        .iter()
        .any(|fragment| matches!(fragment, JsonValue::Bool(false)))
    {
        return JsonValue::Bool(false);
    }
    match fragments.len() {
        0 => JsonValue::Bool(true),
        1 => fragments.pop().unwrap_or(JsonValue::Bool(true)),
        _ => {
            let mut object = JsonMap::new();
            object.insert("allOf".to_owned(), JsonValue::Array(fragments));
            JsonValue::Object(object)
        }
    }
}

pub(in crate::app) fn active_schema_for_value(
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
) -> JsonValue {
    let mut fragments = Vec::new();
    collect_applicable_schema_fragments(root, schema, value, &mut fragments);
    compose_schema_fragments(fragments)
}

pub(in crate::app) fn resolve_schema<'a>(
    root: &'a JsonValue,
    schema: &'a JsonValue,
) -> &'a JsonValue {
    let Some(reference) = schema.get("$ref").and_then(JsonValue::as_str) else {
        return schema;
    };
    if !reference.starts_with("#/") {
        return schema;
    }
    let mut cursor = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        let Some(next) = cursor.get(segment.as_str()) else {
            return schema;
        };
        cursor = next;
    }
    cursor
}

pub(in crate::app) fn combine_schema_constraints(mut schemas: Vec<JsonValue>) -> Option<JsonValue> {
    let had_true = schemas
        .iter()
        .any(|schema| matches!(schema, JsonValue::Bool(true)));
    schemas.retain(|schema| !matches!(schema, JsonValue::Bool(true)));
    if schemas
        .iter()
        .any(|schema| matches!(schema, JsonValue::Bool(false)))
    {
        return Some(JsonValue::Bool(false));
    }
    match schemas.len() {
        0 => had_true.then_some(JsonValue::Bool(true)),
        1 => schemas.pop(),
        _ => {
            let mut object = JsonMap::new();
            object.insert("allOf".to_owned(), JsonValue::Array(schemas));
            Some(JsonValue::Object(object))
        }
    }
}

pub(in crate::app) fn direct_object_property_schema(
    root: &JsonValue,
    schema: &JsonValue,
    key: &str,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let mut matches = Vec::new();
    let mut matched_named_or_pattern = false;

    if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object)
        && let Some(child) = properties.get(key)
    {
        matches.push(child.clone());
        matched_named_or_pattern = true;
    }
    if let Some(patterns) = schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for (pattern, child) in patterns {
            if pattern_key_matches(pattern, key) {
                matches.push(child.clone());
                matched_named_or_pattern = true;
            }
        }
    }
    if !matched_named_or_pattern {
        match schema.get("additionalProperties") {
            Some(JsonValue::Object(object)) => matches.push(JsonValue::Object(object.clone())),
            Some(other) if !matches!(other, JsonValue::Bool(true) | JsonValue::Bool(false)) => {
                matches.push(other.clone());
            }
            _ => {}
        }
    }
    combine_schema_constraints(matches)
}

pub(in crate::app) fn object_property_schema(
    root: &JsonValue,
    schema: &JsonValue,
    key: &str,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let Some(object) = schema.as_object() else {
        return matches!(schema, JsonValue::Bool(false)).then_some(JsonValue::Bool(false));
    };
    let mut matches = Vec::new();
    if let Some(base_match) = direct_object_property_schema(root, schema, key) {
        matches.push(base_match);
    }
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            if let Some(branch_match) = object_property_schema(root, branch, key) {
                matches.push(branch_match);
            }
        }
    }
    combine_schema_constraints(matches)
}

pub(in crate::app) fn direct_array_item_schema(
    root: &JsonValue,
    schema: &JsonValue,
    index: usize,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    if let Some(prefix) = schema.get("prefixItems").and_then(JsonValue::as_array)
        && let Some(item) = prefix.get(index)
    {
        return Some(item.clone());
    }
    if let Some(items) = schema.get("items") {
        return Some(items.clone());
    }
    (effective_schema_kind(schema).as_deref() == Some("array")).then_some(JsonValue::Bool(true))
}

pub(in crate::app) fn array_item_schema(
    root: &JsonValue,
    schema: &JsonValue,
    index: usize,
) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let Some(object) = schema.as_object() else {
        return matches!(schema, JsonValue::Bool(false)).then_some(JsonValue::Bool(false));
    };
    let mut matches = Vec::new();
    if let Some(base_match) = direct_array_item_schema(root, schema, index) {
        matches.push(base_match);
    }
    if let Some(all_of) = object.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            if let Some(branch_match) = array_item_schema(root, branch, index) {
                matches.push(branch_match);
            }
        }
    }
    combine_schema_constraints(matches)
}

pub(in crate::app) fn branch_choices(
    root: &JsonValue,
    schema: &JsonValue,
) -> Option<Vec<BranchChoice>> {
    let key = if schema.get("oneOf").is_some() {
        "oneOf"
    } else if schema.get("anyOf").is_some() {
        "anyOf"
    } else {
        return None;
    };
    let branches = schema.get(key)?.as_array()?;
    let choices = branches
        .iter()
        .enumerate()
        .map(|(index, schema)| BranchChoice {
            id: format!("branch-{index}"),
            label: branch_label(index, schema),
            schema: resolve_schema(root, schema).clone(),
        })
        .collect::<Vec<_>>();
    (!choices.is_empty()).then_some(choices)
}

pub(in crate::app) fn branch_label(index: usize, schema: &JsonValue) -> String {
    schema
        .get("title")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| {
            schema
                .get("properties")
                .and_then(JsonValue::as_object)
                .and_then(|properties| {
                    properties.iter().find_map(|(key, value)| {
                        value
                            .get("const")
                            .and_then(JsonValue::as_str)
                            .map(|constant| format!("{key}: {constant}"))
                    })
                })
        })
        .or_else(|| effective_schema_kind(schema))
        .unwrap_or_else(|| format!("Branch {}", index + 1))
}

pub(in crate::app) fn active_branch_choice<'a>(
    branches: &'a [BranchChoice],
    value: &JsonValue,
) -> Option<&'a BranchChoice> {
    branches
        .iter()
        .find(|branch| schema_matches(&branch.schema, &branch.schema, value))
        .or_else(|| branches.first())
}

pub(in crate::app) fn active_branch_id<'a>(
    branches: &'a [BranchChoice],
    value: &JsonValue,
) -> &'a str {
    active_branch_choice(branches, value)
        .map(|branch| branch.id.as_str())
        .unwrap_or("branch")
}

pub(in crate::app) fn active_branch_label<'a>(
    branches: &'a [BranchChoice],
    value: &JsonValue,
) -> &'a str {
    active_branch_choice(branches, value)
        .map(|branch| branch.label.as_str())
        .unwrap_or("branch")
}

pub(in crate::app) fn plugin_branch_draft_key(
    plugin_id: &str,
    path: &ConfigPath,
    branch_id: &str,
) -> String {
    format!("{plugin_id}:{}:{branch_id}", path_display(path))
}

pub(in crate::app) fn generic_json_type_choices() -> Vec<String> {
    vec![
        "string".to_owned(),
        "number".to_owned(),
        "integer".to_owned(),
        "boolean".to_owned(),
        "object".to_owned(),
        "array".to_owned(),
        "null".to_owned(),
    ]
}

pub(in crate::app) fn schema_type_choices(schema: &JsonValue) -> Vec<String> {
    let direct = match schema.get("type") {
        Some(JsonValue::String(kind)) => BTreeSet::from([kind.clone()]),
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(JsonValue::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        _ => BTreeSet::new(),
    };
    let Some(all_of) = schema.get("allOf").and_then(JsonValue::as_array) else {
        return direct.into_iter().collect();
    };
    let mut combined = direct;
    let mut branch_types = None::<BTreeSet<String>>;
    for branch in all_of {
        let choices = schema_type_choices(branch)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if choices.is_empty() {
            continue;
        }
        branch_types = Some(match branch_types {
            Some(current) => current.intersection(&choices).cloned().collect(),
            None => choices,
        });
    }
    if let Some(branch_types) = branch_types {
        if combined.is_empty() {
            combined = branch_types;
        } else {
            combined = combined.intersection(&branch_types).cloned().collect();
        }
    }
    combined.into_iter().collect()
}

pub(in crate::app) fn schema_type_selector_choices(schema: Option<&JsonValue>) -> Vec<String> {
    let Some(schema) = schema else {
        return generic_json_type_choices();
    };
    if matches!(schema, JsonValue::Bool(false)) {
        return Vec::new();
    }
    if matches!(schema, JsonValue::Bool(true)) {
        return generic_json_type_choices();
    }
    let choices = schema_type_choices(schema);
    if !choices.is_empty() {
        return choices;
    }
    if let Some(kind) = effective_schema_kind(schema) {
        return vec![kind];
    }
    generic_json_type_choices()
}

pub(in crate::app) fn schema_has_object_shape(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("properties")
            || object.contains_key("patternProperties")
            || object.contains_key("additionalProperties")
            || object.contains_key("propertyNames")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_object_shape))
}

pub(in crate::app) fn schema_has_array_shape(schema: &JsonValue) -> bool {
    if schema.as_object().is_some_and(|object| {
        object.contains_key("items")
            || object.contains_key("prefixItems")
            || object.contains_key("contains")
            || object.contains_key("minItems")
            || object.contains_key("maxItems")
            || object.contains_key("uniqueItems")
            || object.contains_key("minContains")
            || object.contains_key("maxContains")
    }) {
        return true;
    }
    schema
        .get("allOf")
        .and_then(JsonValue::as_array)
        .is_some_and(|branches| branches.iter().any(schema_has_array_shape))
}

pub(in crate::app) fn effective_schema_kind(schema: &JsonValue) -> Option<String> {
    let type_choices = schema_type_choices(schema);
    if !type_choices.is_empty() {
        return type_choices
            .into_iter()
            .find(|kind| kind != "null")
            .or_else(|| Some("null".to_owned()));
    }
    if schema_has_object_shape(schema) {
        return Some("object".to_owned());
    }
    if schema_has_array_shape(schema) {
        return Some("array".to_owned());
    }
    None
}

pub(in crate::app) fn value_matches_schema_type(
    value: &JsonValue,
    schema_type: &JsonValue,
) -> bool {
    match schema_type {
        JsonValue::String(kind) => value_matches_type(value, kind),
        JsonValue::Array(kinds) => kinds
            .iter()
            .filter_map(JsonValue::as_str)
            .any(|kind| value_matches_type(value, kind)),
        _ => true,
    }
}

pub(in crate::app) fn value_matches_type(value: &JsonValue, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

pub(in crate::app) fn schema_kind_label(schema: &JsonValue) -> String {
    if schema.get("oneOf").is_some() {
        return "oneOf".to_owned();
    }
    if schema.get("anyOf").is_some() {
        return "anyOf".to_owned();
    }
    if schema.get("allOf").is_some() {
        return "allOf".to_owned();
    }
    effective_schema_kind(schema).unwrap_or_else(|| "value".to_owned())
}

pub(in crate::app) fn json_kind_label(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            "integer"
        }
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

pub(in crate::app) fn get_value_at_path<'a>(
    value: &'a JsonValue,
    path: &ConfigPath,
) -> Option<&'a JsonValue> {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => cursor = cursor.get(key)?,
            PathSegment::Index(index) => cursor = cursor.get(*index)?,
        }
    }
    Some(cursor)
}

pub(in crate::app) fn get_value_mut_at_path<'a>(
    value: &'a mut JsonValue,
    path: &ConfigPath,
) -> Option<&'a mut JsonValue> {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => cursor = cursor.as_object_mut()?.get_mut(key)?,
            PathSegment::Index(index) => cursor = cursor.as_array_mut()?.get_mut(*index)?,
        }
    }
    Some(cursor)
}

pub(in crate::app) fn set_value_at_path(root: &mut JsonValue, path: &ConfigPath, value: JsonValue) {
    if path.is_empty() {
        *root = value;
        return;
    }
    let mut cursor = root;
    for segment in &path[..path.len().saturating_sub(1)] {
        match segment {
            PathSegment::Key(key) => {
                if !cursor.is_object() {
                    *cursor = JsonValue::Object(JsonMap::new());
                }
                cursor = cursor
                    .as_object_mut()
                    .expect("object initialized")
                    .entry(key.clone())
                    .or_insert(JsonValue::Object(JsonMap::new()));
            }
            PathSegment::Index(index) => {
                if !cursor.is_array() {
                    *cursor = JsonValue::Array(Vec::new());
                }
                let array = cursor.as_array_mut().expect("array initialized");
                while array.len() <= *index {
                    array.push(JsonValue::Null);
                }
                cursor = &mut array[*index];
            }
        }
    }
    match path.last().expect("path checked") {
        PathSegment::Key(key) => {
            if !cursor.is_object() {
                *cursor = JsonValue::Object(JsonMap::new());
            }
            cursor
                .as_object_mut()
                .expect("object initialized")
                .insert(key.clone(), value);
        }
        PathSegment::Index(index) => {
            if !cursor.is_array() {
                *cursor = JsonValue::Array(Vec::new());
            }
            let array = cursor.as_array_mut().expect("array initialized");
            while array.len() <= *index {
                array.push(JsonValue::Null);
            }
            array[*index] = value;
        }
    }
}

pub(in crate::app) fn remove_value_at_path(
    root: &mut JsonValue,
    path: &ConfigPath,
) -> Option<JsonValue> {
    let (last, parent_path) = path.split_last()?;
    let parent = get_value_mut_at_path(root, &parent_path.to_vec())?;
    match last {
        PathSegment::Key(key) => parent.as_object_mut()?.remove(key),
        PathSegment::Index(index) => {
            let array = parent.as_array_mut()?;
            (*index < array.len()).then(|| array.remove(*index))
        }
    }
}

pub(in crate::app) fn array_item_path_info(
    value: &JsonValue,
    path: &[PathSegment],
) -> Option<(ConfigPath, usize, usize)> {
    let (last, parent_path) = path.split_last()?;
    let PathSegment::Index(index) = last else {
        return None;
    };
    let parent_path = parent_path.to_vec();
    let len = get_value_at_path(value, &parent_path)?.as_array()?.len();
    (*index < len).then_some((parent_path, *index, len))
}

pub(in crate::app) fn path_key_info(path: &[PathSegment]) -> Option<(ConfigPath, String)> {
    let (last, parent_path) = path.split_last()?;
    let PathSegment::Key(key) = last else {
        return None;
    };
    Some((parent_path.to_vec(), key.clone()))
}

pub(in crate::app) fn replace_last_index(path: &[PathSegment], new_index: usize) -> ConfigPath {
    let mut next = path.to_vec();
    if let Some(last) = next.last_mut() {
        *last = PathSegment::Index(new_index);
    }
    next
}

#[derive(Debug, Clone, Copy)]
pub(in crate::app) struct ArrayItemActionInfo {
    pub(super) can_insert_before: bool,
    pub(super) can_insert_after: bool,
    pub(super) can_duplicate: bool,
    pub(super) can_move_up: bool,
    pub(super) can_move_down: bool,
    pub(super) can_remove: bool,
}

impl ArrayItemActionInfo {
    pub(in crate::app) fn has_any_action(self) -> bool {
        self.can_insert_before
            || self.can_insert_after
            || self.can_duplicate
            || self.can_move_up
            || self.can_move_down
            || self.can_remove
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ConfigRowPrimaryAction {
    InsertAfter,
    Duplicate,
    MoveDown,
    MoveUp,
    Remove,
    AddField,
    AddItem,
    Rename,
}

impl ConfigRowPrimaryAction {
    pub(in crate::app) fn plain_label(self) -> &'static str {
        match self {
            Self::InsertAfter => "Insert",
            Self::Duplicate => "Duplicate",
            Self::MoveDown => "Move down",
            Self::MoveUp => "Move up",
            Self::Remove => "Remove",
            Self::AddField => "Add field",
            Self::AddItem => "Add item",
            Self::Rename => "Rename",
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::InsertAfter => "[ Insert ]",
            Self::Duplicate => "[ Duplicate ]",
            Self::MoveDown => "[ Move down ]",
            Self::MoveUp => "[ Move up ]",
            Self::Remove => "[ Remove ]",
            Self::AddField => "[ Add field ]",
            Self::AddItem => "[ Add item ]",
            Self::Rename => "[ Rename ]",
        }
    }
}
use super::{
    BTreeSet, BranchChoice, ConfigDiffRow, ConfigPath, JsonMap, JsonNumber, JsonValue, PathSegment,
    diff_preview, diff_summary, materialized_value_for_schema, path_display, pattern_key_matches,
    schema_declared_property_keys, schema_matches,
};
