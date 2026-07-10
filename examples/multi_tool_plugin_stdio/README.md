# `example.notes` Multi-Tool Plugin

This example shows the recommended shape for a plugin that owns multiple model-visible tools:

- Use `#[agena_plugin(...)]` as the single plugin entry point.
- Put one `#[tool(...)]` method in the plugin impl per model-visible tool.
- Derive `ToolInput` on non-trivial input structs and keep field constraints next to fields with `#[arg(...)]`.
- Add `output(OutputType)` on tools that return structured data so the manifest includes an output schema.
- Use `stream = tool_method` in `#[tool(...)]` when a tool has a custom streaming implementation.
- Declare direct path/network permissions on input fields with `#[arg(path.write)]`, or computed permissions on the tool with `path(...)` / `network(...)`.
- Keep plugin configuration in `PluginConfig<T>` and enable it with `#[agena_plugin(..., config)]`.

The macro generates hidden schema metadata, manifest definitions, static dispatch, streaming dispatch, and permission dispatch. New plugins should define tools as `#[tool(...)]` methods inside a single `#[agena_plugin(...)]` impl.

## Build

```bash
cargo build -p agena-multi-tool-plugin-stdio
```

## Configure

```json
{
  "plugins": {
    "list": {
      "example.notes": {
        "package": {
          "kind": "stdio",
          "command": "target/debug/agena-multi-tool-plugin-stdio"
        },
        "config": {
          "prefix": "[note] ",
          "uppercase": false
        }
      }
    }
  }
}
```

The plugin exposes `example.notes/format` and `example.notes/write`.
