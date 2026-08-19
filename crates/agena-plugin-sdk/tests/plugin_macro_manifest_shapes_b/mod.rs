#[test]
fn tool_input_format_constraints_apply_to_parse_schema_and_usage() {
    let path_value = PathFormatInput::parse_input(json!({
        "endpoint": "https://example.com/api"
    }))
    .expect("path-level format should accept valid URIs");
    assert_eq!(path_value.endpoint, "https://example.com/api");

    let renamed_value = RenamedFormatInput::parse_input(json!({
        "legacyEndpoint": "https://example.com/v1"
    }))
    .expect("renamed format should accept alias input");
    assert_eq!(renamed_value.endpoint_value, "https://example.com/v1");

    let path_error = PathFormatInput::parse_input(json!({ "endpoint": "not a uri" }))
        .expect_err("path-level format should reject invalid values");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `endpoint` must match format `uri`"#),
        "unexpected path format error: {path_error}"
    );

    let renamed_error = RenamedFormatInput::parse_input(json!({ "endpoint": "not a uri" }))
        .expect_err("renamed format should reject invalid values");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `endpoint` must match format `uri`"#),
        "unexpected renamed format error: {renamed_error}"
    );

    let path_schema = PathFormatInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/endpoint/format"),
        Some(&json!("uri"))
    );

    let renamed_schema = RenamedFormatInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/endpoint/format"),
        Some(&json!("uri"))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/endpoint/x-agena-aliases"),
        Some(&json!(["endpoint_value", "legacyEndpoint"]))
    );
    assert_eq!(
        RenamedFormatInput::input_usage().as_deref(),
        Some("https://example.com")
    );
    assert_eq!(
        PathFormatInput::input_usage().as_deref(),
        Some("https://example.com")
    );
}

#[test]
fn tool_input_pattern_constraints_apply_to_parse_schema_and_usage() {
    let path_pattern = PathPatternInput::parse_input(json!({ "slug": "cargo-check" }))
        .expect("path-level pattern should accept matching values");
    assert_eq!(path_pattern.slug, "cargo-check");
    let renamed = RenamedPatternInput::parse_input(json!({ "legacySlug": "git-status" }))
        .expect("renamed field pattern should accept alias input");
    assert_eq!(renamed.slug_value, "git-status");

    let path_error = PathPatternInput::parse_input(json!({ "slug": "CargoCheck" }))
        .expect_err("path-level pattern should reject invalid values");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected path pattern error: {path_error}"
    );
    let path_min_error = PathPatternInput::parse_input(json!({ "slug": "go" }))
        .expect_err("path-level min_chars should reject short values");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `slug` must be at least 3 characters"#),
        "unexpected path min_chars error: {path_min_error}"
    );
    let renamed_error = RenamedPatternInput::parse_input(json!({ "slug": "Cargo" }))
        .expect_err("renamed field pattern should reject invalid values");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected renamed pattern error: {renamed_error}"
    );
    let renamed_min_error = RenamedPatternInput::parse_input(json!({ "slug": "go" }))
        .expect_err("renamed field min_chars should reject short values");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `slug` must be at least 3 characters"#),
        "unexpected renamed min_chars error: {renamed_min_error}"
    );

    let path_schema = PathPatternInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/slug/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        path_schema.pointer("/properties/slug/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );

    let renamed_schema = RenamedPatternInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/slug/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/slug/maxLength"),
        Some(&json!(16))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/slug/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/slug/x-agena-aliases"),
        Some(&json!(["slug_value", "legacySlug"]))
    );
    assert_eq!(
        RenamedPatternInput::input_usage().as_deref(),
        Some("<slug>")
    );
}

