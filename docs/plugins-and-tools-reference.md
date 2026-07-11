# Agena 内置插件与工具完整参考

> 源码/运行时快照：2026-07-12；Agena `0.1.0`。本文覆盖当前构建实际加载的 16 个内置插件与 60 个工具。

## 文档范围与约定

- 工具的规范名称为 `agena.<plugin>.<tool>`，例如 `agena.runtime.rename`；provider 适配层可能把点号编码成下划线，但权限、catalog 和本文统一使用规范名称。
- 每个工具的“输入参数”表用于快速阅读；紧随其后的 `input_schema` 是运行时 manifest 暴露给模型的完整 JSON Schema，包含嵌套对象、枚举、默认值与正式约束。
- 当前插件 manifest 没有独立的 `output_schema` 字段。本文的“输出”根据实际实现记录 `payload` 形状；所有成功调用还共享下面的 `ToolInvokeOutput` envelope。外部 MCP 工具的内部结果由对应 MCP server 决定。
- `?` 表示字段可能缺省；`[]` 表示数组。失败调用返回插件、参数或权限错误，不返回成功 envelope。
- 本文只枚举 Agena 自带插件。用户通过 `cdylib`、`stdio`、marketplace 或未来扩展安装的第三方插件无法在静态文档中穷举，应使用 `agena plugin status`、`agena plugin inspect <id>` 或 `agena.tools.list` 查看当前运行时。

## 通用成功输出

所有工具成功时返回：

```json
{
  "title": "string",
  "output_text": "string",
  "payload": "object | array | scalar | null (optional)",
  "metadata": { "key": "string" },
  "attachments": []
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `title` | string | UI/记录使用的短标题。 |
| `output_text` | string | 直接提供给模型和 transcript 的可读结果。 |
| `payload` | JSON，可缺省 | 结构化结果；各工具的实际字段见下文。 |
| `metadata` | object<string,string>，可缺省 | 执行、UI、追踪元数据。 |
| `attachments` | array，可缺省 | 图片、文件、嵌入资源等附件。 |

## 插件索引

| Plugin | Tools | Hooks | Config schema | 摘要 |
| --- | ---: | --- | --- | --- |
| `agena.code` | 2 | tool.invoke | 有 | Structured code search and syntax inspection tools. |
| `agena.cron` | 4 | init, tool.invoke | 有 | Cron-style and one-shot wakeup scheduling tools. |
| `agena.fs` | 4 | tool.invoke | 有 | Filesystem command tools for read/search and explicit edits. |
| `agena.lsp` | 5 | init, tool.invoke | 有 | LSP read-only observability and navigation tools. |
| `agena.mcp` | 5 | init, tool.invoke, tool.definition | 有 | MCP discovery and bridge tools. |
| `agena.memory` | 5 | init, tool.invoke, chat.messages.transform, chat.system.transform | 有 | Persistent memory with searchable retrieval and write tools. |
| `agena.plan` | 4 | init, tool.execute.before, tool.invoke, command.execute.before, agent.stop | 有 | Plan orchestration and plan-autorun tools. |
| `agena.process` | 4 | tool.invoke | 有 | Command execution and background process tools. |
| `agena.runtime` | 5 | init, tool.invoke | 有 | Runtime session, agent, and user-interaction tools. |
| `agena.schema_lab` | 2 | tool.invoke | 有 | Deep built-in JSON Schema fixture used to demo and test the structured plugin config editor. |
| `agena.settings` | 6 | init, tool.invoke | 有 | Read and edit Agena runtime settings in agena.json. |
| `agena.skills` | 3 | init, tool.invoke, tool.definition | 有 | Discover, inspect, and render skills and slash commands. |
| `agena.snapshot` | 2 | init, tool.invoke | 有 | Managed snapshot tools backed by Rift or git worktree. |
| `agena.tasks` | 1 | init, tool.invoke | 有 | Delegated subtask orchestration tools. |
| `agena.tools` | 5 | init, tool.invoke | 有 | Tool discovery, help, and gateway tools. |
| `agena.web` | 3 | init, tool.invoke | 有 | Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering. |

## `agena.code`

Structured code search and syntax inspection tools.

- 版本：`0.1.0`
- Hooks：`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.code --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.code.search_ast` | read_only, filesystem_read, discovery | — | 是 | Search code structurally with ast-grep. |
| `agena.code.syntax_tree` | read_only, filesystem_read, discovery | — | 是 | Inspect a parsed syntax tree. |

### `agena.code.search_ast`

Search code structurally with ast-grep.

- Tags：`read_only`、`filesystem_read`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `language` | 否 | `CodeLanguage \| null` | — | — |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `path` | 是 | `string` | minLength=1 | — |
| `pattern` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "CodeLanguage": {
      "enum": [
        "auto",
        "bash",
        "c",
        "cpp",
        "csharp",
        "css",
        "dart",
        "elixir",
        "go",
        "haskell",
        "hcl",
        "html",
        "java",
        "javascript",
        "json",
        "lua",
        "markdown",
        "nix",
        "php",
        "python",
        "ruby",
        "rust",
        "solidity",
        "swift",
        "tsx",
        "typescript",
        "yaml"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "language": {
      "anyOf": [
        {
          "$ref": "#/$defs/CodeLanguage"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "limit": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000",
      "x-agena-path": "read"
    },
    "pattern": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "path",
    "pattern"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ language, scanned_files, matches[] }`; each match is `{ path, start_line, start_col, end_line, end_col, text }`.

### `agena.code.syntax_tree`

Inspect a parsed syntax tree.

