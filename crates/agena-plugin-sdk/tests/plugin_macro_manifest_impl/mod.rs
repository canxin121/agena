use super::{MANIFEST_SETTINGS_METADATA, ManifestEchoEndpoint, ManifestSettings};

#[derive(Default)]
pub(super) struct ManifestPlugin;

#[agena_plugin(
    namespace = "test",
    name = "manifest",
    version = "0.0.0",
    summary = "Manifest macro behavior test plugin.",
    settings = ManifestSettings,
    settings_default = default,
    settings_metadata = MANIFEST_SETTINGS_METADATA,
    imports(PluginServiceImport::optional("test.telemetry", 1))
)]
impl ManifestPlugin {
    #[service(ManifestEchoEndpoint)]
    fn echo_service(&self, input: &ManifestInput) -> Result<ManifestOutput> {
        Ok(ManifestOutput {
            rendered: format!("service:{}", input.text),
        })
    }

    #[service("test.echo", version = 1, method = "status")]
    async fn echo_status(&self) -> ManifestOutput {
        ManifestOutput {
            rendered: "ready".to_string(),
        }
    }

    #[tool(
        summary = "Render text.",
        stream = render_stream,
        operation(
            "/manifest-render",
            id = "manifest.render",
            title = "Manifest Render",
            aliases("render-manifest"),
            usage = "/manifest-render {\"text\":\"hello\"}",
        ),
        concurrency_safe,
        tags(read_only)
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

    /// Render docs summary.
    ///
    /// Render docs help.
    #[tool(operation("/doc-render"), tags(read_only))]
    fn doc_render(&self) -> String {
        "doc".to_string()
    }

    #[tool(summary = "Dynamic output.", tags(read_only))]
    fn dynamic(&self) -> ToolInvokeOutput {
        ToolInvokeOutput::text("dynamic")
    }

    #[tool(summary = "Explicit output.", output(ManifestOutput), tags(read_only))]
    fn explicit(&self) -> ManifestOutput {
        ManifestOutput {
            rendered: "explicit".to_string(),
        }
    }

