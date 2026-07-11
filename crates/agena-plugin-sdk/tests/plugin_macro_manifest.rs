use agena_plugin_sdk::prelude::*;

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

#[derive(Default)]
struct ManifestPlugin;

#[agena_plugin(
    namespace = "test",
    name = "manifest",
    version = "0.0.0",
    summary = "Manifest macro behavior test plugin."
)]
impl ManifestPlugin {
    #[tool(
        summary = "Render text.",
        read_only,
        stream = render_stream,
        path(requests = self.render_paths(input)),
        command(
            "/manifest-render",
            id = "manifest.render",
            title = "Manifest Render",
            aliases("render-manifest"),
            usage = "/manifest-render {\"text\":\"hello\"}",
            submit_output_as_prompt
        ),
        concurrency_safe
    )]
    fn render(&self, input: &ManifestInput) -> Result<ManifestOutput> {
        Ok(ManifestOutput {
            rendered: input.text.clone(),
        })
    }

    fn render_stream(&self, sink: ToolStreamSink, input: &ManifestInput) -> Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(
            sink.stream_id().to_string(),
            input.text.clone(),
        ))
    }

    fn render_paths(&self, input: &ManifestInput) -> Vec<PathRequest> {
        vec![PathRequest::read(input.text.clone())]
    }

    /// Render docs summary.
    ///
    /// Render docs help.
    #[tool(read_only, command)]
    fn doc_render(&self) -> String {
        "doc".to_string()
    }

    #[tool(summary = "Dynamic output.", read_only)]
    fn dynamic(&self) -> ToolInvokeOutput {
        ToolInvokeOutput::text("dynamic")
    }

    #[tool(summary = "Explicit output.", output(ManifestOutput), read_only)]
    fn explicit(&self) -> ManifestOutput {
        ManifestOutput {
            rendered: "explicit".to_string(),
        }
    }

    #[tool(summary = "Semantic permissions.", mutating, command("/semantic"))]
    fn semantic(&self, input: &SemanticInput) -> String {
        format!("{} -> {}", input.path, input.endpoint)
    }

    #[tool(
        summary = "Inline semantic permissions.",
        read_only,
        command("/inline-semantic")
    )]
    fn inline_semantic(
        &self,
        #[arg(path.read, example = "README.md", description = "Path to inspect.")] path: String,
        #[arg(network.private, example = "localhost")] host: String,
    ) -> String {
        format!("{path} @ {host}")
    }

    #[tool(summary = "Inline auto usage.", read_only, command("/inline-auto"))]
    fn inline_auto(&self, path: String, count: usize) -> String {
        format!("{path}:{count}")
    }

    #[tool(summary = "Inline count usage.", read_only, command("/inline-count"))]
    fn inline_count(&self, #[arg(example = 3)] count: usize) -> String {
        count.to_string()
    }

    #[tool(
        summary = "Inline rename support.",
        read_only,
        command("/inline-rename")
    )]
    fn inline_rename(
        &self,
        #[arg(name = "filePath", alias = "path", path.read, trim)] file_path: String,
    ) -> String {
        file_path
    }

    #[tool(
        summary = "Inline default support.",
        read_only,
        command("/inline-default")
    )]
    fn inline_default(&self, #[arg(default = 3)] count: usize) -> String {
        count.to_string()
    }

    #[tool(
        summary = "Inline nested ToolInput support.",
        read_only,
        command("/inline-nested")
    )]
    fn inline_nested(
        &self,
        #[arg(alias = "body", nested_shape)] payload: InlineNestedArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[tool(
        summary = "Inline flatten ToolInput support.",
        read_only,
        command("/inline-flatten")
    )]
    fn inline_flatten(
        &self,
        #[arg(flatten_shape)] payload: InlineFlattenArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[tool(summary = "Plain string input.", read_only, command("/plain-string"))]
    fn plain_string(&self, text: String) -> String {
        text
    }

    #[tool(
        summary = "Dynamic permission DSL.",
        read_only,
        path(read = self.dynamic_path(input.path.as_str()).await?),
        path(read = input.optional_path.as_deref()),
        path(requests = self.extra_paths(input.path.as_str())),
        path(reads = Some(self.related_read_paths(input.path.as_str()))),
        path(writes = Some(self.related_write_paths(input.path.as_str()))),
        network(connect = input.host.clone()),
        network(connect = input.optional_host.as_deref()),
        network(connects = Some(self.related_hosts(input.host.as_str()))),
        network(requests = self.extra_network(input.host.as_str()))
    )]
    async fn dynamic_permission(&self, input: &DynamicPermissionInput) -> String {
        format!("{} @ {}", input.path, input.host)
    }

    async fn dynamic_path(&self, path: &str) -> Result<String> {
        Ok(format!("{path}/resolved"))
    }

    fn extra_paths(&self, path: &str) -> PathRequest {
        PathRequest::write(format!("{path}/extra"))
    }

    fn related_read_paths(&self, path: &str) -> Vec<String> {
        vec![format!("{path}/related-read")]
    }

    fn related_write_paths(&self, path: &str) -> [String; 1] {
        [format!("{path}/related-write")]
    }

    fn related_hosts(&self, host: &str) -> [String; 1] {
        [format!("static.{host}")]
    }

    fn extra_network(&self, host: &str) -> Vec<NetworkRequest> {
        vec![NetworkRequest::connect(format!("api.{host}"))]
    }

    #[command(
        "/manifest-greet",
        id = "manifest.greet",
        title = "Manifest Greet",
        description = "Greet from a typed command.",
        category = "Test",
        aliases("hello-manifest"),
        usage = "/manifest-greet {\"name\":\"Ada\"}"
    )]
    fn greet_command(&self, input: &ManifestCommandInput) -> String {
        format!("hello {}", input.name)
    }

    #[command(
        "/manifest-inline",
        id = "manifest.inline",
        title = "Manifest Inline",
        description = "Greet from inline command arguments.",
        category = "Test"
    )]
    fn inline_command(
        &self,
        #[arg(trim, non_empty, example = "Ada", description = "Name to greet.")] name: String,
    ) -> String {
        format!("hello {name}")
    }

    #[command(
        "/manifest-inline-auto",
        id = "manifest.inline_auto",
        title = "Manifest Inline Auto",
        description = "Greet from inline command arguments without explicit examples.",
        category = "Test"
    )]
    fn inline_auto_command(&self, #[arg(trim)] name: String) -> String {
        format!("hello {name}")
    }

    #[command(
        "/manifest-renamed",
        id = "manifest.renamed",
        title = "Manifest Renamed",
        description = "Command arg rename and alias support.",
        category = "Test"
    )]
    fn renamed_command(
        &self,
        #[arg(name = "filePath", alias = "path", trim)] file_path: String,
    ) -> String {
        file_path
    }

    #[command(
        "/manifest-default",
        id = "manifest.default",
        title = "Manifest Default",
        description = "Inline command default support.",
        category = "Test"
    )]
    fn default_command(&self, #[arg(default = 3)] count: usize) -> String {
        count.to_string()
    }

    #[command(
        "/manifest-inline-nested",
        id = "manifest.inline_nested",
        title = "Manifest Inline Nested",
        description = "Inline command nested ToolInput support.",
        category = "Test"
    )]
    fn inline_nested_command(
        &self,
        #[arg(alias = "body", nested_shape)] payload: InlineNestedArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[command(
        "/manifest-inline-flatten",
        id = "manifest.inline_flatten",
        title = "Manifest Inline Flatten",
        description = "Inline command flatten ToolInput support.",
        category = "Test"
    )]
    fn inline_flatten_command(
        &self,
        #[arg(flatten_shape)] payload: InlineFlattenArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[tool(summary = "Path-level choices.", read_only, command("/path-choice"))]
    fn path_choice(&self, input: &PathChoiceInput) -> String {
        input.mode.clone()
    }

    #[tool(summary = "Field-level choices.", read_only, command("/field-choice"))]
    fn field_choice(&self, input: &FieldChoiceInput) -> String {
        input.tool_name.clone()
    }

    #[tool(summary = "Path-level format.", read_only, command("/path-format"))]
    fn path_format(&self, input: &PathFormatInput) -> String {
        input.endpoint.clone()
    }

    #[tool(summary = "Path-level pattern.", read_only, command("/path-pattern"))]
    fn path_pattern(&self, input: &PathPatternInput) -> String {
        input.slug.clone()
    }

    #[tool(summary = "Path-level numeric.", read_only, command("/path-number"))]
    fn path_number(&self, input: &PathNumericInput) -> String {
        input.count.to_string()
    }

    #[tool(
        summary = "Path-level strict numeric bounds.",
        read_only,
        command("/path-exclusive-number")
    )]
    fn path_exclusive_number(&self, input: &PathExclusiveNumericInput) -> String {
        input.count.to_string()
    }

    #[tool(
        summary = "Path-level object bounds.",
        read_only,
        command("/path-object")
    )]
    fn path_object(&self, input: &PathObjectInput) -> String {
        input.labels.len().to_string()
    }

    #[tool(
        summary = "Path-level item constraints.",
        read_only,
        command("/path-item-pattern")
    )]
    fn path_item_pattern(&self, input: &PathItemPatternInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level item choice constraints.",
        read_only,
        command("/path-item-choice")
    )]
    fn path_item_choice(&self, input: &PathItemChoiceInput) -> String {
        input.tools.join(",")
    }

    #[tool(
        summary = "Path-level item format constraints.",
        read_only,
        command("/path-item-format")
    )]
    fn path_item_format(&self, input: &PathItemFormatInput) -> String {
        input.ids.join(",")
    }

    #[tool(
        summary = "Path-level item numeric bounds.",
        read_only,
        command("/path-item-number")
    )]
    fn path_item_number(&self, input: &PathItemNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level item strict numeric bounds.",
        read_only,
        command("/path-item-exclusive-number")
    )]
    fn path_item_exclusive_number(&self, input: &PathItemExclusiveNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level item object bounds.",
        read_only,
        command("/path-item-object")
    )]
    fn path_item_object(&self, input: &PathItemObjectInput) -> String {
        input.entries.len().to_string()
    }

    #[tool(
        summary = "Path-level item normalization.",
        read_only,
        command("/path-item-normalize")
    )]
    fn path_item_normalize(&self, input: &PathItemNormalizeInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level optional item non-empty.",
        read_only,
        command("/path-item-optional-non-empty")
    )]
    fn path_item_optional_non_empty(&self, input: &PathOptionalItemNonEmptyInput) -> String {
        input.tags.clone().unwrap_or_default().join(",")
    }

    #[tool(
        summary = "Path-level auto item string constraints.",
        read_only,
        command("/path-auto-item-string")
    )]
    fn path_auto_item_string(&self, input: &PathAutoItemStringInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level auto item numeric constraints.",
        read_only,
        command("/path-auto-item-number")
    )]
    fn path_auto_item_number(&self, input: &PathAutoItemNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level auto item choice constraints.",
        read_only,
        command("/path-auto-item-choice")
    )]
    fn path_auto_item_choice(&self, input: &PathAutoItemChoiceInput) -> String {
        input.tools.join(",")
    }

    #[tool(
        summary = "Path-level field relation metadata.",
        read_only,
        command("/path-relation")
    )]
    fn path_relation(&self, input: &PathRelationInput) -> String {
        input.path.clone().unwrap_or_default()
    }

    #[tool(
        summary = "Path-level field group metadata.",
        read_only,
        command("/path-group")
    )]
    fn path_group(&self, input: &PathGroupInput) -> String {
        input
            .path
            .clone()
            .or(input.stdin.clone())
            .unwrap_or_default()
    }

    #[tool(
        summary = "Renamed format metadata.",
        read_only,
        command("/renamed-format")
    )]
    fn renamed_format(&self, input: &RenamedFormatInput) -> String {
        input.endpoint_value.clone()
    }

    #[tool(
        summary = "Renamed field constraint metadata.",
        read_only,
        command("/renamed-pattern")
    )]
    fn renamed_pattern(&self, input: &RenamedPatternInput) -> String {
        input.slug_value.clone()
    }

    #[tool(
        summary = "Renamed numeric constraint metadata.",
        read_only,
        command("/renamed-number")
    )]
    fn renamed_number(&self, input: &RenamedNumericInput) -> String {
        input.count_value.to_string()
    }

    #[tool(
        summary = "Renamed strict numeric metadata.",
        read_only,
        command("/renamed-exclusive-number")
    )]
    fn renamed_exclusive_number(&self, input: &RenamedExclusiveNumericInput) -> String {
        input.count_value.to_string()
    }

    #[tool(
        summary = "Renamed object property bounds metadata.",
        read_only,
        command("/renamed-object")
    )]
    fn renamed_object(&self, input: &RenamedObjectInput) -> String {
        input.metadata_value.len().to_string()
    }

    #[tool(
        summary = "Renamed item format metadata.",
        read_only,
        command("/renamed-item-format")
    )]
    fn renamed_item_format(&self, input: &RenamedItemFormatInput) -> String {
        input.id_values.join(",")
    }

    #[tool(
        summary = "Renamed item constraint metadata.",
        read_only,
        command("/renamed-item-pattern")
    )]
    fn renamed_item_pattern(&self, input: &RenamedItemPatternInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed item choice metadata.",
        read_only,
        command("/renamed-item-choice")
    )]
    fn renamed_item_choice(&self, input: &RenamedItemChoiceInput) -> String {
        input.tool_values.join(",")
    }

    #[tool(
        summary = "Renamed item numeric bounds metadata.",
        read_only,
        command("/renamed-item-number")
    )]
    fn renamed_item_number(&self, input: &RenamedItemNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item strict numeric bounds metadata.",
        read_only,
        command("/renamed-item-exclusive-number")
    )]
    fn renamed_item_exclusive_number(&self, input: &RenamedItemExclusiveNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item object bounds metadata.",
        read_only,
        command("/renamed-item-object")
    )]
    fn renamed_item_object(&self, input: &RenamedItemObjectInput) -> String {
        input.entry_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item normalization metadata.",
        read_only,
        command("/renamed-item-normalize")
    )]
    fn renamed_item_normalize(&self, input: &RenamedItemNormalizeInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed optional item non-empty metadata.",
        read_only,
        command("/renamed-item-optional-non-empty")
    )]
    fn renamed_item_optional_non_empty(&self, input: &RenamedOptionalItemNonEmptyInput) -> String {
        input.tag_values.clone().unwrap_or_default().join(",")
    }

    #[tool(
        summary = "Renamed auto item string metadata.",
        read_only,
        command("/renamed-auto-item-string")
    )]
    fn renamed_auto_item_string(&self, input: &RenamedAutoItemStringInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed auto item numeric metadata.",
        read_only,
        command("/renamed-auto-item-number")
    )]
    fn renamed_auto_item_number(&self, input: &RenamedAutoItemNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed auto item choice metadata.",
        read_only,
        command("/renamed-auto-item-choice")
    )]
    fn renamed_auto_item_choice(&self, input: &RenamedAutoItemChoiceInput) -> String {
        input.tool_values.join(",")
    }

    #[tool(
        summary = "Variant-local enum normalization.",
        read_only,
        command("/variant-normalize")
    )]
    fn variant_normalize(&self, input: &VariantNormalizeInput) -> String {
        match input {
            VariantNormalizeInput::List {} => "list".to_string(),
            VariantNormalizeInput::Query { query } => format!("query:{query}"),
            VariantNormalizeInput::Tags { tags } => format!("tags:{}", tags.join(",")),
            VariantNormalizeInput::AutoTags { auto_tags } => {
                format!("auto_tags:{}", auto_tags.join(","))
            }
            VariantNormalizeInput::RenamedTools { tool_values } => {
                format!("renamed_tools:{}", tool_values.join(","))
            }
        }
    }

    #[command(
        "/manifest-variant-normalize",
        id = "manifest.variant_normalize",
        title = "Manifest Variant Normalize",
        description = "Typed command enum variant normalization support.",
        category = "Test"
    )]
    fn variant_normalize_command(&self, input: &VariantNormalizeInput) -> String {
        self.variant_normalize(input)
    }

    #[tool(
        summary = "Variant renamed field enum input.",
        read_only,
        command("/variant-renamed-fields")
    )]
    fn variant_renamed_fields(&self, input: &VariantRenamedFieldInput) -> String {
        match input {
            VariantRenamedFieldInput::Query { file_path } => format!("query:{file_path}"),
            VariantRenamedFieldInput::Run { file_path, mode } => format!(
                "run:{}:{}",
                file_path.clone().unwrap_or_default(),
                mode.clone().unwrap_or_default()
            ),
            VariantRenamedFieldInput::Tags { tag_values } => {
                format!("tags:{}", tag_values.join(","))
            }
        }
    }

    #[command(
        "/manifest-variant-renamed-fields",
        id = "manifest.variant_renamed_fields",
        title = "Manifest Variant Renamed Fields",
        description = "Typed command enum renamed field support.",
        category = "Test"
    )]
    fn variant_renamed_fields_command(&self, input: &VariantRenamedFieldInput) -> String {
        self.variant_renamed_fields(input)
    }

    #[tool(
        summary = "Variant field arg enum input.",
        read_only,
        command("/variant-field-args")
    )]
    fn variant_field_args(&self, input: &VariantFieldArgInput) -> String {
        match input {
            VariantFieldArgInput::Query { file_path } => format!("query:{file_path}"),
            VariantFieldArgInput::Run { file_path, mode } => {
                format!("run:{}:{}", file_path.clone().unwrap_or_default(), mode)
            }
            VariantFieldArgInput::Tags { tag_values } => format!("tags:{}", tag_values.join(",")),
        }
    }

    #[command(
        "/manifest-variant-field-args",
        id = "manifest.variant_field_args",
        title = "Manifest Variant Field Args",
        description = "Typed command enum variant field arg support.",
        category = "Test"
    )]
    fn variant_field_args_command(&self, input: &VariantFieldArgInput) -> String {
        self.variant_field_args(input)
    }

    #[tool(
        summary = "Variant inference enum input.",
        read_only,
        command("/variant-inference")
    )]
    fn variant_inference(&self, input: &VariantInferenceInput) -> String {
        match input {
            VariantInferenceInput::List {} => "list".to_string(),
            VariantInferenceInput::Query {
                file_path,
                query_text,
            } => {
                format!(
                    "query:{}:{query_text}",
                    file_path.clone().unwrap_or_default()
                )
            }
        }
    }

    #[command(
        "/manifest-variant-inference",
        id = "manifest.variant_inference",
        title = "Manifest Variant Inference",
        description = "Typed command enum inference support.",
        category = "Test"
    )]
    fn variant_inference_command(&self, input: &VariantInferenceInput) -> String {
        self.variant_inference(input)
    }

    #[tool(
        summary = "Variant declarative enum permissions.",
        read_only,
        command("/variant-semantic")
    )]
    fn variant_semantic(&self, input: &VariantSemanticInput) -> String {
        match input {
            VariantSemanticInput::File { file_path } => format!("file:{file_path}"),
            VariantSemanticInput::Remote { endpoint } => format!("remote:{endpoint}"),
        }
    }

    #[tool(
        summary = "Enum flatten semantic permissions.",
        read_only,
        command("/variant-flatten-semantic")
    )]
    fn variant_flatten_semantic(&self, input: &FlattenVariantSemanticInput) -> String {
        match input {
            FlattenVariantSemanticInput::Query { inner } => {
                format!("query:{}:{}", inner.file_path, inner.endpoint)
            }
            FlattenVariantSemanticInput::List {} => "list".to_string(),
        }
    }

    #[tool(
        summary = "Inline item value relations.",
        read_only,
        forbid_substrings("tags", "..", "~"),
        distinct_trimmed("tags"),
        command("/inline-item-value-relations")
    )]
    fn inline_item_value_relations(&self, #[arg] tags: Vec<String>) -> String {
        tags.join(",")
    }

    #[command(
        "/manifest-inline-auto-item-pattern",
        id = "manifest.inline_auto_item_pattern",
        title = "Manifest Inline Auto Item Pattern",
        description = "Inline command direct array string constraints support.",
        category = "Test"
    )]
    fn inline_auto_item_pattern_command(
        &self,
        #[arg(trim, trim_suffix = ".rs", min_chars = 3, pattern = "^[a-z0-9-]+$")] tags: Vec<
            String,
        >,
    ) -> String {
        tags.join(",")
    }

    #[command(
        "/manifest-inline-auto-item-number",
        id = "manifest.inline_auto_item_number",
        title = "Manifest Inline Auto Item Number",
        description = "Inline command direct array numeric constraints support.",
        category = "Test"
    )]
    fn inline_auto_item_number_command(
        &self,
        #[arg(minimum = 2, maximum = 4)] counts: Vec<u32>,
    ) -> String {
        counts.len().to_string()
    }

    #[command(
        "/manifest-inline-auto-item-choice",
        id = "manifest.inline_auto_item_choice",
        title = "Manifest Inline Auto Item Choice",
        description = "Inline command direct array choices support.",
        category = "Test"
    )]
    fn inline_auto_item_choice_command(
        &self,
        #[arg(choices = ["cargo", "git"])] tools: Vec<String>,
    ) -> String {
        tools.join(",")
    }

    #[command(
        "/manifest-inline-relation",
        id = "manifest.inline_relation",
        title = "Manifest Inline Relation",
        description = "Inline command relation and string-list rules support.",
        category = "Test"
    )]
    fn inline_relation_command(
        &self,
        #[arg(requires = "mode")] path: Option<String>,
        mode: Option<String>,
        #[arg(conflicts_with = "mode")] slug: Option<String>,
        #[arg(required_unless_present = "mode")] fallback: Option<String>,
        #[arg(forbid_substrings = ["..", "~"])] file_path: String,
        #[arg(distinct_trimmed)] tags: Vec<String>,
    ) -> String {
        let _ = slug;
        let _ = fallback;
        let _ = file_path;
        format!("{}{}", tags.join(","), mode.or(path).unwrap_or_default())
    }

    #[tool(
        summary = "Renamed group metadata.",
        read_only,
        command("/renamed-group")
    )]
    fn renamed_group(&self, input: &RenamedGroupInput) -> String {
        input
            .file_path_value
            .clone()
            .or(input.stdin_payload.clone())
            .unwrap_or_default()
    }

    #[command(
        "/manifest-inline-group",
        id = "manifest.inline_group",
        title = "Manifest Inline Group",
        description = "Inline command group rules support.",
        category = "Test"
    )]
    fn inline_group_command(
        &self,
        #[arg(name = "filePath", exactly_one_of = ["stdin_payload"])] file_path: Option<String>,
        #[arg(name = "stdinPayload")] stdin_payload: Option<String>,
        #[arg(name = "text", at_least_one_of = ["stdin_payload"])] text: Option<String>,
    ) -> String {
        text.or(file_path).or(stdin_payload).unwrap_or_default()
    }

    #[tool(
        summary = "Renamed relation metadata.",
        read_only,
        command("/renamed-relation")
    )]
    fn renamed_relation(&self, input: &RenamedRelationInput) -> String {
        input.mode_value.clone().unwrap_or_default()
    }

    #[command(
        "/manifest-inline-item-number",
        id = "manifest.inline_item_number",
        title = "Manifest Inline Item Number",
        description = "Inline command item numeric bounds support.",
        category = "Test"
    )]
    fn inline_item_number_command(
        &self,
        #[arg(item_minimum = 2, item_maximum = 4)] counts: Vec<u32>,
    ) -> String {
        counts.len().to_string()
    }

    #[command(
        "/manifest-inline-item-exclusive-number",
        id = "manifest.inline_item_exclusive_number",
        title = "Manifest Inline Item Exclusive Number",
        description = "Inline command item strict numeric bounds support.",
        category = "Test"
    )]
    fn inline_item_exclusive_number_command(
        &self,
        #[arg(item_exclusive_minimum = 2, item_exclusive_maximum = 5)] counts: Vec<i32>,
    ) -> String {
        counts.len().to_string()
    }

    #[command(
        "/manifest-inline-item-object",
        id = "manifest.inline_item_object",
        title = "Manifest Inline Item Object",
        description = "Inline command item object property bounds support.",
        category = "Test"
    )]
    fn inline_item_object_command(
        &self,
        #[arg(item_min_properties = 1, item_max_properties = 2)] entries: Vec<
            std::collections::BTreeMap<String, String>,
        >,
    ) -> String {
        entries.len().to_string()
    }

    #[command(
        "/manifest-inline-choice",
        id = "manifest.inline_choice",
        title = "Manifest Inline Choice",
        description = "Inline command choices support.",
        category = "Test"
    )]
    fn inline_choice_command(&self, #[arg(choices = ["cargo", "git"])] tool: String) -> String {
        tool
    }

    #[command(
        "/manifest-inline-format",
        id = "manifest.inline_format",
        title = "Manifest Inline Format",
        description = "Inline command format support.",
        category = "Test"
    )]
    fn inline_format_command(&self, #[arg(format = "uri")] endpoint: String) -> String {
        endpoint
    }

    #[command(
        "/manifest-inline-pattern",
        id = "manifest.inline_pattern",
        title = "Manifest Inline Pattern",
        description = "Inline command pattern support.",
        category = "Test"
    )]
    fn inline_pattern_command(
        &self,
        #[arg(min_chars = 3, pattern = "^[a-z0-9-]+$")] slug: String,
    ) -> String {
        slug
    }

    #[command(
        "/manifest-inline-number",
        id = "manifest.inline_number",
        title = "Manifest Inline Number",
        description = "Inline command numeric bounds support.",
        category = "Test"
    )]
    fn inline_number_command(&self, #[arg(minimum = 2, maximum = 4)] count: u32) -> String {
        count.to_string()
    }

    #[command(
        "/manifest-inline-exclusive-number",
        id = "manifest.inline_exclusive_number",
        title = "Manifest Inline Exclusive Number",
        description = "Inline command strict numeric bounds support.",
        category = "Test"
    )]
    fn inline_exclusive_number_command(
        &self,
        #[arg(exclusive_minimum = 2, exclusive_maximum = 5)] count: i32,
    ) -> String {
        count.to_string()
    }

    #[command(
        "/manifest-inline-object",
        id = "manifest.inline_object",
        title = "Manifest Inline Object",
        description = "Inline command object property bounds support.",
        category = "Test"
    )]
    fn inline_object_command(
        &self,
        #[arg(min_properties = 1, max_properties = 2)] labels: std::collections::BTreeMap<
            String,
            String,
        >,
    ) -> String {
        labels.len().to_string()
    }

    #[command(
        "/manifest-inline-item-format",
        id = "manifest.inline_item_format",
        title = "Manifest Inline Item Format",
        description = "Inline command item format support.",
        category = "Test"
    )]
    fn inline_item_format_command(&self, #[arg(item_format = "uuid")] ids: Vec<String>) -> String {
        ids.join(",")
    }

    #[command(
        "/manifest-inline-item-pattern",
        id = "manifest.inline_item_pattern",
        title = "Manifest Inline Item Pattern",
        description = "Inline command item constraints support.",
        category = "Test"
    )]
    fn inline_item_pattern_command(
        &self,
        #[arg(item_min_chars = 3, item_pattern = "^[a-z0-9-]+$")] tags: Vec<String>,
    ) -> String {
        tags.join(",")
    }

    #[command(
        "/manifest-inline-item-choice",
        id = "manifest.inline_item_choice",
        title = "Manifest Inline Item Choice",
        description = "Inline command item choices support.",
        category = "Test"
    )]
    fn inline_item_choice_command(
        &self,
        #[arg(item_choices = ["cargo", "git"])] tools: Vec<String>,
    ) -> String {
        tools.join(",")
    }

    #[command(
        "/manifest-inline-item-normalize",
        id = "manifest.inline_item_normalize",
        title = "Manifest Inline Item Normalize",
        description = "Inline command item normalization support.",
        category = "Test"
    )]
    fn inline_item_normalize_command(
        &self,
        #[arg(item_trim, item_trim_suffix = ".rs", item_non_empty)] tags: Vec<String>,
    ) -> String {
        tags.join(",")
    }

    #[command(
        "/manifest-inline-item-non-empty-if-present",
        id = "manifest.inline_item_non_empty_if_present",
        title = "Manifest Inline Item Optional",
        description = "Inline command optional item non-empty support.",
        category = "Test"
    )]
    fn inline_item_non_empty_if_present_command(
        &self,
        #[arg(item_non_empty_if_present)] tags: Option<Vec<String>>,
    ) -> String {
        tags.unwrap_or_default().join(",")
    }

    #[command(
        "/manifest-bool",
        id = "manifest.bool",
        title = "Manifest Bool",
        description = "Top-level primitive command input.",
        category = "Test"
    )]
    fn bool_command(&self, enabled: bool) -> String {
        enabled.to_string()
    }

    #[command(
        "/manifest-context",
        id = "manifest.context",
        title = "Manifest Context",
        description = "Greet with command context.",
        category = "Test"
    )]
    fn context_command(
        &self,
        input: &ManifestCommandInput,
        context: PluginCommandContext<'_>,
    ) -> String {
        format!(
            "{} via {}",
            input.name,
            context.slash.unwrap_or(context.command_id)
        )
    }

    #[hook(tool.before, tool = "render", priority = 10)]
    fn high_priority_before(&self, input: ToolBeforeInput) -> Option<ToolBeforePatch> {
        (input.input.pointer("/text").and_then(Value::as_str) == Some("priority")).then(|| {
            ToolBeforePatch {
                title_override: Some("high".to_string()),
                ..Default::default()
            }
        })
    }

    #[hook(tool.before, tool = "render", priority = 1)]
    fn fallback_before(&self, _input: ToolBeforeInput) -> ToolBeforePatch {
        ToolBeforePatch {
            title_override: Some("fallback".to_string()),
            ..Default::default()
        }
    }

    #[hook(tool.before, tools("doc_render"), priority = 5)]
    fn doc_before(&self, _input: ToolBeforeInput) -> ToolBeforePatch {
        ToolBeforePatch {
            title_override: Some("doc".to_string()),
            ..Default::default()
        }
    }

    #[hook(tool.before, plugins("test.manifest"), tags(filesystem_write), priority = 20)]
    fn write_tag_before(&self, _input: ToolBeforeInput) -> ToolBeforePatch {
        ToolBeforePatch {
            title_override: Some("write".to_string()),
            ..Default::default()
        }
    }

    #[hook(shell.before, command = "cargo")]
    fn cargo_before(&self, _input: CommandBeforeInput) -> CommandBeforeResponse {
        CommandBeforeResponse::Patch(CommandBeforePatch {
            args: Some(vec!["check".to_string()]),
            ..Default::default()
        })
    }
}

