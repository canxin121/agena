use crate::dispatch::{CallInput, HelpInput, ListInput};
use agena_domain::ToolApiFunction;
use agena_provider::ToolApiDefinition;

/// The closed, runtime-owned protocol registry. Plugins cannot register,
/// replace, disable, or add entries to this registry.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolApiRegistry;

impl ToolApiRegistry {
    pub fn definitions(self) -> Vec<ToolApiDefinition> {
        ToolApiFunction::ALL
            .into_iter()
            .map(|function| {
                let input_schema = match function {
                    ToolApiFunction::List | ToolApiFunction::PluginsList => {
                        serde_json::to_value(schemars::schema_for!(ListInput))
                    }
                    ToolApiFunction::Help => serde_json::to_value(schemars::schema_for!(HelpInput)),
                    ToolApiFunction::Call => serde_json::to_value(schemars::schema_for!(CallInput)),
                }
                .expect("protocol input schema must serialize");
                ToolApiDefinition {
                    name: function.function_name().to_owned(),
                    description: tool_api_description(function).to_owned(),
                    input_schema,
                    output_schema: serde_json::json!({}),
                }
            })
            .collect()
    }
}

fn tool_api_description(function: ToolApiFunction) -> &'static str {
    match function {
        ToolApiFunction::List => {
            "Enumerate the current live execution-tool inventory for one plugin or a batch of plugin targets. The plugin argument accepts one plugin id or an array of ids with OR semantics; batch multi-plugin inventory into one call. Use this whenever the pending request asks which tools or capabilities are available or broad inventory is useful; never answer inventory questions from memory. Each result contains an exact current-session identifier. Use tools_help before the first tools_call when that tool's complete live input contract is not already established. Execution-tool identifiers are not provider function names. Enumeration is paginated: continue with offset, and when the owning plugin is unknown enumerate plugins_list first and then call this function for that plugin id. There is no configured maximum for limit. Call this function directly with well-formed JSON arguments (for example {\"plugin\":[\"agena.fs\",\"agena.monitor\"],\"limit\":200}); never put its name inside tools_call.arguments.tool."
        }
        ToolApiFunction::Help => {
            "Get live input schemas, required fields, examples, and usage notes for exact Agena execution-tool names known from the prompt/context or returned by tools_list. Known names need no preliminary lookup; this help checks current availability. If unavailable, return to discovery. The `tool` argument accepts one execution-tool name or an array of names; batch help for several known tools into one call. Values must name execution tools (for example `fs.read`), never Tool API function names. Use this before the first tools_call unless the complete current contract is already in context from reusable or embedded help. This function describes tools but does not run or authorize them. Arguments must be valid JSON with correct syntax."
        }
        ToolApiFunction::Call => {
            "Run one known Agena execution tool. Never invent the tool name or guess its input schema. The arguments of this function must be a single JSON object with exactly two fields: `tool` (a string naming an execution tool) and `input` (an object holding that tool's arguments, or a JSON string encoding that object). One tools_call keeps one durable execution target; when several calls are independent, emit multiple complete tools_call function calls together in the same assistant response so the runtime can batch them. The outer arguments envelope must remain an object; only its `input` field may be a JSON-encoded string. Do not encode the entire envelope or add an extra wrapper layer. Set `tool` to an exact execution-tool name known from the prompt/context or discovery, with its current availability and complete live contract already in context from tools_help or embedded help (for example `fs.read`); `tool` must be an execution tool, never a Tool API function name such as `tools_call`, `tools_help`, or `tools_list`. Set `input` to one complete argument object or a JSON string encoding that object. A string input is decoded once; the resulting object must exactly match the selected tool's live tools_help contract - correct field names, types, and values; omit nothing required. The whole arguments value must be valid JSON: correct quoting and escapes (no invalid escapes such as `\\|`), no stray control characters, no truncation. If the tool is unknown, return to discovery: enumerate plugins_list, then tools_list for the owning plugin. If validation embeds complete help, read it and retry tools_call directly. The provider function name is always tools_call; all ordinary tools execute through this gateway. When the transport rejects a call, read the correction and re-emit with an exact tool name and valid arguments."
        }
        ToolApiFunction::PluginsList => {
            "Enumerate the current live plugin inventory, which is the entry point for capability discovery: call this function first without a plugin selector to page through every loaded plugin, then use tools_list with a plugin id to enumerate that plugin's tools. The plugin argument accepts one exact or compact plugin id or an array of ids with OR semantics, so several known plugin records can be fetched together. Each result lists the plugin id, version, summary, metadata tags, and the number of tools it publishes. There is no configured maximum for limit. Call this function directly with valid JSON arguments; never route it through tools_call."
        }
    }
}
