use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct PositionInput {
    line: u32,
    column: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct HoverInput {
    #[arg(trim, non_empty)]
    file: String,
    #[serde(flatten)]
    #[input(flatten_shape)]
    position: PositionInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SearchInput {
    #[input(default_when_empty = true)]
    List {},
    #[input(non_empty("query"))]
    Query { query: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct HoverOutput {
    file: String,
    line: u32,
    column: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct EchoOutput {
    text: String,
}

#[derive(Default)]
struct UiPlugin;

#[agena_plugin(
    namespace = "test",
    name = "ui",
    version = "0.0.0",
    summary = "UI macro test plugin."
)]
impl UiPlugin {
    #[tool(summary = "Resolve a hover position.", read_only, concurrency_safe)]
    async fn hover(&self, input: &HoverInput) -> Result<Vec<HoverOutput>> {
        Ok(vec![HoverOutput {
            file: input.file.clone(),
            line: input.position.line,
            column: input.position.column,
        }])
    }

    #[tool(summary = "Echo text.", read_only, concurrency_safe)]
    fn echo(&self, #[arg(trim, non_empty)] text: String) -> EchoOutput {
        EchoOutput { text }
    }

    #[tool(summary = "Search.", read_only)]
    fn search(&self, _input: SearchInput) -> Vec<String> {
        Vec::new()
    }

    #[tool(summary = "Echo text with context.", read_only, stream = context_echo_stream)]
    fn context_echo(
        &self,
        context: &ToolInvokeContext<'_>,
        #[arg(trim, non_empty)] text: String,
    ) -> String {
        format!("{}:{text}", context.tool_name)
    }

    fn context_echo_stream(
        &self,
        _sink: ToolStreamSink,
        context: &ToolInvokeContext<'_>,
        text: String,
    ) -> String {
        format!("{}:{text}", context.tool_name)
    }
}

fn main() {}