#[test]
fn tool_macro_manifest_infers_output_and_streaming() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "render");

    assert!(manifest.hooks.contains(HookSubscription::TOOL_INVOKE));
    assert!(
        manifest
            .hooks
            .contains(HookSubscription::TOOL_INVOKE_STREAM)
    );
    assert_eq!(tool.runtime.streaming, ToolStreamingMode::Streaming);
    assert!(tool.runtime.concurrency_safe);
    assert_ne!(tool.contract.output_schema, Value::Null);
    assert!(
        tool.contract
            .output_schema
            .pointer("/properties/rendered")
            .is_some(),
        "typed output schema should be inferred from Result<ManifestOutput>"
    );
}

#[test]
fn tool_macro_manifest_uses_doc_comments_and_dynamic_output() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let doc_tool = tool_by_name(&manifest, "doc_render");
    let dynamic_tool = tool_by_name(&manifest, "dynamic");
    let explicit_tool = tool_by_name(&manifest, "explicit");

    assert_eq!(
        doc_tool.docs.summary.as_deref(),
        Some("Render docs summary.")
    );
    assert!(
        doc_tool
            .docs
            .help
            .as_deref()
            .is_some_and(|help| help.contains("Render docs help."))
    );
    assert_eq!(dynamic_tool.contract.output_schema, Value::Null);
    assert!(
        explicit_tool
            .contract
            .output_schema
            .pointer("/properties/rendered")
            .is_some(),
        "explicit output(Type) should still generate a typed schema"
    );
}