    #[tool(
        summary = "Semantic permissions.",
        operation("/semantic"),
        tags(mutate)
    )]
    fn semantic(&self, input: &SemanticInput) -> String {
        format!("{} -> {}", input.path, input.endpoint)
    }

    #[tool(
        summary = "Inline semantic permissions.",
        operation("/inline-semantic"),
        tags(read_only)
    )]
    fn inline_semantic(
        &self,
        #[arg(example = "README.md", description = "Path to inspect.")] path: String,
        #[arg(example = "localhost")] host: String,
    ) -> String {
        format!("{path} @ {host}")
    }

    #[tool(
        summary = "Inline auto usage.",
        operation("/inline-auto"),
        tags(read_only)
    )]
    fn inline_auto(&self, path: String, count: usize) -> String {
        format!("{path}:{count}")
    }

    #[tool(
        summary = "Inline count usage.",
        operation("/inline-count"),
        tags(read_only)
    )]
    fn inline_count(&self, #[arg(example = 3)] count: usize) -> String {
        count.to_string()
    }

    #[tool(
        summary = "Inline rename support.",
        operation("/inline-rename"),
        tags(read_only)
    )]
    fn inline_rename(
        &self,
        #[arg(name = "filePath", alias = "path", trim)] file_path: String,
    ) -> String {
        file_path
    }

    #[tool(
        summary = "Inline default support.",
        operation("/inline-default"),
        tags(read_only)
    )]
    fn inline_default(&self, #[arg(default = 3)] count: usize) -> String {
        count.to_string()
    }

    #[tool(
        summary = "Inline nested ToolInput support.",
        operation("/inline-nested"),
        tags(read_only)
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
        operation("/inline-flatten"),
        tags(read_only)
    )]
    fn inline_flatten(
        &self,
        #[arg(flatten_shape)] payload: InlineFlattenArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[tool(
        summary = "Plain string input.",
        operation("/plain-string"),
        tags(read_only)
    )]
    fn plain_string(&self, text: String) -> String {
        text
    }

    #[tool(summary = "Dynamic permission DSL.", tags(read_only))]
    async fn dynamic_permission(&self, input: &DynamicPermissionInput) -> String {
        format!("{} @ {}", input.path, input.host)
    }

    #[operation(
        "/manifest-greet",
        id = "manifest.greet",
        title = "Manifest Greet",
        description = "Greet from a typed command.",
        category = "Test",
        aliases("hello-manifest"),
        usage = "/manifest-greet {\"name\":\"Ada\"}"
    )]
    fn greet_operation(&self, input: &ManifestCommandInput) -> String {
        format!("hello {}", input.name)
    }

    #[operation(
        "/manifest-inline",
        id = "manifest.inline",
        title = "Manifest Inline",
        description = "Greet from inline command arguments.",
        category = "Test"
    )]
    fn inline_operation(
        &self,
        #[arg(trim, non_empty, example = "Ada", description = "Name to greet.")] name: String,
    ) -> String {
        format!("hello {name}")
    }

    #[operation(
        "/manifest-inline-auto",
        id = "manifest.inline_auto",
        title = "Manifest Inline Auto",
        description = "Greet from inline command arguments without explicit examples.",
        category = "Test"
    )]
    fn inline_auto_operation(&self, #[arg(trim)] name: String) -> String {
        format!("hello {name}")
    }

    #[operation(
        "/manifest-renamed",
        id = "manifest.renamed",
        title = "Manifest Renamed",
        description = "Command arg rename and alias support.",
        category = "Test"
    )]
    fn renamed_operation(
        &self,
        #[arg(name = "filePath", alias = "path", trim)] file_path: String,
    ) -> String {
        file_path
    }

    #[operation(
        "/manifest-default",
        id = "manifest.default",
        title = "Manifest Default",
        description = "Inline command default support.",
        category = "Test"
    )]
    fn default_operation(&self, #[arg(default = 3)] count: usize) -> String {
        count.to_string()
    }

    #[operation(
        "/manifest-inline-nested",
        id = "manifest.inline_nested",
        title = "Manifest Inline Nested",
        description = "Inline command nested ToolInput support.",
        category = "Test"
    )]
    fn inline_nested_operation(
        &self,
        #[arg(alias = "body", nested_shape)] payload: InlineNestedArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[operation(
        "/manifest-inline-flatten",
        id = "manifest.inline_flatten",
        title = "Manifest Inline Flatten",
        description = "Inline command flatten ToolInput support.",
        category = "Test"
    )]
    fn inline_flatten_operation(
        &self,
        #[arg(flatten_shape)] payload: InlineFlattenArgInner,
        #[arg(trim, non_empty)] query_text: String,
    ) -> String {
        format!("{}:{query_text}", payload.file_path)
    }

    #[tool(
        summary = "Path-level choices.",
        operation("/path-choice"),
        tags(read_only)
    )]
    fn path_choice(&self, input: &PathChoiceInput) -> String {
        input.mode.clone()
    }

    #[tool(
        summary = "Field-level choices.",
        operation("/field-choice"),
        tags(read_only)
    )]
    fn field_choice(&self, input: &FieldChoiceInput) -> String {
        input.tool_name.clone()
    }

    #[tool(
        summary = "Path-level format.",
        operation("/path-format"),
        tags(read_only)
    )]
    fn path_format(&self, input: &PathFormatInput) -> String {
        input.endpoint.clone()
    }

    #[tool(
        summary = "Path-level pattern.",
        operation("/path-pattern"),
        tags(read_only)
    )]
    fn path_pattern(&self, input: &PathPatternInput) -> String {
        input.slug.clone()
    }

    #[tool(
        summary = "Path-level numeric.",
        operation("/path-number"),
        tags(read_only)
    )]
    fn path_number(&self, input: &PathNumericInput) -> String {
        input.count.to_string()
    }

    #[tool(
        summary = "Path-level strict numeric bounds.",
        operation("/path-exclusive-number"),
        tags(read_only)
    )]
    fn path_exclusive_number(&self, input: &PathExclusiveNumericInput) -> String {
        input.count.to_string()
    }

    #[tool(
        summary = "Path-level object bounds.",
        operation("/path-object"),
        tags(read_only)
    )]
    fn path_object(&self, input: &PathObjectInput) -> String {
        input.labels.len().to_string()
    }

    #[tool(
        summary = "Path-level item constraints.",
        operation("/path-item-pattern"),
        tags(read_only)
    )]
    fn path_item_pattern(&self, input: &PathItemPatternInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level item choice constraints.",
        operation("/path-item-choice"),
        tags(read_only)
    )]
    fn path_item_choice(&self, input: &PathItemChoiceInput) -> String {
        input.tools.join(",")
    }

    #[tool(
        summary = "Path-level item format constraints.",
        operation("/path-item-format"),
        tags(read_only)
    )]
    fn path_item_format(&self, input: &PathItemFormatInput) -> String {
        input.ids.join(",")
    }

    #[tool(
        summary = "Path-level item numeric bounds.",
        operation("/path-item-number"),
        tags(read_only)
    )]
    fn path_item_number(&self, input: &PathItemNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level item strict numeric bounds.",
        operation("/path-item-exclusive-number"),
        tags(read_only)
    )]
    fn path_item_exclusive_number(&self, input: &PathItemExclusiveNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level item object bounds.",
        operation("/path-item-object"),
        tags(read_only)
    )]
    fn path_item_object(&self, input: &PathItemObjectInput) -> String {
        input.entries.len().to_string()
    }

    #[tool(
        summary = "Path-level item normalization.",
        operation("/path-item-normalize"),
        tags(read_only)
    )]
    fn path_item_normalize(&self, input: &PathItemNormalizeInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level optional item non-empty.",
        operation("/path-item-optional-non-empty"),
        tags(read_only)
    )]
    fn path_item_optional_non_empty(&self, input: &PathOptionalItemNonEmptyInput) -> String {
        input.tags.clone().unwrap_or_default().join(",")
    }

    #[tool(
        summary = "Path-level auto item string constraints.",
        operation("/path-auto-item-string"),
        tags(read_only)
    )]
    fn path_auto_item_string(&self, input: &PathAutoItemStringInput) -> String {
        input.tags.join(",")
    }

    #[tool(
        summary = "Path-level auto item numeric constraints.",
        operation("/path-auto-item-number"),
        tags(read_only)
    )]
    fn path_auto_item_number(&self, input: &PathAutoItemNumericInput) -> String {
        input.counts.len().to_string()
    }

    #[tool(
        summary = "Path-level auto item choice constraints.",
        operation("/path-auto-item-choice"),
        tags(read_only)
    )]
    fn path_auto_item_choice(&self, input: &PathAutoItemChoiceInput) -> String {
        input.tools.join(",")
    }

    #[tool(
        summary = "Path-level field relation metadata.",
        operation("/path-relation"),
        tags(read_only)
    )]
    fn path_relation(&self, input: &PathRelationInput) -> String {
        input.path.clone().unwrap_or_default()
    }

    #[tool(
        summary = "Path-level field group metadata.",
        operation("/path-group"),
        tags(read_only)
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
        operation("/renamed-format"),
        tags(read_only)
    )]
    fn renamed_format(&self, input: &RenamedFormatInput) -> String {
        input.endpoint_value.clone()
    }

    #[tool(
        summary = "Renamed field constraint metadata.",
        operation("/renamed-pattern"),
        tags(read_only)
    )]
    fn renamed_pattern(&self, input: &RenamedPatternInput) -> String {
        input.slug_value.clone()
    }

    #[tool(
        summary = "Renamed numeric constraint metadata.",
        operation("/renamed-number"),
        tags(read_only)
    )]
    fn renamed_number(&self, input: &RenamedNumericInput) -> String {
        input.count_value.to_string()
    }

    #[tool(
        summary = "Renamed strict numeric metadata.",
        operation("/renamed-exclusive-number"),
        tags(read_only)
    )]
    fn renamed_exclusive_number(&self, input: &RenamedExclusiveNumericInput) -> String {
        input.count_value.to_string()
    }

    #[tool(
        summary = "Renamed object property bounds metadata.",
        operation("/renamed-object"),
        tags(read_only)
    )]
    fn renamed_object(&self, input: &RenamedObjectInput) -> String {
        input.metadata_value.len().to_string()
    }

    #[tool(
        summary = "Renamed item format metadata.",
        operation("/renamed-item-format"),
        tags(read_only)
    )]
    fn renamed_item_format(&self, input: &RenamedItemFormatInput) -> String {
        input.id_values.join(",")
    }

    #[tool(
        summary = "Renamed item constraint metadata.",
        operation("/renamed-item-pattern"),
        tags(read_only)
    )]
    fn renamed_item_pattern(&self, input: &RenamedItemPatternInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed item choice metadata.",
        operation("/renamed-item-choice"),
        tags(read_only)
    )]
    fn renamed_item_choice(&self, input: &RenamedItemChoiceInput) -> String {
        input.tool_values.join(",")
    }

    #[tool(
        summary = "Renamed item numeric bounds metadata.",
        operation("/renamed-item-number"),
        tags(read_only)
    )]
    fn renamed_item_number(&self, input: &RenamedItemNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item strict numeric bounds metadata.",
        operation("/renamed-item-exclusive-number"),
        tags(read_only)
    )]
    fn renamed_item_exclusive_number(&self, input: &RenamedItemExclusiveNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item object bounds metadata.",
        operation("/renamed-item-object"),
        tags(read_only)
    )]
    fn renamed_item_object(&self, input: &RenamedItemObjectInput) -> String {
        input.entry_values.len().to_string()
    }

    #[tool(
        summary = "Renamed item normalization metadata.",
        operation("/renamed-item-normalize"),
        tags(read_only)
    )]
    fn renamed_item_normalize(&self, input: &RenamedItemNormalizeInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed optional item non-empty metadata.",
        operation("/renamed-item-optional-non-empty"),
        tags(read_only)
    )]
    fn renamed_item_optional_non_empty(&self, input: &RenamedOptionalItemNonEmptyInput) -> String {
        input.tag_values.clone().unwrap_or_default().join(",")
    }

    #[tool(
        summary = "Renamed auto item string metadata.",
        operation("/renamed-auto-item-string"),
        tags(read_only)
    )]
    fn renamed_auto_item_string(&self, input: &RenamedAutoItemStringInput) -> String {
        input.tag_values.join(",")
    }

    #[tool(
        summary = "Renamed auto item numeric metadata.",
        operation("/renamed-auto-item-number"),
        tags(read_only)
    )]
    fn renamed_auto_item_number(&self, input: &RenamedAutoItemNumericInput) -> String {
        input.count_values.len().to_string()
    }

    #[tool(
        summary = "Renamed auto item choice metadata.",
        operation("/renamed-auto-item-choice"),
        tags(read_only)
    )]
    fn renamed_auto_item_choice(&self, input: &RenamedAutoItemChoiceInput) -> String {
        input.tool_values.join(",")
    }

    #[tool(
        summary = "Variant-local enum normalization.",
        operation("/variant-normalize"),
        tags(read_only)
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

    #[operation(
        "/manifest-variant-normalize",
        id = "manifest.variant_normalize",
        title = "Manifest Variant Normalize",
        description = "Typed command enum variant normalization support.",
        category = "Test"
    )]
    fn variant_normalize_operation(&self, input: &VariantNormalizeInput) -> String {
        self.variant_normalize(input)
    }

    #[tool(
        summary = "Variant renamed field enum input.",
        operation("/variant-renamed-fields"),
        tags(read_only)
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

    #[operation(
        "/manifest-variant-renamed-fields",
        id = "manifest.variant_renamed_fields",
        title = "Manifest Variant Renamed Fields",
        description = "Typed command enum renamed field support.",
        category = "Test"
    )]
    fn variant_renamed_fields_operation(&self, input: &VariantRenamedFieldInput) -> String {
        self.variant_renamed_fields(input)
    }

    #[tool(
        summary = "Variant field arg enum input.",
        operation("/variant-field-args"),
        tags(read_only)
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

    #[operation(
        "/manifest-variant-field-args",
        id = "manifest.variant_field_args",
        title = "Manifest Variant Field Args",
        description = "Typed command enum variant field arg support.",
        category = "Test"
    )]
    fn variant_field_args_operation(&self, input: &VariantFieldArgInput) -> String {
        self.variant_field_args(input)
    }

    #[tool(
        summary = "Variant inference enum input.",
        operation("/variant-inference"),
        tags(read_only)
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

    #[operation(
        "/manifest-variant-inference",
        id = "manifest.variant_inference",
        title = "Manifest Variant Inference",
        description = "Typed command enum inference support.",
        category = "Test"
    )]
    fn variant_inference_operation(&self, input: &VariantInferenceInput) -> String {
        self.variant_inference(input)
    }

    #[tool(
        summary = "Variant declarative enum permissions.",
        operation("/variant-semantic"),
        tags(read_only)
    )]
    fn variant_semantic(&self, input: &VariantSemanticInput) -> String {
        match input {
            VariantSemanticInput::File { file_path } => format!("file:{file_path}"),
            VariantSemanticInput::Remote { endpoint } => format!("remote:{endpoint}"),
        }
    }

    #[tool(
        summary = "Enum flatten semantic permissions.",
        operation("/variant-flatten-semantic"),
        tags(read_only)
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
        forbid_substrings("tags", "..", "~"),
        distinct_trimmed("tags"),
        operation("/inline-item-value-relations"),
        tags(read_only)
    )]
    fn inline_item_value_relations(&self, #[arg] tags: Vec<String>) -> String {
        tags.join(",")
    }

    #[operation(
        "/manifest-inline-auto-item-pattern",
        id = "manifest.inline_auto_item_pattern",
        title = "Manifest Inline Auto Item Pattern",
        description = "Inline command direct array string constraints support.",
        category = "Test"
    )]
    fn inline_auto_item_pattern_operation(
        &self,
        #[arg(trim, trim_suffix = ".rs", min_chars = 3, pattern = "^[a-z0-9-]+$")] tags: Vec<
            String,
        >,
    ) -> String {
        tags.join(",")
    }

    #[operation(
        "/manifest-inline-auto-item-number",
        id = "manifest.inline_auto_item_number",
        title = "Manifest Inline Auto Item Number",
        description = "Inline command direct array numeric constraints support.",
        category = "Test"
    )]
    fn inline_auto_item_number_operation(
        &self,
        #[arg(minimum = 2, maximum = 4)] counts: Vec<u32>,
    ) -> String {
        counts.len().to_string()
    }

    #[operation(
        "/manifest-inline-auto-item-choice",
        id = "manifest.inline_auto_item_choice",
        title = "Manifest Inline Auto Item Choice",
        description = "Inline command direct array choices support.",
        category = "Test"
    )]
    fn inline_auto_item_choice_operation(
        &self,
        #[arg(choices = ["cargo", "git"])] tools: Vec<String>,
    ) -> String {
        tools.join(",")
    }

    #[operation(
        "/manifest-inline-relation",
        id = "manifest.inline_relation",
        title = "Manifest Inline Relation",
        description = "Inline command relation and string-list rules support.",
        category = "Test"
    )]
    fn inline_relation_operation(
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
        operation("/renamed-group"),
        tags(read_only)
    )]
    fn renamed_group(&self, input: &RenamedGroupInput) -> String {
        input
            .file_path_value
            .clone()
            .or(input.stdin_payload.clone())
            .unwrap_or_default()
    }

    #[operation(
        "/manifest-inline-group",
        id = "manifest.inline_group",
        title = "Manifest Inline Group",
        description = "Inline command group rules support.",
        category = "Test"
    )]
    fn inline_group_operation(
        &self,
        #[arg(name = "filePath", exactly_one_of = ["stdin_payload"])] file_path: Option<String>,
        #[arg(name = "stdinPayload")] stdin_payload: Option<String>,
        #[arg(name = "text", at_least_one_of = ["stdin_payload"])] text: Option<String>,
    ) -> String {
        text.or(file_path).or(stdin_payload).unwrap_or_default()
    }

    #[tool(
        summary = "Renamed relation metadata.",
        operation("/renamed-relation"),
        tags(read_only)
    )]
    fn renamed_relation(&self, input: &RenamedRelationInput) -> String {
        input.mode_value.clone().unwrap_or_default()
    }

    #[operation(
        "/manifest-inline-item-number",
        id = "manifest.inline_item_number",
        title = "Manifest Inline Item Number",
        description = "Inline command item numeric bounds support.",
        category = "Test"
    )]
    fn inline_item_number_operation(
        &self,
        #[arg(item_minimum = 2, item_maximum = 4)] counts: Vec<u32>,
    ) -> String {
        counts.len().to_string()
    }

    #[operation(
        "/manifest-inline-item-exclusive-number",
        id = "manifest.inline_item_exclusive_number",
        title = "Manifest Inline Item Exclusive Number",
        description = "Inline command item strict numeric bounds support.",
        category = "Test"
    )]
    fn inline_item_exclusive_number_operation(
        &self,
        #[arg(item_exclusive_minimum = 2, item_exclusive_maximum = 5)] counts: Vec<i32>,
    ) -> String {
        counts.len().to_string()
    }

    #[operation(
        "/manifest-inline-item-object",
        id = "manifest.inline_item_object",
        title = "Manifest Inline Item Object",
        description = "Inline command item object property bounds support.",
        category = "Test"
    )]
    fn inline_item_object_operation(
        &self,
        #[arg(item_min_properties = 1, item_max_properties = 2)] entries: Vec<
            std::collections::BTreeMap<String, String>,
        >,
    ) -> String {
        entries.len().to_string()
    }

    #[operation(
        "/manifest-inline-choice",
        id = "manifest.inline_choice",
        title = "Manifest Inline Choice",
        description = "Inline command choices support.",
        category = "Test"
    )]
    fn inline_choice_operation(&self, #[arg(choices = ["cargo", "git"])] tool: String) -> String {
        tool
    }

    #[operation(
        "/manifest-inline-format",
        id = "manifest.inline_format",
        title = "Manifest Inline Format",
        description = "Inline command format support.",
        category = "Test"
    )]
    fn inline_format_operation(&self, #[arg(format = "uri")] endpoint: String) -> String {
        endpoint
    }

    #[operation(
        "/manifest-inline-pattern",
        id = "manifest.inline_pattern",
        title = "Manifest Inline Pattern",
        description = "Inline command pattern support.",
        category = "Test"
    )]
    fn inline_pattern_operation(
        &self,
        #[arg(min_chars = 3, pattern = "^[a-z0-9-]+$")] slug: String,
    ) -> String {
        slug
    }

    #[operation(
        "/manifest-inline-number",
        id = "manifest.inline_number",
        title = "Manifest Inline Number",
        description = "Inline command numeric bounds support.",
        category = "Test"
    )]
    fn inline_number_operation(&self, #[arg(minimum = 2, maximum = 4)] count: u32) -> String {
        count.to_string()
    }

    #[operation(
        "/manifest-inline-exclusive-number",
        id = "manifest.inline_exclusive_number",
        title = "Manifest Inline Exclusive Number",
        description = "Inline command strict numeric bounds support.",
        category = "Test"
    )]
    fn inline_exclusive_number_operation(
        &self,
        #[arg(exclusive_minimum = 2, exclusive_maximum = 5)] count: i32,
    ) -> String {
        count.to_string()
    }

    #[operation(
        "/manifest-inline-object",
        id = "manifest.inline_object",
        title = "Manifest Inline Object",
        description = "Inline command object property bounds support.",
        category = "Test"
    )]
    fn inline_object_operation(
        &self,
        #[arg(min_properties = 1, max_properties = 2)] labels: std::collections::BTreeMap<
            String,
            String,
        >,
    ) -> String {
        labels.len().to_string()
    }

    #[operation(
        "/manifest-inline-item-format",
        id = "manifest.inline_item_format",
        title = "Manifest Inline Item Format",
        description = "Inline command item format support.",
        category = "Test"
    )]
    fn inline_item_format_operation(
        &self,
        #[arg(item_format = "uuid")] ids: Vec<String>,
    ) -> String {
        ids.join(",")
    }

    #[operation(
        "/manifest-inline-item-pattern",
        id = "manifest.inline_item_pattern",
        title = "Manifest Inline Item Pattern",
        description = "Inline command item constraints support.",
        category = "Test"
    )]
    fn inline_item_pattern_operation(
        &self,
        #[arg(item_min_chars = 3, item_pattern = "^[a-z0-9-]+$")] tags: Vec<String>,
    ) -> String {
        tags.join(",")
    }

    #[operation(
        "/manifest-inline-item-choice",
        id = "manifest.inline_item_choice",
        title = "Manifest Inline Item Choice",
        description = "Inline command item choices support.",
        category = "Test"
    )]
    fn inline_item_choice_operation(
        &self,
        #[arg(item_choices = ["cargo", "git"])] tools: Vec<String>,
    ) -> String {
        tools.join(",")
    }

    #[operation(
        "/manifest-inline-item-normalize",
        id = "manifest.inline_item_normalize",
        title = "Manifest Inline Item Normalize",
        description = "Inline command item normalization support.",
        category = "Test"
    )]
    fn inline_item_normalize_operation(
        &self,
        #[arg(item_trim, item_trim_suffix = ".rs", item_non_empty)] tags: Vec<String>,
    ) -> String {
        tags.join(",")
    }

    #[operation(
        "/manifest-inline-item-non-empty-if-present",
        id = "manifest.inline_item_non_empty_if_present",
        title = "Manifest Inline Item Optional",
        description = "Inline command optional item non-empty support.",
        category = "Test"
    )]
    fn inline_item_non_empty_if_present_operation(
        &self,
        #[arg(item_non_empty_if_present)] tags: Option<Vec<String>>,
    ) -> String {
        tags.unwrap_or_default().join(",")
    }

    #[operation(
        "/manifest-bool",
        id = "manifest.bool",
        title = "Manifest Bool",
        description = "Top-level primitive command input.",
        category = "Test"
    )]
    fn bool_operation(&self, enabled: bool) -> String {
        enabled.to_string()
    }

    #[operation(
        "/manifest-context",
        id = "manifest.context",
        title = "Manifest Context",
        description = "Greet with command context.",
        category = "Test"
    )]
    fn context_operation(
        &self,
        input: &ManifestCommandInput,
        context: PluginOperationContext<'_>,
    ) -> String {
        format!(
            "{} via {}",
            input.name,
            context.slash.unwrap_or(context.operation_id)
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

    #[hook(tool.before, plugins("test.manifest"), tags(filesystem), priority = 20)]
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
use super::{
    DynamicPermissionInput, FieldChoiceInput, FlattenVariantSemanticInput, InlineFlattenArgInner,
    InlineNestedArgInner, ManifestCommandInput, ManifestInput, ManifestOutput,
    PathAutoItemChoiceInput, PathAutoItemNumericInput, PathAutoItemStringInput, PathChoiceInput,
    PathExclusiveNumericInput, PathFormatInput, PathGroupInput, PathItemChoiceInput,
    PathItemExclusiveNumericInput, PathItemFormatInput, PathItemNormalizeInput,
    PathItemNumericInput, PathItemObjectInput, PathItemPatternInput, PathNumericInput,
    PathObjectInput, PathOptionalItemNonEmptyInput, PathPatternInput, PathRelationInput,
    RenamedAutoItemChoiceInput, RenamedAutoItemNumericInput, RenamedAutoItemStringInput,
    RenamedExclusiveNumericInput, RenamedFormatInput, RenamedGroupInput, RenamedItemChoiceInput,
    RenamedItemExclusiveNumericInput, RenamedItemFormatInput, RenamedItemNormalizeInput,
    RenamedItemNumericInput, RenamedItemObjectInput, RenamedItemPatternInput, RenamedNumericInput,
    RenamedObjectInput, RenamedOptionalItemNonEmptyInput, RenamedPatternInput,
    RenamedRelationInput, SemanticInput, VariantFieldArgInput, VariantInferenceInput,
    VariantNormalizeInput, VariantRenamedFieldInput, VariantSemanticInput,
};
use agena_plugin_sdk::prelude::*;
