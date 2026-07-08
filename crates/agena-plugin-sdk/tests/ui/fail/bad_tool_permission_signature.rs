use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct EchoInput {
    text: String,
}

#[derive(Default)]
struct BadPermissionPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_permission",
    version = "0.0.0",
    summary = "Bad permission signature test plugin."
)]
impl BadPermissionPlugin {
    #[tool(
        summary = "Echo text.",
        read_only,
        permission(paths = echo_permission_paths)
    )]
    fn echo(&self, input: &EchoInput) -> String {
        input.text.clone()
    }

    fn echo_permission_paths(&self, _input: EchoInput) -> Vec<PathRequest> {
        Vec::new()
    }
}

fn main() {}