#[test]
fn tool_input_field_semantics_generate_declarative_permissions() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "semantic");

    assert_eq!(SemanticInput::input_paths().len(), 6);
    assert_eq!(SemanticInput::input_paths()[0].jsonpath, "$.path");
    assert_eq!(SemanticInput::input_paths()[0].kind, PathKind::Write);
    assert_eq!(SemanticInput::input_paths()[0].fallback, None);
    assert!(!SemanticInput::input_paths()[0].optional);
    assert_eq!(SemanticInput::input_paths()[1].jsonpath, "$.config");
    assert_eq!(SemanticInput::input_paths()[1].kind, PathKind::Read);
    assert!(SemanticInput::input_paths()[1].optional);
    assert_eq!(SemanticInput::input_paths()[2].jsonpath, "$.sources[*]");
    assert_eq!(SemanticInput::input_paths()[2].kind, PathKind::Read);
    assert!(!SemanticInput::input_paths()[2].optional);
    assert_eq!(SemanticInput::input_paths()[3].jsonpath, "$.defaulted_path");
    assert_eq!(SemanticInput::input_paths()[3].kind, PathKind::Read);
    assert!(SemanticInput::input_paths()[3].optional);
    assert_eq!(SemanticInput::input_paths()[4].jsonpath, "$.workspace_path");
    assert_eq!(
        SemanticInput::input_paths()[4].fallback.as_deref(),
        Some("")
    );
    assert!(SemanticInput::input_paths()[4].optional);
    assert_eq!(
        SemanticInput::input_paths()[5].jsonpath,
        "$.nested.paths[*]"
    );
    assert!(SemanticInput::input_paths()[5].optional);
    assert_eq!(SemanticInput::input_networks().len(), 1);
    assert_eq!(SemanticInput::input_networks()[0].jsonpath, "$.endpoint");
    assert_eq!(tool.permissions.input_paths, SemanticInput::input_paths());
    assert_eq!(
        tool.permissions.input_networks,
        SemanticInput::input_networks()
    );
    assert!(tool.has_tag(ToolTag::FilesystemWrite));
    assert!(tool.has_tag(ToolTag::FilesystemRead));
    assert!(tool.has_tag(ToolTag::Network));
    assert!(tool.has_tag(ToolTag::Internet));

    let schema = SemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/path/x-agena-path"),
        Some(&json!("write"))
    );
    assert_eq!(
        schema.pointer("/properties/path/x-agena-picker"),
        Some(&json!("file"))
    );
    assert_eq!(
        schema.pointer("/properties/path/examples"),
        Some(&json!(["out.txt"]))
    );
    assert_eq!(
        schema.pointer("/properties/path/description"),
        Some(&json!("Destination path."))
    );
    assert_eq!(
        schema.pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
    assert_eq!(
        schema.pointer("/properties/endpoint/x-agena-order"),
        Some(&json!("000001"))
    );
    assert_eq!(
        schema.pointer("/properties/token/x-agena-secret"),
        Some(&json!(true))
    );
    assert_eq!(
        SemanticInput::input_example(),
        Some(json!({ "path": "out.txt", "endpoint": "https://example.com" }))
    );
    assert_eq!(
        SemanticInput::input_usage().as_deref(),
        Some("path=out.txt endpoint=https://example.com sources=[\"<item>\"] token=<token>")
    );
    assert_eq!(
        ManifestCommandInput::input_example(),
        Some(json!({ "name": "<name>" }))
    );
    assert_eq!(
        ManifestCommandInput::input_usage().as_deref(),
        Some("<name>")
    );

    let semantic_command = command_by_id(&manifest, "semantic");
    assert_eq!(
        semantic_command.usage.as_deref(),
        Some(
            "/semantic path=out.txt endpoint=https://example.com sources=[\"<item>\"] token=<token>"
        )
    );

    let inline = tool_by_name(&manifest, "inline_semantic");
    assert_eq!(inline.permissions.input_paths.len(), 1);
    assert_eq!(inline.permissions.input_paths[0].jsonpath, "$.path");
    assert_eq!(inline.permissions.input_paths[0].kind, PathKind::Read);
    assert_eq!(inline.permissions.input_networks.len(), 1);
    assert_eq!(inline.permissions.input_networks[0].jsonpath, "$.host");
    assert!(inline.has_tag(ToolTag::FilesystemRead));
    assert!(inline.has_tag(ToolTag::Network));
    assert!(inline.has_tag(ToolTag::PrivateNetwork));
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/host/x-agena-network"),
        Some(&json!("private"))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/path/description"),
        Some(&json!("Path to inspect."))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        inline
            .contract
            .input_schema
            .pointer("/properties/host/x-agena-order"),
        Some(&json!("000001"))
    );

    let inline_command = command_by_id(&manifest, "inline_semantic");
    assert_eq!(
        inline_command.usage.as_deref(),
        Some("/inline-semantic path=README.md host=localhost")
    );

    let inline_auto_tool = tool_by_name(&manifest, "inline_auto");
    assert_eq!(
        inline_auto_tool
            .contract
            .input_schema
            .pointer("/properties/path/x-agena-order"),
        Some(&json!("000000"))
    );
    assert_eq!(
        inline_auto_tool
            .contract
            .input_schema
            .pointer("/properties/count/x-agena-order"),
        Some(&json!("000001"))
    );
    let inline_auto_command = command_by_id(&manifest, "inline_auto");
    assert_eq!(
        inline_auto_command.usage.as_deref(),
        Some("/inline-auto path=<path> count=1")
    );

    let inline_count_tool = tool_by_name(&manifest, "inline_count");
    assert_eq!(
        inline_count_tool
            .contract
            .input_schema
            .pointer("/properties/count/examples"),
        Some(&json!([3]))
    );
    let inline_count_command = command_by_id(&manifest, "inline_count");
    assert_eq!(
        inline_count_command.usage.as_deref(),
        Some("/inline-count 3")
    );

    let inline_rename_tool = tool_by_name(&manifest, "inline_rename");
    assert_eq!(inline_rename_tool.permissions.input_paths.len(), 2);
    assert_eq!(
        inline_rename_tool.permissions.input_paths[0].jsonpath,
        "$.filePath"
    );
    assert!(inline_rename_tool.permissions.input_paths[0].optional);
    assert_eq!(
        inline_rename_tool.permissions.input_paths[1].jsonpath,
        "$.path"
    );
    assert!(inline_rename_tool.permissions.input_paths[1].optional);
    assert_eq!(
        inline_rename_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["path"]))
    );
    let inline_rename_command = command_by_id(&manifest, "inline_rename");
    assert_eq!(
        inline_rename_command.usage.as_deref(),
        Some("/inline-rename <filePath>")
    );

    let inline_default_tool = tool_by_name(&manifest, "inline_default");
    assert_eq!(
        inline_default_tool
            .contract
            .input_schema
            .pointer("/properties/count/default"),
        Some(&json!(3))
    );
    let inline_default_command = command_by_id(&manifest, "inline_default");
    assert_eq!(
        inline_default_command.usage.as_deref(),
        Some("/inline-default 3")
    );

    let inline_nested_tool = tool_by_name(&manifest, "inline_nested");
    let nested_paths = inline_nested_tool
        .permissions
        .input_paths
        .iter()
        .map(|spec| spec.jsonpath.as_str())
        .collect::<Vec<_>>();
    assert_eq!(nested_paths.len(), 6);
    assert!(nested_paths.contains(&"$.payload.file_path"));
    assert!(nested_paths.contains(&"$.payload.filePath"));
    assert!(nested_paths.contains(&"$.payload.path"));
    assert!(nested_paths.contains(&"$.body.file_path"));
    assert!(nested_paths.contains(&"$.body.filePath"));
    assert!(nested_paths.contains(&"$.body.path"));
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        inline_nested_tool
            .contract
            .input_schema
            .pointer("/properties/payload/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let inline_flatten_tool = tool_by_name(&manifest, "inline_flatten");
    let flatten_paths = inline_flatten_tool
        .permissions
        .input_paths
        .iter()
        .map(|spec| spec.jsonpath.as_str())
        .collect::<Vec<_>>();
    assert_eq!(flatten_paths.len(), 3);
    assert!(flatten_paths.contains(&"$.file_path"));
    assert!(flatten_paths.contains(&"$.filePath"));
    assert!(flatten_paths.contains(&"$.path"));
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        inline_flatten_tool
            .contract
            .input_schema
            .pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let plain_string_tool = tool_by_name(&manifest, "plain_string");
    assert_eq!(
        plain_string_tool.contract.input_schema.pointer("/type"),
        Some(&json!("string"))
    );
    let plain_string_command = command_by_id(&manifest, "plain_string");
    assert_eq!(
        plain_string_command.usage.as_deref(),
        Some("/plain-string <value>")
    );

    let bool_command = command_by_id(&manifest, "manifest.bool");
    assert_eq!(
        bool_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/type")),
        Some(&json!("boolean"))
    );
    assert_eq!(bool_command.usage.as_deref(), Some("/manifest-bool false"));
}

#[test]
fn tool_input_flatten_shape_propagates_declarative_permissions() {
    assert_eq!(FlattenSemanticInner::input_paths().len(), 1);
    assert_eq!(FlattenSemanticOuter::input_paths().len(), 1);
    assert_eq!(
        FlattenSemanticOuter::input_paths()[0].jsonpath,
        "$.file_path"
    );
    assert_eq!(FlattenSemanticOuter::input_paths()[0].kind, PathKind::Read);
    assert!(FlattenSemanticOuter::input_tags().contains(&ToolTag::FilesystemRead));
}

#[test]
fn tool_input_enum_flatten_shape_propagates_declarative_permissions() {
    let parsed = FlattenVariantSemanticInput::parse_input(json!({
        "action": "query",
        "file_path": "Cargo.toml",
        "endpoint": "https://example.com"
    }))
    .expect("enum flatten_shape should still parse through the flattened ToolInput");
    assert_eq!(
        parsed,
        FlattenVariantSemanticInput::Query {
            inner: FlattenVariantSemanticInner {
                file_path: "Cargo.toml".to_string(),
                endpoint: "https://example.com".to_string(),
            }
        }
    );

    let paths = FlattenVariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(
        paths[0].optional,
        "flattened enum-variant path permissions should be optional on the root shape"
    );

    let networks = FlattenVariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.endpoint");
    assert!(
        networks[0].optional,
        "flattened enum-variant network permissions should be optional on the root shape"
    );

    let schema = FlattenVariantSemanticInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the flattened query branch");
    assert_eq!(
        query_schema.pointer("/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        query_schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "variant_flatten_semantic");
    assert_eq!(tool.permissions.input_paths, paths);
    assert_eq!(tool.permissions.input_networks, networks);
    assert!(tool.has_tag(ToolTag::FilesystemRead));
    assert!(tool.has_tag(ToolTag::Network));
    assert!(tool.has_tag(ToolTag::Internet));
}

#[test]
fn tool_input_nested_shape_propagates_declarative_permissions() {
    let paths = NestedSemanticOuter::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.payload.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert_eq!(paths[1].jsonpath, "$.body.file_path");
    assert_eq!(paths[1].kind, PathKind::Read);

    let networks = NestedSemanticOuter::input_networks();
    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].jsonpath, "$.payload.endpoint");
    assert_eq!(networks[1].jsonpath, "$.body.endpoint");

    let schema = NestedSemanticOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/payload/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
    assert!(NestedSemanticOuter::input_tags().contains(&ToolTag::FilesystemRead));
    assert!(NestedSemanticOuter::input_tags().contains(&ToolTag::Network));
    assert!(NestedSemanticOuter::input_tags().contains(&ToolTag::Internet));
}

#[test]
fn tool_input_enum_nested_shape_propagates_declarative_permissions() {
    let parsed = NestedVariantSemanticInput::parse_input(json!({
        "action": "query",
        "body": {
            "file_path": "Cargo.toml",
            "endpoint": "https://example.com"
        }
    }))
    .expect("enum nested_shape should parse through the nested ToolInput");
    assert_eq!(
        parsed,
        NestedVariantSemanticInput::Query {
            payload: FlattenVariantSemanticInner {
                file_path: "Cargo.toml".to_string(),
                endpoint: "https://example.com".to_string(),
            }
        }
    );

    let paths = NestedVariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.payload.file_path");
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.body.file_path");
    assert!(paths[1].optional);

    let networks = NestedVariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].jsonpath, "$.payload.endpoint");
    assert!(networks[0].optional);
    assert_eq!(networks[1].jsonpath, "$.body.endpoint");
    assert!(networks[1].optional);

    let schema = NestedVariantSemanticInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the nested query branch");
    assert_eq!(
        query_schema.pointer("/properties/payload/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        query_schema.pointer("/properties/payload/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );
}

#[test]
fn tool_input_flatten_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = FlattenArgOuter::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("flatten_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        FlattenArgOuter {
            inner: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = FlattenArgOuter::parse_input(json!({}))
        .expect("flatten_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        FlattenArgOuter {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );

    let schema = FlattenArgOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-parse-name"),
        Some(&json!("file_path"))
    );
}

#[test]
fn tool_input_nested_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = NestedArgOuter::parse_input(json!({
        "body": { "path": " Cargo.toml " }
    }))
    .expect("nested_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        NestedArgOuter {
            payload: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = NestedArgOuter::parse_input(json!({
        "payload": {}
    }))
    .expect("nested_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        NestedArgOuter {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );

    let schema = NestedArgOuter::input_schema();
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        schema.pointer("/properties/payload/properties/filePath/x-agena-parse-name"),
        Some(&json!("file_path"))
    );
}

