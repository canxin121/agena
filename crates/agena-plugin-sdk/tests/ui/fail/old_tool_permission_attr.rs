use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
struct EchoInput {
    text: String,
}

#[derive(Default)]
struct OldPermissionPlugin;

#[agena_plugin(
    namespace = "test",
    name = "old_permission",
    version = "0.0.0",
    summary = "Old permission syntax test plugin."
)]
impl OldPermissionPlugin {
    #[tool(
        summary = "Echo text.",
        read_only,
        permission(paths = echo_permission_paths)
    )]
    fn echo(&self, input: &EchoInput) -> String {
        input.text.clone()
    }

    fn echo_permission_paths(&self, _input: &EchoInput) -> Vec<PathRequest> {
        Vec::new()
    }
}

fn main() {}
