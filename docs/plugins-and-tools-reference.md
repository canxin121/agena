# Agena 内置插件与工具参考

> 实施后源码快照：2026-07-29；Agena `0.1.0`。当前默认 feature 的 source-level bundled catalog 是 **24 个插件、135 个工具定义、14 个 bundled Skills**。其中只有 `agena.tools` 的 5 个稳定 Tool API handler 会作为 Provider function 暴露；其余 **130 个 execution tools** 全部通过同一套实时发现、capability、permission 与 `tools.call` 执行路径使用。`agena.mcp` 仍以 runtime 是否创建 MCP manager 为条件，`agena.schema_lab` 以 `schema-lab` feature 为条件。
>
> 权威计数和 schema identity 由 [`bundled_capability_manifest()`](../crates/agena-bundled-plugins/src/capability_manifest.rs) 从真实 plugin manifest 与 bundled Skill catalog 生成，不再由本文手工数字充当事实源。受控生成的 [`bundled-capability-identities.json`](generated/bundled-capability-identities.json) 保留 schema hash、definition identity、gateway 标记、tags、权限/Host capability、plugin hook 和完整 Skill 执行元数据，但刻意省略 display summary/description 与可从 tags 推导的 effects，避免普通文案调整造成大段无意义 diff。可用 `agena inspect --json --identity-snapshot` 重新生成并审查。下方较长的逐工具 schema 章节保留的是本轮实施前审计快照；新增工具的当前契约应优先查询 machine-readable manifest 或运行时 `agena.tools.help`。完整实施状态与差距见 [`agent-tool-skill-mcp-gap-analysis-2026-07-25.md`](agent-tool-skill-mcp-gap-analysis-2026-07-25.md#13-实施后状态与剩余差距)。

## 文档范围与约定

- execution tool 的内部工具键为 `agena.<plugin>.<tool>`，例如 `agena.session.rename`；模型通常使用较短的工具名 `plugin.tool`，重名时使用完整工具键。工具名只出现在 `tools_help.tool` 或 `tools_call.tool` 中，不会成为 Provider function name。
- 每个工具的“输入参数”表用于快速阅读；紧随其后的 `input_schema` 是运行时 manifest 暴露给模型的完整 JSON Schema，包含嵌套对象、枚举、默认值与正式约束。
- 插件 manifest 可以声明独立的 `output_schema`；本文的“输出”同时按实际实现记录 `payload` 形状。所有成功调用还共享下面的 `ToolInvokeOutput` envelope，外部 MCP 工具的内部结果由对应 MCP server 决定。
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

## 当前权威插件索引

| Plugin | Tools | 注册条件 | 当前工具摘要 |
| --- | ---: | --- | --- |
| `agena.chatgpt` | 17 | bundled | OpenAI/ChatGPT 官方工具目录：web/file/tool search、code/computer/shell、image、MCP 等 |
| `agena.claude` | 11 | bundled | Claude 官方工具目录：web、computer、bash、text editor、code execution、MCP、memory 等 |
| `agena.code` | 2 | bundled | `search_ast`, `syntax_tree` |
| `agena.context` | 1 | bundled | `status` |
| `agena.cron` | 8 | bundled | `list`, `create`, `delete`, `update`, `pause`, `resume`, `history`, `wakeup` |
| `agena.environment` | 1 | bundled | `wait` |
| `agena.fs` | 9 | bundled | `read`, `glob`, `grep`, `apply_patch`, `write`, `replace`, `read_many`, `stat`, `view_image` |
| `agena.gemini` | 11 | bundled | Gemini 官方工具目录：Google search/maps、URL context、code/computer、image、file/retrieval、MCP 等 |
| `agena.interaction` | 2 | bundled | `ask`, `notify` |
| `agena.lsp` | 5 | bundled | `servers`, `definition`, `references`, `hover`, `diagnostics` |
| `agena.mcp` | 9 | `runtime:mcp-manager` | resources list/templates/read；prompts list/get；tools call/search；servers status/reconnect |
| `agena.memory` | 5 | bundled | `search`, `get`, `list`, `write`, `delete` |
| `agena.notebook` | 1 | bundled | `edit_cell` |
| `agena.plan` | 4 | bundled | `get`, `set`, `update`, `clear` |
| `agena.report` | 1 | bundled | `findings` |
| `agena.schema_lab` | 2 | `feature:schema-lab` | `inspect`, `echo` |
| `agena.session` | 2 | bundled | `get`, `rename` |
| `agena.settings` | 7 | bundled | `get`, `list`, `inspect`, `set`, `delete`, `patch`, `validate` |
| `agena.shell` | 4 | bundled | `run`, `list`, `logs`, `stop`；Monitor 作为 `run.monitor` 参数 |
| `agena.skills` | 4 | bundled | `list`, `get`, `read_resource`, `refresh` |
| `agena.snapshot` | 3 | bundled | `enter`, `exit`, `status` |
| `agena.tasks` | 9 | bundled | `run`, `create`, `list`, `get`, `output`, `cancel`, `message`, `followup`, `wait` |
| `agena.tools` | 5 | bundled | `list`, `search`, `help`, `tags`, `call`（Internal gateway） |
| `agena.web` | 12 | bundled | `fetch`, `crawl`, `search` + browser open/list/close/snapshot/click/type/wait/screenshot/download |
| **合计** | **135** | 24 plugins | 5 gateway + 130 execution tools |

## 历史逐工具契约（实施前审计快照）

以下详细章节用于保留审计证据，其中仍会出现实施前的 62-tool schema。它不是新增能力的完整索引；当前工具和 schema fingerprint 以本页顶部所述 machine-readable manifest 为准。

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
- Help：Use `glob` for focused path discovery before reading or editing files. Results are paginated (default 200, maximum 1000) and dependency/VCS/build directories are skipped unless `include_ignored` is true or the base path explicitly names one.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `include_ignored` | 否 | `boolean` | default=false | Include dependency, VCS, and build-output directories that are skipped by default (`.git`, `node_modules`, `target`, `dist`, and caches). |
| `limit` | 否 | `integer \| null` | minimum=0; format=uint32; default=200; runtime range=1..1000 | Maximum paths to return. Defaults to 200 and cannot exceed 1000. |
| `offset` | 否 | `integer \| null` | minimum=0; format=uint32; default=0 | Number of matching paths to skip before returning results. |
| `path` | 否 | `string \| null` | minLength=1 | Optional base path. Defaults to the workspace root. |
| `pattern` | 是 | `string` | minLength=1 | Glob pattern to match. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "properties": {
    "include_ignored": {
      "default": false,
      "description": "Include dependency, VCS, and build-output directories that are skipped by default (`.git`, `node_modules`, `target`, `dist`, and caches).",
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "limit": {
      "description": "Maximum paths to return. Defaults to 200 and cannot exceed 1000.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "offset": {
      "description": "Number of matching paths to skip before returning results.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
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
- Hooks：`init`、`prompt.submit`、`chat.system.transform`、`tool.execute.before`、`session.end`、`tool.definition`
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

## `agena.shell`

Shell command execution and background process tools.

- 版本：`0.1.0`
- Hooks：`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.shell --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.shell.run` | mutating, shell, network, filesystem_read | — | 否 | Run one shell process. |
| `agena.shell.list` | read_only, shell | — | 是 | List active background processes. |
| `agena.shell.logs` | read_only, shell | — | 是 | Read background process logs. |
| `agena.shell.stop` | mutating, shell | — | 否 | Stop one background process. |

### `agena.shell.run`

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

### `agena.shell.list`

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

### `agena.shell.logs`

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

### `agena.shell.stop`

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

## `agena.session`

Runtime session tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.session --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.session.get` | read_only | session_registry | 是 | Inspect the current session metadata. |
| `agena.session.rename` | mutating | session_registry | 否 | Rename the current session. |

### `agena.session.get`

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

### `agena.session.rename`

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

## `agena.interaction`

User interaction tools.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.interaction --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.interaction.ask` | interactive | ask_user | 否 | Ask the user for short structured input. |
| `agena.interaction.notify` | — | — | 是 | Show a non-blocking Markdown notification to the user. |

### `agena.interaction.ask`

Ask the user for short structured input.

- Tags：`interactive`
- Capabilities：`ask_user`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `body_markdown` | 否 | `string` | — | — |
| `auto_resolution_ms` | 否 | `integer \| null` | 60000–600000 | 超时后不再等待用户，自动继续执行。 |
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
        },
        "preview_markdown": {
          "description": "Optional Markdown rendered in a dedicated preview panel while focused.",
          "maxLength": 16000,
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
    "auto_resolution_ms": {
      "description": "Automatically continue without an answer after this many milliseconds.\nValues are limited to 60 seconds through 10 minutes.",
      "format": "uint64",
      "maximum": 600000,
      "minimum": 60000,
      "type": ["integer", "null"],
      "x-agena-order": "000005"
    },
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
      "x-agena-order": "000006"
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

`payload`: `{ answers, timed_out? }`, where `answers` maps each question id to an array of selected/free-text strings. When the optional deadline expires, the successful payload has `timed_out=true` and the agent continues with best judgment. Cancellation is returned as an error.

选项除 `label` 和短 `description` 外还可带 `preview_markdown`。TUI 在焦点停留到该选项时，会打开独立的 Markdown 预览面板；设置 `auto_resolution_ms` 时，问题导航区会显示实时倒计时。

### `agena.interaction.notify`

Show a non-blocking Markdown notification to the user.

- Tags：无
- Capabilities：无
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `title` | 否 | `string` | maxLength=80 | 通知卡标题；为空时按 level 使用默认标题。 |
| `body_markdown` | 是 | `string` | minLength=1; maxLength=16000 | 通知正文。 |
| `level` | 否 | `info \| success \| warning \| error` | `info` | 控制 TUI 图标与颜色。 |

输出：

`payload`: `{ title, body_markdown, level }`。此工具只投递通知卡，不创建待回复请求，也不会阻塞 agent。TUI 折叠态显示一行彩色摘要，展开态使用带边框的 Markdown 卡片。

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

Inspect and edit Agena's global and workspace agena.json settings.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.settings --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.settings.get` | read_only, discovery, settings, settings_read, filesystem_read | read_config | 是 | Read one settings path. |
| `agena.settings.list` | read_only, discovery, settings, settings_read, filesystem_read | read_config | 是 | List settings paths. |
| `agena.settings.inspect` | read_only, discovery, settings, settings_read, filesystem_read | read_config | 是 | Inspect a setting across every config layer. |
| `agena.settings.set` | mutating, filesystem_write, settings, settings_write | read_config, reload_config | 否 | Set one settings value. |
| `agena.settings.delete` | mutating, filesystem_write, settings, settings_write | read_config, reload_config | 否 | Delete one settings value. |
| `agena.settings.patch` | mutating, filesystem_write, settings, settings_write | read_config, reload_config | 否 | Patch settings in agena.json. |
| `agena.settings.validate` | read_only, settings, settings_read, filesystem_read | read_config | 是 | Validate layered agena.json settings. |

### `agena.settings.get`

Read one settings path.

- Tags：`read_only`、`discovery`、`settings`、`settings_read`、`filesystem_read`
- Capabilities：`read_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Use `source=file` with `layer=global|workspace` for persisted values. Effective reads merge both files plus environment and CLI layers; prefer explicit `scope=config|meta` with a relative path.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `layer` | 否 | `SettingsLayer \| null` | default=null | `source=file` 时选择 `global` 或 `workspace` 配置文件。 |
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
    },
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000003"
    },
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

`payload`: `{ config_path, config_found, source, layer?, path?, value }`. Secret values are redacted.

### `agena.settings.list`

List settings paths.

- Tags：`read_only`、`discovery`、`settings`、`settings_read`、`filesystem_read`
- Capabilities：`read_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `layer` | 否 | `SettingsLayer \| null` | default=null | `source=file` 时选择 `global` 或 `workspace` 配置文件。 |
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
    },
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000003"
    },
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
      "x-agena-order": "000004"
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

`payload`: `{ config_path, config_found, source, layer?, path?, items[] }`; each item is `{ path, kind, value? }`. Secret scalar values are redacted.

### `agena.settings.inspect`

Inspect a setting across every config layer.

- Tags：`read_only`、`discovery`、`settings`、`settings_read`、`filesystem_read`
- Capabilities：`read_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`
- Help：Returns the persisted global value, persisted workspace value, effective merged value, source file paths, and applied-layer metadata. Secret values are always redacted.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `path` | 否 | `string \| null` | default=null | 相对于 resolved config 根节点的 settings path。 |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "default": null,
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

`payload`: `{ path?, global, workspace, effective, applied_layers }`; `global` / `workspace` contain `{ layer, config_path, config_found, path?, defined, value }`. All secret values are redacted.

### `agena.settings.set`

Set one settings value.

- Tags：`mutating`、`filesystem_write`、`settings`、`settings_write`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`
- Help：Writes the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing; dry runs request read permission for both config files instead of write permission.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `dry_run` | 否 | `boolean` | default=false | — |
| `layer` | 否 | `SettingsLayer \| null` | — | 写入 `global` 或 `workspace` 配置文件。 |
| `path` | 是 | `string` | minLength=1 | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |
| `value` | 是 | `any` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000003"
    },
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
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
      "x-agena-order": "000005"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
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

`payload`: `{ config_path, config_found, layer, operation, path?, dry_run, changed, created, deleted, validated, reload_requested, reload_required, reload?, previous, current }`; reload is `{ previous_generation, generation, loaded_at }` when performed. Secret values in `previous` / `current` are redacted.

### `agena.settings.delete`

Delete one settings value.

- Tags：`mutating`、`filesystem_write`、`settings`、`settings_write`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`
- Help：Deletes from the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `dry_run` | 否 | `boolean` | default=false | — |
| `layer` | 否 | `SettingsLayer \| null` | — | 修改 `global` 或 `workspace` 配置文件。 |
| `path` | 是 | `string` | minLength=1 | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
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

- Tags：`mutating`、`filesystem_write`、`settings`、`settings_write`
- Capabilities：`read_config`、`reload_config`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`
- Help：Deep-merges a JSON object into the global or workspace config selected by `layer`, then validates the combined layered configuration; null object entries delete keys. Use `dry_run=true` to preview without writing.

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `changes` | 是 | `any` | JSON object | — |
| `dry_run` | 否 | `boolean` | default=false | — |
| `layer` | 否 | `SettingsLayer \| null` | — | 修改 `global` 或 `workspace` 配置文件。 |
| `path` | 否 | `string \| null` | default=null | — |
| `reload` | 否 | `boolean \| null` | — | — |
| `validate` | 否 | `boolean \| null` | — | — |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "changes": {
      "x-agena-order": "000001"
    },
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000003"
    },
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
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
      "x-agena-order": "000005"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
    }
  },
  "required": [
    "changes"
  ],
  "type": "object"
}
```

</details>

输出：

`payload` uses the same settings-edit shape as `set`.

### `agena.settings.validate`

Validate layered agena.json settings.

- Tags：`read_only`、`settings`、`settings_read`、`filesystem_read`
- Capabilities：`read_config`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `layer` | 否 | `SettingsLayer \| null` | default=null | 选择要报告的 `global` 或 `workspace` 文件，并在全局 + 工作区合并上下文中验证。 |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "layer": {
      "anyOf": [
        {
          "$ref": "#/$defs/SettingsLayer"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

</details>

输出：

`payload`: `{ config_path, config_found, layer, valid }`.

## `agena.skills`

Discover and read plain-text skills and slash commands.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`、`tool.definition`
- 描述模式：model=`brief`，UI=`summary`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.skills --format json` 获取完整插件配置 schema。

> 当前实施状态（优先于下方历史 schema 快照）：`agena.skills` 只有 `list/get/read_resource/refresh` 四个 Tool。Skill 是纯文本 instruction package；没有 active status、session 持久状态、隐式路径激活、工具 allowlist 或模型切换。用户可以通过 `/skill` 附加，AI 也可以通过 Tool API 自主发现 `list/get` 并把读到的 body 应用于当前任务。`refresh` 返回 request-driven catalog generation/fingerprint 与 OS watcher generation；config 只支持 catalog roots、disabled names 与 watcher policy。详见 [`configuration.md`](configuration.md#skills-plugin-catalog-policy)。

Web 聊天中的 `/skill` 和 “Attach Skill” 分页调用 `skills.list`、选择后调用
`skills.get`，再把 exact-hash instructions 快照作为用户消息的 `skill_reference` part 发送。该 part
对用户显示为可移除/可排队的 Skill chip，对模型显示为 `<agena_skill_references>` 上下文，并随消息
持久化、回放、导出和 compact。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.skills.list` | read_only, discovery | — | 是 | List discovered skills and slash commands. |
| `agena.skills.get` | read_only, discovery | — | 是 | Read one discovered skill or slash command. |
| `agena.skills.read_resource` | read_only, filesystem_read | — | 是 | Read a bounded UTF-8 resource contained by one skill package. |
| `agena.skills.refresh` | read_only, discovery | — | 是 | Rescan filesystem-backed Skills and report the catalog generation. |

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

Delegated subtask execution tools. This is a synchronous child-task runtime,
not a multi-agent orchestration or Ultra-style scheduling layer.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.tasks --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.tasks.run` | task, subtask | run_subtask, plugin_storage | 否 | Create or resume a delegated subagent task and return its terminal result. |

### `agena.tasks.run`

Create or resume a delegated subagent task.

- Tags：`task`、`subtask`
- Capabilities：`run_subtask`、`plugin_storage`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `description` | 是 | `string` | minLength=1 | Short label for the subtask session. |
| `prompt` | 是 | `string` | minLength=1 | Full instruction payload for the delegated subtask. |
| `access` | 否 | `inherit \| read_only` | default=`inherit` | Capability boundary for this isolated Agena instance. |
| `task_id` | 否 | `string \| null` | non-empty; at most 128 bytes | Resume the child session identified by this parent-scoped task id. |
| `selection` | 否 | `TaskModelSelection \| null` | — | Optional model and mode overrides. |
| `timeout_ms` | 否 | `integer \| null` | `uint64`, minimum=1 | Overall task deadline; timeout cancels the child and returns `timed_out`. |
| `max_tokens` | 否 | `integer \| null` | `uint64`, minimum=1 | Cumulative child token budget, including prompt, output, reasoning, and cache accounting. |
| `max_cost_microusd` | 否 | `integer \| null` | `uint64`, minimum=1 | Cumulative child cost ceiling in millionths of a USD. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "$defs": {
    "TaskModelSelection": {
      "additionalProperties": false,
      "properties": {
        "adapter": { "type": ["string", "null"] },
        "model": { "type": ["string", "null"] },
        "parallel_tool_calls": { "type": ["boolean", "null"] },
        "provider": { "type": ["string", "null"] },
        "speed_mode": { "type": ["string", "null"] },
        "thinking_mode": { "type": ["string", "null"] },
        "verbosity": { "type": ["string", "null"] }
      },
      "type": "object"
    }
  },
  "description": "Input for the delegated `task` subagent command.",
  "properties": {
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
    "access": {
      "default": "inherit",
      "description": "Capability boundary for this isolated Agena instance.",
      "enum": ["inherit", "read_only"],
      "type": "string",
      "x-agena-order": "000002"
    },
    "selection": {
      "anyOf": [
        { "$ref": "#/$defs/TaskModelSelection" },
        { "type": "null" }
      ],
      "description": "Optional model and mode overrides.",
      "x-agena-order": "000004"
    },
    "task_id": {
      "description": "Resume an existing child session by its parent-scoped task id.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "timeout_ms": {
      "description": "Overall task timeout in milliseconds.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "max_tokens": {
      "description": "Cumulative child-completion token budget.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "max_cost_microusd": {
      "description": "Cumulative child-completion cost ceiling in USD micro-units.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000007"
    }
  },
  "required": [
    "description",
    "prompt"
  ],
  "additionalProperties": false,
  "type": "object"
}
```

</details>

输出：

`payload` contains `task_id`, child `session_id`, `parent_session_id`, `access`, terminal `status`, `resumed`, `final_text`, optional `error`, the selected model identity, token usage for this invocation, and its cost in micro-USD. `output_text` is the delegated Agena instance's actual final response (or its terminal error), rather than a spawn acknowledgement.

## `agena.tools`

Tool discovery, help, and Tool API functions.

- 版本：`0.1.0`
- Hooks：`init`、`tool.invoke`
- 描述模式：model=`brief`，UI=`detailed`
- 插件配置：manifest 提供 `config_schema`；本文聚焦工具调用协议，可用 `agena plugin inspect agena.tools --format json` 获取完整插件配置 schema。

### 工具一览

| Tool | Tags | Capabilities | 并发安全 | 摘要 |
| --- | --- | --- | --- | --- |
| `agena.tools.list` | read_only, discovery | list_tools, tool_registry | 是 | Enumerate current tools. |
| `agena.tools.search` | read_only, discovery | list_tools, tool_registry | 是 | Search the Agena execution tools available in this session. |
| `agena.tools.help` | read_only, discovery | list_tools, tool_registry | 是 | Get reusable schema, examples, and usage notes for one Agena execution tool. |
| `agena.tools.tags` | read_only, discovery | list_tools, tool_registry | 是 | List tool tags with pagination. |
| `agena.tools.call` | discovery | list_tools, invoke_tool | 否 | Run one Agena execution tool with complete input; validation errors include tool help for a direct retry. |

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

Search the Agena execution tools available in this session.

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

`payload`: `{ results[], query, tag?, tags?, total, offset, returned }`; `results[]` contains exact current-session tool names. An unknown `tools_call.tool` must return to this search route instead of treating a fuzzy suggestion as schema proof.

### `agena.tools.help`

Get reusable schema, examples, and usage notes for one Agena execution tool.

- Tags：`read_only`、`discovery`
- Capabilities：`list_tools`、`tool_registry`
- Runtime：concurrency_safe=`true`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `tool` | 是 | `string` | minLength=1 | Exact name of the Agena execution tool to inspect, such as `fs.read`; use a name returned by `tools_list` or `tools_search`. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "tool": {
      "description": "Exact name of the Agena execution tool to inspect, such as `fs.read`. Use a name returned by `tools_list` or `tools_search`.",
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

No structured payload; detailed usage, schema-valid generated/declared examples, reusable help text, and exact `tools_call` routing are in `output_text`. Generated examples resolve nested `$ref` definitions and include every required field.

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

Run one Agena execution tool with complete input; validation errors include tool help for a direct retry.

- Tags：`discovery`
- Capabilities：`list_tools`、`invoke_tool`
- Runtime：concurrency_safe=`false`，streaming=`buffered`，strict=`false`

输入参数：

| 参数 | 必填 | 类型 | 默认值/约束 | 说明 |
| --- | --- | --- | --- | --- |
| `input` | 是 | `object` | — | One complete argument object derived from current-session `tools_help` or reusable embedded validation help. Open property names do not permit guessing. |
| `tool` | 是 | `string` | minLength=1 | Exact current-session execution-tool name returned by `tools_list` or `tools_search`; never invent or reuse a name from another product, version, or session. |

<details>
<summary>完整 input_schema</summary>

```json
{
  "additionalProperties": false,
  "properties": {
    "input": {
      "additionalProperties": true,
      "description": "One complete execution-tool argument object. Its keys are intentionally open because every live tool has a different schema; this openness is not permission to guess. Derive it from current-session tools_help or reusable embedded validation help, preserve every required key and task value, and never collapse a populated object to {}. If validation fails, read the embedded help and retry directly without another tools_help call.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000001"
    },
    "tool": {
      "description": "Exact current-session name of the Agena execution tool to run. Obtain it from tools_list or tools_search; never invent or reuse a name from another agent, product, version, or session. The Tool API function name remains tools_call.",
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

On success, returns the execution tool's complete `ToolInvokeOutput` unchanged. If `input` fails the tool's live JSON Schema, the tool is not run and the failed result includes the validation error, complete help with schema-derived usage and schema-valid examples, and a direct `tools_call` retry route; no separate `tools_help` call is needed. If `tool` is unknown, the structured error routes the model through `tools_search` → `tools_help` → corrected `tools_call`; fuzzy suggestions are hints only.

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
agena plugin inspect agena.session --format json
agena plugin inspect agena.interaction --format json
agena plugin inspect <plugin-id> --format json
```

模型会话内还可以调用：

- `tools_list`：分页枚举当前实际可见工具。
- `tools_search`：按任务所需能力、名称、摘要和 tag 搜索；没有当前会话确认过的精确名称时必须先搜索，不能猜工具名。
- `tools_help`：取得某个 execution tool 的实时 schema、schema-valid 示例与可复用 help；第一次调用前若没有当前会话中的完整契约依据，必须先 help。它不是调用授权，也不会被 `tools_call` 消费。
- `tools_tags`：列出可用于发现 execution tool 的 tag。
- `tools_call`：运行 `tool` 指定的 execution tool；`tool` 必须是实时发现的精确名称，`input` 必须来自实时 help 或可复用的内嵌 help。可在一次 help 后调用任意次，也可以对并发安全工具发起完整的并行调用。

unknown tool 的相似名称不证明目标工具或参数结构；模型必须回到 `tools_search`，取得精确名称后再调用 `tools_help`。如果 `tools_call` 的参数校验失败且回执已经内嵌完整 help，则直接按回执修正并重试，不再重复调用 `tools_help`。

Provider 协议和持久化 Tool API identity 只会看到以上五个无点号名称。`session.rename`、
`shell.run` 等名称只作为 `tools_help.tool` / `tools_call.tool` 的 execution-tool 名称；Tool API
definition 不携带 plugin key 或点号 handler identity。这五个协议函数不属于 execution-tool
catalog，也不能成为 `tools_help.tool` 或 `tools_call.tool` 的目标。
工具是否最终对某个模型可见或可执行，还受 plugin disabled/override、execution access、权限策略、动态 capability、当前 workspace 与 Provider 的 tool-calling 能力影响。