#[test]
fn tool_input_enum_flatten_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = FlattenVariantArgInput::parse_input(json!({
        "action": "query",
        "path": " Cargo.toml "
    }))
    .expect("enum flatten_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        FlattenVariantArgInput::Query {
            inner: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = FlattenVariantArgInput::parse_input(json!({
        "action": "query"
    }))
    .expect("enum flatten_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        FlattenVariantArgInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_normalizes_inner_aliases_and_defaults() {
    let aliased = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "body": { "path": " Cargo.toml " }
    }))
    .expect("enum nested_shape should normalize inner arg aliases before outer parsing");
    assert_eq!(
        aliased,
        NestedVariantArgInput::Query {
            payload: FlattenArgInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let defaulted = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "payload": {}
    }))
    .expect("enum nested_shape should insert inner arg defaults before outer parsing");
    assert_eq!(
        defaulted,
        NestedVariantArgInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_inference_resolves_inner_aliases() {
    let aliased = NestedVariantInferenceInput::parse_input(json!({
        "body": { "path": "marker" },
        "query_text": " cargo "
    }))
    .expect("nested_shape inner aliases should participate in action inference and drop_keys");
    assert_eq!(
        aliased,
        NestedVariantInferenceInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );

    let renamed = NestedVariantInferenceInput::parse_input(json!({
        "payload": { "filePath": "marker" },
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape inner schema-side names should participate in action inference and drop_keys",
    );
    assert_eq!(
        renamed,
        NestedVariantInferenceInput::Query {
            payload: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_array_inference_resolves_item_paths_without_brackets() {
    let aliased = NestedVariantArrayInferenceInput::parse_input(json!({
        "body": [{ "path": "marker" }],
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape array inner aliases should participate in action inference and drop_keys",
    );
    assert_eq!(
        aliased,
        NestedVariantArrayInferenceInput::Query {
            payload: vec![FlattenArgInner {
                file_path: "README.md".to_string(),
            }],
            query_text: "cargo".to_string(),
        }
    );

    let renamed = NestedVariantArrayInferenceInput::parse_input(json!({
        "payload": [{ "filePath": "marker" }],
        "query_text": " cargo "
    }))
    .expect(
        "nested_shape array inner schema-side names should participate in action inference and drop_keys",
    );
    assert_eq!(
        renamed,
        NestedVariantArrayInferenceInput::Query {
            payload: vec![FlattenArgInner {
                file_path: "README.md".to_string(),
            }],
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_nested_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = NestedConstraintOuter::parse_input(json!({
        "body": { "path": " Cargo.toml " }
    }))
    .expect("outer type-level rules should resolve nested_shape inner schema-side names");
    assert_eq!(
        parsed,
        NestedConstraintOuter {
            payload: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );

    let error = NestedConstraintOuter::parse_input(json!({
        "body": { "filePath": "   " }
    }))
    .expect_err("nested_shape outer non_empty should validate the resolved inner path");
    assert!(
        error.to_string().contains("must not be empty"),
        "unexpected nested_shape outer constraint error: {error}"
    );
}

#[test]
fn tool_input_nested_shape_array_outer_constraints_resolve_item_schema_side_paths() {
    let parsed = NestedConstraintArrayOuter::parse_input(json!({
        "body": [{ "path": " Cargo.toml " }]
    }))
    .expect("outer type-level rules should resolve nested_shape array item schema-side names");
    assert_eq!(
        parsed,
        NestedConstraintArrayOuter {
            payload: vec![FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }]
        }
    );

    let error = NestedConstraintArrayOuter::parse_input(json!({
        "body": [{ "filePath": "   " }]
    }))
    .expect_err("nested_shape array outer non_empty should validate the resolved inner item path");
    assert!(
        error.to_string().contains("must not be empty"),
        "unexpected nested_shape array outer constraint error: {error}"
    );
}

#[test]
fn tool_input_enum_nested_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = NestedVariantConstraintInput::parse_input(json!({
        "action": "query",
        "body": { "path": " Cargo.toml " }
    }))
    .expect("variant type-level rules should resolve nested_shape inner schema-side names");
    assert_eq!(
        parsed,
        NestedVariantConstraintInput::Query {
            payload: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_nested_shape_array_outer_constraints_resolve_item_schema_side_paths() {
    let parsed = NestedVariantConstraintArrayInput::parse_input(json!({
        "action": "query",
        "body": [{ "path": " Cargo.toml " }]
    }))
    .expect("variant type-level rules should resolve nested_shape array item schema-side names");
    assert_eq!(
        parsed,
        NestedVariantConstraintArrayInput::Query {
            payload: vec![FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }]
        }
    );
}

#[test]
fn tool_input_nested_shape_inner_validation_errors_are_prefixed() {
    let error = NestedArgOuter::parse_input(json!({
        "body": { "filePath": "   " }
    }))
    .expect_err("nested_shape inner validation should surface under the outer field path");
    assert!(
        error
            .to_string()
            .contains(r#"field `payload.filePath` must not be empty"#),
        "unexpected nested_shape validation error: {error}"
    );
}

#[test]
fn tool_input_enum_nested_shape_inner_validation_errors_are_prefixed() {
    let error = NestedVariantArgInput::parse_input(json!({
        "action": "query",
        "body": { "filePath": "   " }
    }))
    .expect_err("enum nested_shape inner validation should surface under the outer field path");
    assert!(
        error
            .to_string()
            .contains(r#"field `payload.filePath` must not be empty"#),
        "unexpected enum nested_shape validation error: {error}"
    );
}

#[test]
fn tool_input_nested_shape_array_inner_validation_errors_include_item_index() {
    let error = NestedArgArrayOuter::parse_input(json!({
        "payload": [
            { "filePath": "Cargo.toml" },
            { "path": "   " }
        ]
    }))
    .expect_err("nested_shape array item validation should keep the failing item index");
    assert!(
        error
            .to_string()
            .contains(r#"field `payload[1].filePath` must not be empty"#),
        "unexpected nested_shape array validation error: {error}"
    );
}

#[test]
fn tool_input_enum_flatten_shape_inference_resolves_inner_aliases() {
    let aliased = FlattenVariantInferenceInput::parse_input(json!({
        "path": "marker",
        "query_text": " cargo "
    }))
    .expect("flattened inner aliases should participate in action inference and drop_keys");
    assert_eq!(
        aliased,
        FlattenVariantInferenceInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );

    let renamed = FlattenVariantInferenceInput::parse_input(json!({
        "filePath": "marker",
        "query_text": " cargo "
    }))
    .expect("flattened inner renamed fields should participate in action inference and drop_keys");
    assert_eq!(
        renamed,
        FlattenVariantInferenceInput::Query {
            inner: FlattenArgInner {
                file_path: "README.md".to_string(),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_infer_when_present_supports_nested_paths() {
    let parsed = VariantNestedInferenceInput::parse_input(json!({
        "selector": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("infer_when_present should match nested json paths");
    assert_eq!(
        parsed,
        VariantNestedInferenceInput::Query {
            selector: Some(VariantInferenceSelector { kind: None }),
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_infer_when_present_supports_nested_alias_paths() {
    let parsed = VariantNestedFieldArgInferenceInput::parse_input(json!({
        "hint": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("nested alias heads should participate in infer_when_present/drop_keys");
    assert_eq!(
        parsed,
        VariantNestedFieldArgInferenceInput::Query {
            selector_value: Some(VariantInferenceSelector { kind: None }),
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_enum_flatten_shape_inference_supports_nested_paths() {
    let parsed = FlattenVariantNestedInferenceInput::parse_input(json!({
        "hint": { "kind": "query" },
        "query_text": " cargo "
    }))
    .expect("flattened inner aliases should participate in nested infer_when_present/drop_keys");
    assert_eq!(
        parsed,
        FlattenVariantNestedInferenceInput::Query {
            inner: FlattenNestedInferenceInner {
                selector: Some(VariantInferenceSelector { kind: None }),
            },
            query_text: "cargo".to_string(),
        }
    );
}

#[test]
fn tool_input_flatten_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = FlattenConstraintOuter::parse_input(json!({
        "filePath": " Cargo.toml "
    }))
    .expect("outer type-level rules should resolve flattened inner schema-side names");
    assert_eq!(
        parsed,
        FlattenConstraintOuter {
            inner: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_enum_flatten_shape_outer_constraints_resolve_schema_side_paths() {
    let parsed = FlattenVariantConstraintInput::parse_input(json!({
        "action": "query",
        "path": " Cargo.toml "
    }))
    .expect("variant type-level rules should resolve flattened inner schema-side names");
    assert_eq!(
        parsed,
        FlattenVariantConstraintInput::Query {
            inner: FlattenConstraintInner {
                file_path: "Cargo.toml".to_string(),
            }
        }
    );
}

#[test]
fn tool_input_field_aliases_generate_alternative_permission_sources() {
    let paths = AliasSemanticInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.path");
    assert_eq!(paths[1].kind, PathKind::Read);
    assert!(paths[1].optional);

    let arg_alias_paths = ArgAliasSemanticInput::input_paths();
    assert_eq!(arg_alias_paths.len(), 2);
    assert_eq!(arg_alias_paths[0].jsonpath, "$.file_path");
    assert_eq!(arg_alias_paths[0].kind, PathKind::Read);
    assert!(arg_alias_paths[0].optional);
    assert_eq!(arg_alias_paths[1].jsonpath, "$.path");
    assert_eq!(arg_alias_paths[1].kind, PathKind::Read);
    assert!(arg_alias_paths[1].optional);

    let parsed = ArgAliasSemanticInput::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("field-level arg alias should normalize into the canonical field");
    assert_eq!(parsed.file_path, "Cargo.toml");

    let schema = ArgAliasSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/file_path/x-agena-aliases"),
        Some(&json!(["path"]))
    );
}

#[test]
fn tool_input_field_name_attr_renames_schema_and_preserves_compat_aliases() {
    let paths = ArgNameSemanticInput::input_paths();
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0].jsonpath, "$.filePath");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.file_path");
    assert_eq!(paths[1].kind, PathKind::Read);
    assert!(paths[1].optional);
    assert_eq!(paths[2].jsonpath, "$.path");
    assert_eq!(paths[2].kind, PathKind::Read);
    assert!(paths[2].optional);

    let canonical = ArgNameSemanticInput::parse_input(json!({ "filePath": " Cargo.toml " }))
        .expect("field-level arg name should become the canonical wire key");
    assert_eq!(canonical.file_path, "Cargo.toml");

    let legacy = ArgNameSemanticInput::parse_input(json!({ "file_path": " Cargo.toml " }))
        .expect("field-level arg name should keep the old field name as an alias");
    assert_eq!(legacy.file_path, "Cargo.toml");

    let explicit_alias = ArgNameSemanticInput::parse_input(json!({ "path": " Cargo.toml " }))
        .expect("field-level arg name should still honor explicit aliases");
    assert_eq!(explicit_alias.file_path, "Cargo.toml");

    let schema = ArgNameSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );
    assert_eq!(
        ArgNameSemanticInput::input_usage().as_deref(),
        Some("<filePath>")
    );
}

#[test]
fn tool_input_serde_field_names_drive_permissions_and_metadata() {
    let paths = RenameAllSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.filePath");
    assert_eq!(paths[0].kind, PathKind::Read);

    let networks = RenameAllSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.apiUrl");

    let schema = RenameAllSemanticInput::input_schema();
    assert_eq!(
        schema.pointer("/properties/filePath/x-agena-path"),
        Some(&json!("read"))
    );
    assert_eq!(
        schema.pointer("/properties/apiUrl/x-agena-network"),
        Some(&json!("internet"))
    );

    let parsed = RenameAllSemanticInput::parse_input(json!({
        "filePath": "Cargo.toml",
        "apiUrl": "https://example.com"
    }))
    .expect("rename_all input should parse with serialized field names");
    assert_eq!(parsed.file_path, "Cargo.toml");
    assert_eq!(parsed.api_url, "https://example.com");

    let renamed = RenameListSemanticInput::input_paths();
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].jsonpath, "$.inputPath");
}

#[test]
fn tool_input_field_default_attrs_apply_to_parse_schema_and_usage() {
    let parsed = FieldDefaultInput::parse_input(json!({}))
        .expect("field-level arg defaults should populate missing values");
    assert_eq!(parsed.count, 3);
    assert!(!parsed.enabled);
    assert_eq!(parsed.file_path, "README.md");

    let aliased = FieldDefaultInput::parse_input(json!({ "path": "Cargo.toml" }))
        .expect("field-level arg defaults should not override aliases");
    assert_eq!(aliased.count, 3);
    assert!(!aliased.enabled);
    assert_eq!(aliased.file_path, "Cargo.toml");

    let schema = FieldDefaultInput::input_schema();
    assert_eq!(schema.pointer("/properties/count/default"), Some(&json!(3)));
    assert_eq!(
        schema.pointer("/properties/enabled/default"),
        Some(&json!(false))
    );
    assert_eq!(
        schema.pointer("/properties/file_path/default"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        FieldDefaultInput::input_example(),
        Some(json!({
            "count": 3,
            "file_path": "README.md",
            "enabled": false
        }))
    );
    assert_eq!(
        FieldDefaultInput::input_usage().as_deref(),
        Some("count=3 file_path=README.md enabled=false")
    );

    let paths = FieldDefaultInput::input_paths();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert!(paths[0].optional);
    assert_eq!(paths[1].jsonpath, "$.path");
    assert!(paths[1].optional);
}

#[test]
fn tool_input_choice_constraints_apply_to_parse_schema_and_usage() {
    let path_choice = PathChoiceInput::parse_input(json!({ "mode": "fast" }))
        .expect("path-level choices should accept allowed values");
    assert_eq!(path_choice.mode, "fast");
    let field_choice = FieldChoiceInput::parse_input(json!({ "legacyTool": "git" }))
        .expect("field-level choices should accept aliases");
    assert_eq!(field_choice.tool_name, "git");

    let path_error =
        PathChoiceInput::parse_input(json!({ "mode": "turbo" })).expect_err("invalid enum value");
    assert!(
        path_error
            .to_string()
            .contains(r#"field `mode` must be one of ["fast","slow"]"#),
        "unexpected path choice error: {path_error}"
    );
    assert!(
        FieldChoiceInput::parse_input(json!({ "tool": "npm" })).is_err(),
        "field-level choices should reject unsupported values",
    );

    let path_schema = PathChoiceInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/mode/enum"),
        Some(&json!(["fast", "slow"]))
    );
    assert_eq!(PathChoiceInput::input_usage().as_deref(), Some("fast"));

    let field_schema = FieldChoiceInput::input_schema();
    assert_eq!(
        field_schema.pointer("/properties/tool/enum"),
        Some(&json!(["cargo", "git"]))
    );
    assert_eq!(
        field_schema.pointer("/properties/tool/x-agena-aliases"),
        Some(&json!(["tool_name", "legacyTool"]))
    );
    assert_eq!(FieldChoiceInput::input_usage().as_deref(), Some("cargo"));

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_choice");
    assert_eq!(path_command.usage.as_deref(), Some("/path-choice fast"));
    let field_command = command_by_id(&manifest, "field_choice");
    assert_eq!(field_command.usage.as_deref(), Some("/field-choice cargo"));
}

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
            .to_string()
            .contains(r#"field `endpoint` must match format `uri`"#),
        "unexpected path format error: {path_error}"
    );

    let renamed_error = RenamedFormatInput::parse_input(json!({ "endpoint": "not a uri" }))
        .expect_err("renamed format should reject invalid values");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_format");
    assert_eq!(
        path_command.usage.as_deref(),
        Some("/path-format https://example.com")
    );
    let renamed_command = command_by_id(&manifest, "renamed_format");
    assert_eq!(
        renamed_command.usage.as_deref(),
        Some("/renamed-format https://example.com")
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
            .to_string()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected path pattern error: {path_error}"
    );
    let path_min_error = PathPatternInput::parse_input(json!({ "slug": "go" }))
        .expect_err("path-level min_chars should reject short values");
    assert!(
        path_min_error
            .to_string()
            .contains(r#"field `slug` must be at least 3 characters"#),
        "unexpected path min_chars error: {path_min_error}"
    );
    let renamed_error = RenamedPatternInput::parse_input(json!({ "slug": "Cargo" }))
        .expect_err("renamed field pattern should reject invalid values");
    assert!(
        renamed_error
            .to_string()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected renamed pattern error: {renamed_error}"
    );
    let renamed_min_error = RenamedPatternInput::parse_input(json!({ "slug": "go" }))
        .expect_err("renamed field min_chars should reject short values");
    assert!(
        renamed_min_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_pattern");
    assert_eq!(path_command.usage.as_deref(), Some("/path-pattern <slug>"));
    let renamed_command = command_by_id(&manifest, "renamed_pattern");
    assert_eq!(
        renamed_command.usage.as_deref(),
        Some("/renamed-pattern <slug>")
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
            .to_string()
            .contains(r#"field `count` must be at least 2"#),
        "unexpected minimum error: {path_min_error}"
    );
    let path_max_error = PathNumericInput::parse_input(json!({ "count": 5 }))
        .expect_err("maximum should reject high values");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `count` must be at most 4"#),
        "unexpected maximum error: {path_max_error}"
    );
    let renamed_min_error = RenamedNumericInput::parse_input(json!({ "count": 1 }))
        .expect_err("renamed numeric bounds should report the wire name");
    assert!(
        renamed_min_error
            .to_string()
            .contains(r#"field `count` must be at least 2"#),
        "unexpected renamed numeric minimum error: {renamed_min_error}"
    );
    let renamed_parse_error = RenamedNumericInput::parse_input(json!({ "count": "oops" }))
        .expect_err("renamed numeric parse errors should report the wire name");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_number");
    assert_eq!(path_command.usage.as_deref(), Some("/path-number 2"));
    let renamed_command = command_by_id(&manifest, "renamed_number");
    assert_eq!(renamed_command.usage.as_deref(), Some("/renamed-number 2"));
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
            .to_string()
            .contains(r#"field `count` must be greater than 2"#),
        "unexpected exclusive minimum error: {path_min_error}"
    );
    let path_max_error = PathExclusiveNumericInput::parse_input(json!({ "count": 5 }))
        .expect_err("exclusive_maximum should reject equal values");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `count` must be less than 5"#),
        "unexpected exclusive maximum error: {path_max_error}"
    );
    let renamed_error = RenamedExclusiveNumericInput::parse_input(json!({ "count": 2 }))
        .expect_err("renamed strict bounds should report the wire name");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_exclusive_number");
    assert_eq!(
        path_command.usage.as_deref(),
        Some("/path-exclusive-number 3")
    );
    let renamed_command = command_by_id(&manifest, "renamed_exclusive_number");
    assert_eq!(
        renamed_command.usage.as_deref(),
        Some("/renamed-exclusive-number 3")
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
            .to_string()
            .contains(r#"field `labels` requires at least 1 property"#),
        "unexpected path min_properties error: {path_min_error}"
    );

    let path_max_error = PathObjectInput::parse_input(json!({
        "labels": { "a": "1", "b": "2", "c": "3" }
    }))
    .expect_err("max_properties should reject oversized objects");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `labels` accepts at most 2 properties"#),
        "unexpected path max_properties error: {path_max_error}"
    );

    let renamed_min_error = RenamedObjectInput::parse_input(json!({ "metadata": {} }))
        .expect_err("renamed min_properties should reject empty objects");
    assert!(
        renamed_min_error
            .to_string()
            .contains(r#"field `metadata` requires at least 1 property"#),
        "unexpected renamed min_properties error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedObjectInput::parse_input(json!({ "metadata": [] }))
        .expect_err("renamed object parse errors should use wire names");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_object");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/labels/minProperties")),
        Some(&json!(1))
    );
    let renamed_command = command_by_id(&manifest, "renamed_object");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/metadata/maxProperties")),
        Some(&json!(2))
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
            .to_string()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#),
        "unexpected path item pattern error: {path_pattern_error}"
    );

    let renamed_min_error = RenamedItemPatternInput::parse_input(json!({
        "tags": ["go"]
    }))
    .expect_err("renamed item min_chars should reject short values");
    assert!(
        renamed_min_error
            .to_string()
            .contains(r#"field `tags[]` must be at least 3 characters"#),
        "unexpected renamed item min_chars error: {renamed_min_error}"
    );

    let renamed_max_error = RenamedItemPatternInput::parse_input(json!({
        "tags": ["abcdefghijklmnopq"]
    }))
    .expect_err("renamed item max_chars should reject long values");
    assert!(
        renamed_max_error
            .to_string()
            .contains(r#"field `tags[]` must be at most 16 characters"#),
        "unexpected renamed item max_chars error: {renamed_max_error}"
    );

    let renamed_parse_error = RenamedItemPatternInput::parse_input(json!({
        "tags": [1]
    }))
    .expect_err("renamed item parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_pattern");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_pattern");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/maxLength")),
        Some(&json!(16))
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
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected path item choice error: {path_error}"
    );

    let renamed_error = RenamedItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("renamed item choices should use wire names");
    assert!(
        renamed_error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed item choice error: {renamed_error}"
    );

    let renamed_parse_error = RenamedItemChoiceInput::parse_input(json!({
        "tools": [1]
    }))
    .expect_err("renamed item choice parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_choice");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_choice");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
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
            .to_string()
            .contains(r#"field `ids[]` must match format `uuid`"#),
        "unexpected path item format error: {path_error}"
    );

    let renamed_error = RenamedItemFormatInput::parse_input(json!({ "ids": ["not-a-uuid"] }))
        .expect_err("renamed item format should reject invalid values");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_format");
    assert_eq!(
        path_command.usage.as_deref(),
        Some("/path-item-format [\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/ids/items/format")),
        Some(&json!("uuid"))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_format");
    assert_eq!(
        renamed_command.usage.as_deref(),
        Some("/renamed-item-format [\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/ids/items/format")),
        Some(&json!("uuid"))
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
            .to_string()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected path item minimum error: {path_min_error}"
    );

    let path_max_error = PathItemNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("item maximum should reject high values");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `counts[]` must be at most 4"#),
        "unexpected path item maximum error: {path_max_error}"
    );

    let renamed_min_error = RenamedItemNumericInput::parse_input(json!({
        "counts": [1]
    }))
    .expect_err("renamed item numeric bounds should report the wire name");
    assert!(
        renamed_min_error
            .to_string()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected renamed item minimum error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedItemNumericInput::parse_input(json!({
        "counts": ["oops"]
    }))
    .expect_err("renamed item numeric parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_number");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_number");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
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
            .to_string()
            .contains(r#"field `counts[]` must be greater than 2"#),
        "unexpected item exclusive minimum error: {path_min_error}"
    );

    let path_max_error = PathItemExclusiveNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("item exclusive_maximum should reject equal values");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `counts[]` must be less than 5"#),
        "unexpected item exclusive maximum error: {path_max_error}"
    );

    let renamed_error = RenamedItemExclusiveNumericInput::parse_input(json!({
        "counts": [2]
    }))
    .expect_err("renamed item strict bounds should report the wire name");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_exclusive_number");
    assert_eq!(
        path_command.usage.as_deref(),
        Some("/path-item-exclusive-number [3]")
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_exclusive_number");
    assert_eq!(
        renamed_command.usage.as_deref(),
        Some("/renamed-item-exclusive-number [3]")
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
            .to_string()
            .contains(r#"field `entries[]` requires at least 1 property"#),
        "unexpected path item min_properties error: {path_min_error}"
    );

    let path_max_error = PathItemObjectInput::parse_input(json!({
        "entries": [{ "a": "1", "b": "2", "c": "3" }]
    }))
    .expect_err("item max_properties should reject oversized objects");
    assert!(
        path_max_error
            .to_string()
            .contains(r#"field `entries[]` accepts at most 2 properties"#),
        "unexpected path item max_properties error: {path_max_error}"
    );

    let renamed_min_error = RenamedItemObjectInput::parse_input(json!({
        "entries": [{}]
    }))
    .expect_err("renamed item object bounds should report the wire name");
    assert!(
        renamed_min_error
            .to_string()
            .contains(r#"field `entries[]` requires at least 1 property"#),
        "unexpected renamed item object min_properties error: {renamed_min_error}"
    );

    let renamed_parse_error = RenamedItemObjectInput::parse_input(json!({
        "entries": ["oops"]
    }))
    .expect_err("renamed item object parse errors should use wire names and indexes");
    assert!(
        renamed_parse_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_object");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/minProperties")),
        Some(&json!(1))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_object");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/maxProperties")),
        Some(&json!(2))
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
            .to_string()
            .contains(r#"field `tags[]` must not be empty"#),
        "unexpected path item normalize error: {path_error}"
    );

    let renamed_error = RenamedItemNormalizeInput::parse_input(json!({
        "tags": [" .rs "]
    }))
    .expect_err("renamed item normalization sugar should report the wire name");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_normalize");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_normalize");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
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
            .to_string()
            .contains(r#"field `tags[]` must not be empty when present"#),
        "unexpected optional item non-empty error: {path_error}"
    );

    let renamed_error = RenamedOptionalItemNonEmptyInput::parse_input(json!({
        "tags": [""]
    }))
    .expect_err("renamed item_non_empty_if_present should report the wire name");
    assert!(
        renamed_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_item_optional_non_empty");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
    let renamed_command = command_by_id(&manifest, "renamed_item_optional_non_empty");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
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
            .to_string()
            .contains(r#"field `tags[]` must not contain `..`"#),
        "unexpected type-level item forbid_substrings error: {path_forbid_error}"
    );

    let path_distinct_error = PathItemValueRelationInput::parse_input(json!({
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("type-level distinct_trimmed should target array items");
    assert!(
        path_distinct_error
            .to_string()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected type-level item distinct_trimmed error: {path_distinct_error}"
    );

    let renamed_forbid_error = RenamedItemValueRelationInput::parse_input(json!({
        "tags": ["../etc/passwd"]
    }))
    .expect_err("renamed type-level forbid_substrings should report schema-side paths");
    assert!(
        renamed_forbid_error
            .to_string()
            .contains(r#"field `tags[]` must not contain `..`"#),
        "unexpected renamed type-level item forbid_substrings error: {renamed_forbid_error}"
    );

    let renamed_distinct_error = RenamedItemValueRelationInput::parse_input(json!({
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("renamed type-level distinct_trimmed should report schema-side paths");
    assert!(
        renamed_distinct_error
            .to_string()
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
            .to_string()
            .contains(r#"field `tags[]` must be at least 3 characters"#),
        "unexpected direct array min_chars error: {path_min_error}"
    );

    let renamed_pattern_error = RenamedAutoItemStringInput::parse_input(json!({
        "tags": [" Cargo.rs "]
    }))
    .expect_err("direct pattern on array fields should target items");
    assert!(
        renamed_pattern_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_auto_item_string");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(3))
    );
    let renamed_command = command_by_id(&manifest, "renamed_auto_item_string");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
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
            .to_string()
            .contains(r#"field `counts[]` must be at least 2"#),
        "unexpected direct array minimum error: {path_min_error}"
    );

    let renamed_max_error = RenamedAutoItemNumericInput::parse_input(json!({
        "counts": [5]
    }))
    .expect_err("direct maximum on array fields should target items");
    assert!(
        renamed_max_error
            .to_string()
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

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_auto_item_number");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    let renamed_command = command_by_id(&manifest, "renamed_auto_item_number");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn tool_input_direct_array_choices_auto_target_items() {
    let path_value = PathAutoItemChoiceInput::parse_input(json!({
        "tools": ["cargo", "git"]
    }))
    .expect("type-level direct array choices should accept matching items");
    assert_eq!(path_value.tools, vec!["cargo", "git"]);

    let renamed_value = RenamedAutoItemChoiceInput::parse_input(json!({
        "legacyTools": ["cargo"]
    }))
    .expect("field-level direct array choices should accept aliases");
    assert_eq!(renamed_value.tool_values, vec!["cargo"]);

    let path_error = PathAutoItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("direct choices on array fields should target items");
    assert!(
        path_error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected direct array choices error: {path_error}"
    );

    let renamed_error = RenamedAutoItemChoiceInput::parse_input(json!({
        "tools": ["npm"]
    }))
    .expect_err("direct choices on aliased array fields should target items");
    assert!(
        renamed_error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed direct array choices error: {renamed_error}"
    );

    let path_schema = PathAutoItemChoiceInput::input_schema();
    assert_eq!(
        path_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );

    let renamed_schema = RenamedAutoItemChoiceInput::input_schema();
    assert_eq!(
        renamed_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tools/x-agena-aliases"),
        Some(&json!(["tool_values", "legacyTools"]))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_auto_item_choice");
    assert_eq!(
        path_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
    let renamed_command = command_by_id(&manifest, "renamed_auto_item_choice");
    assert_eq!(
        renamed_command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn tool_input_enum_variant_local_normalization_applies_to_parse_and_schema() {
    let default_value = VariantNormalizeInput::parse_input(json!({}))
        .expect("enum default_when_empty should still apply with variant-local normalization");
    assert_eq!(default_value, VariantNormalizeInput::List {});

    let query_value = VariantNormalizeInput::parse_input(json!({
        "query": " cargo "
    }))
    .expect("variant-level trim should apply after infer_when_present");
    assert_eq!(
        query_value,
        VariantNormalizeInput::Query {
            query: "cargo".to_string()
        }
    );

    let tags_value = VariantNormalizeInput::parse_input(json!({
        "tags": [" cargo.rs ", " git.rs "]
    }))
    .expect("variant-level item normalization should trim and strip suffixes");
    assert_eq!(
        tags_value,
        VariantNormalizeInput::Tags {
            tags: vec!["cargo".to_string(), "git".to_string()]
        }
    );

    let tags_error = VariantNormalizeInput::parse_input(json!({
        "tags": [" .rs "]
    }))
    .expect_err("variant-level item_non_empty should validate normalized items");
    assert!(
        tags_error
            .to_string()
            .contains(r#"field `tags[]` must not be empty"#),
        "unexpected enum variant item normalization error: {tags_error}"
    );

    let schema = VariantNormalizeInput::input_schema();
    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    assert_eq!(
        tags_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.variant_normalize");
    let command_schema = command
        .input_schema
        .as_ref()
        .expect("typed command should expose enum input schema");
    let command_tags_schema = enum_variant_schema_by_action(command_schema, "tags")
        .expect("typed command schema should include the tags variant branch");
    assert_eq!(
        command_tags_schema.pointer("/properties/tags/items/minLength"),
        Some(&json!(1))
    );
}

#[test]
fn tool_input_enum_variant_direct_array_constraints_auto_target_items() {
    let auto_tags_value = VariantNormalizeInput::parse_input(json!({
        "auto_tags": [" cargo.rs ", " git.rs "]
    }))
    .expect("variant-level direct array string constraints should target items");
    assert_eq!(
        auto_tags_value,
        VariantNormalizeInput::AutoTags {
            auto_tags: vec!["cargo".to_string(), "git".to_string()]
        }
    );

    let distinct_error = VariantNormalizeInput::parse_input(json!({
        "auto_tags": [" cargo.rs ", "cargo.rs"]
    }))
    .expect_err("variant-level distinct_trimmed should target normalized array items");
    assert!(
        distinct_error
            .to_string()
            .contains(r#"field `auto_tags[]` must not contain duplicate values"#),
        "unexpected variant direct-array distinct error: {distinct_error}"
    );

    let renamed_tools_value = VariantNormalizeInput::parse_input(json!({
        "action": "renamed_tools",
        "legacyTools": ["cargo"]
    }))
    .expect("variant-level direct array choices should accept aliases after remapping");
    assert_eq!(
        renamed_tools_value,
        VariantNormalizeInput::RenamedTools {
            tool_values: vec!["cargo".to_string()]
        }
    );

    let renamed_tools_error = VariantNormalizeInput::parse_input(json!({
        "action": "renamed_tools",
        "tools": ["npm"]
    }))
    .expect_err("variant-level direct array choices should target renamed array items");
    assert!(
        renamed_tools_error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed variant direct-array choice error: {renamed_tools_error}"
    );

    let schema = VariantNormalizeInput::input_schema();
    let auto_tags_schema = enum_variant_schema_by_action(&schema, "auto_tags")
        .expect("enum schema should include the auto_tags variant branch");
    assert_eq!(
        auto_tags_schema.pointer("/properties/auto_tags/items/minLength"),
        Some(&json!(3))
    );
    assert_eq!(
        auto_tags_schema.pointer("/properties/auto_tags/items/pattern"),
        Some(&json!("^[a-z0-9-]+$"))
    );
    let auto_tags_relations = schema_relation_labels(auto_tags_schema);
    assert!(auto_tags_relations.contains(&"forbid_substrings `auto_tags[]`: \"..\"".to_string()));
    assert!(auto_tags_relations.contains(&"distinct_trimmed `auto_tags[]`".to_string()));

    let renamed_tools_schema = enum_variant_schema_by_action(&schema, "renamed_tools")
        .expect("enum schema should include the renamed_tools variant branch");
    assert_eq!(
        renamed_tools_schema.pointer("/properties/tools/items/enum"),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn tool_input_enum_variant_renamed_fields_resolve_constraint_paths() {
    let query_value = VariantRenamedFieldInput::parse_input(json!({
        "action": "query",
        "filePath": " Cargo.toml "
    }))
    .expect("variant-level trim should resolve rust field names through rename_all_fields");
    assert_eq!(
        query_value,
        VariantRenamedFieldInput::Query {
            file_path: "Cargo.toml".to_string()
        }
    );

    let query_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "query",
        "filePath": " "
    }))
    .expect_err("variant-level non_empty should use the renamed field key");
    assert!(
        query_error
            .to_string()
            .contains(r#"field `filePath` must not be empty"#),
        "unexpected renamed variant non-empty error: {query_error}"
    );

    let run_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "run",
        "filePath": "Cargo.toml"
    }))
    .expect_err("variant-level requires should resolve renamed field paths");
    assert!(
        run_error
            .to_string()
            .contains(r#"field `filePath` requires `mode`"#),
        "unexpected renamed variant requires error: {run_error}"
    );

    let tags_error = VariantRenamedFieldInput::parse_input(json!({
        "action": "tags",
        "tagValues": [" cargo ", "cargo"]
    }))
    .expect_err("variant-level direct array relation rules should resolve renamed field paths");
    assert!(
        tags_error
            .to_string()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected renamed variant array relation error: {tags_error}"
    );

    let schema = VariantRenamedFieldInput::input_schema();
    let run_schema = enum_variant_schema_by_action(&schema, "run")
        .expect("enum schema should include the run variant branch");
    let run_relations = schema_relation_labels(run_schema);
    assert!(run_relations.contains(&"requires `filePath` -> `mode`".to_string()));

    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    let tags_relations = schema_relation_labels(tags_schema);
    assert!(tags_relations.contains(&"distinct_trimmed `tagValues[]`".to_string()));

    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.variant_renamed_fields");
    let command_schema = command
        .input_schema
        .as_ref()
        .expect("typed command should expose renamed enum input schema");
    let query_schema = enum_variant_schema_by_action(command_schema, "query")
        .expect("typed command schema should include the query variant branch");
    assert!(query_schema.pointer("/properties/filePath").is_some());
}

#[test]
fn tool_input_enum_variant_field_args_support_name_alias_default_and_constraints() {
    let query_value = VariantFieldArgInput::parse_input(json!({
        "path": " Cargo.toml "
    }))
    .expect("variant field arg aliases should participate in action inference and trim");
    assert_eq!(
        query_value,
        VariantFieldArgInput::Query {
            file_path: "Cargo.toml".to_string()
        }
    );

    let query_error = VariantFieldArgInput::parse_input(json!({
        "action": "query",
        "filePath": " "
    }))
    .expect_err("variant field arg non_empty should report the schema-side key");
    assert!(
        query_error
            .to_string()
            .contains(r#"field `filePath` must not be empty"#),
        "unexpected variant field arg error: {query_error}"
    );

    let run_value = VariantFieldArgInput::parse_input(json!({
        "action": "run",
        "path": "Cargo.toml"
    }))
    .expect("variant field arg defaults should populate missing fields after alias normalization");
    assert_eq!(
        run_value,
        VariantFieldArgInput::Run {
            file_path: Some("Cargo.toml".to_string()),
            mode: "read".to_string()
        }
    );

    let tags_error = VariantFieldArgInput::parse_input(json!({
        "action": "tags",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("variant field arg array rules should use the renamed schema key");
    assert!(
        tags_error
            .to_string()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected variant field arg array error: {tags_error}"
    );

    let schema = VariantFieldArgInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the query variant branch");
    assert!(query_schema.pointer("/properties/filePath").is_some());
    assert_eq!(
        query_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path", "path"]))
    );

    let run_schema = enum_variant_schema_by_action(&schema, "run")
        .expect("enum schema should include the run variant branch");
    assert_eq!(
        run_schema.pointer("/properties/mode/default"),
        Some(&json!("read"))
    );

    let tags_schema = enum_variant_schema_by_action(&schema, "tags")
        .expect("enum schema should include the tags variant branch");
    let tags_relations = schema_relation_labels(tags_schema);
    assert!(tags_relations.contains(&"distinct_trimmed `tagValues[]`".to_string()));

    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.variant_field_args");
    let command_schema = command
        .input_schema
        .as_ref()
        .expect("typed command should expose variant field arg schema");
    let command_run_schema = enum_variant_schema_by_action(command_schema, "run")
        .expect("typed command schema should include the run variant branch");
    assert_eq!(
        command_run_schema.pointer("/properties/mode/default"),
        Some(&json!("read"))
    );
}

#[test]
fn tool_input_enum_variant_inference_resolves_renamed_paths() {
    let query_value = VariantInferenceInput::parse_input(json!({
        "filePath": "marker",
        "queryText": " cargo "
    }))
    .expect("variant inference should resolve rename_all_fields paths");
    assert_eq!(
        query_value,
        VariantInferenceInput::Query {
            file_path: None,
            query_text: "cargo".to_string()
        }
    );

    let list_value = VariantInferenceInput::parse_input(json!({}))
        .expect("enum default_when_empty should still apply");
    assert_eq!(list_value, VariantInferenceInput::List {});

    let schema = VariantInferenceInput::input_schema();
    let query_schema = enum_variant_schema_by_action(&schema, "query")
        .expect("enum schema should include the query branch");
    assert!(query_schema.pointer("/properties/filePath").is_some());
    assert!(query_schema.pointer("/properties/queryText").is_some());
}

#[test]
fn tool_input_enum_variant_permissions_are_optional_at_root() {
    let paths = VariantSemanticInput::input_paths();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].jsonpath, "$.file_path");
    assert_eq!(paths[0].kind, PathKind::Read);
    assert!(
        paths[0].optional,
        "variant-derived enum path permissions should be optional on the root shape"
    );

    let networks = VariantSemanticInput::input_networks();
    assert_eq!(networks.len(), 1);
    assert_eq!(networks[0].jsonpath, "$.endpoint");
    assert!(
        networks[0].optional,
        "variant-derived enum network permissions should be optional on the root shape"
    );

    let schema = VariantSemanticInput::input_schema();
    let file_schema = enum_variant_schema_by_action(&schema, "file")
        .expect("enum schema should include the file branch");
    assert_eq!(
        file_schema.pointer("/properties/file_path/x-agena-path"),
        Some(&json!("read"))
    );
    let remote_schema = enum_variant_schema_by_action(&schema, "remote")
        .expect("enum schema should include the remote branch");
    assert_eq!(
        remote_schema.pointer("/properties/endpoint/x-agena-network"),
        Some(&json!("internet"))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "variant_semantic");
    assert_eq!(tool.permissions.input_paths, paths);
    assert_eq!(tool.permissions.input_networks, networks);
    assert!(tool.has_tag(ToolTag::FilesystemRead));
    assert!(tool.has_tag(ToolTag::Network));
    assert!(tool.has_tag(ToolTag::Internet));
}

#[test]
fn tool_input_field_relation_constraints_apply_to_parse_and_schema() {
    let path_value = PathRelationInput::parse_input(json!({
        "path": "README.md",
        "mode": "read",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo", "git"]
    }))
    .expect("path-level field relation sugar should accept valid input");
    assert_eq!(path_value.path.as_deref(), Some("README.md"));
    assert_eq!(path_value.mode.as_deref(), Some("read"));

    let renamed_value = RenamedRelationInput::parse_input(json!({
        "legacyPath": "README.md",
        "legacyMode": "read",
        "filePath": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect("renamed field relation sugar should accept aliases");
    assert_eq!(renamed_value.file_path_value.as_deref(), Some("README.md"));
    assert_eq!(renamed_value.mode_value.as_deref(), Some("read"));

    let requires_error = PathRelationInput::parse_input(json!({
        "path": "README.md",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("requires should reject missing peer fields");
    assert!(
        requires_error
            .to_string()
            .contains(r#"field `path` requires `mode`"#),
        "unexpected requires error: {requires_error}"
    );

    let conflicts_error = PathRelationInput::parse_input(json!({
        "mode": "read",
        "slug": "docs",
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("conflicts_with should reject simultaneous fields");
    assert!(
        conflicts_error
            .to_string()
            .contains(r#"field `slug` conflicts with `mode`"#),
        "unexpected conflicts_with error: {conflicts_error}"
    );

    let required_unless_error = PathRelationInput::parse_input(json!({
        "file_path": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("required_unless_present should reject missing fallback");
    assert!(
        required_unless_error
            .to_string()
            .contains(r#"field `fallback` is required unless `mode` is present"#),
        "unexpected required_unless_present error: {required_unless_error}"
    );

    let forbid_error = PathRelationInput::parse_input(json!({
        "fallback": "default",
        "file_path": "../etc/passwd",
        "tags": ["cargo"]
    }))
    .expect_err("forbid_substrings should reject matching substrings");
    assert!(
        forbid_error
            .to_string()
            .contains(r#"field `file_path` must not contain `..`"#),
        "unexpected forbid_substrings error: {forbid_error}"
    );

    let distinct_error = PathRelationInput::parse_input(json!({
        "fallback": "default",
        "file_path": "src/lib.rs",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("distinct_trimmed should reject duplicate trimmed values");
    assert!(
        distinct_error
            .to_string()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected distinct_trimmed error: {distinct_error}"
    );

    let renamed_requires_error = RenamedRelationInput::parse_input(json!({
        "path": "README.md",
        "filePath": "src/lib.rs",
        "tags": ["cargo"]
    }))
    .expect_err("renamed requires should report schema-side wire names");
    assert!(
        renamed_requires_error
            .to_string()
            .contains(r#"field `path` requires `mode`"#),
        "unexpected renamed requires error: {renamed_requires_error}"
    );

    let renamed_forbid_error = RenamedRelationInput::parse_input(json!({
        "filePath": "../etc/passwd",
        "tags": ["cargo"]
    }))
    .expect_err("renamed forbid_substrings should use schema-side wire names");
    assert!(
        renamed_forbid_error
            .to_string()
            .contains(r#"field `filePath` must not contain `..`"#),
        "unexpected renamed forbid_substrings error: {renamed_forbid_error}"
    );

    let renamed_distinct_error = RenamedRelationInput::parse_input(json!({
        "filePath": "src/lib.rs",
        "tags": [" cargo ", "cargo"]
    }))
    .expect_err("renamed distinct_trimmed should use schema-side wire names");
    assert!(
        renamed_distinct_error
            .to_string()
            .contains(r#"field `tags[]` must not contain duplicate values"#),
        "unexpected renamed distinct_trimmed error: {renamed_distinct_error}"
    );

    let path_schema = PathRelationInput::input_schema();
    let path_relations = schema_relation_labels(&path_schema);
    assert!(path_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(path_relations.contains(&"conflicts_with `slug` x `mode`".to_string()));
    assert!(
        path_relations
            .contains(&"required_unless_present `fallback` unless `mode` present".to_string())
    );
    assert!(path_relations.contains(&"forbid_substrings `file_path`: \"..\", \"~\"".to_string()));
    assert!(path_relations.contains(&"distinct_trimmed `tags[]`".to_string()));

    let renamed_schema = RenamedRelationInput::input_schema();
    let renamed_relations = schema_relation_labels(&renamed_schema);
    assert!(renamed_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(renamed_relations.contains(&"forbid_substrings `filePath`: \"..\", \"~\"".to_string()));
    assert!(renamed_relations.contains(&"distinct_trimmed `tags[]`".to_string()));
    assert_eq!(
        renamed_schema.pointer("/properties/path/x-agena-aliases"),
        Some(&json!(["file_path_value", "legacyPath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["output_path", "legacyFilePath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/tags/x-agena-aliases"),
        Some(&json!(["tag_values", "legacyTags"]))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_relation");
    let path_command_relations = schema_relation_labels(
        path_command
            .input_schema
            .as_ref()
            .expect("tool command schema"),
    );
    assert!(path_command_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(
        path_command_relations
            .contains(&"required_unless_present `fallback` unless `mode` present".to_string())
    );
    let renamed_command = command_by_id(&manifest, "renamed_relation");
    let renamed_command_relations = schema_relation_labels(
        renamed_command
            .input_schema
            .as_ref()
            .expect("renamed schema"),
    );
    assert!(renamed_command_relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(
        renamed_command_relations
            .contains(&"forbid_substrings `filePath`: \"..\", \"~\"".to_string())
    );
}

#[test]
fn tool_input_field_group_constraints_apply_to_parse_and_schema() {
    let path_value = PathGroupInput::parse_input(json!({
        "path": "README.md",
        "text": "hello"
    }))
    .expect("path-level field group sugar should accept valid input");
    assert_eq!(path_value.path.as_deref(), Some("README.md"));

    let renamed_value = RenamedGroupInput::parse_input(json!({
        "legacyPath": "README.md",
        "text": "hello"
    }))
    .expect("renamed field group sugar should accept aliases");
    assert_eq!(renamed_value.file_path_value.as_deref(), Some("README.md"));

    let exactly_one_error = PathGroupInput::parse_input(json!({
        "path": "README.md",
        "stdin": "payload",
        "text": "hello"
    }))
    .expect_err("exactly_one_of should reject both fields present");
    assert!(
        exactly_one_error
            .to_string()
            .contains(r#"exactly one of `path` or `stdin` is required"#),
        "unexpected exactly_one_of error: {exactly_one_error}"
    );

    let exactly_one_missing_error = PathGroupInput::parse_input(json!({
        "text": "hello"
    }))
    .expect_err("exactly_one_of should reject both fields missing");
    assert!(
        exactly_one_missing_error
            .to_string()
            .contains(r#"exactly one of `path` or `stdin` is required"#),
        "unexpected exactly_one_of missing error: {exactly_one_missing_error}"
    );

    let at_least_one_error = PathGroupInput::parse_input(json!({
        "path": "README.md"
    }))
    .expect_err("at_least_one_of should reject missing peers");
    assert!(
        at_least_one_error
            .to_string()
            .contains(r#"at least one of `text` or `stdin` is required"#),
        "unexpected at_least_one_of error: {at_least_one_error}"
    );

    let renamed_exactly_one_error = RenamedGroupInput::parse_input(json!({
        "filePath": "README.md",
        "stdinPayload": "payload",
        "text": "hello"
    }))
    .expect_err("renamed exactly_one_of should use schema-side wire names");
    assert!(
        renamed_exactly_one_error
            .to_string()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#),
        "unexpected renamed exactly_one_of error: {renamed_exactly_one_error}"
    );

    let renamed_at_least_one_error = RenamedGroupInput::parse_input(json!({
        "filePath": "README.md"
    }))
    .expect_err("renamed at_least_one_of should use schema-side wire names");
    assert!(
        renamed_at_least_one_error
            .to_string()
            .contains(r#"at least one of `text` or `stdinPayload` is required"#),
        "unexpected renamed at_least_one_of error: {renamed_at_least_one_error}"
    );

    let path_schema = PathGroupInput::input_schema();
    let path_relations = schema_relation_labels(&path_schema);
    assert!(path_relations.contains(&"exactly_one_of: `path`, `stdin`".to_string()));
    assert!(path_relations.contains(&"at_least_one_of: `text`, `stdin`".to_string()));

    let renamed_schema = RenamedGroupInput::input_schema();
    let renamed_relations = schema_relation_labels(&renamed_schema);
    assert!(renamed_relations.contains(&"exactly_one_of: `filePath`, `stdinPayload`".to_string()));
    assert!(renamed_relations.contains(&"at_least_one_of: `text`, `stdinPayload`".to_string()));
    assert_eq!(
        renamed_schema.pointer("/properties/filePath/x-agena-aliases"),
        Some(&json!(["file_path_value", "legacyPath"]))
    );
    assert_eq!(
        renamed_schema.pointer("/properties/stdinPayload/x-agena-aliases"),
        Some(&json!(["stdin_payload", "legacyStdin"]))
    );

    let manifest = Plugin::manifest(&ManifestPlugin);
    let path_command = command_by_id(&manifest, "path_group");
    let path_command_relations = schema_relation_labels(
        path_command
            .input_schema
            .as_ref()
            .expect("tool group schema"),
    );
    assert!(path_command_relations.contains(&"exactly_one_of: `path`, `stdin`".to_string()));
    let renamed_command = command_by_id(&manifest, "renamed_group");
    let renamed_command_relations = schema_relation_labels(
        renamed_command
            .input_schema
            .as_ref()
            .expect("renamed group schema"),
    );
    assert!(
        renamed_command_relations
            .contains(&"exactly_one_of: `filePath`, `stdinPayload`".to_string())
    );
}

#[test]
fn tool_input_root_example_attr_drives_schema_example_and_usage() {
    assert_eq!(
        RootExampleInput::input_example(),
        Some(json!({
            "query": "rust",
            "filters": ["code"],
            "limit": 3
        }))
    );
    assert_eq!(
        RootExampleInput::input_usage().as_deref(),
        Some("query=rust filters=[\"code\"] limit=3")
    );

    let schema = RootExampleInput::input_schema();
    assert_eq!(
        schema.pointer("/examples"),
        Some(&json!([{
            "query": "rust",
            "filters": ["code"],
            "limit": 3
        }]))
    );
}

#[test]
fn tool_input_partial_root_example_still_fills_required_usage_fields() {
    assert_eq!(
        RootPartialExampleInput::input_example(),
        Some(json!({
            "query": "rust"
        }))
    );
    assert_eq!(
        RootPartialExampleInput::input_usage().as_deref(),
        Some("query=rust limit=1")
    );
}

#[test]
fn tool_input_root_default_attr_applies_to_null_input_and_schema() {
    let parsed = RootDefaultInput::parse_input(Value::Null)
        .expect("root input default should populate null input");
    assert_eq!(parsed.query, "rust");
    assert_eq!(parsed.limit, 3);

    let schema = RootDefaultInput::input_schema();
    assert_eq!(
        schema.pointer("/default"),
        Some(&json!({
            "query": "rust",
            "limit": 3
        }))
    );
    assert_eq!(
        RootDefaultInput::input_usage().as_deref(),
        Some("query=rust limit=3")
    );

    assert!(
        RootDefaultInput::parse_input(json!({ "query": "go" })).is_err(),
        "root input default should not silently merge partial object payloads",
    );
}

#[test]
fn tool_macro_permission_dispatch_parses_tool_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let input = json!({ "text": "/tmp/render.txt" });
    let requests = runtime
        .block_on(Plugin::permission_paths(&plugin, "render", &input))
        .expect("permission dispatch should succeed");

    assert_eq!(requests, vec![PathRequest::read("/tmp/render.txt")]);
}

#[test]
fn tool_macro_permission_dsl_generates_dynamic_permissions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let input = json!({
        "path": "/tmp/dynamic",
        "host": "example.com",
        "optional_path": "/tmp/dynamic/optional",
        "optional_host": "optional.example.com"
    });
    let paths = runtime
        .block_on(Plugin::permission_paths(
            &plugin,
            "dynamic_permission",
            &input,
        ))
        .expect("path permission DSL should succeed");
    let networks = runtime
        .block_on(Plugin::permission_networks(
            &plugin,
            "dynamic_permission",
            &input,
        ))
        .expect("network permission DSL should succeed");

    assert_eq!(
        paths,
        vec![
            PathRequest::read("/tmp/dynamic/resolved"),
            PathRequest::read("/tmp/dynamic/optional"),
            PathRequest::write("/tmp/dynamic/extra"),
            PathRequest::read("/tmp/dynamic/related-read"),
            PathRequest::write("/tmp/dynamic/related-write")
        ]
    );
    assert_eq!(
        networks,
        vec![
            NetworkRequest::connect("example.com"),
            NetworkRequest::connect("optional.example.com"),
            NetworkRequest::connect("static.example.com"),
            NetworkRequest::connect("api.example.com")
        ]
    );

    let manifest = Plugin::manifest(&plugin);
    let tool = tool_by_name(&manifest, "dynamic_permission");
    assert_eq!(
        tool.permissions
            .tags
            .iter()
            .filter(|tag| **tag == ToolTag::FilesystemRead)
            .count(),
        1,
        "macro-generated tags should be deduplicated"
    );
    assert_eq!(
        tool.permissions
            .tags
            .iter()
            .filter(|tag| **tag == ToolTag::Network)
            .count(),
        1,
        "macro-generated tags should be deduplicated"
    );
}

#[test]
fn tool_macro_invoke_dispatch_parses_and_serializes_output() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "render".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "text": "hello" }),
            },
        ))
        .expect("tool invoke should succeed");

    assert_eq!(output.payload, Some(json!({ "rendered": "hello" })));
    assert_eq!(output.output_text, r#"{"rendered":"hello"}"#);
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_rename_and_alias() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_rename".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "path": " README.md " }),
            },
        ))
        .expect("inline rename tool invoke should succeed");

    assert_eq!(output.output_text, "README.md");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_default_expr() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_default".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({}),
            },
        ))
        .expect("inline default tool invoke should succeed");

    assert_eq!(output.output_text, "3");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_nested_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_nested".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({
                    "body": { "path": " Cargo.toml " },
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline nested tool invoke should succeed");

    assert_eq!(output.output_text, "Cargo.toml:cargo");
}

#[test]
fn tool_macro_invoke_dispatch_supports_inline_arg_flatten_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_flatten".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({
                    "path": " Cargo.toml ",
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline flatten tool invoke should succeed");

    assert_eq!(output.output_text, "Cargo.toml:cargo");
}

#[test]
fn tool_macro_manifest_supports_type_level_inline_item_value_relations() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let tool = tool_by_name(&manifest, "inline_item_value_relations");
    let relations = schema_relation_labels(&tool.contract.input_schema);

    assert!(relations.contains(&"forbid_substrings `tags[]`: \"..\", \"~\"".to_string()));
    assert!(relations.contains(&"distinct_trimmed `tags[]`".to_string()));
}

#[test]
fn tool_macro_invoke_dispatch_applies_type_level_inline_item_value_relations() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": ["cargo", "git"] }),
            },
        ))
        .expect("inline item value relations tool invoke should succeed");
    assert_eq!(output.output_text, "cargo,git");

    let forbid_error = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": ["../etc/passwd"] }),
            },
        ))
        .expect_err("type-level inline forbid_substrings should target array items");
    assert!(
        forbid_error
            .to_string()
            .contains(r#"field `tags[]` must not contain `..`"#)
    );

    let distinct_error = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "inline_item_value_relations".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                input: json!({ "tags": [" cargo ", "cargo"] }),
            },
        ))
        .expect_err("type-level inline distinct_trimmed should target array items");
    assert!(
        distinct_error
            .to_string()
            .contains(r#"field `tags[]` must not contain duplicate values"#)
    );
}

#[test]
fn command_macro_manifest_generates_command_definition() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.greet");

    assert_eq!(command.title, "Manifest Greet");
    assert_eq!(command.description, "Greet from a typed command.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-greet"));
    assert_eq!(command.aliases, vec!["hello-manifest"]);
    assert_eq!(command.handler.as_deref(), Some("manifest.greet"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .is_some(),
        "typed command input should generate JSON schema"
    );
    match &command.action {
        PluginUiAction::InvokeCommand { command, input } => {
            assert_eq!(command, "manifest.greet");
            assert!(input.is_none());
        }
        other => panic!("expected default InvokeCommand action, got {other:?}"),
    }
}

#[test]
fn command_macro_supports_inline_arg_generated_input() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline");

    assert_eq!(command.title, "Manifest Inline");
    assert_eq!(command.description, "Greet from inline command arguments.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .and_then(Value::as_object)
            .is_some(),
        true,
        "inline command args should generate an input schema"
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name/description")),
        Some(&json!("Name to greet."))
    );
    assert_eq!(command.usage.as_deref(), Some("/manifest-inline Ada"));
    match &command.action {
        PluginUiAction::InvokeCommand { command, input } => {
            assert_eq!(command, "manifest.inline");
            assert!(input.is_none());
        }
        other => panic!("expected default InvokeCommand action, got {other:?}"),
    }
}

#[test]
fn command_macro_supports_inline_generated_input_without_examples() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto");

    assert_eq!(command.title, "Manifest Inline Auto");
    assert_eq!(
        command.description,
        "Greet from inline command arguments without explicit examples."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-auto"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_auto"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-auto <name>")
    );
}

#[test]
fn command_macro_supports_inline_arg_rename_and_alias() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.renamed");

    assert_eq!(command.title, "Manifest Renamed");
    assert_eq!(command.description, "Command arg rename and alias support.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-renamed"));
    assert_eq!(command.handler.as_deref(), Some("manifest.renamed"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-renamed <filePath>")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath"))
            .and_then(Value::as_object)
            .is_some(),
        true,
        "renamed inline command args should expose the renamed input field"
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/x-agena-aliases")),
        Some(&json!(["path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_default_expr() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.default");

    assert_eq!(command.title, "Manifest Default");
    assert_eq!(command.description, "Inline command default support.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-default"));
    assert_eq!(command.handler.as_deref(), Some("manifest.default"));
    assert_eq!(command.usage.as_deref(), Some("/manifest-default 3"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/default")),
        Some(&json!(3))
    );
}

#[test]
fn command_macro_supports_inline_arg_nested_shape() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_nested");

    assert_eq!(command.title, "Manifest Inline Nested");
    assert_eq!(
        command.description,
        "Inline command nested ToolInput support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-nested"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_nested"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/payload/properties/filePath/default")),
        Some(&json!("README.md"))
    );
    assert_eq!(
        command.input_schema.as_ref().and_then(
            |schema| schema.pointer("/properties/payload/properties/filePath/x-agena-aliases")
        ),
        Some(&json!(["file_path", "path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_flatten_shape() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_flatten");

    assert_eq!(command.title, "Manifest Inline Flatten");
    assert_eq!(
        command.description,
        "Inline command flatten ToolInput support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-inline-flatten"));
    assert_eq!(command.handler.as_deref(), Some("manifest.inline_flatten"));
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-flatten filePath=Cargo.toml query_text=<query_text>")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/required")),
        Some(&json!(["filePath", "query_text"]))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/file_path")),
        None
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/default")),
        Some(&json!("README.md"))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/filePath/x-agena-aliases")),
        Some(&json!(["file_path", "path"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_choice");

    assert_eq!(command.title, "Manifest Inline Choice");
    assert_eq!(command.description, "Inline command choices support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-choice cargo")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tool/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_format() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_format");

    assert_eq!(command.title, "Manifest Inline Format");
    assert_eq!(command.description, "Inline command format support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-format https://example.com")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/endpoint/format")),
        Some(&json!("uri"))
    );
}

#[test]
fn command_macro_supports_inline_arg_pattern() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_pattern");

    assert_eq!(command.title, "Manifest Inline Pattern");
    assert_eq!(command.description, "Inline command pattern support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-pattern <slug>")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/slug/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/slug/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_number");

    assert_eq!(command.title, "Manifest Inline Number");
    assert_eq!(
        command.description,
        "Inline command numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(command.usage.as_deref(), Some("/manifest-inline-number 2"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_exclusive_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_exclusive_number");

    assert_eq!(command.title, "Manifest Inline Exclusive Number");
    assert_eq!(
        command.description,
        "Inline command strict numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-exclusive-number 3")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/exclusiveMinimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/count/exclusiveMaximum")),
        Some(&json!(5))
    );
}

#[test]
fn command_macro_supports_inline_arg_object_property_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_object");

    assert_eq!(command.title, "Manifest Inline Object");
    assert_eq!(
        command.description,
        "Inline command object property bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/labels/minProperties")),
        Some(&json!(1))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/labels/maxProperties")),
        Some(&json!(2))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_format() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_format");

    assert_eq!(command.title, "Manifest Inline Item Format");
    assert_eq!(command.description, "Inline command item format support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-item-format [\"550e8400-e29b-41d4-a716-446655440000\"]")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/ids/items/format")),
        Some(&json!("uuid"))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_pattern");

    assert_eq!(command.title, "Manifest Inline Item Pattern");
    assert_eq!(
        command.description,
        "Inline command item constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_choice");

    assert_eq!(command.title, "Manifest Inline Item Choice");
    assert_eq!(command.description, "Inline command item choices support.");
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_normalization_and_non_empty() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_normalize");

    assert_eq!(command.title, "Manifest Inline Item Normalize");
    assert_eq!(
        command.description,
        "Inline command item normalization support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_non_empty_if_present() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_non_empty_if_present");

    assert_eq!(command.title, "Manifest Inline Item Optional");
    assert_eq!(
        command.description,
        "Inline command optional item non-empty support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(1))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_string_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_pattern");

    assert_eq!(command.title, "Manifest Inline Auto Item Pattern");
    assert_eq!(
        command.description,
        "Inline command direct array string constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/minLength")),
        Some(&json!(3))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tags/items/pattern")),
        Some(&json!("^[a-z0-9-]+$"))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_numeric_constraints() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_number");

    assert_eq!(command.title, "Manifest Inline Auto Item Number");
    assert_eq!(
        command.description,
        "Inline command direct array numeric constraints support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_direct_array_choices() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_auto_item_choice");

    assert_eq!(command.title, "Manifest Inline Auto Item Choice");
    assert_eq!(
        command.description,
        "Inline command direct array choices support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/tools/items/enum")),
        Some(&json!(["cargo", "git"]))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_number");

    assert_eq!(command.title, "Manifest Inline Item Number");
    assert_eq!(
        command.description,
        "Inline command item numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/minimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/maximum")),
        Some(&json!(4))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_exclusive_numeric_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_exclusive_number");

    assert_eq!(command.title, "Manifest Inline Item Exclusive Number");
    assert_eq!(
        command.description,
        "Inline command item strict numeric bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command.usage.as_deref(),
        Some("/manifest-inline-item-exclusive-number [3]")
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/exclusiveMinimum")),
        Some(&json!(2))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/counts/items/exclusiveMaximum")),
        Some(&json!(5))
    );
}

#[test]
fn command_macro_supports_inline_arg_item_object_bounds() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_item_object");

    assert_eq!(command.title, "Manifest Inline Item Object");
    assert_eq!(
        command.description,
        "Inline command item object property bounds support."
    );
    assert_eq!(command.category, "Test");
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/minProperties")),
        Some(&json!(1))
    );
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/entries/items/maxProperties")),
        Some(&json!(2))
    );
}

#[test]
fn command_macro_supports_inline_arg_relations() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_relation");

    assert_eq!(command.title, "Manifest Inline Relation");
    assert_eq!(
        command.description,
        "Inline command relation and string-list rules support."
    );
    assert_eq!(command.category, "Test");
    let relations = schema_relation_labels(
        command
            .input_schema
            .as_ref()
            .expect("inline relation command should expose schema"),
    );
    assert!(relations.contains(&"requires `path` -> `mode`".to_string()));
    assert!(relations.contains(&"conflicts_with `slug` x `mode`".to_string()));
    assert!(
        relations.contains(&"required_unless_present `fallback` unless `mode` present".to_string())
    );
    assert!(relations.contains(&"forbid_substrings `file_path`: \"..\", \"~\"".to_string()));
    assert!(relations.contains(&"distinct_trimmed `tags[]`".to_string()));
}

#[test]
fn command_macro_supports_inline_arg_groups() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.inline_group");

    assert_eq!(command.title, "Manifest Inline Group");
    assert_eq!(command.description, "Inline command group rules support.");
    assert_eq!(command.category, "Test");
    let relations = schema_relation_labels(
        command
            .input_schema
            .as_ref()
            .expect("inline group command should expose schema"),
    );
    assert!(relations.contains(&"exactly_one_of: `filePath`, `stdinPayload`".to_string()));
    assert!(relations.contains(&"at_least_one_of: `text`, `stdinPayload`".to_string()));
}

#[test]
fn command_macro_supports_typed_input_with_command_context() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.context");

    assert_eq!(command.title, "Manifest Context");
    assert_eq!(command.description, "Greet with command context.");
    assert_eq!(command.category, "Test");
    assert_eq!(command.slash.as_deref(), Some("/manifest-context"));
    assert_eq!(command.handler.as_deref(), Some("manifest.context"));
    assert_eq!(command.usage.as_deref(), Some("/manifest-context <name>"));
    assert_eq!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/name"))
            .and_then(Value::as_object)
            .is_some(),
        true,
        "typed command + context should still expose the typed input schema"
    );
}

#[test]
fn tool_command_macro_generates_command_definition() {
    let manifest = Plugin::manifest(&ManifestPlugin);
    let command = command_by_id(&manifest, "manifest.render");
    let default_command = command_by_id(&manifest, "doc_render");

    assert_eq!(command.title, "Manifest Render");
    assert_eq!(command.description, "Render text.");
    assert_eq!(command.slash.as_deref(), Some("/manifest-render"));
    assert_eq!(command.aliases, vec!["render-manifest"]);
    assert_eq!(command.handler.as_deref(), Some("manifest.render"));
    assert!(
        command
            .input_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/text"))
            .is_some(),
        "tool-backed command should reuse the tool input schema"
    );
    match &command.action {
        PluginUiAction::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => {
            assert_eq!(tool, "render");
            assert!(input.is_none());
            assert!(*submit_output_as_prompt);
        }
        other => panic!("expected default InvokeTool action, got {other:?}"),
    }

    assert_eq!(default_command.title, "Doc Render");
    assert_eq!(default_command.description, "Render docs summary.");
    assert!(default_command.slash.is_none());
    assert_eq!(default_command.handler.as_deref(), Some("doc_render"));
    match &default_command.action {
        PluginUiAction::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => {
            assert_eq!(tool, "doc_render");
            assert!(input.is_none());
            assert!(!*submit_output_as_prompt);
        }
        other => panic!("expected default InvokeTool action, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_parses_typed_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.greet".to_string(),
                slash: Some("/manifest-greet".to_string()),
                raw: "/manifest-greet Ada".to_string(),
                input: json!({ "name": " Ada " }),
            },
        ))
        .expect("command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "hello Ada"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_parses_inline_arg_generated_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline".to_string(),
                slash: Some("/manifest-inline".to_string()),
                raw: "/manifest-inline Ada".to_string(),
                input: json!({ "name": " Ada " }),
            },
        ))
        .expect("inline command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "hello Ada"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_parses_inline_arg_aliases() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.renamed".to_string(),
                slash: Some("/manifest-renamed".to_string()),
                raw: "/manifest-renamed README.md".to_string(),
                input: json!({ "path": " README.md " }),
            },
        ))
        .expect("renamed inline command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "README.md"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_parses_inline_arg_nested_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_nested".to_string(),
                slash: Some("/manifest-inline-nested".to_string()),
                raw: "/manifest-inline-nested query_text=cargo".to_string(),
                input: json!({
                    "payload": {},
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline nested command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "README.md:cargo"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_parses_inline_arg_flatten_shape() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_flatten".to_string(),
                slash: Some("/manifest-inline-flatten".to_string()),
                raw: "/manifest-inline-flatten query_text=cargo".to_string(),
                input: json!({
                    "query_text": " cargo "
                }),
            },
        ))
        .expect("inline flatten command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "README.md:cargo"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_applies_inline_arg_default_expr() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.default".to_string(),
                slash: Some("/manifest-default".to_string()),
                raw: "/manifest-default".to_string(),
                input: json!({}),
            },
        ))
        .expect("default inline command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "3"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_rejects_values_outside_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_choice".to_string(),
                slash: Some("/manifest-inline-choice".to_string()),
                raw: "/manifest-inline-choice npm".to_string(),
                input: json!({ "tool": "npm" }),
            },
        ))
        .expect_err("inline choice command should reject unsupported values");

    assert!(
        error
            .to_string()
            .contains(r#"field `tool` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_format() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_format".to_string(),
                slash: Some("/manifest-inline-format".to_string()),
                raw: "/manifest-inline-format not-a-uri".to_string(),
                input: json!({ "endpoint": "not a uri" }),
            },
        ))
        .expect_err("inline format command should reject invalid values");

    assert!(
        error
            .to_string()
            .contains(r#"field `endpoint` must match format `uri`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_exclusive_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_exclusive_number".to_string(),
                slash: Some("/manifest-inline-exclusive-number".to_string()),
                raw: "/manifest-inline-exclusive-number 2".to_string(),
                input: json!({ "count": 2 }),
            },
        ))
        .expect_err("inline strict numeric bounds should reject equal lower values");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `count` must be greater than 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_exclusive_number".to_string(),
                slash: Some("/manifest-inline-exclusive-number".to_string()),
                raw: "/manifest-inline-exclusive-number 5".to_string(),
                input: json!({ "count": 5 }),
            },
        ))
        .expect_err("inline strict numeric bounds should reject equal upper values");
    assert!(
        max_error
            .to_string()
            .contains(r#"field `count` must be less than 5"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_pattern() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_pattern".to_string(),
                slash: Some("/manifest-inline-pattern".to_string()),
                raw: "/manifest-inline-pattern Cargo".to_string(),
                input: json!({ "slug": "Cargo" }),
            },
        ))
        .expect_err("inline pattern command should reject unsupported values");

    assert!(
        error
            .to_string()
            .contains(r#"field `slug` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_below_min_chars() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_pattern".to_string(),
                slash: Some("/manifest-inline-pattern".to_string()),
                raw: "/manifest-inline-pattern go".to_string(),
                input: json!({ "slug": "go" }),
            },
        ))
        .expect_err("inline pattern command should reject short values");

    assert!(
        error
            .to_string()
            .contains(r#"field `slug` must be at least 3 characters"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_object_property_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_object".to_string(),
                slash: Some("/manifest-inline-object".to_string()),
                raw: "/manifest-inline-object {}".to_string(),
                input: json!({ "labels": {} }),
            },
        ))
        .expect_err("inline object command should reject empty objects");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `labels` requires at least 1 property"#)
    );

    let max_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_object".to_string(),
                slash: Some("/manifest-inline-object".to_string()),
                raw: "/manifest-inline-object a=1 b=2 c=3".to_string(),
                input: json!({
                    "labels": { "a": "1", "b": "2", "c": "3" }
                }),
            },
        ))
        .expect_err("inline object command should reject oversized objects");
    assert!(
        max_error
            .to_string()
            .contains(r#"field `labels` accepts at most 2 properties"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_format() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_format".to_string(),
                slash: Some("/manifest-inline-item-format".to_string()),
                raw: "/manifest-inline-item-format not-a-uuid".to_string(),
                input: json!({ "ids": ["not-a-uuid"] }),
            },
        ))
        .expect_err("inline item format command should reject invalid values");

    assert!(
        error
            .to_string()
            .contains(r#"field `ids[]` must match format `uuid`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_pattern".to_string(),
                slash: Some("/manifest-inline-item-pattern".to_string()),
                raw: "/manifest-inline-item-pattern go".to_string(),
                input: json!({ "tags": ["go"] }),
            },
        ))
        .expect_err("inline item constraints should reject short values");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `tags[]` must be at least 3 characters"#)
    );

    let pattern_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_pattern".to_string(),
                slash: Some("/manifest-inline-item-pattern".to_string()),
                raw: "/manifest-inline-item-pattern Cargo".to_string(),
                input: json!({ "tags": ["Cargo"] }),
            },
        ))
        .expect_err("inline item constraints should reject invalid patterns");
    assert!(
        pattern_error
            .to_string()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_choice".to_string(),
                slash: Some("/manifest-inline-item-choice".to_string()),
                raw: "/manifest-inline-item-choice npm".to_string(),
                input: json!({ "tools": ["npm"] }),
            },
        ))
        .expect_err("inline item choices should reject unsupported values");

    assert!(
        error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn command_macro_dispatch_normalizes_inline_item_values() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_normalize".to_string(),
                slash: Some("/manifest-inline-item-normalize".to_string()),
                raw: "/manifest-inline-item-normalize cargo.rs git.rs".to_string(),
                input: json!({ "tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("inline item normalization command should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "cargo,git"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_rejects_empty_normalized_inline_item_values() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_normalize".to_string(),
                slash: Some("/manifest-inline-item-normalize".to_string()),
                raw: "/manifest-inline-item-normalize .rs".to_string(),
                input: json!({ "tags": [" .rs "] }),
            },
        ))
        .expect_err("inline item normalization command should reject empty normalized items");

    assert!(
        error
            .to_string()
            .contains(r#"field `tags[]` must not be empty"#)
    );
}

#[test]
fn command_macro_dispatch_handles_item_non_empty_if_present() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let missing_output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_non_empty_if_present".to_string(),
                slash: Some("/manifest-inline-item-non-empty-if-present".to_string()),
                raw: "/manifest-inline-item-non-empty-if-present".to_string(),
                input: json!({}),
            },
        ))
        .expect("inline optional item command should allow missing values");

    match missing_output {
        PluginCommandOutput::Message { text } => assert_eq!(text, ""),
        other => panic!("expected message output, got {other:?}"),
    }

    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_non_empty_if_present".to_string(),
                slash: Some("/manifest-inline-item-non-empty-if-present".to_string()),
                raw: "/manifest-inline-item-non-empty-if-present \"\"".to_string(),
                input: json!({ "tags": [""] }),
            },
        ))
        .expect_err("inline optional item command should reject present empty items");

    assert!(
        error
            .to_string()
            .contains(r#"field `tags[]` must not be empty when present"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_string_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_pattern".to_string(),
                slash: Some("/manifest-inline-auto-item-pattern".to_string()),
                raw: "/manifest-inline-auto-item-pattern cargo.rs".to_string(),
                input: json!({ "tags": [" cargo.rs "] }),
            },
        ))
        .expect("inline direct array string constraints should normalize items");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "cargo"),
        other => panic!("expected message output, got {other:?}"),
    }

    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_pattern".to_string(),
                slash: Some("/manifest-inline-auto-item-pattern".to_string()),
                raw: "/manifest-inline-auto-item-pattern Cargo.rs".to_string(),
                input: json!({ "tags": [" Cargo.rs "] }),
            },
        ))
        .expect_err("inline direct array string constraints should validate items");

    assert!(
        error
            .to_string()
            .contains(r#"field `tags[]` must match pattern `^[a-z0-9-]+$`"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_numeric_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_number".to_string(),
                slash: Some("/manifest-inline-auto-item-number".to_string()),
                raw: "/manifest-inline-auto-item-number 2 4".to_string(),
                input: json!({ "counts": [2, 4] }),
            },
        ))
        .expect("inline direct array numeric constraints should accept matching items");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "2"),
        other => panic!("expected message output, got {other:?}"),
    }

    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_number".to_string(),
                slash: Some("/manifest-inline-auto-item-number".to_string()),
                raw: "/manifest-inline-auto-item-number 1".to_string(),
                input: json!({ "counts": [1] }),
            },
        ))
        .expect_err("inline direct array numeric constraints should validate items");

    assert!(
        error
            .to_string()
            .contains(r#"field `counts[]` must be at least 2"#)
    );
}