#[test]
fn tool_input_numeric_constraints_apply_to_parse_schema_and_usage() {
    let path_numeric = PathNumericInput::parse_input(json!({ "count": 3 }))
        .expect("path-level numeric bounds should accept matching values");
    assert_eq!(path_numeric.count, 3);
    let renamed = RenamedNumericInput::parse_input(json!({ "legacyCount": 4 }))
        .expect("renamed numeric bounds should accept alias input");
    assert_eq!(renamed.count_value, 4);

    let path_min_error = PathNumericInput::parse_input(json!({ "count": 1 }))
        .expect_err("minimum should reject low values");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `count` must be at least 2"#),
        "unexpected minimum error: {path_min_error}"
    );
    let path_max_error = PathNumericInput::parse_input(json!({ "count": 5 }))
        .expect_err("maximum should reject high values");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `count` must be at most 4"#),
        "unexpected maximum error: {path_max_error}"
    );
    let renamed_min_error = RenamedNumericInput::parse_input(json!({ "count": 1 }))
        .expect_err("renamed numeric bounds should report the wire name");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `count` must be at least 2"#),
        "unexpected renamed numeric minimum error: {renamed_min_error}"
    );
    let renamed_parse_error = RenamedNumericInput::parse_input(json!({ "count": "oops" }))
        .expect_err("renamed numeric parse errors should report the wire name");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `count`"#),
        "unexpected renamed numeric parse error: {renamed_parse_error}"
    );

    let path_schema = PathNumericInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/count/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        path_schema.pointer("/properties/count/maximum"),
        Some(&json!(4))
    );
    assert_eq!(PathNumericInput::input_usage().as_deref(), Some("2"));

    let renamed_schema = RenamedNumericInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/count/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/count/maximum"),
        Some(&json!(4))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/count/x-agena-aliases"),
        Some(&json!(["count_value", "legacyCount"]))
    );
    assert_eq!(RenamedNumericInput::input_usage().as_deref(), Some("2"));
}

#[test]
fn tool_input_exclusive_numeric_constraints_apply_to_parse_schema_and_usage() {
    let path_numeric = PathExclusiveNumericInput::parse_input(json!({ "count": 3 }))
        .expect("path-level strict bounds should accept matching values");
    assert_eq!(path_numeric.count, 3);
    let renamed = RenamedExclusiveNumericInput::parse_input(json!({ "legacyCount": 4 }))
        .expect("renamed strict bounds should accept alias input");
    assert_eq!(renamed.count_value, 4);

    let path_min_error = PathExclusiveNumericInput::parse_input(json!({ "count": 2 }))
        .expect_err("exclusive_minimum should reject equal values");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `count` must be greater than 2"#),
        "unexpected exclusive minimum error: {path_min_error}"
    );
    let path_max_error = PathExclusiveNumericInput::parse_input(json!({ "count": 5 }))
        .expect_err("exclusive_maximum should reject equal values");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `count` must be less than 5"#),
        "unexpected exclusive maximum error: {path_max_error}"
    );
    let renamed_error = RenamedExclusiveNumericInput::parse_input(json!({ "count": 2 }))
        .expect_err("renamed strict bounds should report the wire name");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `count` must be greater than 2"#),
        "unexpected renamed exclusive minimum error: {renamed_error}"
    );

    let path_schema = PathExclusiveNumericInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/count/exclusiveMinimum"),
        Some(&json!(2))
    );
    assert_eq!(
        path_schema.pointer("/properties/count/exclusiveMaximum"),
        Some(&json!(5))
    );
    assert_eq!(
        PathExclusiveNumericInput::input_usage().as_deref(),
        Some("3")
    );

    let renamed_schema = RenamedExclusiveNumericInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/count/exclusiveMinimum"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/count/exclusiveMaximum"),
        Some(&json!(5))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/count/x-agena-aliases"),
        Some(&json!(["count_value", "legacyCount"]))
    );
    assert_eq!(
        RenamedExclusiveNumericInput::input_usage().as_deref(),
        Some("3")
    );
}

