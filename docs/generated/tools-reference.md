# Agena Bundled Tools Reference

> Generated file — do not edit by hand. Regenerate with:
>
> ```bash
> agena inspect --tools-reference > docs/generated/tools-reference.md
> ```

This document is deterministically generated from the real `agena-bundled-plugins` plugin manifests, covering **24 plugins and 142 tool definitions**.

- Each tool entry includes: name, summary, detailed help (`before_help` / `help` / `after_help`), tags, concurrency / streaming / strict runtime flags, examples, an input parameter table, and the full input / output JSON Schema.
- The `list` / `search` / `help` / `tags` / `call` tools of `agena.tools` are the stable Tool API gateway handlers; all other tools are ordinary execution tools.
- Tool names (`plugin.tool`, full key `agena.<plugin>.<tool>`) appear only in `tools_help.tool` / `tools_call.tool`; they never become Provider function names.

## Table of Contents

- [`agena.chatgpt`](#agenachatgpt) — OpenAI Responses and image service tools exposed as ordinary Agena tools. (17 tools)
- [`agena.claude`](#agenaclaude) — Anthropic Claude server and client tools exposed as ordinary Agena tools. (11 tools)
- [`agena.code`](#agenacode) — Structured code search and syntax inspection tools. (2 tools)
- [`agena.context`](#agenacontext) — Safe context-window budget, model identity, and compaction status. (2 tools)
- [`agena.cron`](#agenacron) — Cron-style and one-shot wakeup scheduling tools. (8 tools)
- [`agena.environment`](#agenaenvironment) — Wait for filesystem, TCP, or HTTP environment readiness. (1 tools)
- [`agena.fs`](#agenafs) — Filesystem command tools for read/search and explicit edits. (9 tools)
- [`agena.gemini`](#agenagemini) — Google Gemini Interactions and image capabilities exposed as ordinary Agena tools. (11 tools)
- [`agena.interaction`](#agenainteraction) — User interaction tools. (2 tools)
- [`agena.lsp`](#agenalsp) — LSP read-only observability and navigation tools. (5 tools)
- [`agena.mcp`](#agenamcp) — MCP discovery and bridge tools. (9 tools)
- [`agena.memory`](#agenamemory) — Persistent memory with searchable retrieval and write tools. (5 tools)
- [`agena.notebook`](#agenanotebook) — Revision-safe Jupyter notebook cell editing. (1 tools)
- [`agena.plan`](#agenaplan) — Plan orchestration and plan-autorun tools. (4 tools)
- [`agena.report`](#agenareport) — Structured review and verification findings. (1 tools)
- [`agena.schema_lab`](#agenaschema_lab) — Deep built-in JSON Schema fixture used to demo and test the structured plugin config editor. (2 tools)
- [`agena.session`](#agenasession) — Runtime session tools. (2 tools)
- [`agena.settings`](#agenasettings) — Inspect and edit Agena's global and workspace agena.json settings. (7 tools)
- [`agena.shell`](#agenashell) — Shell command execution and background process tools. (4 tools)
- [`agena.skills`](#agenaskills) — Discover and read plain-text skills and slash commands. (7 tools)
- [`agena.snapshot`](#agenasnapshot) — Managed snapshot tools backed by Rift or git worktree. (3 tools)
- [`agena.tasks`](#agenatasks) — Delegated subtask orchestration tools. (9 tools)
- [`agena.tools`](#agenatools) — Tool API discovery functions. The runtime resolves tools_call directly to its execution target. (7 tools)
- [`agena.web`](#agenaweb) — Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering. (13 tools)

## agena.chatgpt

**Version** `0.1.0` · **Tools** 17

OpenAI Responses and image service tools exposed as ordinary Agena tools.

### apply_patch

`agena.chatgpt.apply_patch` · **Summary**: Expose OpenAI's apply_patch protocol tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Execute returned apply_patch_call operations through Agena's permission-checked fs.apply_patch path, then continue with apply_patch_call_output items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### code_interpreter

`agena.chatgpt.code_interpreter` · **Summary**: Run Python with OpenAI's hosted code_interpreter tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### computer

`agena.chatgpt.computer` · **Summary**: Run OpenAI's current computer tool and return pending computer calls.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> When the response contains computer_call items, execute the requested actions in Agena's browser/computer environment and call this tool again with previous_response_id plus official computer_call_output items in input_items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### computer_use_preview

`agena.chatgpt.computer_use_preview` · **Summary**: Run OpenAI's computer_use_preview compatibility tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set display_width, display_height, and environment in tool_options. Continue with computer_call_output items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### custom

`agena.chatgpt.custom` · **Summary**: Send an official OpenAI custom tool declaration.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set the official custom tool name, description, and format fields in tool_options; continue custom_tool_call outputs through input_items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### file_search

`agena.chatgpt.file_search` · **Summary**: Search OpenAI vector stores with the official file_search tool.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### function

`agena.chatgpt.function` · **Summary**: Send an official OpenAI function tool declaration.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set tool_options.name, description, parameters, and strict. This remains an ordinary Agena wrapper; returned function calls are continued through input_items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### image_edit

`agena.chatgpt.image_edit` · **Summary**: Edit permitted local images through OpenAI's Images edit endpoint.

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> This convenience entry preserves the official image edit endpoint alongside the Responses image_generation tool. Every input and output path is permission checked.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `images` | `array<string>` | ✓ | — |  |
| `model` | `string / null` | — | — |  |
| `options` | `object` | — | — |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "images": {
      "items": {
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 16,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000001"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    },
    "prompt": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "prompt",
    "images"
  ],
  "type": "object"
}
```

### image_generation

`agena.chatgpt.image_generation` · **Summary**: Generate or edit an image with OpenAI's Responses image_generation tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options supports action, model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size. Returned base64 images are persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### local_shell

`agena.chatgpt.local_shell` · **Summary**: Expose OpenAI's local_shell protocol tool as an ordinary Agena request.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> The provider returns local_shell_call items. Execute them with Agena shell permissions, then continue using previous_response_id and local_shell_call_output items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### mcp

`agena.chatgpt.mcp` · **Summary**: Connect OpenAI Responses to an official remote MCP server or connector.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set server_label and one of server_url, connector_id, or tunnel_id in tool_options. Official allowed_tools, authorization, headers, require_approval, defer_loading, and allowed_callers fields are preserved.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### namespace

`agena.chatgpt.namespace` · **Summary**: Send an official OpenAI namespace tool declaration.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use tool_options to define the namespace and nested tools according to the current Responses schema.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### programmatic_tool_calling

`agena.chatgpt.programmatic_tool_calling` · **Summary**: Enable OpenAI programmatic tool calling.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> This official Responses tool lets generated programs invoke eligible tools. Use input_items to continue any resulting calls.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### shell

`agena.chatgpt.shell` · **Summary**: Expose OpenAI's shell tool with official environment configuration.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options.environment accepts OpenAI local/container environment objects. Execute pending shell_call items under Agena permissions and continue with shell_call_output items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### tool_search

`agena.chatgpt.tool_search` · **Summary**: Use OpenAI hosted or client tool_search.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set tool_options.execution to server or client, plus optional description and parameters. Continue client calls with tool_search_output items in input_items.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### web_search

`agena.chatgpt.web_search` · **Summary**: Use OpenAI's current Responses web_search tool.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Pending calls and response_id are returned for continuation.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### web_search_preview

`agena.chatgpt.web_search_preview` · **Summary**: Use OpenAI's compatibility web_search_preview tool.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Supports official preview fields such as search_content_types, search_context_size, and user_location.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new request. Optional when continuation items are supplied. |
| `request_options` | `object` | — | — | Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected. |
| `stable_instructions` | `string / null` | — | — | Stable developer prefix eligible for an explicit OpenAI cache breakpoint. |
| `tool_options` | `object` | — | — | Official fields merged into this tool's declaration. `type` is protected. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "include": {
      "description": "Optional Responses include selectors.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000007"
    },
    "input_items": {
      "description": "Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "Responses API continuation token from an earlier call.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Instruction for a new request. Optional when continuation items are supplied.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Stable developer prefix eligible for an explicit OpenAI cache breakpoint.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Official fields merged into this tool's declaration. `type` is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

## agena.claude

**Version** `0.1.0` · **Tools** 11

Anthropic Claude server and client tools exposed as ordinary Agena tools.

### advisor

`agena.claude.advisor` · **Summary**: Ask an Anthropic advisor model with Claude's advisor server tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### bash

`agena.claude.bash` · **Summary**: Use Claude's current Bash client tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> The declaration uses bash_20250124. Execute returned bash tool_use blocks through Agena shell permissions, append assistant content and user tool_result content to messages, then call this tool again.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### code_execution

`agena.claude.code_execution` · **Summary**: Run Claude's latest hosted code execution tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### computer

`agena.claude.computer` · **Summary**: Run Claude Computer Use and return pending computer actions.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses computer_20251124. Set display_width_px and display_height_px in tool_options. Agena executors should normalize left_mouse_down/left_mouse_up, drag paths, key combinations, screenshots, zoom, and cursor actions before returning tool_result blocks.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### mcp_toolset

`agena.claude.mcp_toolset` · **Summary**: Configure a Claude MCP toolset.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set tool_options.mcp_server_name plus optional configs and default_config. The wrapper sends the official mcp_toolset declaration and returns approval/tool-use content for continuation.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### memory

`agena.claude.memory` · **Summary**: Use Claude's memory client tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses memory_20250818. Execute returned memory commands against Agena's permission-checked memory store and continue with tool_result blocks.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### text_editor

`agena.claude.text_editor` · **Summary**: Use Claude's current text editor client tool.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses text_editor_20250728 with name str_replace_based_edit_tool. Execute view/create/str_replace/insert operations through Agena filesystem permissions and continue with tool_result blocks.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### tool_search_bm25

`agena.claude.tool_search_bm25` · **Summary**: Use Claude's BM25 deferred tool search.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses tool_search_tool_bm25_20251119. The returned tool_reference/server_tool_use content remains in the provider response and can be continued through messages.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### tool_search_regex

`agena.claude.tool_search_regex` · **Summary**: Use Claude's regex deferred tool search.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses tool_search_tool_regex_20251119 and supports official allowed_callers, cache_control, defer_loading, and strict options.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### web_fetch

`agena.claude.web_fetch` · **Summary**: Fetch web documents with Claude's latest server web fetch.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### web_search

`agena.claude.web_search` · **Summary**: Search the web with Claude's latest server web search.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Full Anthropic messages used to continue tool_use/tool_result loops. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages already contain tool_result continuation. |
| `request_options` | `object` | — | — |  |
| `stable_system` | `string / null` | — | — | Stable system prefix placed before dynamic messages for cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ClaudeCacheTtl": {
      "enum": [
        "disabled",
        "five_minutes",
        "one_hour"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "beta_headers": {
      "description": "Additional official Anthropic beta feature headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    },
    "cache_ttl": {
      "anyOf": [
        {
          "$ref": "#/$defs/ClaudeCacheTtl"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000002"
    },
    "max_tokens": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "messages": {
      "description": "Full Anthropic messages used to continue tool_use/tool_result loops.",
      "items": true,
      "type": "array",
      "x-agena-order": "000007"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "New user instruction. Optional when messages already contain tool_result continuation.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000006"
    },
    "stable_system": {
      "description": "Stable system prefix placed before dynamic messages for cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

## agena.code

**Version** `0.1.0` · **Tools** 2

Structured code search and syntax inspection tools.

### search_ast

`agena.code.search_ast` · **Summary**: Search code structurally with ast-grep.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `language` | `CodeLanguage / null` | — | — |  |
| `limit` | `integer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `pattern` | `string` | ✓ | — |  |

**Input schema**:
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

### syntax_tree

`agena.code.syntax_tree` · **Summary**: Inspect a parsed syntax tree.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `language` | `CodeLanguage / null` | — | — |  |
| `max_depth` | `integer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
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

## agena.context

**Version** `0.1.0` · **Tools** 2

Safe context-window budget, model identity, and compaction status.

### environment

`agena.context.environment` · **Summary**: Inspect the current session environment: working directory, git state, shell, OS, and session identity.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### status

`agena.context.status` · **Summary**: Inspect remaining context budget, model identity, and compaction health without exposing prompts.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

## agena.cron

**Version** `0.1.0` · **Tools** 8

Cron-style and one-shot wakeup scheduling tools.

### create

`agena.cron.create` · **Summary**: Create one cron schedule.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expression` | `string` | ✓ | — | 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`. |
| `max_age_days` | `integer` | — | `7` |  |
| `misfire_policy` | `CronMisfirePolicyInput` | — | `run_once_now` | What to do after a restart when this fire is materially overdue. |
| `prompt` | `string` | ✓ | — | Prompt to enqueue when the job fires. |
| `retry_policy` | `CronRetryPolicyInput` | — | `{"initial_delay_seconds":15,"max_attempts":3,"max_delay_seconds":300,"multiplier":2}` |  |

**Input schema**:
```json
{
  "$defs": {
    "CronMisfirePolicyInput": {
      "description": "What to do after a restart when this fire is materially overdue.",
      "enum": [
        "skip",
        "run_once_now",
        "reschedule"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "CronRetryPolicyInput": {
      "additionalProperties": false,
      "description": "Bounded exponential retry settings for a cron delivery. `max_attempts`\nincludes the initial attempt, so the default permits two retries after the\nnormal delivery attempt.",
      "properties": {
        "initial_delay_seconds": {
          "default": 15,
          "format": "uint32",
          "maximum": 3600,
          "minimum": 1,
          "type": "integer"
        },
        "max_attempts": {
          "default": 3,
          "format": "uint32",
          "maximum": 20,
          "minimum": 1,
          "type": "integer"
        },
        "max_delay_seconds": {
          "default": 300,
          "format": "uint32",
          "maximum": 86400,
          "minimum": 1,
          "type": "integer"
        },
        "multiplier": {
          "default": 2,
          "format": "uint32",
          "maximum": 10,
          "minimum": 1,
          "type": "integer"
        }
      },
      "type": "object",
      "x-agena-order": "000004"
    }
  },
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
    "misfire_policy": {
      "$ref": "#/$defs/CronMisfirePolicyInput",
      "default": "run_once_now",
      "description": "What to do after a restart when this fire is materially overdue."
    },
    "prompt": {
      "description": "Prompt to enqueue when the job fires.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "retry_policy": {
      "$ref": "#/$defs/CronRetryPolicyInput",
      "default": {
        "initial_delay_seconds": 15,
        "max_attempts": 3,
        "max_delay_seconds": 300,
        "multiplier": 2
      }
    }
  },
  "required": [
    "expression",
    "prompt"
  ],
  "type": "object"
}
```

### delete

`agena.cron.delete` · **Summary**: Delete one cron schedule.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
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

### history

`agena.cron.history` · **Summary**: Inspect bounded persisted delivery history for scheduled jobs.

**Tags**: `query` `scheduler`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string / null` | — | — | Restrict history to one job. Omitting it returns newest records across<br>all retained jobs. |
| `limit` | `integer` | — | `50` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Restrict history to one job. Omitting it returns newest records across\nall retained jobs.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "limit": {
      "default": 50,
      "format": "uint32",
      "maximum": 200,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

### list

`agena.cron.list` · **Summary**: List registered cron jobs and wakeups.

**Tags**: `query` `scheduler` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "properties": {},
  "type": "object"
}
```

### pause

`agena.cron.pause` · **Summary**: Pause one scheduled job without deleting it.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
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

### resume

`agena.cron.resume` · **Summary**: Resume one paused scheduled job.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
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

### update

`agena.cron.update` · **Summary**: Update the prompt or schedule parameters of one retained job.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expression` | `string / null` | — | — | Optional replacement cron expression. Valid only for cron jobs. |
| `id` | `string` | ✓ | — |  |
| `max_age_days` | `integer / null` | — | — | Optional replacement retention period. Valid only for cron jobs. |
| `misfire_policy` | `CronMisfirePolicyInput / null` | — | — | Optional replacement recovery policy. Valid only for cron jobs. |
| `prompt` | `string / null` | — | — | Optional replacement prompt. At least one update field is required. |
| `retry_policy` | `CronRetryPolicyInput / null` | — | — | Optional replacement bounded retry policy. Valid only for cron jobs. |

**Input schema**:
```json
{
  "$defs": {
    "CronMisfirePolicyInput": {
      "enum": [
        "skip",
        "run_once_now",
        "reschedule"
      ],
      "type": "string"
    },
    "CronRetryPolicyInput": {
      "additionalProperties": false,
      "description": "Bounded exponential retry settings for a cron delivery. `max_attempts`\nincludes the initial attempt, so the default permits two retries after the\nnormal delivery attempt.",
      "properties": {
        "initial_delay_seconds": {
          "default": 15,
          "format": "uint32",
          "maximum": 3600,
          "minimum": 1,
          "type": "integer"
        },
        "max_attempts": {
          "default": 3,
          "format": "uint32",
          "maximum": 20,
          "minimum": 1,
          "type": "integer"
        },
        "max_delay_seconds": {
          "default": 300,
          "format": "uint32",
          "maximum": 86400,
          "minimum": 1,
          "type": "integer"
        },
        "multiplier": {
          "default": 2,
          "format": "uint32",
          "maximum": 10,
          "minimum": 1,
          "type": "integer"
        }
      },
      "type": "object"
    }
  },
  "properties": {
    "expression": {
      "description": "Optional replacement cron expression. Valid only for cron jobs.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "max_age_days": {
      "description": "Optional replacement retention period. Valid only for cron jobs.",
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "misfire_policy": {
      "anyOf": [
        {
          "$ref": "#/$defs/CronMisfirePolicyInput"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional replacement recovery policy. Valid only for cron jobs.",
      "x-agena-order": "000004"
    },
    "prompt": {
      "description": "Optional replacement prompt. At least one update field is required.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "retry_policy": {
      "anyOf": [
        {
          "$ref": "#/$defs/CronRetryPolicyInput"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional replacement bounded retry policy. Valid only for cron jobs.",
      "x-agena-order": "000005"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

### wakeup

`agena.cron.wakeup` · **Summary**: Create one one-shot wakeup.

**Tags**: `mutate` `scheduler`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `delay_seconds` | `integer` | ✓ | — |  |
| `prompt` | `string` | ✓ | — |  |
| `reason` | `string / null` | — | — | Short reason logged for diagnostics / shown back to the user. |

**Input schema**:
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

## agena.environment

**Version** `0.1.0` · **Tools** 1

Wait for filesystem, TCP, or HTTP environment readiness.

### wait

`agena.environment.wait` · **Summary**: Wait until a path, TCP endpoint, or HTTP health check is ready.

**Tags**: `query` `filesystem` `network`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `condition` | `WaitCondition` | ✓ | — |  |
| `interval_ms` | `integer` | — | `500` |  |
| `timeout_ms` | `integer` | — | `60000` |  |

**Input schema**:
```json
{
  "$defs": {
    "WaitCondition": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "kind": {
              "const": "path",
              "type": "string"
            },
            "path": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "host": {
              "type": "string"
            },
            "kind": {
              "const": "tcp",
              "type": "string"
            },
            "port": {
              "format": "uint16",
              "maximum": 65535,
              "minimum": 0,
              "type": "integer"
            }
          },
          "required": [
            "kind",
            "host",
            "port"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "contains": {
              "type": [
                "string",
                "null"
              ]
            },
            "expected_status": {
              "format": "uint16",
              "maximum": 65535,
              "minimum": 0,
              "type": [
                "integer",
                "null"
              ]
            },
            "kind": {
              "const": "http",
              "type": "string"
            },
            "url": {
              "type": "string"
            }
          },
          "required": [
            "kind",
            "url"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "condition": {
      "$ref": "#/$defs/WaitCondition"
    },
    "interval_ms": {
      "default": 500,
      "format": "uint64",
      "maximum": 30000,
      "minimum": 50,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "timeout_ms": {
      "default": 60000,
      "format": "uint64",
      "maximum": 600000,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "condition"
  ],
  "type": "object"
}
```

## agena.fs

**Version** `0.1.0` · **Tools** 9

Filesystem command tools for read/search and explicit edits.

### apply_patch

`agena.fs.apply_patch` · **Summary**: Apply a text patch to workspace files.

**Tags**: `mutate` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `apply_patch` for explicit text patch operations against workspace files. The `patch` argument is a plain-text patch that MUST start with the exact marker line `*** Begin Patch` and end with the exact marker line `*** End Patch`. Inside, use only these directives: `*** Update File: <path>` followed by `@@`-separated hunks (context lines start with a space, removed lines with `-`, added lines with `+`), `*** Add File: <path>` with every content line prefixed by `+`, or `*** Delete File: <path>`. A patch that does not start with `*** Begin Patch` is rejected. Use paths relative to the workspace root.

**Examples**:
```json
{
  "patch": "*** Begin Patch\n*** Update File: README.md\n@@\n-old line\n+new line\n*** End Patch"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `patch` | `string` | ✓ | — | Unified patch text to apply to the workspace. |

**Input schema**:
```json
{
  "description": "Textual patch payload in the agena patch format. Must start with the exact\nmarker line `*** Begin Patch` and end with the exact marker line\n`*** End Patch`; use `*** Update File:` / `*** Add File:` / `*** Delete File:`\ndirectives with `@@` hunks (context lines start with a space, removed lines\nwith `-`, added lines with `+`).",
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

### glob

`agena.fs.glob` · **Summary**: Find paths with glob patterns.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `glob` for focused path discovery before reading or editing files. Results are paginated (default 200, maximum 1000) and dependency/VCS/build directories are skipped unless `include_ignored` is true or the base path explicitly names one.

**Examples**:
```json
{
  "path": "crates",
  "pattern": "**/*.rs"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include_ignored` | `boolean` | — | `false` | Include dependency, VCS, and build-output directories that are skipped<br>by default (`.git`, `node_modules`, `target`, `dist`, and caches). |
| `limit` | `integer / null` | — | — | Maximum paths to return. Defaults to 200 and cannot exceed 1000. |
| `offset` | `integer / null` | — | — | Number of matching paths to skip before returning results. |
| `path` | `string / null` | — | — | Optional base path. Defaults to the workspace root. |
| `pattern` | `string` | ✓ | — | Glob pattern to match. |

**Input schema**:
```json
{
  "properties": {
    "include_ignored": {
      "default": false,
      "description": "Include dependency, VCS, and build-output directories that are skipped\nby default (`.git`, `node_modules`, `target`, `dist`, and caches).",
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

### grep

`agena.fs.grep` · **Summary**: Search file contents with regex.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `grep` for regex text search. `path` may be a directory (searched recursively) or a single file; it defaults to the workspace root.

**Examples**:
```json
{
  "path": "crates",
  "pattern": "agena_plugin"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `string / null` | — | — | Optional glob filter applied before matching lines. |
| `path` | `string / null` | — | — | Optional target: a directory to search recursively, or a single file.<br>Defaults to the workspace root. |
| `pattern` | `string` | ✓ | — | Regex pattern to search for. |

**Input schema**:
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
      "description": "Optional target: a directory to search recursively, or a single file.\nDefaults to the workspace root.",
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

### read

`agena.fs.read` · **Summary**: Read workspace files.

**Tags**: `query` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).

**Examples**:
```json
{
  "path": "Cargo.toml"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `file_path` | `string` | ✓ | — | File or directory path to read. Relative paths are resolved from the<br>workspace root. |
| `limit` | `integer / null` | — | — | Maximum number of lines or directory entries to return. |
| `mode` | `ReadMode` | — | `auto` | How to render the target: `text`, `attachment`, or `auto`. |
| `offset` | `integer / null` | — | — | 1-based offset for file lines or directory entries. |

**Input schema**:
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

### read_many

`agena.fs.read_many` · **Summary**: Read multiple UTF-8 files within one bounded byte budget.

**Tags**: `query` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_total_bytes` | `integer` | — | `131072` |  |
| `paths` | `array<string>` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "max_total_bytes": {
      "default": 131072,
      "format": "uint32",
      "maximum": 1048576,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "paths": {
      "items": {
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 64,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "paths"
  ],
  "type": "object"
}
```

### replace

`agena.fs.replace` · **Summary**: Replace exact UTF-8 text with occurrence and revision checks.

**Tags**: `mutate` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_occurrences` | `integer` | — | `1` |  |
| `expected_sha256` | `string / null` | — | — |  |
| `new` | `string` | ✓ | — |  |
| `old` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |
| `replace_all` | `boolean` | — | `false` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_occurrences": {
      "default": 1,
      "format": "uint32",
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000003"
    },
    "expected_sha256": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "new": {
      "type": "string",
      "x-agena-order": "000002"
    },
    "old": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "replace_all": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000004"
    }
  },
  "required": [
    "path",
    "old",
    "new"
  ],
  "type": "object"
}
```

### stat

`agena.fs.stat` · **Summary**: Inspect file metadata and an optional SHA-256 revision.

**Tags**: `query` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `hash` | `boolean` | — | `true` |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "hash": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### view_image

`agena.fs.view_image` · **Summary**: Attach a local image for visual inspection with an explicit detail hint.

**Tags**: `query` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `high` |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "low",
        "high",
        "original"
      ],
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "high"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### write

`agena.fs.write` · **Summary**: Create a UTF-8 text file or replace one at an expected revision.

**Tags**: `mutate` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Creating a new file needs no hash. Replacing an existing file requires expected_sha256 from fs.stat, preventing stale or parallel overwrites.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `content` | `string` | ✓ | — |  |
| `create_parents` | `boolean` | — | `false` |  |
| `expected_sha256` | `string / null` | — | — | Required when replacing an existing file. Use the hash returned by<br>`fs.stat` or a prior mutating result. |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "content": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "create_parents": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "expected_sha256": {
      "description": "Required when replacing an existing file. Use the hash returned by\n`fs.stat` or a prior mutating result.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path",
    "content"
  ],
  "type": "object"
}
```

## agena.gemini

**Version** `0.1.0` · **Tools** 11

Google Gemini Interactions and image capabilities exposed as ordinary Agena tools.

### code_execution

`agena.gemini.code_execution` · **Summary**: Run Gemini hosted code execution.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses the official Interactions code_execution declaration. Continue any function calls with function_result steps in input_steps.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### computer_use

`agena.gemini.computer_use` · **Summary**: Run Gemini Computer Use and return official pending calls.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options supports browser/mobile/desktop environments, safety policy controls, prompt-injection detection, and excluded predefined functions. Continue with function_result steps.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### file_search

`agena.gemini.file_search` · **Summary**: Search Gemini File Search stores.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options supports file_search_store_names, metadata_filter, and top_k.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### function

`agena.gemini.function` · **Summary**: Send an official Gemini function declaration through Interactions.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Set the official name, description, and JSON schema fields in tool_options; continue with function_result steps.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### google_maps

`agena.gemini.google_maps` · **Summary**: Use Google Maps grounding through Gemini.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options supports enable_widget, latitude, and longitude.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### google_search

`agena.gemini.google_search` · **Summary**: Search Google with Gemini grounding.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options.search_types accepts web_search, image_search, and enterprise_web_search.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### image_edit

`agena.gemini.image_edit` · **Summary**: Edit permitted local images with Gemini multimodal image generation.

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cached_content` | `string / null` | — | — |  |
| `generation_config` | `object` | — | — |  |
| `images` | `array<string>` | ✓ | — |  |
| `model` | `string / null` | — | — |  |
| `prompt` | `string` | ✓ | — |  |
| `request_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cached_content": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "generation_config": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    },
    "images": {
      "items": {
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 16,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000001"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    }
  },
  "required": [
    "prompt",
    "images"
  ],
  "type": "object"
}
```

### image_generation

`agena.gemini.image_generation` · **Summary**: Generate images with Gemini's image response modality.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses generateContent with responseModalities TEXT and IMAGE. Configure GEMINI_IMAGE_MODEL or input.model. Inline image data is persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cached_content` | `string / null` | — | — | Existing Gemini cachedContents resource name. |
| `generation_config` | `object` | — | — |  |
| `model` | `string / null` | — | — |  |
| `prompt` | `string` | ✓ | — |  |
| `request_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cached_content": {
      "description": "Existing Gemini cachedContents resource name.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "generation_config": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000002"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    }
  },
  "required": [
    "prompt"
  ],
  "type": "object"
}
```

### mcp_server

`agena.gemini.mcp_server` · **Summary**: Connect Gemini to a remote MCP server.

**Tags**: `network` `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> tool_options supports url, name, headers, and allowed_tools according to the current Interactions MCPServer schema.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### retrieval

`agena.gemini.retrieval` · **Summary**: Use Gemini Retrieval across Vertex AI Search, RAG Store, Exa, or Parallel AI Search.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Pass retrieval_types and the official *_search_config fields in tool_options.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### url_context

`agena.gemini.url_context` · **Summary**: Fetch and ground URLs with Gemini URL Context.

**Tags**: `network` `interactive` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Uses the official url_context tool. Put URLs in the prompt or official request fields.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions steps, including function_result callbacks. |
| `model` | `string / null` | — | — |  |
| `previous_interaction_id` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `request_options` | `object` | — | — |  |
| `stable_system_instruction` | `string / null` | — | — | Stable prefix used to improve Gemini implicit cache reuse. |
| `tool_options` | `object` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "input_steps": {
      "description": "Official Interactions steps, including function_result callbacks.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_interaction_id": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_system_instruction": {
      "description": "Stable prefix used to improve Gemini implicit cache reuse.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

## agena.interaction

**Version** `0.1.0` · **Tools** 2

User interaction tools.

### ask

`agena.interaction.ask` · **Summary**: Ask the user for short structured input.

**Tags**: `interactive`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use only when you are blocked on a decision that belongs to the user: a preference, a direction choice, or a choice with no reasonable default. If a sensible default exists or you can verify the answer yourself, proceed instead of asking. Ask all necessary clarifying questions at once. Never use this tool to ask whether you should proceed or to seek plan approval.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `auto_resolution_ms` | `integer / null` | — | — | Automatically continue without an answer after this many milliseconds.<br>Values are limited to 60 seconds through 10 minutes. |
| `body_markdown` | `string` | — | — |  |
| `cancel_label` | `string` | — | — |  |
| `kind` | `string` | — | — |  |
| `questions` | `array<UserInputQuestion>` | — | — |  |
| `submit_label` | `string` | — | — |  |
| `title` | `string` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "UserInputOption": {
      "properties": {
        "description": {
          "type": "string"
        },
        "label": {
          "minLength": 1,
          "type": "string"
        },
        "preview_markdown": {
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
          "type": "boolean"
        },
        "header": {
          "maxLength": 12,
          "type": "string"
        },
        "id": {
          "minLength": 1,
          "type": "string"
        },
        "multiple": {
          "type": "boolean"
        },
        "options": {
          "items": {
            "$ref": "#/$defs/UserInputOption"
          },
          "maxItems": 8,
          "type": "array"
        },
        "question": {
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
      "type": [
        "integer",
        "null"
      ],
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

### notify

`agena.interaction.notify` · **Summary**: Show a non-blocking Markdown notification to the user.

**Tags**: `interactive`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `body_markdown` | `string` | ✓ | — | Markdown notification body. This tool never waits for a reply. |
| `level` | `InteractionNotificationLevel` | — | `info` | Visual severity used by the TUI notification card. |
| `title` | `string` | — | — | Short heading displayed in the transcript notification card. |

**Input schema**:
```json
{
  "$defs": {
    "InteractionNotificationLevel": {
      "description": "Visual severity used by the TUI notification card.",
      "enum": [
        "info",
        "success",
        "warning",
        "error"
      ],
      "type": "string",
      "x-agena-order": "000002"
    }
  },
  "properties": {
    "body_markdown": {
      "description": "Markdown notification body. This tool never waits for a reply.",
      "maxLength": 16000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "level": {
      "$ref": "#/$defs/InteractionNotificationLevel",
      "default": "info",
      "description": "Visual severity used by the TUI notification card."
    },
    "title": {
      "description": "Short heading displayed in the transcript notification card.",
      "maxLength": 80,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "body_markdown"
  ],
  "type": "object"
}
```

## agena.lsp

**Version** `0.1.0` · **Tools** 5

LSP read-only observability and navigation tools.

### definition

`agena.lsp.definition` · **Summary**: Resolve symbol definitions.

**Tags**: `query` `lsp` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — |  |
| `file_path` | `string` | ✓ | — |  |
| `line` | `integer` | ✓ | — |  |

**Input schema**:
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

### diagnostics

`agena.lsp.diagnostics` · **Summary**: Fetch file diagnostics.

**Tags**: `query` `lsp` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `file_path` | `string` | ✓ | — |  |

**Input schema**:
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

### hover

`agena.lsp.hover` · **Summary**: Fetch hover text.

**Tags**: `query` `lsp` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — |  |
| `file_path` | `string` | ✓ | — |  |
| `line` | `integer` | ✓ | — |  |

**Input schema**:
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

### references

`agena.lsp.references` · **Summary**: Find symbol references.

**Tags**: `query` `lsp` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — |  |
| `file_path` | `string` | ✓ | — |  |
| `include_declaration` | `boolean` | — | `true` |  |
| `line` | `integer` | ✓ | — |  |

**Input schema**:
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

### servers

`agena.lsp.servers` · **Summary**: List configured language servers.

**Tags**: `query` `lsp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

## agena.mcp

**Version** `0.1.0` · **Tools** 9 · **Condition** `runtime:mcp-manager`

MCP discovery and bridge tools.

### prompts.get

`agena.mcp.prompts.get` · **Summary**: Fetch one MCP prompt template.

**Tags**: `query` `mcp`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `arguments` | `object / null` | — | `null` |  |
| `name` | `string` | ✓ | — |  |
| `server` | `string` | ✓ | — |  |

**Input schema**:
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

### prompts.list

`agena.mcp.prompts.list` · **Summary**: List MCP prompt templates from one server.

**Tags**: `query` `mcp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cursor` | `string / null` | — | — |  |
| `server` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cursor": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
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

### resources.list

`agena.mcp.resources.list` · **Summary**: List MCP resources from one server.

**Tags**: `query` `mcp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cursor` | `string / null` | — | — |  |
| `server` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cursor": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
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

### resources.read

`agena.mcp.resources.read` · **Summary**: Read one MCP resource by URI.

**Tags**: `query` `mcp`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `server` | `string` | ✓ | — |  |
| `uri` | `string` | ✓ | — |  |

**Input schema**:
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

### resources.templates.list

`agena.mcp.resources.templates.list` · **Summary**: List MCP resource templates from one server.

**Tags**: `query` `mcp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cursor` | `string / null` | — | — |  |
| `server` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cursor": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
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

### servers.reconnect

`agena.mcp.servers.reconnect` · **Summary**: Reconnect one configured MCP server and refresh its tool cache.

**Tags**: `mutate` `mcp`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `server` | `string` | ✓ | — |  |

**Input schema**:
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

### servers.status

`agena.mcp.servers.status` · **Summary**: Inspect configured MCP connection health and discovered tool counts.

**Tags**: `query` `mcp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### tools.call

`agena.mcp.tools.call` · **Summary**: Call one discovered MCP tool.

**Tags**: `execute` `mcp`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `arguments` | `any` | — | `null` |  |
| `name` | `string` | ✓ | — |  |
| `server` | `string` | ✓ | — |  |

**Input schema**:
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

### tools.search

`agena.mcp.tools.search` · **Summary**: Search the current MCP tool index without expanding all schemas.

**Tags**: `query` `mcp` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer` | — | `20` |  |
| `query` | `string` | — | `` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "limit": {
      "default": 20,
      "format": "uint32",
      "maximum": 100,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "query": {
      "default": "",
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

## agena.memory

**Version** `0.1.0` · **Tools** 5

Persistent memory with searchable retrieval and write tools.

### delete

`agena.memory.delete` · **Summary**: Delete one durable memory record.

**Tags**: `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — |  |

**Input schema**:
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

### get

`agena.memory.get` · **Summary**: Read one durable memory record.

**Tags**: `query` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — |  |

**Input schema**:
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

### list

`agena.memory.list` · **Summary**: List durable memory records.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — |  |

**Input schema**:
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

### search

`agena.memory.search` · **Summary**: Search durable memory records.

**Tags**: `query` `filesystem` `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — |  |
| `query` | `string` | ✓ | — |  |

**Input schema**:
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

### write

`agena.memory.write` · **Summary**: Write one durable memory record.

**Tags**: `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `content` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `memory_type` | `MemoryType / null` | — | — |  |
| `name` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "MemoryType": {
      "description": "Classification stored in a persistent memory document's frontmatter.",
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

## agena.notebook

**Version** `0.1.0` · **Tools** 1

Revision-safe Jupyter notebook cell editing.

### edit_cell

`agena.notebook.edit_cell` · **Summary**: Replace, insert, or delete one Jupyter notebook cell with a revision check.

**Tags**: `mutate` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `NotebookEditAction` | ✓ | — |  |
| `cell_index` | `integer` | ✓ | — |  |
| `cell_type` | `NotebookCellType / null` | — | — |  |
| `expected_sha256` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |
| `preserve_outputs` | `boolean` | — | `true` |  |
| `source` | `string` | — | `` |  |

**Input schema**:
```json
{
  "$defs": {
    "NotebookCellType": {
      "enum": [
        "code",
        "markdown",
        "raw"
      ],
      "type": "string"
    },
    "NotebookEditAction": {
      "enum": [
        "replace",
        "insert_before",
        "insert_after",
        "delete"
      ],
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/NotebookEditAction"
    },
    "cell_index": {
      "format": "uint",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "cell_type": {
      "anyOf": [
        {
          "$ref": "#/$defs/NotebookCellType"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000003"
    },
    "expected_sha256": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000006"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "preserve_outputs": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000005"
    },
    "source": {
      "default": "",
      "type": "string",
      "x-agena-order": "000004"
    }
  },
  "required": [
    "path",
    "action",
    "cell_index",
    "expected_sha256"
  ],
  "type": "object"
}
```

## agena.plan

**Version** `0.1.0` · **Tools** 4

Plan orchestration and plan-autorun tools.

### clear

`agena.plan.clear` · **Summary**: Remove the current plan.

**Tags**: `mutate` `planning`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### get

`agena.plan.get` · **Summary**: Inspect the current plan state.

**Tags**: `query` `planning`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `view` | `PlanGetView` | — | `current` |  |

**Input schema**:
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

### set

`agena.plan.set` · **Summary**: Create or replace the current plan.

**Tags**: `mutate` `planning`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `tasks.run` exploration when the scope spans multiple areas), clarify with `ask`, and refine. Present the finished plan for approval through the plan phase transition; never ask whether the plan is acceptable via `ask`.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `autorun` | `boolean / null` | — | — |  |
| `document_markdown` | `string / null` | — | — |  |
| `objective` | `string` | ✓ | — |  |
| `steps` | `array<WorkflowPlanStepInput>` | — | — | Ordered plan steps. Each step item uses `title`; nested checks use `text`. |
| `title` | `string / null` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "WorkflowPlanCheckpointInput": {
      "additionalProperties": false,
      "description": "Plan check input. Each check item should use `text`.",
      "properties": {
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

### update

`agena.plan.update` · **Summary**: Update the current plan.

**Tags**: `mutate` `planning`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Keep plan-level updates separate from step/check updates: do not send `phase` together with `step`, `check`, `status`, `wait_until_ms`, or `note`. Address steps and checks by 1-based index (`step`, `check`). To complete a plan with steps, mark the required steps/checks `completed` first, then call update separately with `phase: completed`.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `autorun` | `boolean / null` | — | — | Whether an approved active plan should keep running automatically. |
| `check` | `integer / null` | — | — | 1-based index of the check within the step to update (1 = first check). Requires `step`. |
| `note` | `string / null` | — | — |  |
| `phase` | `WorkflowPlanPhase / null` | — | — | Canonical plan phase. Use `planning`, `active`, `blocked`, `completed`, or `cancelled`. |
| `status` | `WorkflowPlanStepStatus / null` | — | — |  |
| `step` | `integer / null` | — | — | 1-based index of the step to update (1 = first step). |
| `summary` | `string / null` | — | — | Optional completion summary. This is only applied when `phase` is `completed`. |
| `wait_until_ms` | `integer / null` | — | — |  |

**Input schema**:
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
  "description": "Update the current plan. Use `phase` / `autorun` for plan-level state changes, `step` + `status` to update a step, or `step` + `check` + `status` to update a check. Steps and checks are addressed by their 1-based index (step 1 is the first step; check 1 is the first check within the step). Do not combine plan-level fields (`phase`, `autorun`, `summary`) with step/check fields. To complete a plan with steps, first mark the relevant steps or checks `completed`, then make a separate plan-level update with `phase: completed`. Canonical phase values are `planning`, `active`, `blocked`, `completed`, and `cancelled`.",
  "properties": {
    "autorun": {
      "description": "Whether an approved active plan should keep running automatically.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "check": {
      "description": "1-based index of the check within the step to update (1 = first check). Requires `step`.",
      "format": "uint",
      "minimum": 0,
      "type": [
        "integer",
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
    "step": {
      "description": "1-based index of the step to update (1 = first step).",
      "format": "uint",
      "minimum": 0,
      "type": [
        "integer",
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

## agena.report

**Version** `0.1.0` · **Tools** 1

Structured review and verification findings.

### findings

`agena.report.findings` · **Summary**: Publish structured file-and-line findings for UI and integrations.

**Tags**: `mutate` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `findings` | `array<Finding>` | — | `[]` |  |
| `summary` | `string` | — | `` |  |

**Input schema**:
```json
{
  "$defs": {
    "Finding": {
      "additionalProperties": false,
      "properties": {
        "body": {
          "minLength": 1,
          "type": "string"
        },
        "code": {
          "type": [
            "string",
            "null"
          ]
        },
        "confidence": {
          "default": 1.0,
          "format": "double",
          "maximum": 1,
          "minimum": 0,
          "type": "number"
        },
        "end_line": {
          "format": "uint32",
          "minimum": 1,
          "type": [
            "integer",
            "null"
          ]
        },
        "file": {
          "minLength": 1,
          "type": "string"
        },
        "line": {
          "format": "uint32",
          "minimum": 1,
          "type": "integer"
        },
        "severity": {
          "$ref": "#/$defs/FindingSeverity"
        },
        "title": {
          "minLength": 1,
          "type": "string"
        }
      },
      "required": [
        "severity",
        "file",
        "line",
        "title",
        "body"
      ],
      "type": "object"
    },
    "FindingSeverity": {
      "enum": [
        "critical",
        "high",
        "medium",
        "low",
        "info"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "findings": {
      "default": [],
      "items": {
        "$ref": "#/$defs/Finding"
      },
      "maxItems": 200,
      "type": "array",
      "x-agena-order": "000001"
    },
    "summary": {
      "default": "",
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

## agena.schema_lab

**Version** `0.1.0` · **Tools** 2 · **Condition** `feature:schema-lab`

Deep built-in JSON Schema fixture used to demo and test the structured plugin config editor.

### echo

`agena.schema_lab.echo` · **Summary**: Echo schema lab input without mutating external state.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Round-trip a label and arbitrary payload into the tool result. The tool is intentionally inert and exists only to populate the Tools tab for the schema lab demo plugin.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `label` | `string / null` | — | — |  |
| `payload` | `any` | — | — |  |

**Input schema**:
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

### inspect

`agena.schema_lab.inspect` · **Summary**: Inspect the schema lab fixture without mutating external state.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Summarize one schema lab config section. The tool is intentionally inert and exists only to populate the Tools tab for the schema lab demo plugin.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include_defaults` | `boolean` | — | `false` |  |
| `section` | `string / null` | — | — |  |

**Input schema**:
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

## agena.session

**Version** `0.1.0` · **Tools** 2

Runtime session tools.

### get

`agena.session.get` · **Summary**: Inspect the current session metadata.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### rename

`agena.session.rename` · **Summary**: Rename the current session.

**Tags**: `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `title` | `string` | ✓ | — |  |

**Input schema**:
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

## agena.settings

**Version** `0.1.0` · **Tools** 7

Inspect and edit Agena's global and workspace agena.json settings.

### delete

`agena.settings.delete` · **Summary**: Delete one settings value.

**Tags**: `mutate` `filesystem` `settings` `settings_write`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Deletes from the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `dry_run` | `boolean` | — | `false` |  |
| `layer` | `SettingsLayer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `reload` | `boolean / null` | — | — |  |
| `validate` | `boolean / null` | — | — |  |

**Input schema**:
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

### get

`agena.settings.get` · **Summary**: Read one settings path.

**Tags**: `query` `discovery` `filesystem` `settings` `settings_read`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use `source=file` with `layer=global|workspace` for persisted values. Effective reads merge both files plus environment and CLI layers; prefer explicit `scope=config|meta` with a relative path.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `layer` | `SettingsLayer / null` | — | `null` |  |
| `path` | `string / null` | — | `null` |  |
| `scope` | `SettingsScope / null` | — | `null` |  |
| `source` | `ConfigSettingsSource / null` | — | `null` |  |

**Input schema**:
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
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
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

### inspect

`agena.settings.inspect` · **Summary**: Inspect a setting across every config layer.

**Tags**: `query` `discovery` `filesystem` `settings` `settings_read`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Returns the persisted global value, persisted workspace value, effective merged value, source file paths, and applied-layer metadata. Secret values are always redacted.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `path` | `string / null` | — | `null` |  |

**Input schema**:
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

### list

`agena.settings.list` · **Summary**: List settings paths.

**Tags**: `query` `discovery` `filesystem` `settings` `settings_read`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `layer` | `SettingsLayer / null` | — | `null` |  |
| `path` | `string / null` | — | `null` |  |
| `recursive` | `boolean / null` | — | `null` |  |
| `scope` | `SettingsScope / null` | — | `null` |  |
| `source` | `ConfigSettingsSource / null` | — | `null` |  |

**Input schema**:
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
    "SettingsLayer": {
      "enum": [
        "global",
        "workspace"
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

### patch

`agena.settings.patch` · **Summary**: Patch settings in agena.json.

**Tags**: `mutate` `filesystem` `settings` `settings_write`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Deep-merges a JSON object into the global or workspace config selected by `layer`, then validates the combined layered configuration; null object entries delete keys. Use `dry_run=true` to preview without writing.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `changes` | `any` | ✓ | — |  |
| `dry_run` | `boolean` | — | `false` |  |
| `layer` | `SettingsLayer / null` | — | — |  |
| `path` | `string / null` | — | `null` |  |
| `reload` | `boolean / null` | — | — |  |
| `validate` | `boolean / null` | — | — |  |

**Input schema**:
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

### set

`agena.settings.set` · **Summary**: Set one settings value.

**Tags**: `mutate` `filesystem` `settings` `settings_write`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Writes the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing; dry runs request read permission for both config files instead of write permission.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `dry_run` | `boolean` | — | `false` |  |
| `layer` | `SettingsLayer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `reload` | `boolean / null` | — | — |  |
| `validate` | `boolean / null` | — | — |  |
| `value` | `any` | ✓ | — |  |

**Input schema**:
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

### validate

`agena.settings.validate` · **Summary**: Validate layered agena.json settings.

**Tags**: `query` `filesystem` `settings` `settings_read`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `layer` | `SettingsLayer / null` | — | `null` |  |

**Input schema**:
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

## agena.shell

**Version** `0.1.0` · **Tools** 4

Shell command execution and background process tools.

### list

`agena.shell.list` · **Summary**: List active background processes.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### logs

`agena.shell.logs` · **Summary**: Read background process logs.

**Tags**: `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — |  |
| `process_id` | `string` | ✓ | — |  |
| `since_seq` | `integer` | — | `0` |  |
| `wait_ms` | `integer` | — | `0` |  |

**Input schema**:
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

### run

`agena.shell.run` · **Summary**: Run one shell process.

**Tags**: `execute`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Run one shell process. Always pass the required `reads` and `writes` path arrays declaring every file or directory the command reads or modifies - empty arrays `[]` when the command touches only its executables (never list the executables). Pass the `network` array of outbound targets (host names, `host:port`, or URLs) the command may connect to - empty array `[]` when none. Set `background = true` to keep the process attached to the session. Add `monitor` for success/failure regex or literal conditions, quiet-period completion, bounded capture, and timeout. Both modes return one `process_id` used by shell.list/logs/stop.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `background` | `boolean` | — | `false` |  |
| `command` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `monitor` | `ShellMonitorInput / null` | — | — |  |
| `network` | `array<string>` | — | `[]` | Outbound network targets the command may connect to: host names,<br>`host:port`, or URLs. Pass an empty array `[]` when the command has no<br>network effect. |
| `reads` | `array<string>` | — | `[]` | Files and directories the command may read. Declare only the actual<br>files/directories affected - never the executables, interpreters, or<br>tools being invoked (e.g. `node`, `python`, `uv`, `git`, `cargo`) or<br>their installation directories. Pass an empty array `[]` when the<br>command reads nothing beyond its executables. |
| `shell` | `ProcessShell` | — | `bash` |  |
| `timeout_ms` | `integer / null` | — | — |  |
| `workdir` | `string / null` | — | — |  |
| `writes` | `array<string>` | — | `[]` | Files and directories the command may create, modify, or delete.<br>Declare only the actual files/directories affected - never the<br>executables, interpreters, or tools being invoked (e.g. `node`,<br>`python`, `uv`, `git`, `cargo`) or their installation directories.<br>Pass an empty array `[]` when the command writes nothing. |

**Input schema**:
```json
{
  "$defs": {
    "ProcessShell": {
      "enum": [
        "bash",
        "powershell"
      ],
      "type": "string",
      "x-agena-order": "000000"
    },
    "ShellMonitorInput": {
      "additionalProperties": false,
      "description": "Optional completion and capture policy for a managed shell process. Adding\nthis object makes `shell.run` a monitored background invocation and returns\nthe same `process_id` consumed by `shell.list`, `shell.logs` and `shell.stop`.",
      "properties": {
        "capture_stderr": {
          "default": true,
          "type": "boolean"
        },
        "failure_pattern": {
          "type": [
            "string",
            "null"
          ]
        },
        "include_pattern": {
          "description": "Optional regex selecting which output lines are retained in the buffer.",
          "type": [
            "string",
            "null"
          ]
        },
        "max_buffered_lines": {
          "format": "uint32",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        },
        "pattern_kind": {
          "$ref": "#/$defs/ShellMonitorPatternKind",
          "default": "regex"
        },
        "persistent": {
          "default": false,
          "description": "Keep running until explicit stop or natural exit, ignoring timeout and\nquiet-period completion. Pattern matches still terminate the monitor.",
          "type": "boolean"
        },
        "quiet_period_ms": {
          "description": "Complete successfully after this many milliseconds without output.",
          "format": "uint64",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        },
        "success_pattern": {
          "type": [
            "string",
            "null"
          ]
        },
        "timeout_ms": {
          "description": "Overall monitor timeout. Defaults to the command timeout, then five minutes.",
          "format": "uint64",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        }
      },
      "type": "object"
    },
    "ShellMonitorPatternKind": {
      "enum": [
        "literal",
        "regex"
      ],
      "type": "string"
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
    "monitor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ShellMonitorInput"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000003"
    },
    "network": {
      "default": [],
      "description": "Outbound network targets the command may connect to: host names,\n`host:port`, or URLs. Pass an empty array `[]` when the command has no\nnetwork effect.",
      "examples": [
        [
          "<target>"
        ]
      ],
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000001.000006"
    },
    "reads": {
      "default": [],
      "description": "Files and directories the command may read. Declare only the actual\nfiles/directories affected - never the executables, interpreters, or\ntools being invoked (e.g. `node`, `python`, `uv`, `git`, `cargo`) or\ntheir installation directories. Pass an empty array `[]` when the\ncommand reads nothing beyond its executables.",
      "examples": [
        [
          "src/lib.rs"
        ]
      ],
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000001.000004"
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
    },
    "writes": {
      "default": [],
      "description": "Files and directories the command may create, modify, or delete.\nDeclare only the actual files/directories affected - never the\nexecutables, interpreters, or tools being invoked (e.g. `node`,\n`python`, `uv`, `git`, `cargo`) or their installation directories.\nPass an empty array `[]` when the command writes nothing.",
      "examples": [
        [
          "target/out.txt"
        ]
      ],
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000001.000005"
    }
  },
  "required": [
    "command"
  ],
  "type": "object"
}
```

### stop

`agena.shell.stop` · **Summary**: Stop one background process.

**Tags**: `mutate` `execute`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `process_id` | `string` | ✓ | — |  |

**Input schema**:
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

## agena.skills

**Version** `0.1.0` · **Tools** 7

Discover and read plain-text skills and slash commands.

### create

`agena.skills.create` · **Summary**: Create a workspace-managed Skill document.

**Tags**: `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Creates `.agena/skills/<name>/SKILL.md` from a complete SKILL.md document. Only workspace-local Skills are mutable; built-in, plugin, user-global, and compatibility Skills remain read-only.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "A complete `SKILL.md` document. Keeping the editor boundary at the native\ndocument format lets callers preserve a Skill's YAML frontmatter alongside\nits Markdown instructions instead of maintaining a second, lossy model.",
  "properties": {
    "document": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### delete

`agena.skills.delete` · **Summary**: Delete a workspace-managed Skill document.

**Tags**: `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Deletes only `.agena/skills/<name>/SKILL.md`; bundled, plugin, user-global, and compatibility Skills cannot be deleted through this tool.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — | Canonical name (or alias) of the workspace-managed Skill to remove. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "description": "Canonical name (or alias) of the workspace-managed Skill to remove.",
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

### get

`agena.skills.get` · **Summary**: Read one discovered skill or slash command.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — |  |

**Input schema**:
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

### list

`agena.skills.list` · **Summary**: List discovered skills and slash commands.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `kind` | `string / null` | — | — |  |
| `limit` | `integer / null` | — | — |  |
| `offset` | `integer / null` | — | — |  |
| `verbose` | `boolean` | — | `false` |  |

**Input schema**:
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

### read_resource

`agena.skills.read_resource` · **Summary**: Read a bounded UTF-8 resource contained by one skill package.

**Tags**: `query` `filesystem`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_bytes` | `integer` | — | `262144` |  |
| `name` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "max_bytes": {
      "default": 262144,
      "format": "uint32",
      "maximum": 1048576,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "name": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "name",
    "path"
  ],
  "type": "object"
}
```

### refresh

`agena.skills.refresh` · **Summary**: Rescan filesystem-backed Skills and report the catalog generation.

**Tags**: `mutate` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `verbose` | `boolean` | — | `false` | Include discovery diagnostics in the human-readable response. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "verbose": {
      "default": false,
      "description": "Include discovery diagnostics in the human-readable response.",
      "type": "boolean",
      "x-agena-order": "000000"
    }
  },
  "type": "object"
}
```

### update

`agena.skills.update` · **Summary**: Update a workspace-managed Skill document.

**Tags**: `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Replaces an existing `.agena/skills/<name>/SKILL.md` document. The replacement frontmatter must keep the same canonical name.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — | Replacement `SKILL.md` document. Its frontmatter name must not change. |
| `name` | `string` | ✓ | — | Canonical name (or alias) of the workspace-managed Skill to replace. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "description": "Replacement `SKILL.md` document. Its frontmatter name must not change.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "name": {
      "description": "Canonical name (or alias) of the workspace-managed Skill to replace.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "name",
    "document"
  ],
  "type": "object"
}
```

## agena.snapshot

**Version** `0.1.0` · **Tools** 3

Managed snapshot tools backed by Rift or git worktree.

### enter

`agena.snapshot.enter` · **Summary**: Enter a managed repository snapshot.

**Tags**: `mutate` `snapshot`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
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

### exit

`agena.snapshot.exit` · **Summary**: Exit a managed repository snapshot.

**Tags**: `mutate` `snapshot`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `discard_changes` | `boolean` | — | `false` |  |
| `exit_action` | `ExitSnapshotAction` | ✓ | — |  |

**Input schema**:
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

### status

`agena.snapshot.status` · **Summary**: List active managed repository snapshots.

**Tags**: `query` `snapshot`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

## agena.tasks

**Version** `0.1.0` · **Tools** 9

Delegated subtask orchestration tools.

### cancel

`agena.tasks.cancel` · **Summary**: Cancel a running delegated task and its child execution.

**Tags**: `subtask` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `task_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "task_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "task_id"
  ],
  "type": "object"
}
```

### create

`agena.tasks.create` · **Summary**: Create a delegated subagent task in the background. Attach Skill names in `skills` so the child session applies them as task guidance.

**Tags**: `subtask` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Creates a delegated background task. Set `skills` to Skill names or aliases (for example a read-only review skill for a review task, or an explore skill for an exploration task); the child session receives the resolved Skill instructions and should follow them. Unknown Skill names are rejected before the subtask starts. Use `agena.skills.list` to discover available Skills.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `access` | `TaskAccess` | — | `inherit` | Hard capability boundary for this delegated Agena instance. |
| `description` | `string` | ✓ | — | Short label for the subtask session. |
| `max_cost_microusd` | `integer / null` | — | — | Cumulative child-completion cost ceiling in USD micro-units (one<br>millionth of a USD). Integer micro-units avoid a floating-point value<br>becoming a durable budget boundary; for example, 250000 means $0.25. |
| `max_tokens` | `integer / null` | — | — | Cumulative child-completion token budget. This includes prompt,<br>output, reasoning and cache token accounting reported by the route. |
| `prompt` | `string` | ✓ | — | Full instruction payload for the delegated subtask. |
| `selection` | `TaskModelSelection / null` | — | — | Optional model and mode overrides. Explicit values take precedence over<br>the parent session. |
| `skills` | `array<string>` | — | — | Optional Skill names or aliases to attach to the delegated subtask's<br>first user message as immutable Skill references. The child session<br>receives the resolved Skill instructions as task guidance and should<br>apply them while completing the task. Use skills appropriate to the<br>task: for example a read-only review task can attach a review/read-only<br>skill, an exploration task can attach an explore skill. Unknown names<br>or aliases are rejected before the subtask starts. |
| `task_id` | `string / null` | — | — | Resume an existing subtask session instead of creating a new one. |
| `timeout_ms` | `integer / null` | — | — | Overall task timeout. A timeout cancels the child execution and returns<br>a structured `timed_out` task result. |

**Input schema**:
```json
{
  "$defs": {
    "TaskAccess": {
      "description": "Hard capability boundary for this delegated Agena instance.",
      "enum": [
        "inherit",
        "read_only"
      ],
      "type": "string",
      "x-agena-order": "000002"
    },
    "TaskModelSelection": {
      "additionalProperties": false,
      "description": "Optional provider/model selection overrides for a delegated task.",
      "properties": {
        "adapter": {
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "type": [
            "string",
            "null"
          ]
        },
        "parallel_tool_calls": {
          "type": [
            "boolean",
            "null"
          ]
        },
        "provider": {
          "type": [
            "string",
            "null"
          ]
        },
        "speed_mode": {
          "type": [
            "string",
            "null"
          ]
        },
        "thinking_mode": {
          "type": [
            "string",
            "null"
          ]
        },
        "verbosity": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "access": {
      "$ref": "#/$defs/TaskAccess",
      "default": "inherit",
      "description": "Hard capability boundary for this delegated Agena instance."
    },
    "description": {
      "description": "Short label for the subtask session.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "max_cost_microusd": {
      "description": "Cumulative child-completion cost ceiling in USD micro-units (one\nmillionth of a USD). Integer micro-units avoid a floating-point value\nbecoming a durable budget boundary; for example, 250000 means $0.25.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000008"
    },
    "max_tokens": {
      "description": "Cumulative child-completion token budget. This includes prompt,\noutput, reasoning and cache token accounting reported by the route.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000007"
    },
    "prompt": {
      "description": "Full instruction payload for the delegated subtask.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "selection": {
      "anyOf": [
        {
          "$ref": "#/$defs/TaskModelSelection"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional model and mode overrides. Explicit values take precedence over\nthe parent session.",
      "x-agena-order": "000005"
    },
    "skills": {
      "description": "Optional Skill names or aliases to attach to the delegated subtask's\nfirst user message as immutable Skill references. The child session\nreceives the resolved Skill instructions as task guidance and should\napply them while completing the task. Use skills appropriate to the\ntask: for example a read-only review task can attach a review/read-only\nskill, an exploration task can attach an explore skill. Unknown names\nor aliases are rejected before the subtask starts.",
      "items": {
        "type": "string"
      },
      "type": [
        "array",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "task_id": {
      "description": "Resume an existing subtask session instead of creating a new one.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "timeout_ms": {
      "description": "Overall task timeout. A timeout cancels the child execution and returns\na structured `timed_out` task result.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    }
  },
  "required": [
    "description",
    "prompt"
  ],
  "type": "object"
}
```

### followup

`agena.tasks.followup` · **Summary**: Resume a terminal delegated task with a follow-up prompt.

**Tags**: `subtask` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_cost_microusd` | `integer / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `prompt` | `string` | ✓ | — |  |
| `task_id` | `string` | ✓ | — |  |
| `timeout_ms` | `integer / null` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "max_cost_microusd": {
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "max_tokens": {
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "prompt": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "task_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "timeout_ms": {
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    }
  },
  "required": [
    "task_id",
    "prompt"
  ],
  "type": "object"
}
```

### get

`agena.tasks.get` · **Summary**: Get delegated task metadata and terminal result.

**Tags**: `subtask` `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `task_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "task_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "task_id"
  ],
  "type": "object"
}
```

### list

`agena.tasks.list` · **Summary**: List delegated background tasks.

**Tags**: `subtask` `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `status` | `string / null` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "status": {
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

### message

`agena.tasks.message` · **Summary**: Send additional guidance to a running delegated task.

**Tags**: `subtask` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `message` | `string` | ✓ | — |  |
| `task_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "message": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "task_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "task_id",
    "message"
  ],
  "type": "object"
}
```

### output

`agena.tasks.output` · **Summary**: Read incremental delegated-task transcript output after a cursor.

**Tags**: `subtask` `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cursor` | `integer` | — | `0` |  |
| `limit` | `integer` | — | `100` |  |
| `task_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cursor": {
      "default": 0,
      "format": "int64",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "limit": {
      "default": 100,
      "format": "uint32",
      "maximum": 500,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "task_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "task_id"
  ],
  "type": "object"
}
```

### run

`agena.tasks.run` · **Summary**: Create or resume a delegated subagent task. Attach Skill names in `skills` so the child session applies them as task guidance.

**Tags**: `subtask` `execute`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Reach for this tool when the work matches an available Skill or subagent type, when you have independent work to run in parallel, or when answering would mean reading across several files — delegate it and you keep the conclusion, not the file dumps. For a single-fact lookup where you already know the file, symbol, or value, search directly; once you have delegated a search, do not also run it yourself — wait for the result. Do small tasks yourself instead of delegating; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; do not redo work you already delegated. Never delegate understanding: brief the subagent with concrete file paths, line numbers, and what to change, then check its result. Delegates a bounded task to a subagent session. Set `skills` to Skill names or aliases (for example a read-only review skill for a review task, or an explore skill for an exploration task); the child session receives the resolved Skill instructions and should follow them. Unknown Skill names are rejected before the subtask starts. Use `agena.skills.list` to discover available Skills.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `access` | `TaskAccess` | — | `inherit` | Hard capability boundary for this delegated Agena instance. |
| `description` | `string` | ✓ | — | Short label for the subtask session. |
| `max_cost_microusd` | `integer / null` | — | — | Cumulative child-completion cost ceiling in USD micro-units (one<br>millionth of a USD). Integer micro-units avoid a floating-point value<br>becoming a durable budget boundary; for example, 250000 means $0.25. |
| `max_tokens` | `integer / null` | — | — | Cumulative child-completion token budget. This includes prompt,<br>output, reasoning and cache token accounting reported by the route. |
| `prompt` | `string` | ✓ | — | Full instruction payload for the delegated subtask. |
| `selection` | `TaskModelSelection / null` | — | — | Optional model and mode overrides. Explicit values take precedence over<br>the parent session. |
| `skills` | `array<string>` | — | — | Optional Skill names or aliases to attach to the delegated subtask's<br>first user message as immutable Skill references. The child session<br>receives the resolved Skill instructions as task guidance and should<br>apply them while completing the task. Use skills appropriate to the<br>task: for example a read-only review task can attach a review/read-only<br>skill, an exploration task can attach an explore skill. Unknown names<br>or aliases are rejected before the subtask starts. |
| `task_id` | `string / null` | — | — | Resume an existing subtask session instead of creating a new one. |
| `timeout_ms` | `integer / null` | — | — | Overall task timeout. A timeout cancels the child execution and returns<br>a structured `timed_out` task result. |

**Input schema**:
```json
{
  "$defs": {
    "TaskAccess": {
      "description": "Hard capability boundary for this delegated Agena instance.",
      "enum": [
        "inherit",
        "read_only"
      ],
      "type": "string",
      "x-agena-order": "000002"
    },
    "TaskModelSelection": {
      "additionalProperties": false,
      "description": "Optional provider/model selection overrides for a delegated task.",
      "properties": {
        "adapter": {
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "type": [
            "string",
            "null"
          ]
        },
        "parallel_tool_calls": {
          "type": [
            "boolean",
            "null"
          ]
        },
        "provider": {
          "type": [
            "string",
            "null"
          ]
        },
        "speed_mode": {
          "type": [
            "string",
            "null"
          ]
        },
        "thinking_mode": {
          "type": [
            "string",
            "null"
          ]
        },
        "verbosity": {
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "access": {
      "$ref": "#/$defs/TaskAccess",
      "default": "inherit",
      "description": "Hard capability boundary for this delegated Agena instance."
    },
    "description": {
      "description": "Short label for the subtask session.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "max_cost_microusd": {
      "description": "Cumulative child-completion cost ceiling in USD micro-units (one\nmillionth of a USD). Integer micro-units avoid a floating-point value\nbecoming a durable budget boundary; for example, 250000 means $0.25.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000008"
    },
    "max_tokens": {
      "description": "Cumulative child-completion token budget. This includes prompt,\noutput, reasoning and cache token accounting reported by the route.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000007"
    },
    "prompt": {
      "description": "Full instruction payload for the delegated subtask.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "selection": {
      "anyOf": [
        {
          "$ref": "#/$defs/TaskModelSelection"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional model and mode overrides. Explicit values take precedence over\nthe parent session.",
      "x-agena-order": "000005"
    },
    "skills": {
      "description": "Optional Skill names or aliases to attach to the delegated subtask's\nfirst user message as immutable Skill references. The child session\nreceives the resolved Skill instructions as task guidance and should\napply them while completing the task. Use skills appropriate to the\ntask: for example a read-only review task can attach a review/read-only\nskill, an exploration task can attach an explore skill. Unknown names\nor aliases are rejected before the subtask starts.",
      "items": {
        "type": "string"
      },
      "type": [
        "array",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "task_id": {
      "description": "Resume an existing subtask session instead of creating a new one.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "timeout_ms": {
      "description": "Overall task timeout. A timeout cancels the child execution and returns\na structured `timed_out` task result.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    }
  },
  "required": [
    "description",
    "prompt"
  ],
  "type": "object"
}
```

### wait

`agena.tasks.wait` · **Summary**: Wait for any or all delegated tasks to finish.

**Tags**: `subtask` `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `mode` | `TaskWaitMode` | — | `all` |  |
| `task_ids` | `array<string>` | ✓ | — |  |
| `timeout_ms` | `integer` | — | `30000` |  |

**Input schema**:
```json
{
  "$defs": {
    "TaskWaitMode": {
      "enum": [
        "any",
        "all"
      ],
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "additionalProperties": false,
  "properties": {
    "mode": {
      "$ref": "#/$defs/TaskWaitMode",
      "default": "all"
    },
    "task_ids": {
      "items": {
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 64,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000000"
    },
    "timeout_ms": {
      "default": 30000,
      "format": "uint64",
      "maximum": 60000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "task_ids"
  ],
  "type": "object"
}
```

## agena.tools

**Version** `0.1.0` · **Tools** 7

Tool API discovery functions. The runtime resolves tools_call directly to its execution target.

### help

`agena.tools.help` · **Tool API gateway handler** · **Summary**: Get reusable schema, examples, and usage notes for one Agena execution tool.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `tool` | `string` | ✓ | — | Exact name of the Agena execution tool to inspect, such as `fs.read`.<br>Use a name returned by `tools_list` or `tools_search`. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "tool": {
      "description": "Exact name of the Agena execution tool to inspect, such as `fs.read`.\nUse a name returned by `tools_list` or `tools_search`.",
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

### list

`agena.tools.list` · **Tool API gateway handler** · **Summary**: Enumerate current tools.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tools to return. |
| `offset` | `integer / null` | — | — | Number of tools to skip before returning results. |
| `plugin` | `string / null` | — | — | Optional plugin filter: only list tools published by this plugin id,<br>such as `agena.fs` or `agena.web`. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
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
    "plugin": {
      "description": "Optional plugin filter: only list tools published by this plugin id,\nsuch as `agena.fs` or `agena.web`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "tag": {
      "description": "Optional single tag filter such as `query` or `network`.",
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

### plugins_list

`agena.tools.plugins_list` · **Summary**: Enumerate the current live plugin inventory with version, summary, tags, and tool count.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tools to return. |
| `offset` | `integer / null` | — | — | Number of tools to skip before returning results. |
| `plugin` | `string / null` | — | — | Optional plugin filter: only list tools published by this plugin id,<br>such as `agena.fs` or `agena.web`. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
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
    "plugin": {
      "description": "Optional plugin filter: only list tools published by this plugin id,\nsuch as `agena.fs` or `agena.web`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "tag": {
      "description": "Optional single tag filter such as `query` or `network`.",
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

### plugins_search

`agena.tools.plugins_search` · **Summary**: Search the loaded plugins by id, summary, or tag.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of search results to return. |
| `offset` | `integer / null` | — | — | Number of matching tools to skip before returning results. |
| `plugin` | `string / null` | — | — | Optional plugin filter: only search tools published by this plugin id,<br>such as `agena.fs` or `agena.web`. |
| `query` | `string` | — | `` | Search text used to rank matching tool names and summaries. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
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
    "plugin": {
      "description": "Optional plugin filter: only search tools published by this plugin id,\nsuch as `agena.fs` or `agena.web`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "query": {
      "default": "",
      "description": "Search text used to rank matching tool names and summaries.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "tag": {
      "description": "Optional single tag filter such as `query` or `network`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
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
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### plugins_tags

`agena.tools.plugins_tags` · **Summary**: List plugin tags with pagination.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tags to return. |
| `offset` | `integer / null` | — | — | Number of tags to skip before returning results. |

**Input schema**:
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

### search

`agena.tools.search` · **Tool API gateway handler** · **Summary**: Search the Agena execution tools available in this session.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of search results to return. |
| `offset` | `integer / null` | — | — | Number of matching tools to skip before returning results. |
| `plugin` | `string / null` | — | — | Optional plugin filter: only search tools published by this plugin id,<br>such as `agena.fs` or `agena.web`. |
| `query` | `string` | — | `` | Search text used to rank matching tool names and summaries. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
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
    "plugin": {
      "description": "Optional plugin filter: only search tools published by this plugin id,\nsuch as `agena.fs` or `agena.web`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "query": {
      "default": "",
      "description": "Search text used to rank matching tool names and summaries.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "tag": {
      "description": "Optional single tag filter such as `query` or `network`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
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
      "x-agena-order": "000005"
    }
  },
  "type": "object"
}
```

### tags

`agena.tools.tags` · **Tool API gateway handler** · **Summary**: List tool tags with pagination.

**Tags**: `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tags to return. |
| `offset` | `integer / null` | — | — | Number of tags to skip before returning results. |

**Input schema**:
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

## agena.web

**Version** `0.1.0` · **Tools** 13

Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering.

### browser_click

`agena.web.browser_click` · **Summary**: Click a browser element selected by CSS or the latest snapshot ref.

**Tags**: `network` `interactive` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `ref` | `integer / null` | — | — | Snapshot-local index returned by `browser_snapshot.elements[].ref`.<br>It is valid only while the page DOM has not materially changed. |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "ref": {
      "description": "Snapshot-local index returned by `browser_snapshot.elements[].ref`.\nIt is valid only while the page DOM has not materially changed.",
      "format": "uint16",
      "maximum": 199,
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-aliases": [
        "element_ref"
      ],
      "x-agena-order": "000002"
    },
    "selector": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "session_id"
  ],
  "type": "object"
}
```

### browser_close

`agena.web.browser_close` · **Summary**: Close one page target in the managed interactive browser.

**Tags**: `network` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `session_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "session_id"
  ],
  "type": "object"
}
```

### browser_download

`agena.web.browser_download` · **Summary**: Download one HTTP(S) URL through a managed browser session and return a local artifact.

**Tags**: `network` `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `session_id` | `string` | ✓ | — | Existing managed browser page to use for the navigation. Its browser<br>profile (for example, authenticated cookies) remains intact. |
| `timeout_ms` | `integer` | — | `30000` |  |
| `url` | `string` | ✓ | — | HTTP(S) download URL. The artifact is always written under the<br>managed workspace artifact directory; callers cannot choose an<br>arbitrary destination path. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "session_id": {
      "description": "Existing managed browser page to use for the navigation. Its browser\nprofile (for example, authenticated cookies) remains intact.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "timeout_ms": {
      "default": 30000,
      "format": "uint64",
      "maximum": 120000,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "url": {
      "description": "HTTP(S) download URL. The artifact is always written under the\nmanaged workspace artifact directory; callers cannot choose an\narbitrary destination path.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "session_id",
    "url"
  ],
  "type": "object"
}
```

### browser_list

`agena.web.browser_list` · **Summary**: List open page targets in the managed interactive browser.

**Tags**: `network` `query` `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### browser_open

`agena.web.browser_open` · **Summary**: Open a page in a managed interactive browser session.

**Tags**: `network` `interactive` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `timeout_ms` | `integer` | — | `30000` |  |
| `url` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "timeout_ms": {
      "default": 30000,
      "format": "uint64",
      "maximum": 120000,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "url": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "url"
  ],
  "type": "object"
}
```

### browser_screenshot

`agena.web.browser_screenshot` · **Summary**: Capture a browser screenshot and return it as an image attachment.

**Tags**: `network` `mutate` `filesystem`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `full_page` | `boolean` | — | `false` |  |
| `path` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "full_page": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
    },
    "path": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "session_id"
  ],
  "type": "object"
}
```

### browser_shutdown

`agena.web.browser_shutdown` · **Summary**: Shut down the managed browser process and all its sessions.

**Tags**: `network` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Closes the underlying Chrome/Chromium process used for rendered fetches and interactive browsing, and removes its temporary profile. All browser sessions are discarded; the next browser_open starts a fresh browser. Use this to release memory without exiting Agena.

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### browser_snapshot

`agena.web.browser_snapshot` · **Summary**: Inspect visible text and interactive elements in a browser session.

**Tags**: `network` `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `session_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "session_id"
  ],
  "type": "object"
}
```

### browser_type

`agena.web.browser_type` · **Summary**: Fill a browser input selected by CSS or the latest snapshot ref, optionally pressing Enter.

**Tags**: `network` `interactive` `mutate`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `press_enter` | `boolean` | — | `false` |  |
| `ref` | `integer / null` | — | — |  |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |
| `text` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "press_enter": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "ref": {
      "format": "uint16",
      "maximum": 199,
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-aliases": [
        "element_ref"
      ],
      "x-agena-order": "000002"
    },
    "selector": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "text": {
      "type": "string",
      "x-agena-order": "000003"
    }
  },
  "required": [
    "session_id",
    "text"
  ],
  "type": "object"
}
```

### browser_wait

`agena.web.browser_wait` · **Summary**: Wait for page readiness, a CSS selector, or visible text.

**Tags**: `network` `query`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |
| `text` | `string / null` | — | — |  |
| `timeout_ms` | `integer` | — | `30000` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "selector": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "text": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "timeout_ms": {
      "default": 30000,
      "format": "uint64",
      "maximum": 120000,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000003"
    }
  },
  "required": [
    "session_id"
  ],
  "type": "object"
}
```

### crawl

`agena.web.crawl` · **Summary**: Crawl a site and cache indexed pages locally.

**Tags**: `discovery`

**Runtime**: ✗ not concurrency-safe · streaming `buffered` · non-strict

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_depth` | `integer / null` | — | — |  |
| `max_pages` | `integer / null` | — | — |  |
| `render_js` | `boolean / null` | — | — |  |
| `same_host_only` | `boolean / null` | — | — |  |
| `start_url` | `string` | ✓ | — |  |
| `use_cache` | `boolean` | — | — |  |

**Input schema**:
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

### fetch

`agena.web.fetch` · **Summary**: Fetch one web page and inspect its actual content.

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use this tool after search when you need evidence from the actual page rather than search snippets. If you already know what facts you need, set `prompt` so Agena prioritizes the most relevant excerpts from the page in the returned text output.

**Examples**:
```json
{
  "url": "https://openai.com"
}
```
```json
{
  "prompt": "extract the release date and breaking changes",
  "url": "https://example.com/docs"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `prompt` | `string / null` | — | — |  |
| `render_js` | `boolean / null` | — | — |  |
| `url` | `string` | ✓ | — |  |
| `use_cache` | `boolean` | — | — |  |

**Input schema**:
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

### search

`agena.web.search` · **Summary**: Find candidate public-web pages to fetch.

**Tags**: `discovery`

**Runtime**: ✓ concurrency-safe · streaming `buffered` · non-strict

**Help**:
> Use this tool to discover candidate pages, not to answer from result snippets alone. After searching, fetch 1-3 relevant result URLs before answering when the user needs facts, summaries, comparisons, or latest information. Use allowed_domains and blocked_domains to steer source quality.

**Examples**:
```json
{
  "max_results": 5,
  "query": "Agena plugin architecture"
}
```
```json
{
  "allowed_domains": [
    "docs.rs",
    "github.com"
  ],
  "query": "Rust schemars derive examples"
}
```

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `allowed_domains` | `array<string>` | — | — |  |
| `blocked_domains` | `array<string>` | — | — |  |
| `engine` | `WebSearchEngineSelection / null` | — | — |  |
| `max_results` | `integer / null` | — | — | Maximum number of results to return. `limit` remains accepted as a<br>backwards-compatible input alias, but is deliberately omitted from<br>the advertised schema so callers see one unambiguous control. |
| `query` | `string` | ✓ | — |  |

**Input schema**:
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