#[test]
fn command_macro_dispatch_applies_direct_array_choices() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_choice".to_string(),
                slash: Some("/manifest-inline-auto-item-choice".to_string()),
                raw: "/manifest-inline-auto-item-choice cargo".to_string(),
                input: json!({ "tools": ["cargo"] }),
            },
        ))
        .expect("inline direct array choices should accept matching items");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "cargo"),
        other => panic!("expected message output, got {other:?}"),
    }

    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_auto_item_choice".to_string(),
                slash: Some("/manifest-inline-auto-item-choice".to_string()),
                raw: "/manifest-inline-auto-item-choice npm".to_string(),
                input: json!({ "tools": ["npm"] }),
            },
        ))
        .expect_err("inline direct array choices should validate items");

    assert!(
        error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#)
    );
}

#[test]
fn tool_macro_dispatch_handles_enum_variant_local_normalization() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let query_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "query": " cargo " }),
            },
        ))
        .expect("typed tool input should normalize variant-local string fields");
    assert_eq!(query_output.output_text, "query:cargo");

    let tags_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("typed tool input should normalize variant-local array items");
    assert_eq!(tags_output.output_text, "tags:cargo,git");

    let auto_tags_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_normalize".to_string(),
                input: json!({ "auto_tags": [" cargo.rs ", " git.rs "] }),
            },
        ))
        .expect("typed tool input should auto-target direct array variant constraints");
    assert_eq!(auto_tags_output.output_text, "auto_tags:cargo,git");
}

