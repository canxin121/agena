use agena_plugin_sdk::prelude::*;

mod plugin_macro_manifest_basic;
mod plugin_macro_manifest_commands;
mod plugin_macro_manifest_constraints;
mod plugin_macro_manifest_dispatch;
mod plugin_macro_manifest_impl;
mod plugin_macro_manifest_shapes_a;
mod plugin_macro_manifest_shapes_b;
pub(crate) use plugin_macro_manifest_impl::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ManifestInput {
    #[arg(trim, non_empty)]
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ManifestOutput {
    rendered: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ManifestCommandInput {
    #[arg(trim, non_empty)]
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct SemanticInput {
    /// Original path doc.
    #[arg(
        path.write,
        file,
        example = "out.txt",
        description = "Destination path."
    )]
    path: String,
    #[arg(network.internet, example = "https://example.com")]
    endpoint: String,
    #[arg(path.read, optional)]
    config: Option<String>,
    #[arg(path.read)]
    sources: Vec<String>,
    #[arg(secret)]
    token: String,
    #[serde(default)]
    #[arg(path.read)]
    defaulted_path: String,
    #[serde(default)]
    #[arg(path.read, fallback = "")]
    workspace_path: Option<String>,
    #[serde(default)]
    #[arg(path.read, jsonpath = "$.nested.paths[*]")]
    nested_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenSemanticInner {
    #[arg(path.read)]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenSemanticOuter {
    #[serde(flatten)]
    #[input(flatten_shape)]
    inner: FlattenSemanticInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenVariantSemanticInner {
    #[arg(path.read)]
    file_path: String,
    #[arg(network.internet)]
    endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum FlattenVariantSemanticInput {
    Query {
        #[serde(flatten)]
        #[input(flatten_shape)]
        inner: FlattenVariantSemanticInner,
    },
    List {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenArgInner {
    #[arg(
        name = "filePath",
        alias = "path",
        trim,
        non_empty,
        default = String::from("README.md")
    )]
    file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct InlineNestedArgInner {
    #[arg(
        name = "filePath",
        alias = "path",
        path.read,
        trim,
        non_empty,
        default = String::from("README.md")
    )]
    file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(example = serde_json::json!({
    "filePath": "Cargo.toml"
}))]
struct InlineFlattenArgInner {
    #[arg(
        name = "filePath",
        alias = "path",
        path.read,
        trim,
        non_empty,
        default = String::from("README.md")
    )]
    file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenArgOuter {
    #[serde(flatten)]
    #[input(flatten_shape)]
    inner: FlattenArgInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum FlattenVariantArgInput {
    Query {
        #[serde(flatten)]
        #[input(flatten_shape)]
        inner: FlattenArgInner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum FlattenVariantInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("file_path"),
        drop_keys("file_path"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        #[serde(flatten)]
        #[input(flatten_shape)]
        inner: FlattenArgInner,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct NestedSemanticOuter {
    #[arg(alias = "body")]
    #[input(nested_shape)]
    payload: FlattenVariantSemanticInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantSemanticInput {
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: FlattenVariantSemanticInner,
    },
    List {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct NestedArgOuter {
    #[arg(alias = "body")]
    #[input(nested_shape)]
    payload: FlattenArgInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct NestedArgArrayOuter {
    #[input(nested_shape)]
    payload: Vec<FlattenArgInner>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantArgInput {
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: FlattenArgInner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("payload.file_path"),
        drop_keys("payload.file_path"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: FlattenArgInner,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantArrayInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("payload.file_path"),
        drop_keys("payload.file_path"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: Vec<FlattenArgInner>,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(trim("body.filePath"), non_empty("body.filePath"))]
struct NestedConstraintOuter {
    #[arg(alias = "body")]
    #[input(nested_shape)]
    payload: FlattenConstraintInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(trim("body.filePath"), non_empty("body.filePath"))]
struct NestedConstraintArrayOuter {
    #[arg(alias = "body")]
    #[input(nested_shape)]
    payload: Vec<FlattenConstraintInner>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantConstraintInput {
    #[input(trim("body.filePath"), non_empty("body.filePath"))]
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: FlattenConstraintInner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum NestedVariantConstraintArrayInput {
    #[input(trim("body.filePath"), non_empty("body.filePath"))]
    Query {
        #[arg(alias = "body")]
        #[input(nested_shape)]
        payload: Vec<FlattenConstraintInner>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenConstraintInner {
    #[arg(name = "filePath", alias = "path")]
    file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(trim("filePath"), non_empty("filePath"))]
struct FlattenConstraintOuter {
    #[serde(flatten)]
    #[input(flatten_shape)]
    inner: FlattenConstraintInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum FlattenVariantConstraintInput {
    #[input(trim("filePath"), non_empty("filePath"))]
    Query {
        #[serde(flatten)]
        #[input(flatten_shape)]
        inner: FlattenConstraintInner,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct AliasSemanticInput {
    #[serde(alias = "path")]
    #[arg(path.read)]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ArgAliasSemanticInput {
    #[arg(path.read, alias = "path", trim)]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ArgNameSemanticInput {
    #[arg(name = "filePath", alias = "path", path.read, trim)]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameAllSemanticInput {
    #[arg(path.read)]
    file_path: String,
    #[arg(network.internet)]
    api_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenameListSemanticInput {
    #[serde(rename(deserialize = "inputPath", serialize = "outputPath"))]
    #[arg(path.read)]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FieldDefaultInput {
    #[arg(default = 3)]
    count: usize,
    #[arg(default)]
    enabled: bool,
    #[arg(path.read, alias = "path", default = String::from("README.md"))]
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(choices("mode", "fast", "slow"))]
struct PathChoiceInput {
    mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FieldChoiceInput {
    #[arg(name = "tool", alias = "legacyTool", choices = ["cargo", "git"])]
    tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(format("endpoint", "uri"))]
struct PathFormatInput {
    endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(min_chars("slug", 3), pattern("slug", "^[a-z0-9-]+$"))]
struct PathPatternInput {
    slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(minimum("count", 2), maximum("count", 4))]
struct PathNumericInput {
    count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(exclusive_minimum("count", 2), exclusive_maximum("count", 5))]
struct PathExclusiveNumericInput {
    count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(min_properties("labels", 1), max_properties("labels", 2))]
struct PathObjectInput {
    labels: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_min_chars("tags", 3), item_pattern("tags", "^[a-z0-9-]+$"))]
struct PathItemPatternInput {
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_choices("tools", "cargo", "git"))]
struct PathItemChoiceInput {
    tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_format("ids", "uuid"))]
struct PathItemFormatInput {
    ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_minimum("counts", 2), item_maximum("counts", 4))]
struct PathItemNumericInput {
    counts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(
    item_exclusive_minimum("counts", 2),
    item_exclusive_maximum("counts", 5)
)]
struct PathItemExclusiveNumericInput {
    counts: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_min_properties("entries", 1), item_max_properties("entries", 2))]
struct PathItemObjectInput {
    entries: Vec<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(
    item_trim("tags"),
    item_trim_suffix("tags", ".rs"),
    item_non_empty("tags")
)]
struct PathItemNormalizeInput {
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(item_non_empty_if_present("tags"))]
struct PathOptionalItemNonEmptyInput {
    tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(forbid_substrings("tags", "..", "~"), distinct_trimmed("tags"))]
struct PathItemValueRelationInput {
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(
    trim("tags"),
    trim_suffix("tags", ".rs"),
    min_chars("tags", 3),
    pattern("tags", "^[a-z0-9-]+$")
)]
struct PathAutoItemStringInput {
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(minimum("counts", 2), maximum("counts", 4))]
struct PathAutoItemNumericInput {
    counts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(choices("tools", "cargo", "git"))]
struct PathAutoItemChoiceInput {
    tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct PathRelationInput {
    #[arg(requires = "mode")]
    path: Option<String>,
    mode: Option<String>,
    #[arg(conflicts_with = "mode")]
    slug: Option<String>,
    #[arg(required_unless_present = "mode")]
    fallback: Option<String>,
    #[arg(forbid_substrings = ["..", "~"])]
    file_path: String,
    #[arg(distinct_trimmed)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct PathGroupInput {
    #[arg(exactly_one_of = ["stdin"])]
    path: Option<String>,
    stdin: Option<String>,
    #[arg(at_least_one_of = ["stdin"])]
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedFormatInput {
    #[arg(name = "endpoint", alias = "legacyEndpoint", format = "uri")]
    endpoint_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedPatternInput {
    #[arg(
        name = "slug",
        alias = "legacySlug",
        non_empty,
        min_chars = 3,
        max_chars = 16,
        pattern = "^[a-z0-9-]+$"
    )]
    slug_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedNumericInput {
    #[arg(name = "count", alias = "legacyCount", minimum = 2, maximum = 4)]
    count_value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedExclusiveNumericInput {
    #[arg(
        name = "count",
        alias = "legacyCount",
        exclusive_minimum = 2,
        exclusive_maximum = 5
    )]
    count_value: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedObjectInput {
    #[arg(
        name = "metadata",
        alias = "legacyMetadata",
        min_properties = 1,
        max_properties = 2
    )]
    metadata_value: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemFormatInput {
    #[arg(name = "ids", alias = "legacyIds", item_format = "uuid")]
    id_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemPatternInput {
    #[arg(
        name = "tags",
        alias = "legacyTags",
        item_min_chars = 3,
        item_max_chars = 16,
        item_pattern = "^[a-z0-9-]+$"
    )]
    tag_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemChoiceInput {
    #[arg(name = "tools", alias = "legacyTools", item_choices = ["cargo", "git"])]
    tool_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemNumericInput {
    #[arg(
        name = "counts",
        alias = "legacyCounts",
        item_minimum = 2,
        item_maximum = 4
    )]
    count_values: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemExclusiveNumericInput {
    #[arg(
        name = "counts",
        alias = "legacyCounts",
        item_exclusive_minimum = 2,
        item_exclusive_maximum = 5
    )]
    count_values: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemObjectInput {
    #[arg(
        name = "entries",
        alias = "legacyEntries",
        item_min_properties = 1,
        item_max_properties = 2
    )]
    entry_values: Vec<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedItemNormalizeInput {
    #[arg(
        name = "tags",
        alias = "legacyTags",
        item_trim,
        item_trim_suffix = ".rs",
        item_non_empty
    )]
    tag_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedOptionalItemNonEmptyInput {
    #[arg(name = "tags", alias = "legacyTags", item_non_empty_if_present)]
    tag_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(forbid_substrings("tags", "..", "~"), distinct_trimmed("tags"))]
struct RenamedItemValueRelationInput {
    #[arg(name = "tags", alias = "legacyTags")]
    tag_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedAutoItemStringInput {
    #[arg(
        name = "tags",
        alias = "legacyTags",
        trim,
        trim_suffix = ".rs",
        min_chars = 3,
        pattern = "^[a-z0-9-]+$"
    )]
    tag_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedAutoItemNumericInput {
    #[arg(name = "counts", alias = "legacyCounts", minimum = 2, maximum = 4)]
    count_values: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedAutoItemChoiceInput {
    #[arg(name = "tools", alias = "legacyTools", choices = ["cargo", "git"])]
    tool_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedRelationInput {
    #[arg(name = "path", alias = "legacyPath", requires = "mode")]
    file_path_value: Option<String>,
    #[arg(name = "mode", alias = "legacyMode")]
    mode_value: Option<String>,
    #[arg(name = "filePath", alias = "legacyFilePath", forbid_substrings = ["..", "~"])]
    output_path: String,
    #[arg(name = "tags", alias = "legacyTags", distinct_trimmed)]
    tag_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RenamedGroupInput {
    #[arg(name = "filePath", alias = "legacyPath", exactly_one_of = ["stdin_payload"])]
    file_path_value: Option<String>,
    #[arg(name = "stdinPayload", alias = "legacyStdin")]
    stdin_payload: Option<String>,
    #[arg(name = "text", alias = "legacyText", at_least_one_of = ["stdin_payload"])]
    text_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct DynamicPermissionInput {
    #[arg(trim, non_empty)]
    path: String,
    #[arg(trim, non_empty)]
    host: String,
    #[serde(default)]
    optional_path: Option<String>,
    #[serde(default)]
    optional_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(example = serde_json::json!({
    "query": "rust",
    "filters": ["code"],
    "limit": 3
}))]
struct RootExampleInput {
    query: String,
    filters: Vec<String>,
    limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(default = Self {
    query: String::from("rust"),
    limit: 3,
})]
struct RootDefaultInput {
    query: String,
    limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(example = serde_json::json!({
    "query": "rust"
}))]
struct RootPartialExampleInput {
    query: String,
    limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum VariantNormalizeInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(infer_when_present("query"), trim("query"), non_empty("query"))]
    Query { query: String },
    #[input(
        infer_when_present("tags"),
        item_trim("tags"),
        item_trim_suffix("tags", ".rs"),
        item_non_empty("tags")
    )]
    Tags { tags: Vec<String> },
    #[input(
        infer_when_present("auto_tags"),
        trim("auto_tags"),
        trim_suffix("auto_tags", ".rs"),
        forbid_substrings("auto_tags", ".."),
        distinct_trimmed("auto_tags"),
        min_chars("auto_tags", 3),
        pattern("auto_tags", "^[a-z0-9-]+$")
    )]
    AutoTags { auto_tags: Vec<String> },
    #[input(choices("tools", "cargo", "git"))]
    RenamedTools {
        #[serde(rename = "tools", alias = "legacyTools")]
        tool_values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum VariantRenamedFieldInput {
    #[input(trim("file_path"), non_empty("file_path"))]
    Query { file_path: String },
    #[input(requires("file_path", "mode"))]
    Run {
        file_path: Option<String>,
        mode: Option<String>,
    },
    #[input(distinct_trimmed("tag_values"))]
    Tags { tag_values: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum VariantFieldArgInput {
    #[input(infer_when_present("file_path"))]
    Query {
        #[arg(name = "filePath", alias = "path", trim, non_empty)]
        file_path: String,
    },
    Run {
        #[arg(name = "filePath", alias = "path")]
        file_path: Option<String>,
        #[arg(default = String::from("read"))]
        mode: String,
    },
    Tags {
        #[arg(name = "tagValues", alias = "tags", distinct_trimmed)]
        tag_values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum VariantInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("file_path"),
        drop_keys("file_path"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        file_path: Option<String>,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct VariantInferenceSelector {
    kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum VariantNestedInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("selector.kind"),
        drop_keys("selector.kind"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        selector: Option<VariantInferenceSelector>,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum VariantNestedFieldArgInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("selector.kind"),
        drop_keys("selector.kind"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        #[arg(name = "selector", alias = "hint")]
        selector_value: Option<VariantInferenceSelector>,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FlattenNestedInferenceInner {
    #[arg(name = "selector", alias = "hint")]
    selector: Option<VariantInferenceSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum FlattenVariantNestedInferenceInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(
        infer_when_present("selector.kind"),
        drop_keys("selector.kind"),
        trim("query_text"),
        non_empty("query_text")
    )]
    Query {
        #[serde(flatten)]
        #[input(flatten_shape)]
        inner: FlattenNestedInferenceInner,
        query_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum VariantSemanticInput {
    File {
        #[arg(path.read)]
        file_path: String,
    },
    Remote {
        #[arg(network.internet)]
        endpoint: String,
    },
}

fn tool_by_name<'a>(manifest: &'a PluginManifest, name: &str) -> &'a ToolDefinition {
    manifest
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} tool should be generated"))
}

fn command_by_id<'a>(manifest: &'a PluginManifest, id: &str) -> &'a PluginCommandDefinition {
    manifest
        .commands
        .iter()
        .find(|command| command.id == id)
        .unwrap_or_else(|| panic!("{id} command should be generated"))
}

fn schema_relation_labels(schema: &Value) -> Vec<String> {
    schema
        .pointer("/x-agena-relations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn enum_variant_schema_by_action<'a>(schema: &'a Value, action: &str) -> Option<&'a Value> {
    let action_is_match = schema.pointer("/properties/action/const") == Some(&json!(action))
        || schema
            .pointer("/properties/action/enum")
            .and_then(Value::as_array)
            .is_some_and(|values| values.len() == 1 && values[0] == json!(action));
    if action_is_match {
        return Some(schema);
    }

    match schema {
        Value::Array(items) => items
            .iter()
            .find_map(|item| enum_variant_schema_by_action(item, action)),
        Value::Object(object) => object
            .values()
            .find_map(|value| enum_variant_schema_by_action(value, action)),
        _ => None,
    }
}

fn tool_before_input(tool: &str, input: Value) -> ToolBeforeInput {
    tool_before_input_with_tags(tool, Vec::new(), input)
}

fn tool_before_input_with_tags(tool: &str, tags: Vec<ToolTag>, input: Value) -> ToolBeforeInput {
    ToolBeforeInput {
        tool: format!("test.manifest.{tool}")
            .parse()
            .expect("test tool key should parse"),
        session_id: 1,
        call_id: 2,
        workspace_root: "/workspace".to_string(),
        tags,
        input,
        title_override: None,
        metadata: Default::default(),
    }
}

fn command_before_input(command: &str) -> CommandBeforeInput {
    CommandBeforeInput {
        session_id: Some(1),
        call_id: Some(2),
        workspace_root: Some("/workspace".to_string()),
        command: command.to_string(),
        args: vec!["test".to_string()],
        cwd: std::path::PathBuf::from("/workspace"),
        env: Default::default(),
    }
}
