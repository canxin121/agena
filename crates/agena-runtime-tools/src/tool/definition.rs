pub use agena_plugin_sdk::macro_support::{
    json_schema_for_default, schema_example_texts, schema_usage_text, set_schema_metadata,
};

pub fn json_schema_for_default_with_metadata<T>(
    default: T,
    metadata: &[(&str, &str, &str)],
) -> serde_json::Value
where
    T: schemars::JsonSchema + serde::Serialize,
{
    let mut schema = json_schema_for_default(default);
    for (pointer, title, description) in metadata {
        set_schema_metadata(&mut schema, pointer, Some(title), Some(description));
    }
    schema
}