#[test]
fn command_macro_dispatch_handles_enum_variant_local_normalization() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let query_output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize cargo".to_string(),
                input: json!({ "query": " cargo " }),
            },
        ))
        .expect("typed command should normalize variant-local string fields");
    match query_output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "query:cargo"),
        other => panic!("expected message output, got {other:?}"),
    }

    let tags_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize .rs".to_string(),
                input: json!({ "tags": [" .rs "] }),
            },
        ))
        .expect_err("typed command should reject empty normalized array items");
    assert!(
        tags_error
            .to_string()
            .contains(r#"field `tags[]` must not be empty"#)
    );

    let renamed_tools_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_normalize".to_string(),
                slash: Some("/manifest-variant-normalize".to_string()),
                raw: "/manifest-variant-normalize npm".to_string(),
                input: json!({
                    "action": "renamed_tools",
                    "tools": ["npm"]
                }),
            },
        ))
        .expect_err("typed command should validate direct array variant choices");
    assert!(
        renamed_tools_error
            .to_string()
            .contains(r#"field `tools[]` must be one of ["cargo","git"]"#),
        "unexpected renamed tools command error: {renamed_tools_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_renamed_fields() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_renamed_fields".to_string(),
                input: json!({
                    "action": "query",
                    "filePath": " Cargo.toml "
                }),
            },
        ))
        .expect("typed tool input should normalize renamed variant fields");
    assert_eq!(tool_output.output_text, "query:Cargo.toml");

    let command_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_renamed_fields".to_string(),
                slash: Some("/manifest-variant-renamed-fields".to_string()),
                raw: "/manifest-variant-renamed-fields filePath=Cargo.toml".to_string(),
                input: json!({
                    "action": "run",
                    "filePath": "Cargo.toml"
                }),
            },
        ))
        .expect_err("typed command should validate renamed variant relations");
    assert!(
        command_error
            .to_string()
            .contains(r#"field `filePath` requires `mode`"#),
        "unexpected renamed variant command error: {command_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_field_args() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_field_args".to_string(),
                input: json!({
                    "path": " Cargo.toml "
                }),
            },
        ))
        .expect("typed tool input should normalize aliased variant field args");
    assert_eq!(tool_output.output_text, "query:Cargo.toml");

    let command_output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_field_args".to_string(),
                slash: Some("/manifest-variant-field-args".to_string()),
                raw: "/manifest-variant-field-args path=Cargo.toml".to_string(),
                input: json!({
                    "action": "run",
                    "path": "Cargo.toml"
                }),
            },
        ))
        .expect("typed command should apply alias normalization and defaults for variant fields");
    match command_output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "run:Cargo.toml:read"),
        other => panic!("expected message output, got {other:?}"),
    }

    let command_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_field_args".to_string(),
                slash: Some("/manifest-variant-field-args".to_string()),
                raw: "/manifest-variant-field-args tags=cargo tags=cargo".to_string(),
                input: json!({
                    "action": "tags",
                    "tags": [" cargo ", "cargo"]
                }),
            },
        ))
        .expect_err("typed command should validate renamed array field args");
    assert!(
        command_error
            .to_string()
            .contains(r#"field `tagValues[]` must not contain duplicate values"#),
        "unexpected variant field arg command error: {command_error}"
    );
}