#[test]
fn tool_input_object_property_constraints_apply_to_parse_and_schema() {
    let path_value = PathObjectInput::parse_input(json!({
        "labels": { "first": "1", "second": "2" }
    }))
    .expect("path-level object bounds should accept values in range");
    assert_eq!(path_value.labels.len(), 2);

    let renamed_value = RenamedObjectInput::parse_input(json!({
        "legacyMetadata": { "alpha": "1" }
    }))
    .expect("renamed object bounds should accept aliases");
    assert_eq!(renamed_value.metadata_value.len(), 1);

    let path_min_error = PathObjectInput::parse_input(json!({ "labels": {} }))
        .expect_err("min_properties should reject empty objects");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `labels` requires at least 1 property"#),
        "unexpected path min_properties error: {path_min_error}"
    );

    let path_max_error = PathObjectInput::parse_input(json!({
        "labels": { "a": "1", "b": "2", "c": "3" }
    }))
    .expect_err("max_properties should reject oversized objects");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `labels` accepts at most 2 properties"#),
        "unexpected path max_properties error: {path_max_error}"
    );

    let renamed_min_error = RenamedObjectInput::parse_input(json!({ "metadata": {} }))
        .expect_err("renamed min_properties should reject empty objects");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `metadata` requires at least 1 property"#),
        "unexpected renamed min_properties error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedObjectInput::parse_input(json!({ "metadata": [] }))
        .expect_err("renamed object parse errors should use wire names");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `metadata`"#),
        "unexpected renamed object parse error: {renamed_parse_error}"
    );

    let path_schema = PathObjectInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/labels/minProperties"),
        Some(&json!(1))
    );
    assert_eq!(
        path_schema.pointer("/properties/labels/maxProperties"),
        Some(&json!(2))
    );

    let renamed_schema = RenamedObjectInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/metadata/minProperties"),
        Some(&json!(1))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/metadata/maxProperties"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/metadata/x-agena-aliases"),
        Some(&json!(["metadata_value", "legacyMetadata"]))
    );
}

#[test]
fn tool_input_item_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemPatternInput::parse_input(json!({
        "tags": ["cargo-check", "git-status"]
    }))
    .expect("path-level item constraints should accept matching values");
    assert_eq!(path_value.tags, vec!["cargo-check", "git-status"]);

    let renamed_value = RenamedItemPatternInput::parse_input(json!({
        "legacyTags": ["cargo-check"]
    }))
    .expect("renamed item constraints should accept aliases");
    assert_eq!(renamed_value.tag_values, vec!["cargo-check"]);

    let path_pattern_error = PathItemPatternInput::parse_input(json!({
        "tags": ["CargoCheck"]
    }))
    .expect_err("item pattern should reject invalid values");
    assert!(
        path_pattern_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected path item pattern error: {path_pattern_error}"
    );

    let renamed_min_error = RenamedItemPatternInput::parse_input(json!({
        "tags": ["go"]
    }))
    .expect_err("renamed item min_chars should reject short values");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must be at least 3 characters"#),
        "unexpected renamed item min_chars error: {renamed_min_error}"
    );

    let renamed_max_error = RenamedItemPatternInput::parse_input(json!({
        "tags": ["abcdefghijklmnopq"]
    }))
    .expect_err("renamed item max_chars should reject long values");
    assert!(
        renamed_max_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must be at most 16 characters"#),
        "unexpected renamed item max_chars error: {renamed_max_error}"
    );

    let renamed_parse_error = RenamedItemPatternInput::parse_input(json!({
        "tags": [1]
    }))
    .expect_err("renamed item parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `tags[0]`"#),
        "unexpected renamed item parse error: {renamed_parse_error}"
    );

    let path_schema = PathItemPatternInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        path_schema.pointer("/properties/tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );

    let renamed_schema = RenamedItemPatternInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/maxLength"),
        Some(&json!(16))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_item_choice_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemChoiceInput::parse_input(json!({
        "tools": ["cargo", "git"]
    }))
    .expect("path-level item choices should accept allowed values");
    assert_eq!(path_value.tools, vec!["cargo", "git"]);

    let renamed_value = RenamedItemChoiceInput::parse_input(json!({
        "legacyTools": ["cargo"]
    }))
    .expect("renamed item choices should accept aliases");
    assert_eq!(renamed_value.tool_values, vec!["cargo"]);

    let path_error = PathItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("item choices should reject unsupported values");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected path item choice error: {path_error}"
    );

    let renamed_error = RenamedItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("renamed item choices should use wire names");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed item choice error: {renamed_error}"
    );

    let renamed_parse_error = RenamedItemChoiceInput::parse_input(json!({
        "tools": [1]
    }))
    .expect_err("renamed item choice parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `tools[0]`"#),
        "unexpected renamed item choice parse error: {renamed_parse_error}"
    );

    let path_schema = PathItemChoiceInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );

    let renamed_schema = RenamedItemChoiceInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tools/x-agena-aliases"),
        Some(&json!(["tool_values", "legacyTools"]))
    );
}

