# `example.notes` Multi-Tool Plugin

This example shows the recommended shape for a plugin that owns multiple model-visible tools:

- Define one `ToolCommand` input type per tool.
- Group those input types with `ToolSubcommands`.
- Expose one `#[tool_suite]` method for normal invocation.
- Expose one `#[tool_suite_stream]` method when any tool can stream.
- Expose `#[permission(paths, suite)]` for dynamic path auditing.
- Keep plugin configuration in `PluginConfig<T>` and enable it with `#[plugin(..., config)]`.

## Build

```bash
cargo build -p agena-multi-tool-plugin-stdio
```

## Configure

```json
{
  "plugins": {
    "list": {
      "notes": {
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