- Tags：`read_only`、`filesystem_read`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `language` | 否 | `CodeLanguage \| null` | — | — |
| `max_depth` | 否 | `integer \| null` | minimum=0; maximum=255; format=uint8 | — |
| `path` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "CodeLanguage": {
      "enum": [
        "auto",
        "bash",
        "c",
        "cpp",
        "csharp",
        "css",
        "dart",
        "elixir",
        "go",
        "haskell",
        "hcl",
        "html",
        "java",
        "javascript",
        "json",
        "lua",
        "markdown",
        "nix",
        "php",
        "python",
        "ruby",
        "rust",
        "solidity",
        "swift",
        "tsx",
        "typescript",
        "yaml"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "language": {
      "anyOf": [
        {
          "$ref": "#/$defs/CodeLanguage"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000001"
    },
    "max_depth": {
      "format": "uint8",
      "maximum": 255,
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000",
      "x-agena-path": "read"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ path, language, root_kind, has_error, tree }`; `tree` is recursive `{ kind, start_line, start_col, end_line, end_col, text_preview, children[] }`.

## `agena.cron`

Cron-style and one-shot wakeup scheduling tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.cron --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.cron.list` | read_only, scheduler | scheduler | 是 | List registered cron jobs and wakeups. |
| `agena.cron.create` | mutating, scheduler | scheduler | 否 | Create one cron schedule. |
| `agena.cron.delete` | mutating, scheduler | scheduler | 否 | Delete one cron schedule. |
| `agena.cron.wakeup` | mutating, scheduler | scheduler | 否 | Create one one-shot wakeup. |

### `agena.cron.list`

List registered cron jobs and wakeups.

- Tags：`read_only`、`scheduler`
- Capabilities：`scheduler`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ jobs[] }`; each job is `{ id, kind, expression?, at?, prompt, next_fire_at?, last_fired_at? }`.

### `agena.cron.create`

Create one cron schedule.

- Tags：`mutating`、`scheduler`
- Capabilities：`scheduler`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `expression` | 是 | `string` | minLength=1 | 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`. |
| `max_age_days` | 否 | `integer` | minimum=0; format=uint32; default=7 | — |
| `prompt` | 是 | `string` | minLength=1 | Prompt to enqueue when the job fires. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "expression": {
      "description": "6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "max_age_days": {
      "default": 7,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "prompt": {
      "description": "Prompt to enqueue when the job fires.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "expression",
    "prompt"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ id, next_fire_at? }`.

### `agena.cron.delete`

Delete one cron schedule.

- Tags：`mutating`、`scheduler`
- Capabilities：`scheduler`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `id` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ id, removed }`.

### `agena.cron.wakeup`

Create one one-shot wakeup.

- Tags：`mutating`、`scheduler`
- Capabilities：`scheduler`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `delay_seconds` | 是 | `integer` | minimum=0; format=uint32 | — |
| `prompt` | 是 | `string` | minLength=1 | — |
| `reason` | 否 | `string \| null` | — | Short reason logged for diagnostics / shown back to the user. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "delay_seconds": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000"
    },
    "prompt": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "reason": {
      "description": "Short reason logged for diagnostics / shown back to the user.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    }
  },
  "required": [
    "delay_seconds",
    "prompt"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ id, next_fire_at }`.

## `agena.fs`

Filesystem command tools for read/search and explicit edits.

- 版本：`0.1.0`
- Hooks：`tool.invoke`
- 描述模式：model=`detailed`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.fs --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.fs.read` | read_only, filesystem_read | — | 是 | Read workspace files. |
| `agena.fs.glob` | read_only, filesystem_read, discovery | — | 是 | Find paths with glob patterns. |
| `agena.fs.grep` | read_only, filesystem_read, discovery | — | 是 | Search file contents with regex. |
| `agena.fs.apply_patch` | mutating, filesystem_write | — | 是 | Apply a text patch. |

### `agena.fs.read`

Read workspace files.

- Tags：`read_only`、`filesystem_read`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `file_path` | 是 | `string` | minLength=1 | File or directory path to read. Relative paths are resolved from the<br>workspace root. |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | Maximum number of lines or directory entries to return. |
| `mode` | 否 | `ReadMode` | default=auto | How to render the target: `text`, `attachment`, or `auto`. |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32 | 1-based offset for file lines or directory entries. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "ReadMode": {
      "description": "How to render the target: `text`, `attachment`, or `auto`.",
      "enum": [
        "text",
        "attachment",
        "auto"
      ],
      "type": "string",
      "x-agena-order": "000003"
    }
  },
  "properties": {
    "file_path": {
      "description": "File or directory path to read. Relative paths are resolved from the\nworkspace root.",
      "minLength": 1,
      "type": "string",
      "x-agena-aliases": [
        "path"
      ],
      "x-agena-order": "000000",
      "x-agena-path": "read"
    },
    "limit": {
      "description": "Maximum number of lines or directory entries to return.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "mode": {
      "$ref": "#/$defs/ReadMode",
      "default": "auto",
      "description": "How to render the target: `text`, `attachment`, or `auto`."
    },
    "offset": {
      "description": "1-based offset for file lines or directory entries.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    }
  },
  "required": [
    "file_path"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ preview?, truncated, loaded_paths[], attachment? }`; attachment is `{ path, kind, mime, size_bytes, filename?, width?, height?, duration_ms?, page_count? }`. Binary/attachment mode may also fill envelope `attachments[]`.

### `agena.fs.glob`

Find paths with glob patterns.

- Tags：`read_only`、`filesystem_read`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `glob` for path discovery before reading or editing files.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `path` | 否 | `string \| null` | minLength=1 | Optional base path. Defaults to the workspace root. |
| `pattern` | 是 | `string` | minLength=1 | Glob pattern to match. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "path": {
      "description": "Optional base path. Defaults to the workspace root.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001",
      "x-agena-path": "read"
    },
    "pattern": {
      "description": "Glob pattern to match.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "pattern"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ count?, paths[], truncated }`.

### `agena.fs.grep`

Search file contents with regex.

- Tags：`read_only`、`filesystem_read`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `grep` for regex text search across files in the workspace.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `include` | 否 | `string \| null` | minLength=1 | Optional glob filter applied before matching lines. |
| `path` | 否 | `string \| null` | minLength=1 | Optional base path. Defaults to the workspace root. |
| `pattern` | 是 | `string` | minLength=1 | Regex pattern to search for. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "include": {
      "description": "Optional glob filter applied before matching lines.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "path": {
      "description": "Optional base path. Defaults to the workspace root.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001",
      "x-agena-path": "read"
    },
    "pattern": {
      "description": "Regex pattern to search for.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "pattern"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ matches?, results[], truncated }`; each result is a `path:line: text` string.

### `agena.fs.apply_patch`

Apply a text patch.

- Tags：`mutating`、`filesystem_write`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `apply_patch` for explicit text patch operations against workspace files.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `patch` | 是 | `string` | — | Unified patch text to apply to the workspace. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "description": "Textual patch payload in the agena patch format.",
  "properties": {
    "patch": {
      "description": "Unified patch text to apply to the workspace.",
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "patch"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ operation_id, changes[], before_hash?, after_hash?, inverse_patch, diff, progress[] }`; each change is `{ path, kind, from_path? }`, with `kind = added|updated|deleted|moved`.

## `agena.lsp`

LSP read-only observability and navigation tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.lsp --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.lsp.servers` | read_only, lsp | lsp_registry | 是 | List configured language servers. |
| `agena.lsp.definition` | read_only, filesystem_read, lsp | lsp_registry | 是 | Resolve symbol definitions. |
| `agena.lsp.references` | read_only, filesystem_read, lsp | lsp_registry | 是 | Find symbol references. |
| `agena.lsp.hover` | read_only, filesystem_read, lsp | lsp_registry | 是 | Fetch hover text. |
| `agena.lsp.diagnostics` | read_only, filesystem_read, lsp | lsp_registry | 是 | Fetch file diagnostics. |

### `agena.lsp.servers`

List configured language servers.

- Tags：`read_only`、`lsp`
- Capabilities：`lsp_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ servers[] }`; each server is `{ name, command, args[], file_extensions[] }`.

### `agena.lsp.definition`

Resolve symbol definitions.

- Tags：`read_only`、`filesystem_read`、`lsp`
- Capabilities：`lsp_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `character` | 是 | `integer` | minimum=0; format=uint32 | — |
| `file_path` | 是 | `string` | minLength=1 | — |
| `line` | 是 | `integer` | minimum=0; format=uint32 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "character": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000",
      "x-agena-path": "read"
    },
    "line": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000001"
    }
  },
  "required": [
    "character",
    "file_path",
    "line"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ locations[] }`.

### `agena.lsp.references`

Find symbol references.

- Tags：`read_only`、`filesystem_read`、`lsp`
- Capabilities：`lsp_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `character` | 是 | `integer` | minimum=0; format=uint32 | — |
| `file_path` | 是 | `string` | minLength=1 | — |
| `include_declaration` | 否 | `boolean` | default=true | — |
| `line` | 是 | `integer` | minimum=0; format=uint32 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "character": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000",
      "x-agena-path": "read"
    },
    "include_declaration": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "line": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000001"
    }
  },
  "required": [
    "character",
    "file_path",
    "line"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ locations[] }`.

### `agena.lsp.hover`

Fetch hover text.

- Tags：`read_only`、`filesystem_read`、`lsp`
- Capabilities：`lsp_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `character` | 是 | `integer` | minimum=0; format=uint32 | — |
| `file_path` | 是 | `string` | minLength=1 | — |
| `line` | 是 | `integer` | minimum=0; format=uint32 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "character": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000",
      "x-agena-path": "read"
    },
    "line": {
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000001"
    }
  },
  "required": [
    "character",
    "file_path",
    "line"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ contents? }`.

### `agena.lsp.diagnostics`

Fetch file diagnostics.

- Tags：`read_only`、`filesystem_read`、`lsp`
- Capabilities：`lsp_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `file_path` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000",
      "x-agena-path": "read"
    }
  },
  "required": [
    "file_path"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ entries[] }`.

## `agena.mcp`

MCP discovery and bridge tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`、`tool.definition`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.mcp --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.mcp.resources.list` | read_only, mcp, network | — | 是 | List MCP resources from one server. |
| `agena.mcp.resources.read` | read_only, mcp, network | — | 是 | Read one MCP resource by URI. |
| `agena.mcp.prompts.list` | read_only, mcp, network | — | 是 | List MCP prompt templates from one server. |
| `agena.mcp.prompts.get` | read_only, mcp, network | — | 是 | Fetch one MCP prompt template. |
| `agena.mcp.tools.call` | mutating, mcp, network | — | 否 | Call one discovered MCP tool. |

### `agena.mcp.resources.list`

List MCP resources from one server.

- Tags：`read_only`、`mcp`、`network`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `server` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "server": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "server"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ server, resources[], next_cursor? }`. Resource members come from the connected MCP server and normally include `uri`, optional `name`, `description`, and `mimeType`/`mime_type`.

### `agena.mcp.resources.read`

Read one MCP resource by URI.

- Tags：`read_only`、`mcp`、`network`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `server` | 是 | `string` | minLength=1 | — |
| `uri` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "server": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "uri": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "server",
    "uri"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ server, uri, contents[] }`; resource contents may contain text or base64 data and may also produce envelope `attachments[]`.

### `agena.mcp.prompts.list`

List MCP prompt templates from one server.

- Tags：`read_only`、`mcp`、`network`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `server` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "server": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "server"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ server, prompts[], next_cursor? }`; prompts include name, optional description, and argument descriptors.

### `agena.mcp.prompts.get`

Fetch one MCP prompt template.

- Tags：`read_only`、`mcp`、`network`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `arguments` | 否 | `object \| null` | default=null | — |
| `name` | 是 | `string` | minLength=1 | — |
| `server` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "arguments": {
      "additionalProperties": {
        "type": "string"
      },
      "default": null,
      "type": [
        "object",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "server": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "server",
    "name"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ server, prompt, description?, messages[] }`.

### `agena.mcp.tools.call`

Call one discovered MCP tool.

- Tags：`mutating`、`mcp`、`network`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `arguments` | 否 | `any` | default=null | — |
| `name` | 是 | `string` | minLength=1 | — |
| `server` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "arguments": {
      "default": null,
      "x-agena-order": "000002"
    },
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "server": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "server",
    "name"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ server, tool, content_blocks[] }`; blocks are MCP-dependent text/image/resource blocks and image/resource results may also appear in envelope `attachments[]`.

## `agena.memory`

Persistent memory with searchable retrieval and write tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`、`chat.messages.transform`、`chat.system.transform`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.memory --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.memory.search` | read_only, filesystem_write | — | 否 | Search durable memory records. |
| `agena.memory.get` | read_only, filesystem_read | — | 否 | Read one durable memory record. |
| `agena.memory.list` | read_only, filesystem_read | — | 否 | List durable memory records. |
| `agena.memory.write` | mutating, filesystem_write | — | 否 | Write one durable memory record. |
| `agena.memory.delete` | mutating, filesystem_write | — | 否 | Delete one durable memory record. |

### `agena.memory.search`

Search durable memory records.

- Tags：`read_only`、`filesystem_write`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `query` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "query": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "query"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ query, limit, results[] }`; each result is `{ id, name, description, memory_type?, body, path, searchable_text }`.

### `agena.memory.get`

Read one durable memory record.

- Tags：`read_only`、`filesystem_read`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `name` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name"
  ],
  "type": "object",
  "x-agena-relations": [
    "forbid_substrings `name`: \"/\", \"\\\""
  ]
}
```

</details>

输出：

`payload`: `{ name, description, memory_type?, file_name, body }`.

### `agena.memory.list`

List durable memory records.

- Tags：`read_only`、`filesystem_read`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ limit, memories[] }`; each memory is `{ name, description, memory_type?, file_name, body }`.