#[test]
fn tool_and_command_dispatch_handle_enum_variant_inference() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let tool_output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "variant_inference".to_string(),
                input: json!({
                    "filePath": "marker",
                    "queryText": " cargo "
                }),
            },
        ))
        .expect("typed tool input should infer variants through renamed fields");
    assert_eq!(tool_output.output_text, "query::cargo");

    let command_output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.variant_inference".to_string(),
                slash: Some("/manifest-variant-inference".to_string()),
                raw: "/manifest-variant-inference filePath=marker queryText=cargo".to_string(),
                input: json!({
                    "filePath": "marker",
                    "queryText": " cargo "
                }),
            },
        ))
        .expect("typed command should infer variants through renamed fields");
    match command_output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "query::cargo"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_number".to_string(),
                slash: Some("/manifest-inline-item-number".to_string()),
                raw: "/manifest-inline-item-number 1".to_string(),
                input: json!({ "counts": [1] }),
            },
        ))
        .expect_err("inline item numeric bounds should reject low values");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `counts[]` must be at least 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_number".to_string(),
                slash: Some("/manifest-inline-item-number".to_string()),
                raw: "/manifest-inline-item-number 5".to_string(),
                input: json!({ "counts": [5] }),
            },
        ))
        .expect_err("inline item numeric bounds should reject high values");
    assert!(
        max_error
            .to_string()
            .contains(r#"field `counts[]` must be at most 4"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_exclusive_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_exclusive_number".to_string(),
                slash: Some("/manifest-inline-item-exclusive-number".to_string()),
                raw: "/manifest-inline-item-exclusive-number 2".to_string(),
                input: json!({ "counts": [2] }),
            },
        ))
        .expect_err("inline item strict numeric bounds should reject equal lower values");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `counts[]` must be greater than 2"#)
    );

    let max_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_exclusive_number".to_string(),
                slash: Some("/manifest-inline-item-exclusive-number".to_string()),
                raw: "/manifest-inline-item-exclusive-number 5".to_string(),
                input: json!({ "counts": [5] }),
            },
        ))
        .expect_err("inline item strict numeric bounds should reject equal upper values");
    assert!(
        max_error
            .to_string()
            .contains(r#"field `counts[]` must be less than 5"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_item_object_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let min_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_object".to_string(),
                slash: Some("/manifest-inline-item-object".to_string()),
                raw: "/manifest-inline-item-object [{}]".to_string(),
                input: json!({ "entries": [{}] }),
            },
        ))
        .expect_err("inline item object bounds should reject empty objects");
    assert!(
        min_error
            .to_string()
            .contains(r#"field `entries[]` requires at least 1 property"#)
    );

    let max_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_item_object".to_string(),
                slash: Some("/manifest-inline-item-object".to_string()),
                raw: "/manifest-inline-item-object [{\"a\":\"1\",\"b\":\"2\",\"c\":\"3\"}]"
                    .to_string(),
                input: json!({
                    "entries": [{ "a": "1", "b": "2", "c": "3" }]
                }),
            },
        ))
        .expect_err("inline item object bounds should reject oversized objects");
    assert!(
        max_error
            .to_string()
            .contains(r#"field `entries[]` accepts at most 2 properties"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_inline_relation_rules() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let requires_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation path=README.md".to_string(),
                input: json!({
                    "path": "README.md",
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce requires");
    assert!(
        requires_error
            .to_string()
            .contains(r#"field `path` requires `mode`"#)
    );

    let conflicts_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation slug=docs mode=read".to_string(),
                input: json!({
                    "mode": "read",
                    "slug": "docs",
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce conflicts_with");
    assert!(
        conflicts_error
            .to_string()
            .contains(r#"field `slug` conflicts with `mode`"#)
    );

    let required_unless_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation".to_string(),
                input: json!({
                    "file_path": "src/lib.rs",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce required_unless_present");
    assert!(
        required_unless_error
            .to_string()
            .contains(r#"field `fallback` is required unless `mode` is present"#)
    );

    let forbid_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation file_path=../etc".to_string(),
                input: json!({
                    "fallback": "default",
                    "file_path": "../etc/passwd",
                    "tags": ["cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce forbid_substrings");
    assert!(
        forbid_error
            .to_string()
            .contains(r#"field `file_path` must not contain `..`"#)
    );

    let distinct_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_relation".to_string(),
                slash: Some("/manifest-inline-relation".to_string()),
                raw: "/manifest-inline-relation tags=cargo".to_string(),
                input: json!({
                    "fallback": "default",
                    "file_path": "src/lib.rs",
                    "tags": [" cargo ", "cargo"]
                }),
            },
        ))
        .expect_err("inline relation command should enforce distinct_trimmed");
    assert!(
        distinct_error
            .to_string()
            .contains(r#"field `tags[]` must not contain duplicate values"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_inline_group_rules() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;

    let exactly_one_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group".to_string(),
                input: json!({
                    "filePath": "README.md",
                    "stdinPayload": "payload",
                    "text": "hello"
                }),
            },
        ))
        .expect_err("inline group command should enforce exactly_one_of");
    assert!(
        exactly_one_error
            .to_string()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#)
    );

    let exactly_one_missing_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group".to_string(),
                input: json!({
                    "text": "hello"
                }),
            },
        ))
        .expect_err("inline group command should reject missing exactly_one_of group");
    assert!(
        exactly_one_missing_error
            .to_string()
            .contains(r#"exactly one of `filePath` or `stdinPayload` is required"#)
    );

    let at_least_one_error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_group".to_string(),
                slash: Some("/manifest-inline-group".to_string()),
                raw: "/manifest-inline-group filePath=README.md".to_string(),
                input: json!({
                    "filePath": "README.md"
                }),
            },
        ))
        .expect_err("inline group command should enforce at_least_one_of");
    assert!(
        at_least_one_error
            .to_string()
            .contains(r#"at least one of `text` or `stdinPayload` is required"#)
    );
}

#[test]
fn command_macro_dispatch_rejects_values_outside_numeric_bounds() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let error = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.inline_number".to_string(),
                slash: Some("/manifest-inline-number".to_string()),
                raw: "/manifest-inline-number 1".to_string(),
                input: json!({ "count": 1 }),
            },
        ))
        .expect_err("inline numeric command should reject out-of-range values");

    assert!(
        error
            .to_string()
            .contains(r#"field `count` must be at least 2"#)
    );
}

#[test]
fn command_macro_dispatch_parses_top_level_primitive_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.bool".to_string(),
                slash: Some("/manifest-bool".to_string()),
                raw: "/manifest-bool true".to_string(),
                input: json!(true),
            },
        ))
        .expect("primitive command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => assert_eq!(text, "true"),
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn command_macro_dispatch_supports_typed_input_with_command_context() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.context".to_string(),
                slash: Some("/manifest-context".to_string()),
                raw: "/manifest-context Ada".to_string(),
                input: json!({ "name": "Ada" }),
            },
        ))
        .expect("context command invoke should succeed");

    match output {
        PluginCommandOutput::Message { text } => {
            assert_eq!(text, "Ada via /manifest-context")
        }
        other => panic!("expected message output, got {other:?}"),
    }
}