#[test]
fn tool_input_item_format_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemFormatInput::parse_input(json!({
        "ids": ["550e8400-e29b-41d4-a716-446655440000"]
    }))
    .expect("path-level item format should accept valid UUIDs");
    assert_eq!(path_value.ids, vec!["550e8400-e29b-41d4-a716-446655440000"]);

    let renamed_value = RenamedItemFormatInput::parse_input(json!({
        "legacyIds": ["550e8400-e29b-41d4-a716-446655440000"]
    }))
    .expect("renamed item format should accept alias input");
    assert_eq!(
        renamed_value.id_values,
        vec!["550e8400-e29b-41d4-a716-446655440000"]
    );

    let path_error = PathItemFormatInput::parse_input(json!({ "ids": ["not-a-uuid"] }))
        .expect_err("item format should reject invalid values");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `ids[]` must match format `uuid`"#),
        "unexpected path item format error: {path_error}"
    );

    let renamed_error = RenamedItemFormatInput::parse_input(json!({ "ids": ["not-a-uuid"] }))
        .expect_err("renamed item format should reject invalid values");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `ids[]` must match format `uuid`"#),
        "unexpected renamed item format error: {renamed_error}"
    );

    let path_schema = PathItemFormatInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/ids/items/format"),
        Some(&json!("uuid"))
    );

    let renamed_schema = RenamedItemFormatInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/ids/items/format"),
        Some(&json!("uuid"))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/ids/x-agena-aliases"),
        Some(&json!(["id_values", "legacyIds"]))
    );
    assert_eq!(
        PathItemFormatInput::input_usage().as_deref(),
        Some("[\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
    assert_eq!(
        RenamedItemFormatInput::input_usage().as_deref(),
        Some("[\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
}

#[test]
fn tool_input_item_numeric_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemNumericInput::parse_input(json!({
        "counts": [2, 4]
    }))
    .expect("path-level item numeric bounds should accept matching values");
    assert_eq!(path_value.counts, vec![2, 4]);

    let renamed_value = RenamedItemNumericInput::parse_input(json!({
        "legacyCounts": [3]
    }))
    .expect("renamed item numeric bounds should accept aliases");
    assert_eq!(renamed_value.count_values, vec![3]);

    let path_min_error = PathItemNumericInput::parse_input(json!({
        "counts": [1]
    }))
    .expect_err("item minimum should reject low values");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected path item minimum error: {path_min_error}"
    );

    let path_max_error = PathItemNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("item maximum should reject high values");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at most 4"#),
        "unexpected path item maximum error: {path_max_error}"
    );

    let renamed_min_error = RenamedItemNumericInput::parse_input(json!({
        "counts": [1]
    }))
    .expect_err("renamed item numeric bounds should report the wire name");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected renamed item minimum error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedItemNumericInput::parse_input(json!({
        "counts": ["oops"]
    }))
    .expect_err("renamed item numeric parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `counts[0]`"#),
        "unexpected renamed item numeric parse error: {renamed_parse_error}"
    );

    let path_schema = PathItemNumericInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/counts/items/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        path_schema.pointer("/properties/counts/items/maximum"),
        Some(&json!(4))
    );

    let renamed_schema = RenamedItemNumericInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/maximum"),
        Some(&json!(4))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/x-agena-aliases"),
        Some(&json!(["count_values", "legacyCounts"]))
    );
}