### `agena.memory.write`

Write one durable memory record.

- Tags：`mutating`、`filesystem_write`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `content` | 是 | `string` | minLength=1 | — |
| `description` | 否 | `string` | default= | — |
| `memory_type` | 否 | `MemoryType \| null` | — | — |
| `name` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "MemoryType": {
      "enum": [
        "user",
        "feedback",
        "project",
        "reference",
        "other"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "content": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000003"
    },
    "description": {
      "default": "",
      "type": "string",
      "x-agena-order": "000001"
    },
    "memory_type": {
      "anyOf": [
        {
          "$ref": "#/$defs/MemoryType"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name",
    "content"
  ],
  "type": "object",
  "x-agena-relations": [
    "forbid_substrings `name`: \"/\", \"\\\""
  ]
}
```

</details>

输出：

`payload`: `{ name, description, memory_type?, file_name, body }`.

### `agena.memory.delete`

Delete one durable memory record.

- Tags：`mutating`、`filesystem_write`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `name` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name"
  ],
  "type": "object",
  "x-agena-relations": [
    "forbid_substrings `name`: \"/\", \"\\\""
  ]
}
```

</details>

输出：

Success has no structured payload (`payload` is absent); `output_text` confirms the deleted name.

## `agena.plan`

Plan orchestration and plan-autorun tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.execute.before`、`tool.invoke`、`command.execute.before`、`agent.stop`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.plan --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.plan.get` | planning, read_only | ask_user, plugin_storage, statusline | 是 | Inspect the current plan state. |
| `agena.plan.set` | planning, mutating | ask_user, plugin_storage, statusline | 否 | Create or replace the current plan. |
| `agena.plan.update` | planning, mutating | ask_user, plugin_storage, statusline | 否 | Update the current plan. |
| `agena.plan.clear` | planning, mutating | ask_user, plugin_storage, statusline | 否 | Remove the current plan. |

### `agena.plan.get`

Inspect the current plan state.

- Tags：`planning`、`read_only`
- Capabilities：`ask_user`、`plugin_storage`、`statusline`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `view` | 否 | `PlanGetView` | default=current | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "PlanGetView": {
      "enum": [
        "current",
        "summary",
        "full"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "view": {
      "$ref": "#/$defs/PlanGetView",
      "default": "current"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ plan, view, current_step, current_step_index?, current_step_goal }`. `plan` is null or `{ title, objective, phase, autorun, document_markdown?, steps[] }`; steps contain `{ id, title, description, executor, status, wait_until_ms?, note?, checks[] }`, and checks contain `{ id, text, status }`.

### `agena.plan.set`

Create or replace the current plan.

- Tags：`planning`、`mutating`
- Capabilities：`ask_user`、`plugin_storage`、`statusline`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `autorun` | 否 | `boolean \| null` | — | — |
| `document_markdown` | 否 | `string \| null` | — | — |
| `objective` | 是 | `string` | — | — |
| `steps` | 否 | `array<WorkflowPlanStepInput>` | — | Ordered plan steps. Each step item uses `title`; nested checks use `text`. |
| `title` | 否 | `string \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "WorkflowPlanCheckpointInput": {
      "additionalProperties": false,
      "description": "Plan check input. Each check item should use `text`.",
      "properties": {
        "id": {
          "type": [
            "string",
            "null"
          ]
        },
        "status": {
          "anyOf": [
            {
              "$ref": "#/$defs/WorkflowPlanStepStatus"
            },
            {
              "type": "null"
            }
          ]
        },
        "text": {
          "default": "",
          "description": "Check text.",
          "type": "string"
        }
      },
      "type": "object"
    },
    "WorkflowPlanExecutor": {
      "enum": [
        "ai",
        "human"
      ],
      "type": "string"
    },
    "WorkflowPlanStepInput": {
      "additionalProperties": false,
      "description": "Plan step input. Each step uses `title`; nested checks under `checks` use `text`.",
      "properties": {
        "checks": {
          "description": "Optional checklist checks for this step. Each check item uses `text`, not `title`.",
          "items": {
            "$ref": "#/$defs/WorkflowPlanCheckpointInput"
          },
          "type": "array"
        },
        "description": {
          "default": "",
          "description": "Optional longer explanation for the step. If omitted, the step title can serve as the short description.",
          "type": "string"
        },
        "executor": {
          "$ref": "#/$defs/WorkflowPlanExecutor",
          "default": "ai",
          "description": "Who should execute the step. Use `ai` for agent work and `human` for manual work."
        },
        "id": {
          "type": [
            "string",
            "null"
          ]
        },
        "note": {
          "type": [
            "string",
            "null"
          ]
        },
        "status": {
          "anyOf": [
            {
              "$ref": "#/$defs/WorkflowPlanStepStatus"
            },
            {
              "type": "null"
            }
          ]
        },
        "title": {
          "default": "",
          "description": "Human-readable step title.",
          "type": "string"
        },
        "wait_until_ms": {
          "format": "int64",
          "type": [
            "integer",
            "null"
          ]
        }
      },
      "type": "object"
    },
    "WorkflowPlanStepStatus": {
      "enum": [
        "pending",
        "in_progress",
        "blocked",
        "completed",
        "skipped"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "description": "Create or overwrite the current active-session plan in planning. If a plan already exists, this replaces it and resets the phase to planning. Use `steps[].title` for steps, `steps[].checks[].text` for checks, and `autorun` to control whether approved active plans should keep running automatically.",
  "properties": {
    "autorun": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "document_markdown": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "objective": {
      "type": "string",
      "x-agena-order": "000000"
    },
    "steps": {
      "description": "Ordered plan steps. Each step item uses `title`; nested checks use `text`.",
      "items": {
        "$ref": "#/$defs/WorkflowPlanStepInput"
      },
      "type": "array",
      "x-agena-order": "000003"
    },
    "title": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    }
  },
  "required": [
    "objective"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ plan }`, using the plan shape documented for `agena.plan.get`.

### `agena.plan.update`

Update the current plan.

- Tags：`planning`、`mutating`
- Capabilities：`ask_user`、`plugin_storage`、`statusline`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`
- Help：Keep plan-level updates separate from step/check updates: do not send `phase` together with `step_id`, `check_id`, `status`, `wait_until_ms`, or `note`. To complete a plan with steps, mark the required steps/checks `completed` first, then call update separately with `phase: completed`.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `autorun` | 否 | `boolean \| null` | — | Whether an approved active plan should keep running automatically. |
| `check_id` | 否 | `string \| null` | — | — |
| `note` | 否 | `string \| null` | — | — |
| `phase` | 否 | `WorkflowPlanPhase \| null` | — | Canonical plan phase. Use `planning`, `active`, `blocked`, `completed`, or `cancelled`. |
| `status` | 否 | `WorkflowPlanStepStatus \| null` | — | — |
| `step_id` | 否 | `string \| null` | — | — |
| `summary` | 否 | `string \| null` | — | Optional completion summary. This is only applied when `phase` is `completed`. |
| `wait_until_ms` | 否 | `integer \| null` | format=int64 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "WorkflowPlanPhase": {
      "enum": [
        "planning",
        "active",
        "blocked",
        "completed",
        "cancelled"
      ],
      "type": "string"
    },
    "WorkflowPlanStepStatus": {
      "enum": [
        "pending",
        "in_progress",
        "blocked",
        "completed",
        "skipped"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "description": "Update the current plan. Use `phase` / `autorun` for plan-level state changes, `step_id` + `status` to update a step, or `step_id` + `check_id` + `status` to update a check. Do not combine plan-level fields (`phase`, `autorun`, `summary`) with step/check fields. To complete a plan with steps, first mark the relevant steps or checks `completed`, then make a separate plan-level update with `phase: completed`. Canonical phase values are `planning`, `active`, `blocked`, `completed`, and `cancelled`.",
  "properties": {
    "autorun": {
      "description": "Whether an approved active plan should keep running automatically.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "check_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "note": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000007"
    },
    "phase": {
      "anyOf": [
        {
          "$ref": "#/$defs/WorkflowPlanPhase"
        },
        {
          "type": "null"
        }
      ],
      "description": "Canonical plan phase. Use `planning`, `active`, `blocked`, `completed`, or `cancelled`.",
      "x-agena-order": "000000"
    },
    "status": {
      "anyOf": [
        {
          "$ref": "#/$defs/WorkflowPlanStepStatus"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000005"
    },
    "step_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "summary": {
      "description": "Optional completion summary. This is only applied when `phase` is `completed`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "wait_until_ms": {
      "format": "int64",
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    }
  },
  "type": "object"
}
```

</details>

输出：

Normally `payload`: `{ plan }`. When an approval/review path is used, the result may be `{ plan, decision }`.

### `agena.plan.clear`

Remove the current plan.

- Tags：`planning`、`mutating`
- Capabilities：`ask_user`、`plugin_storage`、`statusline`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ cleared }`.

## `agena.process`

Command execution and background process tools.

- 版本：`0.1.0`
- Hooks：`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.process --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.process.run` | mutating, shell, network, filesystem_read | — | 否 | Run one shell process. |
| `agena.process.list` | read_only, shell | — | 是 | List active background processes. |
| `agena.process.logs` | read_only, shell | — | 是 | Read background process logs. |
| `agena.process.stop` | mutating, shell | — | 否 | Stop one background process. |

### `agena.process.run`

Run one shell process.

- Tags：`mutating`、`shell`、`network`、`filesystem_read`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`
- Help：Set `background = true` to keep the process attached to the session and receive a `process_id` for later inspection.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `background` | 否 | `boolean` | default=false | — |
| `command` | 是 | `string` | minLength=1 | — |
| `description` | 否 | `string` | default= | — |
| `filesystem_effects` | 是 | `array<FilesystemEffect>` | — | Filesystem paths the command may read or write. Pass an empty list only<br>when the command has no filesystem effect beyond entering `workdir`. |
| `network_effects` | 是 | `array<NetworkEffect>` | — | Outbound network targets the command may connect to. Pass an empty list<br>when the command has no network effect. |
| `shell` | 否 | `ProcessShell` | default=bash | — |
| `timeout_ms` | 否 | `integer \| null` | minimum=0; format=uint64 | — |
| `workdir` | 否 | `string \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "FilesystemAccess": {
      "description": "Filesystem access mode declared by a tool invocation.",
      "enum": [
        "read",
        "write",
        "read_write"
      ],
      "type": "string"
    },
    "FilesystemEffect": {
      "description": "One path a command may read, write, or both.",
      "properties": {
        "access": {
          "$ref": "#/$defs/FilesystemAccess"
        },
        "path": {
          "description": "File or directory path affected by the command. For shell tools,\nrelative paths are resolved from the command working directory.",
          "type": "string"
        }
      },
      "required": [
        "path",
        "access"
      ],
      "type": "object"
    },
    "NetworkEffect": {
      "description": "One outbound network target a command may access.",
      "properties": {
        "target": {
          "description": "Absolute URL or `host[:port]` target. Shell tools must declare every\nremote endpoint they may connect to; pass an empty list when the\ncommand has no network effect.",
          "type": "string"
        }
      },
      "required": [
        "target"
      ],
      "type": "object"
    },
    "ProcessShell": {
      "description": "Shell launcher used for a process run.",
      "enum": [
        "bash",
        "powershell"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "background": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "command": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001.000000"
    },
    "description": {
      "default": "",
      "type": "string",
      "x-agena-order": "000001.000001"
    },
    "filesystem_effects": {
      "description": "Filesystem paths the command may read or write. Pass an empty list only\nwhen the command has no filesystem effect beyond entering `workdir`.",
      "items": {
        "$ref": "#/$defs/FilesystemEffect"
      },
      "type": "array",
      "x-agena-order": "000001.000004"
    },
    "network_effects": {
      "description": "Outbound network targets the command may connect to. Pass an empty list\nwhen the command has no network effect.",
      "items": {
        "$ref": "#/$defs/NetworkEffect"
      },
      "type": "array",
      "x-agena-order": "000001.000005"
    },
    "shell": {
      "$ref": "#/$defs/ProcessShell",
      "default": "bash"
    },
    "timeout_ms": {
      "format": "uint64",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000002"
    },
    "workdir": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001.000000",
      "x-agena-path": "read"
    }
  },
  "required": [
    "command",
    "filesystem_effects",
    "network_effects"
  ],
  "type": "object"
}
```

</details>

输出：

`payload` uses the shared process shape `{ action, shell?, background, process_id?, status?, output?, description?, events[], processes[], last_seq, has_more, dropped_lines, exit_code? }`. Events are `{ seq, stream, ts_ms, line }`; statuses are `running|exited|timed_out|stopped|failed`.

### `agena.process.list`

List active background processes.

- Tags：`read_only`、`shell`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload` uses the shared process shape and primarily fills `processes[]`. A process summary is `{ process_id, command, description, status, background, started_at_ms, ended_at_ms?, buffered_lines, last_seq, dropped_lines, exit_code? }`.

### `agena.process.logs`

Read background process logs.

- Tags：`read_only`、`shell`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `process_id` | 是 | `string` | minLength=1 | — |
| `since_seq` | 否 | `integer` | minimum=0; format=uint64; default=0 | — |
| `wait_ms` | 否 | `integer` | minimum=0; format=uint64; default=0 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "since_seq": {
      "default": 0,
      "format": "uint64",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "wait_ms": {
      "default": 0,
      "format": "uint64",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000003"
    }
  },
  "required": [
    "process_id"
  ],
  "type": "object"
}
```

</details>

输出：

`payload` uses the shared process shape and primarily fills `events[]`, `last_seq`, `has_more`, `dropped_lines`, `status`, and `exit_code?`.

### `agena.process.stop`

Stop one background process.

- Tags：`mutating`、`shell`
- Capabilities：无
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `process_id` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "process_id"
  ],
  "type": "object"
}
```

</details>

输出：

`payload` uses the shared process shape and reports the stopped process through `process_id`, `status`, `output?`, and `exit_code?`.

## `agena.runtime`

Runtime session, agent, and user-interaction tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.runtime --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.runtime.switch` | — | agent_registry | 否 | Switch the current runtime agent profile. |
| `agena.runtime.restore` | — | agent_registry | 否 | Restore the previous runtime agent profile. |
| `agena.runtime.get` | read_only | session_registry | 是 | Inspect the current session metadata. |
| `agena.runtime.rename` | mutating | session_registry | 否 | Rename the current session. |
| `agena.runtime.request_input` | interactive | ask_user | 否 | Request short structured input from the user. |

### `agena.runtime.switch`

Switch the current runtime agent profile.

- Tags：无
- Capabilities：`agent_registry`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `agent` | 否 | `string \| null` | — | Target agent profile. Omit or pass an empty string to clear the<br>explicit runtime agent selection. |
| `push_previous` | 否 | `boolean` | default=false | Push the current agent so `agent_restore` can return to it later. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "agent": {
      "description": "Target agent profile. Omit or pass an empty string to clear the\nexplicit runtime agent selection.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "push_previous": {
      "default": false,
      "description": "Push the current agent so `agent_restore` can return to it later.",
      "type": "boolean",
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ session_id, previous_agent?, current_agent?, stack_depth }`.

### `agena.runtime.restore`

Restore the previous runtime agent profile.

- Tags：无
- Capabilities：`agent_registry`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ session_id, restored, previous_agent?, current_agent?, stack_depth }`.

### `agena.runtime.get`

Inspect the current session metadata.

- Tags：`read_only`
- Capabilities：`session_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ session }`; session is `{ id, parent_id?, root_id, workspace_id, title, is_subagent }`.

### `agena.runtime.rename`

Rename the current session.

- Tags：`mutating`
- Capabilities：`session_registry`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `title` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "title": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "title"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ session }`; session is `{ id, parent_id?, root_id, workspace_id, title, is_subagent }`.

### `agena.runtime.request_input`

Request short structured input from the user.

- Tags：`interactive`
- Capabilities：`ask_user`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `body_markdown` | 否 | `string` | — | — |
| `cancel_label` | 否 | `string` | — | — |
| `kind` | 否 | `string` | — | — |
| `questions` | 否 | `array<UserInputQuestion>` | minItems=1; maxItems=3 | — |
| `submit_label` | 否 | `string` | — | — |
| `title` | 否 | `string` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "UserInputOption": {
      "properties": {
        "description": {
          "description": "Optional explanatory text shown alongside the label.",
          "type": "string"
        },
        "label": {
          "description": "Visible label for the option.",
          "minLength": 1,
          "type": "string"
        }
      },
      "required": [
        "label"
      ],
      "type": "object"
    },
    "UserInputQuestion": {
      "properties": {
        "allow_custom": {
          "description": "Allow a custom answer even when options are present.",
          "type": "boolean"
        },
        "header": {
          "description": "Short header displayed above the question body.",
          "maxLength": 12,
          "type": "string"
        },
        "id": {
          "description": "Stable identifier for the question; used in replies.",
          "minLength": 1,
          "type": "string"
        },
        "multiple": {
          "description": "Allow multiple options to be selected.",
          "type": "boolean"
        },
        "options": {
          "description": "Optional answer options. Leave empty when the user may answer freely.",
          "items": {
            "$ref": "#/$defs/UserInputOption"
          },
          "maxItems": 8,
          "type": "array"
        },
        "question": {
          "description": "Prompt text presented to the user.",
          "minLength": 1,
          "type": "string"
        }
      },
      "required": [
        "id",
        "question"
      ],
      "type": "object"
    }
  },
  "properties": {
    "body_markdown": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "cancel_label": {
      "type": "string",
      "x-agena-order": "000004"
    },
    "kind": {
      "type": "string",
      "x-agena-order": "000002"
    },
    "questions": {
      "items": {
        "$ref": "#/$defs/UserInputQuestion"
      },
      "maxItems": 3,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000005"
    },
    "submit_label": {
      "type": "string",
      "x-agena-order": "000003"
    },
    "title": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "type": "object",
  "x-agena-relations": [
    "required_unless_present `questions[].allow_custom` unless `questions[].options` present",
    "distinct_trimmed `questions[].id`",
    "distinct_trimmed_within `questions[].options[].label` within `questions[]`"
  ]
}
```

</details>

输出：

`payload`: `{ answers }`, where `answers` maps each question id to an array of selected/free-text strings. Cancellation is returned as an error rather than a successful payload.

## `agena.schema_lab`

Deep built-in JSON Schema fixture used to demo and test the structured plugin config editor.

- 版本：`0.1.0`
- Hooks：`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.schema_lab --format json` 获取完整插件配置 schema。
- Plugin commands：
  - `schema_lab.open_fixture`：Schema Lab: Open Fixture（slash: `/schema-lab`）
  - `schema_lab.show_defaults`：Schema Lab: Show Defaults
  - `schema_lab.run_probe`：Schema Lab: Run Probe

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.schema_lab.inspect` | read_only, discovery | — | 是 | Inspect the schema lab fixture without mutating external state. |
| `agena.schema_lab.echo` | read_only, discovery | — | 是 | Echo schema lab input without mutating external state. |

### `agena.schema_lab.inspect`

Inspect the schema lab fixture without mutating external state.

- Tags：`read_only`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Summarize one schema lab config section. The tool is intentionally inert and exists only to populate the Tools tab for the schema lab demo plugin.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `include_defaults` | 否 | `boolean` | default=false | — |
| `section` | 否 | `string \| null` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "include_defaults": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "section": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ section, include_defaults, mode: "inspect" }`.

### `agena.schema_lab.echo`

Echo schema lab input without mutating external state.

- Tags：`read_only`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Round-trip a label and arbitrary payload into the tool result. The tool is intentionally inert and exists only to populate the Tools tab for the schema lab demo plugin.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `label` | 否 | `string \| null` | minLength=1 | — |
| `payload` | 否 | `any` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "label": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "payload": {
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ label, payload, mode: "echo" }`.