#[test]
fn tool_macro_dispatch_parses_top_level_primitive_input() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                session_id: 1,
                call_id: 2,
                workspace_root: "/workspace".to_string(),
                tool_name: "plain_string".to_string(),
                input: json!("Ada"),
            },
        ))
        .expect("plain string tool invoke should succeed");
    assert_eq!(output.output_text, "Ada");
}

#[test]
fn tool_command_macro_dispatch_routes_to_tool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let output = runtime
        .block_on(Plugin::command_invoke(
            &plugin,
            PluginCommandInvokeInput {
                session_id: Some(1),
                call_id: Some(2),
                workspace_root: Some("/workspace".to_string()),
                command_id: "manifest.render".to_string(),
                slash: Some("/manifest-render".to_string()),
                raw: "/manifest-render hi".to_string(),
                input: json!({ "text": " hi " }),
            },
        ))
        .expect("tool command invoke should succeed");

    match output {
        PluginCommandOutput::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => {
            assert_eq!(tool, "render");
            assert_eq!(input, Some(json!({ "text": " hi " })));
            assert!(submit_output_as_prompt);
        }
        other => panic!("expected tool invocation output, got {other:?}"),
    }
}

#[test]
fn hook_macro_allows_multiple_handlers_ordered_by_priority() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let fallback = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("render", json!({ "text": "normal" })),
        ))
        .expect("hook dispatch should succeed")
        .expect("fallback hook should return a patch");
    let high = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("render", json!({ "text": "priority" })),
        ))
        .expect("hook dispatch should succeed")
        .expect("high-priority hook should return a patch");

    assert_eq!(fallback.title_override.as_deref(), Some("fallback"));
    assert_eq!(high.title_override.as_deref(), Some("high"));
}

#[test]
fn hook_macro_filters_by_tool_and_command() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime should build");
    let plugin = ManifestPlugin;
    let doc = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("doc_render", json!({})),
        ))
        .expect("tool hook dispatch should succeed")
        .expect("doc hook should match");
    let dynamic = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input("dynamic", json!({})),
        ))
        .expect("tool hook dispatch should succeed");
    let tagged = runtime
        .block_on(Plugin::tool_execute_before(
            &plugin,
            tool_before_input_with_tags("dynamic", vec![ToolTag::FilesystemWrite], json!({})),
        ))
        .expect("tagged tool hook dispatch should succeed")
        .expect("tagged hook should match");
    let cargo = runtime
        .block_on(Plugin::command_execute_before(
            &plugin,
            command_before_input("cargo"),
        ))
        .expect("command hook dispatch should succeed");
    let git = runtime
        .block_on(Plugin::command_execute_before(
            &plugin,
            command_before_input("git"),
        ))
        .expect("command hook dispatch should succeed");

    assert_eq!(doc.title_override.as_deref(), Some("doc"));
    assert!(
        dynamic.is_none(),
        "unmatched tool filters should skip handlers"
    );
    assert_eq!(tagged.title_override.as_deref(), Some("write"));
    match cargo {
        Some(CommandBeforeResponse::Patch(patch)) => {
            assert_eq!(patch.args, Some(vec!["check".to_string()]));
        }
        other => panic!("expected cargo command patch, got {other:?}"),
    }
    assert!(
        git.is_none(),
        "unmatched command filters should skip handlers"
    );
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
