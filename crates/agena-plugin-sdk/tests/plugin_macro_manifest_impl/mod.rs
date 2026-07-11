#[derive(Default)]
pub(super) struct ManifestPlugin;

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