## `agena.settings`

Read and edit Agena runtime settings in agena.json.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.settings --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.settings.get` | read_only, discovery, settings, filesystem_read | read_config, reload_config | 是 | Read one settings path. |
| `agena.settings.list` | read_only, discovery, settings, filesystem_read | read_config, reload_config | 是 | List settings paths. |
| `agena.settings.set` | mutating, filesystem_write, settings | read_config, reload_config | 否 | Set one settings value. |
| `agena.settings.delete` | mutating, filesystem_write, settings | read_config, reload_config | 否 | Delete one settings value. |
| `agena.settings.patch` | mutating, filesystem_write, settings | read_config, reload_config | 否 | Patch settings in agena.json. |
| `agena.settings.validate` | read_only, settings, filesystem_read | read_config, reload_config | 是 | Validate agena.json. |

### `agena.settings.get`

Read one settings path.

- Tags：`read_only`、`discovery`、`settings`、`filesystem_read`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：For effective reads, prefer explicit `scope = config|meta` with a relative `path` instead of relying on prefixed paths like `config.foo`.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `path` | 否 | `string \| null` | default=null | — |
| `scope` | 否 | `SettingsScope \| null` | default=null | — |
| `source` | 否 | `ConfigSettingsSource \| null` | default=null | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "ConfigSettingsSource": {
      "enum": [
        "effective",
        "file"
      ],
      "type": "string"
    },
    "SettingsScope": {
      "enum": [
        "config",
        "meta"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "scope": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsScope"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000001"
    },
    "source": {
      "anyOf": [
        {
          "$ref": "#/$defs/ConfigSettingsSource"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000002"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ config_path, config_found, source, path?, value }`.