#[test]
fn tool_input_item_exclusive_numeric_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemExclusiveNumericInput::parse_input(json!({
        "counts": [3, 4]
    }))
    .expect("path-level item strict bounds should accept matching values");
    assert_eq!(path_value.counts, vec![3, 4]);

    let renamed_value = RenamedItemExclusiveNumericInput::parse_input(json!({
        "legacyCounts": [3]
    }))
    .expect("renamed item strict bounds should accept aliases");
    assert_eq!(renamed_value.count_values, vec![3]);

    let path_min_error = PathItemExclusiveNumericInput::parse_input(json!({
        "counts": [2]
    }))
    .expect_err("item exclusive_minimum should reject equal values");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be greater than 2"#),
        "unexpected item exclusive minimum error: {path_min_error}"
    );

    let path_max_error = PathItemExclusiveNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("item exclusive_maximum should reject equal values");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be less than 5"#),
        "unexpected item exclusive maximum error: {path_max_error}"
    );

    let renamed_error = RenamedItemExclusiveNumericInput::parse_input(json!({
        "counts": [2]
    }))
    .expect_err("renamed item strict bounds should report the wire name");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be greater than 2"#),
        "unexpected renamed item exclusive minimum error: {renamed_error}"
    );

    let path_schema = PathItemExclusiveNumericInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/counts/items/exclusiveMinimum"),
        Some(&json!(2))
    );
    assert_eq!(
        path_schema.pointer("/properties/counts/items/exclusiveMaximum"),
        Some(&json!(5))
    );
    assert_eq!(
        PathItemExclusiveNumericInput::input_usage().as_deref(),
        Some("[3]")
    );

    let renamed_schema = RenamedItemExclusiveNumericInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/exclusiveMinimum"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/exclusiveMaximum"),
        Some(&json!(5))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/x-agena-aliases"),
        Some(&json!(["count_values", "legacyCounts"]))
    );
    assert_eq!(
        RenamedItemExclusiveNumericInput::input_usage().as_deref(),
        Some("[3]")
    );
}

#[test]
fn tool_input_item_object_constraints_apply_to_parse_and_schema() {
    let path_value = PathItemObjectInput::parse_input(json!({
        "entries": [{ "first": "1" }, { "first": "1", "second": "2" }]
    }))
    .expect("path-level item object bounds should accept values in range");
    assert_eq!(path_value.entries.len(), 2);

    let renamed_value = RenamedItemObjectInput::parse_input(json!({
        "legacyEntries": [{ "alpha": "1" }]
    }))
    .expect("renamed item object bounds should accept aliases");
    assert_eq!(renamed_value.entry_values.len(), 1);

    let path_min_error = PathItemObjectInput::parse_input(json!({
        "entries": [{}]
    }))
    .expect_err("item min_properties should reject empty objects");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `entries[]` requires at least 1 property"#),
        "unexpected path item min_properties error: {path_min_error}"
    );

    let path_max_error = PathItemObjectInput::parse_input(json!({
        "entries": [{ "a": "1", "b": "2", "c": "3" }]
    }))
    .expect_err("item max_properties should reject oversized objects");
    assert!(
        path_max_error
            .diagnostic_message()
            .contains(r#"field `entries[]` accepts at most 2 properties"#),
        "unexpected path item max_properties error: {path_max_error}"
    );

    let renamed_min_error = RenamedItemObjectInput::parse_input(json!({
        "entries": [{}]
    }))
    .expect_err("renamed item object bounds should report the wire name");
    assert!(
        renamed_min_error
            .diagnostic_message()
            .contains(r#"field `entries[]` requires at least 1 property"#),
        "unexpected renamed item object min_properties error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedItemObjectInput::parse_input(json!({
        "entries": ["oops"]
    }))
    .expect_err("renamed item object parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .diagnostic_message()
            .contains(r#"invalid JSON value at `entries[0]`"#),
        "unexpected renamed item object parse error: {renamed_parse_error}"
    );

    let path_schema = PathItemObjectInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/entries/items/minProperties"),
        Some(&json!(1))
    );
    assert_eq!(
        path_schema.pointer("/properties/entries/items/maxProperties"),
        Some(&json!(2))
    );

    let renamed_schema = RenamedItemObjectInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/entries/items/minProperties"),
        Some(&json!(1))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/entries/items/maxProperties"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/entries/x-agena-aliases"),
        Some(&json!(["entry_values", "legacyEntries"]))
    );
}