### `agena.settings.list`

List settings paths.

- Tags：`read_only`、`discovery`、`settings`、`filesystem_read`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `path` | 否 | `string \| null` | default=null | — |
| `recursive` | 否 | `boolean \| null` | default=null | — |
| `scope` | 否 | `SettingsScope \| null` | default=null | — |
| `source` | 否 | `ConfigSettingsSource \| null` | default=null | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "ConfigSettingsSource": {
      "enum": [
        "effective",
        "file"
      ],
      "type": "string"
    },
    "SettingsScope": {
      "enum": [
        "config",
        "meta"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "recursive": {
      "default": null,
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "scope": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsScope"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000001"
    },
    "source": {
      "anyOf": [
        {
          "$ref": "#/$defs/ConfigSettingsSource"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000002"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ config_path, config_found, source, path?, items[] }`; each item is `{ path, kind, value? }`.

### `agena.settings.set`

Set one settings value.

- Tags：`mutating`、`filesystem_write`、`settings`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `dry_run` | 否 | `boolean` | default=false | — |
| `path` | 是 | `string` | minLength=1 | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |
| `value` | 是 | `any` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "reload": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "value": {
      "x-agena-order": "000001"
    }
  },
  "required": [
    "path",
    "value"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ config_path, config_found, operation, path?, dry_run, changed, created, deleted, validated, reload_requested, reload_required, reload?, previous, current }`; reload is `{ previous_generation, generation, loaded_at }` when performed.

### `agena.settings.delete`

Delete one settings value.

- Tags：`mutating`、`filesystem_write`、`settings`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `dry_run` | 否 | `boolean` | default=false | — |
| `path` | 是 | `string` | minLength=1 | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "reload": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000002"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

</details>

输出：

`payload` uses the same settings-edit shape as `set`.

### `agena.settings.patch`

Patch settings in agena.json.

- Tags：`mutating`、`filesystem_write`、`settings`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `changes` | 否 | `any` | default=null | — |
| `dry_run` | 否 | `boolean` | default=false | — |
| `path` | 否 | `string \| null` | default=null | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "changes": {
      "default": null,
      "x-agena-order": "000001"
    },
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "reload": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload` uses the same settings-edit shape as `set`.

### `agena.settings.validate`

Validate agena.json.

- Tags：`read_only`、`settings`、`filesystem_read`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ config_path, config_found, valid }`.

## `agena.skills`

Discover, inspect, and render skills and slash commands.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`、`tool.definition`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.skills --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.skills.list` | read_only, discovery | — | 是 | List discovered skills and slash commands. |
| `agena.skills.get` | read_only, discovery | — | 是 | Read one discovered skill or slash command. |
| `agena.skills.run` | read_only | — | 是 | Render one discovered skill or slash command prompt. |

### `agena.skills.list`

List discovered skills and slash commands.

- Tags：`read_only`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `kind` | 否 | `string \| null` | — | — |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `verbose` | 否 | `boolean` | default=false | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "kind": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "limit": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "offset": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "verbose": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ tools[], total, offset, returned, kind? }`; each item is `{ name, kind, summary }`.

### `agena.skills.get`

Read one discovered skill or slash command.

- Tags：`read_only`、`discovery`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `name` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ name, kind, summary, body }`.

### `agena.skills.run`

Render one discovered skill or slash command prompt.

- Tags：`read_only`
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `args` | 否 | `string \| null` | — | — |
| `name` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "args": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

</details>

输出：

No structured payload; the rendered skill/command prompt is returned in `output_text`. Metadata includes `workflow` and `skill_tool_kind`.

## `agena.snapshot`

Managed snapshot tools backed by Rift or git worktree.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.snapshot --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.snapshot.enter` | mutating, filesystem_write, snapshot | snapshot_registry, plugin_storage | 否 | Enter a managed repository snapshot. |
| `agena.snapshot.exit` | mutating, filesystem_write, snapshot | snapshot_registry, plugin_storage | 否 | Exit a managed repository snapshot. |

### `agena.snapshot.enter`

Enter a managed repository snapshot.

- Tags：`mutating`、`filesystem_write`、`snapshot`
- Capabilities：`snapshot_registry`、`plugin_storage`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

_无输入参数；调用时传 `{}`。_

<details>
<summary>完整 input_schema</summary>

```json
{
  "oneOf": [
    {
      "description": "Create a new managed snapshot under the managed `snapshots` directory.",
      "properties": {
        "name": {
          "minLength": 1,
          "type": [
            "string",
            "null"
          ],
          "x-agena-order": "000000"
        },
        "target": {
          "const": "new",
          "type": "string"
        }
      },
      "required": [
        "target"
      ],
      "type": "object"
    },
    {
      "description": "Attach to an already-existing snapshot at the provided path.",
      "properties": {
        "path": {
          "minLength": 1,
          "type": "string",
          "x-agena-order": "000000"
        },
        "target": {
          "const": "existing",
          "type": "string"
        }
      },
      "required": [
        "target",
        "path"
      ],
      "type": "object"
    }
  ],
  "properties": {},
  "type": "object"
}
```

</details>

输出：

`payload`: `{ path, branch, backend?, note? }`.

### `agena.snapshot.exit`

Exit a managed repository snapshot.

- Tags：`mutating`、`filesystem_write`、`snapshot`
- Capabilities：`snapshot_registry`、`plugin_storage`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `discard_changes` | 否 | `boolean` | default=false | — |
| `exit_action` | 是 | `ExitSnapshotAction` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "ExitSnapshotAction": {
      "enum": [
        "keep",
        "remove"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "properties": {
    "discard_changes": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "exit_action": {
      "$ref": "#/$defs/ExitSnapshotAction"
    }
  },
  "required": [
    "exit_action"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ action, path }`.

## `agena.tasks`

Delegated subtask orchestration tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.tasks --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.tasks.run` | task, subtask | spawn_subtask, plugin_storage | 否 | Create or resume a delegated subagent task. |

### `agena.tasks.run`

Create or resume a delegated subagent task.

- Tags：`task`、`subtask`
- Capabilities：`spawn_subtask`、`plugin_storage`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `command` | 否 | `string \| null` | — | Optional command to run in the subtask shell context. |
| `description` | 是 | `string` | minLength=1 | Short label for the subtask session. |
| `prompt` | 是 | `string` | minLength=1 | Full instruction payload for the delegated subtask. |
| `subagent_type` | 是 | `TaskSubagentType` | — | Which subagent profile should execute the subtask. |
| `task_id` | 否 | `string \| null` | — | Resume an existing subtask session instead of creating a new one. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "TaskSubagentType": {
      "description": "Which subagent profile should execute the subtask.",
      "enum": [
        "explore",
        "implement",
        "verify"
      ],
      "type": "string",
      "x-agena-order": "000002"
    }
  },
  "description": "Input for the delegated `task` subagent command.",
  "properties": {
    "command": {
      "description": "Optional command to run in the subtask shell context.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "description": {
      "description": "Short label for the subtask session.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "prompt": {
      "description": "Full instruction payload for the delegated subtask.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "subagent_type": {
      "$ref": "#/$defs/TaskSubagentType",
      "description": "Which subagent profile should execute the subtask."
    },
    "task_id": {
      "description": "Resume an existing subtask session instead of creating a new one.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    }
  },
  "required": [
    "description",
    "prompt",
    "subagent_type"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ session_id?, model_provider_id?, model_id? }`; the delegated agent's final response is in `output_text`, with additional task details in `metadata`.

## `agena.tools`

Tool discovery, help, and gateway tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.tools --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.tools.list` | read_only, discovery | list_tools, tool_registry | 是 | Enumerate current tools. |
| `agena.tools.search` | read_only, discovery | list_tools, tool_registry | 是 | Search the current tool catalog. |
| `agena.tools.help` | read_only, discovery | list_tools, tool_registry, plugin_storage | 是 | Fetch detailed tool help. |
| `agena.tools.tags` | read_only, discovery | list_tools, tool_registry | 是 | List tool tags with pagination. |
| `agena.tools.call` | discovery | list_tools, invoke_tool, plugin_storage | 否 | Invoke a tool after reading its help. |

### `agena.tools.list`

Enumerate current tools.

- Tags：`read_only`、`discovery`
- Capabilities：`list_tools`、`tool_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | Maximum number of tools to return. |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32 | Number of tools to skip before returning results. |
| `tag` | 否 | `string \| null` | — | Optional single tag filter such as `read_only` or `network`. |
| `tags` | 否 | `array \| null` | — | Optional tag filters. When present, all normalized tags must match. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "description": "Maximum number of tools to return.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "offset": {
      "description": "Number of tools to skip before returning results.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "tag": {
      "description": "Optional single tag filter such as `read_only` or `network`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "tags": {
      "description": "Optional tag filters. When present, all normalized tags must match.",
      "items": {
        "type": "string"
      },
      "type": [
        "array",
        "null"
      ],
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ tools[], total, offset, returned, tag?, tags? }`; each tool is `{ name, summary, tags[] }`.

### `agena.tools.search`

Search the current tool catalog.

- Tags：`read_only`、`discovery`
- Capabilities：`list_tools`、`tool_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | Maximum number of search results to return. |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32 | Number of matching tools to skip before returning results. |
| `query` | 否 | `string` | minLength=1; default= | Search text used to rank matching tool names and summaries. |
| `tag` | 否 | `string \| null` | — | Optional single tag filter such as `read_only` or `network`. |
| `tags` | 否 | `array \| null` | — | Optional tag filters. When present, all normalized tags must match. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "description": "Maximum number of search results to return.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "offset": {
      "description": "Number of matching tools to skip before returning results.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "query": {
      "default": "",
      "description": "Search text used to rank matching tool names and summaries.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "tag": {
      "description": "Optional single tag filter such as `read_only` or `network`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "tags": {
      "description": "Optional tag filters. When present, all normalized tags must match.",
      "items": {
        "type": "string"
      },
      "type": [
        "array",
        "null"
      ],
      "x-agena-order": "000004"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ results[], query, tag?, tags?, total, offset, returned }`; `results[]` contains exact tool names.

### `agena.tools.help`

Fetch detailed tool help.

- Tags：`read_only`、`discovery`
- Capabilities：`list_tools`、`tool_registry`、`plugin_storage`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `tool` | 是 | `string` | minLength=1 | Registered gateway-visible tool name to inspect. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "tool": {
      "description": "Registered gateway-visible tool name to inspect.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "tool"
  ],
  "type": "object"
}
```

</details>

输出：

No structured payload; detailed usage, generated/declared examples, help text, and the one-call preflight grant are in `output_text`.

### `agena.tools.tags`

List tool tags with pagination.

- Tags：`read_only`、`discovery`
- Capabilities：`list_tools`、`tool_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32 | Maximum number of tags to return. |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32 | Number of tags to skip before returning results. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "description": "Maximum number of tags to return.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "offset": {
      "description": "Number of tags to skip before returning results.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ tags[], total, offset, returned }`; each tag is `{ tag, tool_count }`.

### `agena.tools.call`

Invoke a tool after reading its help.

- Tags：`discovery`
- Capabilities：`list_tools`、`invoke_tool`、`plugin_storage`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `input` | 是 | `object` | — | Tool input object passed through verbatim to the target tool. |
| `tool` | 是 | `string` | minLength=1 | Registered gateway-visible tool name to invoke. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "input": {
      "description": "Tool input object passed through verbatim to the target tool.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000001"
    },
    "tool": {
      "description": "Registered gateway-visible tool name to invoke.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "tool",
    "input"
  ],
  "type": "object"
}
```

</details>

输出：

Returns the target tool's complete `ToolInvokeOutput` unchanged; consult that target's output entry in this document.

## `agena.web`

Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.web --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.web.fetch` | read_only, network, internet | permission_check | 是 | Fetch one web page and inspect its actual content. |
| `agena.web.crawl` | mutating, network, internet, discovery, filesystem_write | permission_check | 否 | Crawl a site and cache indexed pages locally. |
| `agena.web.search` | read_only, network, internet, discovery | permission_check | 是 | Find candidate public-web pages to fetch. |

### `agena.web.fetch`

Fetch one web page and inspect its actual content.

- Tags：`read_only`、`network`、`internet`
- Capabilities：`permission_check`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use this tool after search when you need evidence from the actual page rather than search snippets. If you already know what facts you need, set `prompt` so Agena prioritizes the most relevant excerpts from the page in the returned text output.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `prompt` | 否 | `string \| null` | minLength=1 | — |
| `render_js` | 否 | `boolean \| null` | — | — |
| `url` | 是 | `string` | minLength=1 | — |
| `use_cache` | 否 | `boolean` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "prompt": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "render_js": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "url": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "use_cache": {
      "type": "boolean",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "url"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ url, canonical_url, title, markdown, content_type, status, truncated, rendered, raw_html_hash, etag?, last_modified?, links[] }`.

### `agena.web.crawl`

Crawl a site and cache indexed pages locally.

- Tags：`mutating`、`network`、`internet`、`discovery`、`filesystem_write`
- Capabilities：`permission_check`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `max_depth` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `max_pages` | 否 | `integer \| null` | minimum=0; format=uint32 | — |
| `render_js` | 否 | `boolean \| null` | — | — |
| `same_host_only` | 否 | `boolean \| null` | — | — |
| `start_url` | 是 | `string` | minLength=1 | — |
| `use_cache` | 否 | `boolean` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "max_depth": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "max_pages": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "render_js": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "same_host_only": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "start_url": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "use_cache": {
      "type": "boolean",
      "x-agena-order": "000004"
    }
  },
  "required": [
    "start_url"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ start_url, engine, rendered, stored_count, cached_count, duplicate_count, near_duplicate_count, pruned_document_count, pruned_document_bytes, failure_count, total_documents, documents[], failures[] }`; each document is `{ id, url, title, depth, fetched_at, chunk_count }`.

### `agena.web.search`

Find candidate public-web pages to fetch.

- Tags：`read_only`、`network`、`internet`、`discovery`
- Capabilities：`permission_check`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use this tool to discover candidate pages, not to answer from result snippets alone. After searching, fetch 1-3 relevant result URLs before answering when the user needs facts, summaries, comparisons, or latest information. Use allowed_domains and blocked_domains to steer source quality.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `allowed_domains` | 否 | `array<string>` | — | — |
| `blocked_domains` | 否 | `array<string>` | — | — |
| `engine` | 否 | `WebSearchEngineSelection \| null` | — | — |
| `max_results` | 否 | `integer \| null` | minimum=0; format=uint32 | Maximum number of results to return. `limit` remains accepted as a<br>backwards-compatible input alias, but is deliberately omitted from<br>the advertised schema so callers see one unambiguous control. |
| `query` | 是 | `string` | minLength=1 | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "WebSearchEngineSelection": {
      "enum": [
        "auto",
        "bing",
        "duckduckgo",
        "baidu"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "allowed_domains": {
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000003"
    },
    "blocked_domains": {
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000004"
    },
    "engine": {
      "anyOf": [
        {
          "$ref": "#/$defs/WebSearchEngineSelection"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_results": {
      "description": "Maximum number of results to return. `limit` remains accepted as a\nbackwards-compatible input alias, but is deliberately omitted from\nthe advertised schema so callers see one unambiguous control.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-aliases": [
        "limit"
      ],
      "x-agena-order": "000001"
    },
    "query": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "query"
  ],
  "type": "object"
}
```

</details>

输出：

`payload`: `{ query, engine, attempted_engines[], results[] }`; each result is `{ title, url, description, source, engine }`.

## 运行时核对与维护

本文是源码与当前二进制 manifest 的静态快照。新增、删除或修改工具后，应至少核对以下命令：

```bash
agena plugin status --format json
agena plugin inspect agena.runtime --format json
agena plugin inspect <plugin-id> --format json
```

模型会话内还可以调用：

- `agena.tools.list`：分页枚举当前实际可见工具。
- `agena.tools.search`：按名称、摘要和 tag 搜索。
- `agena.tools.help`：取得某个工具的实时 schema、示例与 help，并为一次 `agena.tools.call` 建立 preflight。
- `agena.tools.call`：调用 catalog target；返回目标工具原始输出。

工具是否最终对某个模型可见或可执行，还受 plugin disabled/override、model tool profile、权限策略、动态 capability、当前 workspace 与 provider function-name 编码限制影响。