#[test]
fn tool_input_item_normalization_and_non_empty_sugar_apply_to_parse_and_schema() {
    let path_value = PathItemNormalizeInput::parse_input(json!({
        "tags": [" cargo.rs ", " git.rs "]
    }))
    .expect("path-level item normalization sugar should normalize matching items");
    assert_eq!(path_value.tags, vec!["cargo", "git"]);

    let renamed_value = RenamedItemNormalizeInput::parse_input(json!({
        "legacyTags": [" cargo.rs ", " git.rs "]
    }))
    .expect("renamed item normalization sugar should accept aliases and normalize items");
    assert_eq!(renamed_value.tag_values, vec!["cargo", "git"]);

    let path_error = PathItemNormalizeInput::parse_input(json!({
        "tags": [" .rs "]
    }))
    .expect_err("item_non_empty should reject empty normalized items");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty"#),
        "unexpected path item normalize error: {path_error}"
    );

    let renamed_error = RenamedItemNormalizeInput::parse_input(json!({
        "tags": [" .rs "]
    }))
    .expect_err("renamed item normalization sugar should report the wire name");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty"#),
        "unexpected renamed item normalize error: {renamed_error}"
    );

    let path_schema = PathItemNormalizeInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );

    let renamed_schema = RenamedItemNormalizeInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_item_non_empty_if_present_sugar_applies_to_optional_arrays() {
    let path_missing = PathOptionalItemNonEmptyInput::parse_input(json!({}))
        .expect("item_non_empty_if_present should allow missing optional arrays");
    assert_eq!(path_missing.tags, None);

    let renamed_missing = RenamedOptionalItemNonEmptyInput::parse_input(json!({}))
        .expect("renamed item_non_empty_if_present should allow missing optional arrays");
    assert_eq!(renamed_missing.tag_values, None);

    let path_value = PathOptionalItemNonEmptyInput::parse_input(json!({
        "tags": ["cargo"]
    }))
    .expect("item_non_empty_if_present should accept present non-empty items");
    assert_eq!(path_value.tags, Some(vec!["cargo".to_string()]));

    let path_error = PathOptionalItemNonEmptyInput::parse_input(json!({
        "tags": [""]
    }))
    .expect_err("item_non_empty_if_present should reject present empty items");
    assert!(
        path_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty when present"#),
        "unexpected optional item non-empty error: {path_error}"
    );

    let renamed_error = RenamedOptionalItemNonEmptyInput::parse_input(json!({
        "tags": [""]
    }))
    .expect_err("renamed item_non_empty_if_present should report the wire name");
    assert!(
        renamed_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not be empty when present"#),
        "unexpected renamed optional item non-empty error: {renamed_error}"
    );

    let path_schema = PathOptionalItemNonEmptyInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );

    let renamed_schema = RenamedOptionalItemNonEmptyInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_type_level_array_value_relations_apply_to_item_paths() {
    let path_value = PathItemValueRelationInput::parse_input(json!({
        "tags": ["cargo", "git"]
    }))
    .expect("type-level array value relations should accept valid string items");
    assert_eq!(path_value.tags, vec!["cargo", "git"]);

    let renamed_value = RenamedItemValueRelationInput::parse_input(json!({
        "legacyTags": ["cargo"]
    }))
    .expect("renamed type-level array value relations should accept aliases");
    assert_eq!(renamed_value.tag_values, vec!["cargo"]);

    let path_forbid_error = PathItemValueRelationInput::parse_input(json!({
        "tags": ["../etc/passwd"]
    }))
    .expect_err("type-level forbid_substrings should target array items");
    assert!(
        path_forbid_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain `..`"#),
        "unexpected type-level item forbid_substrings error: {path_forbid_error}"
    );

    let path_distinct_error = PathItemValueRelationInput::parse_input(json!({
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("type-level distinct_trimmed should target array items");
    assert!(
        path_distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected type-level item distinct_trimmed error: {path_distinct_error}"
    );

    let renamed_forbid_error = RenamedItemValueRelationInput::parse_input(json!({
        "tags": ["../etc/passwd"]
    }))
    .expect_err("renamed type-level forbid_substrings should report schema-side paths");
    assert!(
        renamed_forbid_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain `..`"#),
        "unexpected renamed type-level item forbid_substrings error: {renamed_forbid_error}"
    );

    let renamed_distinct_error = RenamedItemValueRelationInput::parse_input(json!({
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("renamed type-level distinct_trimmed should report schema-side paths");
    assert!(
        renamed_distinct_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected renamed type-level item distinct_trimmed error: {renamed_distinct_error}"
    );

    let path_relations = schema_relation_labels(&PathItemValueRelationInput::input_schema());
    assert!(path_relations.contains(&"forbid_substrings `tags[]`: \"..\", \"~\"".to_string()));
    assert!(path_relations.contains(&"distinct_trimmed `tags[]`".to_string()));

    let renamed_schema = RenamedItemValueRelationInput::input_schema();
    let renamed_relations = schema_relation_labels(&renamed_schema);
    assert!(renamed_relations.contains(&"forbid_substrings `tags[]`: \"..\", \"~\"".to_string()));
    assert!(renamed_relations.contains(&"distinct_trimmed `tags[]`".to_string()));
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_direct_array_string_constraints_auto_target_items() {
    let path_value = PathAutoItemStringInput::parse_input(json!({
        "tags": [" cargo.rs "]
    }))
    .expect("type-level direct array string constraints should normalize and validate items");
    assert_eq!(path_value.tags, vec!["cargo"]);

    let renamed_value = RenamedAutoItemStringInput::parse_input(json!({
        "legacyTags": [" cargo.rs "]
    }))
    .expect("field-level direct array string constraints should normalize aliased items");
    assert_eq!(renamed_value.tag_values, vec!["cargo"]);

    let path_min_error = PathAutoItemStringInput::parse_input(json!({
        "tags": [" go.rs "]
    }))
    .expect_err("direct min_chars on array fields should target items");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must be at least 3 characters"#),
        "unexpected direct array min_chars error: {path_min_error}"
    );

    let renamed_pattern_error = RenamedAutoItemStringInput::parse_input(json!({
        "tags": [" Cargo.rs "]
    }))
    .expect_err("direct pattern on array fields should target items");
    assert!(
        renamed_pattern_error
            .diagnostic_message()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected direct array pattern error: {renamed_pattern_error}"
    );

    let path_schema = PathAutoItemStringInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        path_schema.pointer("/properties/tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );

    let renamed_schema = RenamedAutoItemStringInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );
}

#[test]
fn tool_input_direct_array_numeric_constraints_auto_target_items() {
    let path_value = PathAutoItemNumericInput::parse_input(json!({
        "counts": [2, 4]
    }))
    .expect("type-level direct array numeric constraints should accept matching items");
    assert_eq!(path_value.counts, vec![2, 4]);

    let renamed_value = RenamedAutoItemNumericInput::parse_input(json!({
        "legacyCounts": [3]
    }))
    .expect("field-level direct array numeric constraints should accept aliases");
    assert_eq!(renamed_value.count_values, vec![3]);

    let path_min_error = PathAutoItemNumericInput::parse_input(json!({
        "counts": [1]
    }))
    .expect_err("direct minimum on array fields should target items");
    assert!(
        path_min_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected direct array minimum error: {path_min_error}"
    );

    let renamed_max_error = RenamedAutoItemNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("direct maximum on array fields should target items");
    assert!(
        renamed_max_error
            .diagnostic_message()
            .contains(r#"field `counts[]` must be at most 4"#),
        "unexpected direct array maximum error: {renamed_max_error}"
    );

    let path_schema = PathAutoItemNumericInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/counts/items/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        path_schema.pointer("/properties/counts/items/maximum"),
        Some(&json!(4))
    );

    let renamed_schema = RenamedAutoItemNumericInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/minimum"),
        Some(&json!(2))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/items/maximum"),
        Some(&json!(4))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/counts/x-agena-aliases"),
        Some(&json!(["count_values", "legacyCounts"]))
    );
}
use super::{
    PathAutoItemNumericInput, PathAutoItemStringInput, PathExclusiveNumericInput, PathFormatInput,
    PathItemChoiceInput, PathItemExclusiveNumericInput, PathItemFormatInput,
    PathItemNormalizeInput, PathItemNumericInput, PathItemObjectInput, PathItemPatternInput,
    PathItemValueRelationInput, PathNumericInput, PathObjectInput, PathOptionalItemNonEmptyInput,
    PathPatternInput, RenamedAutoItemNumericInput, RenamedAutoItemStringInput,
    RenamedExclusiveNumericInput, RenamedFormatInput, RenamedItemChoiceInput,
    RenamedItemExclusiveNumericInput, RenamedItemFormatInput, RenamedItemNormalizeInput,
    RenamedItemNumericInput, RenamedItemObjectInput, RenamedItemPatternInput,
    RenamedItemValueRelationInput, RenamedNumericInput, RenamedObjectInput,
    RenamedOptionalItemNonEmptyInput, RenamedPatternInput, schema_relation_labels,
};
use agena_plugin_sdk::prelude::*;
