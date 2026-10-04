# Agena Bundled Tools Reference

> Generated file — do not edit by hand. Regenerate with:
>
> ```bash
> agena inspect --tools-reference > crates/agena-bundled-plugins/generated/tools-reference.md
> ```

This document is deterministically generated from the real `agena-bundled-plugins` plugin manifests, covering **22 plugins and 138 tool definitions**.

- Each tool entry includes: name, summary, detailed help (`before_help` / `help` / `after_help`), tags, the streaming runtime flag, an input parameter table, and the full input / output JSON Schema.
- The `agena.tools` discovery handlers expose Tool API gateway functions (`tools_*` and `plugins_*`); `tools_call` is synthesized by the runtime. All other entries are ordinary execution tools.
- Execution-tool names (`plugin.tool`, full key `agena.<plugin>.<tool>`) appear only in `tools_help.tool` / `tools_call.tool`; they never become Provider function names.

## Table of Contents

- [`agena.chatgpt`](#agenachatgpt) — OpenAI cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback. (11 tools)
- [`agena.claude`](#agenaclaude) — Anthropic cloud search, fetch, computation and advisor capabilities. Inputs leave this computer; no local execution fallback. (9 tools)
- [`agena.code`](#agenacode) — Structured code search and syntax inspection tools. (3 tools)
- [`agena.commands`](#agenacommands) — Declare the built-in commands every Agena client renders locally, and project skills from other agent ecosystems into the same surface. (6 tools)
- [`agena.cron`](#agenacron) — Cron-style and one-shot wakeup scheduling tools. (7 tools)
- [`agena.fs`](#agenafs) — Filesystem command tools for read/search and explicit edits. (9 tools)
- [`agena.gemini`](#agenagemini) — Google cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback. (12 tools)
- [`agena.interaction`](#agenainteraction) — User interaction tools. (2 tools)
- [`agena.lsp`](#agenalsp) — LSP read-only observability and navigation tools. (5 tools)
- [`agena.mcp`](#agenamcp) — MCP discovery and bridge tools. (9 tools)
- [`agena.memory`](#agenamemory) — Persistent memory with searchable retrieval and write tools. (5 tools)
- [`agena.monitor`](#agenamonitor) — Continuous-stream background monitoring tools. (2 tools)
- [`agena.notebook`](#agenanotebook) — Revision-safe Jupyter notebook cell editing. (1 tools)
- [`agena.plan`](#agenaplan) — Plan orchestration and plan-autorun tools. (6 tools)
- [`agena.report`](#agenareport) — Structured review and verification findings. (1 tools)
- [`agena.session`](#agenasession) — Inspect and manage the current runtime session and its environment, model, and token state. (6 tools)
- [`agena.settings`](#agenasettings) — Inspect and edit Agena's global and workspace agena.json settings. (7 tools)
- [`agena.shell`](#agenashell) — Shell command execution and background process tools. (7 tools)
- [`agena.snapshot`](#agenasnapshot) — Managed snapshot tools backed by Rift or git worktree. (3 tools)
- [`agena.tasks`](#agenatasks) — Delegated subtask orchestration tools. (7 tools)
- [`agena.tools`](#agenatools) — Tool API discovery functions. The runtime resolves tools_call directly to its execution target. (7 tools)
- [`agena.web`](#agenaweb) — Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering. (13 tools)

## agena.chatgpt

**Version** `0.1.0` · **Tools** 11

OpenAI cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.

### cloud_code_interpreter

`agena.chatgpt.cloud_code_interpreter` · **Summary**: Run Python in an OpenAI cloud container, not the Agena local workspace.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new hosted request. May be omitted when message history is supplied. |
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
      "description": "Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.",
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
      "description": "Instruction for a new hosted request. May be omitted when message history is supplied.",
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

### cloud_document_understanding

`agena.chatgpt.cloud_document_understanding` · **Summary**: Send explicit PDF/text documents to OpenAI cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_file_delete

`agena.chatgpt.cloud_file_delete` · **Summary**: Request deletion of an owned file from OpenAI cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_search

`agena.chatgpt.cloud_file_search` · **Summary**: Search configured OpenAI cloud file stores, not files on this computer.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new hosted request. May be omitted when message history is supplied. |
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
      "description": "Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.",
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
      "description": "Instruction for a new hosted request. May be omitted when message history is supplied.",
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

### cloud_file_status

`agena.chatgpt.cloud_file_status` · **Summary**: Query the remote status of an owned OpenAI cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_upload

`agena.chatgpt.cloud_file_upload` · **Summary**: Upload one permitted local file to OpenAI cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_sha256` | `string / null` | — | `null` |  |
| `expires_in_seconds` | `integer / null` | — | `null` | Optional provider expiry. OpenAI/Anthropic default to one day.<br>Google uses its own lifecycle and rejects custom expiry. |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "expires_in_seconds": {
      "default": null,
      "description": "Optional provider expiry. OpenAI/Anthropic default to one day.\nGoogle uses its own lifecycle and rejects custom expiry.",
      "format": "uint32",
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
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### cloud_image_edit

`agena.chatgpt.cloud_image_edit` · **Summary**: Upload permitted images for editing in OpenAI cloud; save the returned image separately.

**Tags**: `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to OpenAI; returned images are saved as separate local artifacts. This convenience entry preserves the official image edit endpoint alongside the Responses image_generation tool. Every input and output path is permission checked.

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

### cloud_image_generation

`agena.chatgpt.cloud_image_generation` · **Summary**: Generate images in OpenAI cloud; save returned images as local attachments.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports action, model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size. Returned base64 images are persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new hosted request. May be omitted when message history is supplied. |
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
      "description": "Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.",
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
      "description": "Instruction for a new hosted request. May be omitted when message history is supplied.",
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

### cloud_image_understanding

`agena.chatgpt.cloud_image_understanding` · **Summary**: Send explicit images to OpenAI cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_shell

`agena.chatgpt.cloud_shell` · **Summary**: Run shell commands in an OpenAI cloud container, never in the local terminal.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Defaults to container_auto. Only container_auto or container_reference with container_id is accepted. Local/custom environments and client callbacks are rejected. Uploaded provider files are separate from Agena local files; there is no local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new hosted request. May be omitted when message history is supplied. |
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
      "description": "Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.",
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
      "description": "Instruction for a new hosted request. May be omitted when message history is supplied.",
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

### cloud_web_search

`agena.chatgpt.cloud_web_search` · **Summary**: Search the web in OpenAI cloud and return sources; not a local browser operation.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Hosted results and response_id are returned for follow-up; this plugin never executes client tool callbacks.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used. |
| `previous_response_id` | `string / null` | — | — | Responses API continuation token from an earlier call. |
| `prompt` | `string / null` | — | — | Instruction for a new hosted request. May be omitted when message history is supplied. |
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
      "description": "Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.",
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
      "description": "Instruction for a new hosted request. May be omitted when message history is supplied.",
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

**Version** `0.1.0` · **Tools** 9

Anthropic cloud search, fetch, computation and advisor capabilities. Inputs leave this computer; no local execution fallback.

### cloud_advisor

`agena.claude.cloud_advisor` · **Summary**: Consult an advisor model in Anthropic cloud using the supplied context.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Only supplied context is available; the local repository and session transcript are not automatically uploaded. Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages continue hosted server execution. |
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
      "description": "Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.",
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
      "description": "New user instruction. Optional when messages continue hosted server execution.",
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

### cloud_code_execution

`agena.claude.cloud_code_execution` · **Summary**: Execute code in Anthropic cloud infrastructure, not on this computer.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages continue hosted server execution. |
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
      "description": "Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.",
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
      "description": "New user instruction. Optional when messages continue hosted server execution.",
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

### cloud_document_understanding

`agena.claude.cloud_document_understanding` · **Summary**: Send explicit PDF/text documents to Anthropic cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_file_delete

`agena.claude.cloud_file_delete` · **Summary**: Request deletion of an owned file from Anthropic cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_status

`agena.claude.cloud_file_status` · **Summary**: Query the remote status of an owned Anthropic cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_upload

`agena.claude.cloud_file_upload` · **Summary**: Upload one permitted local file to Anthropic cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_sha256` | `string / null` | — | `null` |  |
| `expires_in_seconds` | `integer / null` | — | `null` | Optional provider expiry. OpenAI/Anthropic default to one day.<br>Google uses its own lifecycle and rejects custom expiry. |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "expires_in_seconds": {
      "default": null,
      "description": "Optional provider expiry. OpenAI/Anthropic default to one day.\nGoogle uses its own lifecycle and rejects custom expiry.",
      "format": "uint32",
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
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### cloud_image_understanding

`agena.claude.cloud_image_understanding` · **Summary**: Send explicit images to Anthropic cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_web_fetch

`agena.claude.cloud_web_fetch` · **Summary**: Fetch and process web content in Anthropic cloud, not through the local browser.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages continue hosted server execution. |
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
      "description": "Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.",
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
      "description": "New user instruction. Optional when messages continue hosted server execution.",
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

### cloud_web_search

`agena.claude.cloud_web_search` · **Summary**: Search the web in Anthropic cloud and return sources; not a local browser operation.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — |  |
| `prompt` | `string / null` | — | — | New user instruction. Optional when messages continue hosted server execution. |
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
      "description": "Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.",
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
      "description": "New user instruction. Optional when messages continue hosted server execution.",
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

**Version** `0.1.0` · **Tools** 3

Structured code search and syntax inspection tools.

### rewrite_ast

`agena.code.rewrite_ast` · **Summary**: Preview or apply a revision-checked ast-grep rewrite in one file.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Defaults to apply=false: returns a bounded unified diff, replacement count and before_sha256 without writing. Repeat with apply=true and expected_sha256 from the reviewed preview to publish. Provide exactly one pattern or structured rule and a replacement template (metavariables supported; empty deletes). Requires valid UTF-8 source, at most 8 MiB/file and 100 non-overlapping matches; rejects parse errors, unknown replacement variables, stale revisions and partial plans. Same file locks and publication checks as fs.replace; no directory-wide rewrite.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `apply` | `boolean` | — | `false` | False previews; true publishes after checking expected_sha256. |
| `expected_sha256` | `string / null` | — | — |  |
| `language` | `CodeLanguage / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `pattern` | `string / null` | — | — |  |
| `replacement` | `string` | ✓ | — | ast-grep replacement template. Empty text deletes matched nodes. |
| `rule` | `any` | — | — | Structured ast-grep rule; the same shape as search_ast.rule. |

**Input schema**:
```json
{
  "$defs": {
    "CodeLanguage": {
      "description": "Language of a code search target.",
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
    "apply": {
      "default": false,
      "description": "False previews; true publishes after checking expected_sha256.",
      "type": "boolean",
      "x-agena-order": "000005"
    },
    "expected_sha256": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "language": {
      "anyOf": [
        {
          "$ref": "#/$defs/CodeLanguage"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000003"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "pattern": {
      "maxLength": 16384,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "replacement": {
      "description": "ast-grep replacement template. Empty text deletes matched nodes.",
      "maxLength": 16384,
      "type": "string",
      "x-agena-order": "000002"
    },
    "rule": {
      "description": "Structured ast-grep rule; the same shape as search_ast.rule.",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "path",
    "replacement"
  ],
  "type": "object",
  "x-agena-relations": [
    "exactly_one_of: `pattern`, `rule`"
  ]
}
```

### search_ast

`agena.code.search_ast` · **Summary**: Search code structurally with ast-grep.

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly. Provide exactly one of pattern or a structured ast-grep rule object; relational/composite rules are supported. Rule bounds: 16 KiB, 16 levels, 512 values. Search returns at most 100 matches, explicitly marks incomplete scans, and flags shortened text previews. Use rewrite_ast to preview a single-file structural edit.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `language` | `CodeLanguage / null` | — | — |  |
| `limit` | `integer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `pattern` | `string / null` | — | — | Simple ast-grep pattern; provide exactly one of pattern or rule. |
| `rule` | `any` | — | — | Structured ast-grep rule object (kind, pattern, all/any/not, inside/has, etc.). |

**Input schema**:
```json
{
  "$defs": {
    "CodeLanguage": {
      "description": "Language of a code search target.",
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
      "x-agena-order": "000003"
    },
    "limit": {
      "format": "uint32",
      "maximum": 100,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "pattern": {
      "description": "Simple ast-grep pattern; provide exactly one of pattern or rule.",
      "maxLength": 16384,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "rule": {
      "description": "Structured ast-grep rule object (kind, pattern, all/any/not, inside/has, etc.).",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "path"
  ],
  "type": "object",
  "x-agena-relations": [
    "exactly_one_of: `pattern`, `rule`"
  ]
}
```

### syntax_tree

`agena.code.syntax_tree` · **Summary**: Inspect a parsed syntax tree.

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension. The preview has at most 512 nodes, 50 children per node and max_depth 1–6 (default 2); children_truncated and truncated report omitted descendants. Source files are limited to 8 MiB.

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
      "description": "Language of a code search target.",
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
      "maximum": 6,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
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

## agena.commands

**Version** `0.1.0` · **Tools** 6

Declare the built-in commands every Agena client renders locally, and project skills from other agent ecosystems into the same surface.

### get

`agena.commands.get` · **Summary**: Read one discovered command package.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Read one package's instructions in full and apply them to the current task.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — | Canonical name, or an alias, of the command package to read. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "description": "Canonical name, or an alias, of the command package to read.",
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

### install

`agena.commands.install` · **Summary**: Install a workspace-managed command package document.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Writes `.agena/skills/<name>/SKILL.md` from a complete package document. Only workspace-local packages are mutable; declared, plugin and user-global packages remain read-only.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "A complete package document. Keeping the editor boundary at the native\ndocument format lets callers preserve a package's YAML frontmatter\nalongside its Markdown instructions instead of maintaining a second,\nlossy model.",
  "properties": {
    "document": {
      "type": "string"
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### list

`agena.commands.list` · **Summary**: List discovered command packages.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> List discovered command packages, with paging. Each entry names the package, its source, and whether it can be edited in this workspace.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — |  |
| `offset` | `integer / null` | — | — |  |
| `verbose` | `boolean` | — | `false` |  |

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
      ]
    },
    "offset": {
      "format": "uint32",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ]
    },
    "verbose": {
      "default": false,
      "type": "boolean"
    }
  },
  "type": "object"
}
```

### read_resource

`agena.commands.read_resource` · **Summary**: Read a bounded UTF-8 resource contained by one package.

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

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
      "type": "integer"
    },
    "name": {
      "type": "string"
    },
    "path": {
      "type": "string"
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

`agena.commands.refresh` · **Summary**: Rescan filesystem-backed packages and report the catalog generation.

**Tags**: `mutate` `discovery` `read_only`

**Runtime**: streaming `buffered`

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
      "type": "boolean"
    }
  },
  "type": "object"
}
```

### remove

`agena.commands.remove` · **Summary**: Remove a workspace-managed command package document.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Deletes only `.agena/skills/<name>/SKILL.md`; declared, plugin and user-global packages cannot be removed through this tool.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — | Canonical name (or alias) of the workspace-managed package to remove. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "description": "Canonical name (or alias) of the workspace-managed package to remove.",
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

## agena.cron

**Version** `0.1.0` · **Tools** 7

Cron-style and one-shot wakeup scheduling tools.

### create

`agena.cron.create` · **Summary**: Create one cron schedule.

**Tags**: `mutate` `scheduler`

**Runtime**: streaming `buffered`

**Help**:
> Schedule a recurring wake with a 6-field cron expression (second minute hour day-of-month month day-of-week). Always pass the IANA timezone from environment_context; wall-clock fields are evaluated in that timezone and returned times are explicit RFC 3339 instants. When the job fires while the session is idle, its prompt is appended chronologically as a typed system_notification and wakes the model; never use it to poll. Jobs are session-only and auto-expire after seven days. When the exact time does not matter, avoid :00 and :30 to reduce clumping.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expression` | `string` | ✓ | — | 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`. |
| `max_age_days` | `integer` | — | `7` |  |
| `misfire_policy` | `CronMisfirePolicyInput` | — | `run_once_now` | What to do after a restart when this fire is materially overdue. |
| `prompt` | `string` | ✓ | — | Prompt to enqueue when the job fires. |
| `retry_policy` | `CronRetryPolicyInput` | — | `{"initial_delay_seconds":15,"max_attempts":3,"max_delay_seconds":300,"multiplier":2}` |  |
| `timezone` | `string` | ✓ | — | IANA timezone in which the cron expression is evaluated (for example<br>`Asia/Shanghai`). This is required so local wall-clock requests cannot<br>be silently interpreted as UTC. |

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
      "x-agena-order": "000004"
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
      "x-agena-order": "000005"
    }
  },
  "description": "Input of the cron create tool.",
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
      "x-agena-order": "000003"
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
    },
    "timezone": {
      "description": "IANA timezone in which the cron expression is evaluated (for example\n`Asia/Shanghai`). This is required so local wall-clock requests cannot\nbe silently interpreted as UTC.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "expression",
    "prompt",
    "timezone"
  ],
  "type": "object"
}
```

### delete

`agena.cron.delete` · **Summary**: Delete one cron schedule.

**Tags**: `mutate` `scheduler`

**Runtime**: streaming `buffered`

**Help**:
> Permanently remove a scheduled job from this session. Deleting stops future firings immediately.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "description": "Input of the cron delete tool.",
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

**Tags**: `query` `scheduler` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Read the bounded delivery history (fire times, outcome, last error) for scheduled jobs. Never poll this waiting for a job to fire — the firing itself appends its prompt to the session and wakes you.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string / null` | — | — | Restrict history to one job. Omitting it returns newest records across<br>all retained jobs. |
| `limit` | `integer` | — | `50` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Input of the cron history tool.",
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

**Tags**: `query` `scheduler` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> List every scheduled job registered in this session. Jobs are session-only — they exist for this session's lifetime and are gone when it ends — and recurring jobs auto-expire after seven days. Use this to review schedules you created; never poll it waiting for a job to fire.

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Input of the cron list tool.",
  "properties": {},
  "type": "object"
}
```

### pause

`agena.cron.pause` · **Summary**: Pause one scheduled job without deleting it.

**Tags**: `mutate` `scheduler`

**Runtime**: streaming `buffered`

**Help**:
> Temporarily suspend a job's future firings while keeping its definition. Use resume to start it again.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "description": "Input of the cron job control tool.",
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

**Runtime**: streaming `buffered`

**Help**:
> Re-enable a job that was paused so its future firings happen again.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "description": "Input of the cron job control tool.",
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

**Runtime**: streaming `buffered`

**Help**:
> Change the prompt or cron parameters of an existing job. The updated schedule takes effect for subsequent firings.

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
      "description": "Policy applied when a scheduled job misses its fire time.",
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
  "description": "Input of the cron update tool.",
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

## agena.fs

**Version** `0.1.0` · **Tools** 9

Filesystem command tools for read/search and explicit edits.

### apply_patch

`agena.fs.apply_patch` · **Summary**: Apply a text patch to workspace files.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Use `apply_patch` for explicit text patch operations against workspace files. The `patch` argument is a plain-text patch that MUST start with the exact marker line `*** Begin Patch` and end with the exact marker line `*** End Patch`. Inside, use only these directives: `*** Update File: <path>` followed by `@@`-separated hunks (context lines start with a space, removed lines with `-`, added lines with `+`), `*** Add File: <path>` with every content line prefixed by `+`, or `*** Delete File: <path>`. A patch that does not start with `*** Begin Patch` is rejected. Use paths relative to the workspace root.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `patch` | `string` | ✓ | — | Unified patch text to apply to the workspace. |

**Input schema**:
```json
{
  "description": "Textual patch payload in the agena patch format. Must start with the exact\nmarker line `*** Begin Patch` and end with the exact marker line\n`*** End Patch`; use `*** Update File:` / `*** Add File:` / `*** Delete File:`\ndirectives with `@@` or `@@ exact context line` hunks. Context lines start\nwith a space, removed lines with `-`, and added lines with `+`. Matching is\nexact and line-oriented; ambiguous targets are rejected. `*** End of File`\nrestricts the last hunk to EOF. `\\ No newline at end of file` after a content\nline specifies a missing final newline. Existing line endings are preserved.",
  "properties": {
    "patch": {
      "description": "Unified patch text to apply to the workspace.",
      "maxLength": 16777216,
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

### document

`agena.fs.document` · **Summary**: Extract or search text in one local PDF/Office document.

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Supported formats: pdf, docx, pptx, xlsx. backend=auto prefers pdftotext for PDFs and otherwise uses selected local MarkItDown converters. MarkItDown needs a Python environment with its format extras; set AGENA_DOCUMENT_PYTHON to that interpreter, or install in the host python3 environment. No dependency installation, plugins, audio/image transcription or remote document service is enabled. Supply pattern to search extracted lines (fixed_strings defaults true); start_line is 1-based, max_lines 1–500. Outputs include source_sha256, extraction warnings, line counts and explicit truncation. Source limit 32 MiB; conversion 30 seconds / 2 MiB per output stream; displayed records 128 KiB. Empty text can indicate a scanned PDF needing OCR. Extracted lines are not source page numbers.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `backend` | `Backend` | — | `auto` |  |
| `fixed_strings` | `boolean` | — | `true` |  |
| `ignore_case` | `boolean` | — | `false` |  |
| `max_lines` | `integer` | — | `100` |  |
| `path` | `string` | ✓ | — |  |
| `pattern` | `string / null` | — | — |  |
| `start_line` | `integer` | — | `1` |  |

**Input schema**:
```json
{
  "$defs": {
    "Backend": {
      "enum": [
        "auto",
        "pdftotext",
        "markitdown"
      ],
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "additionalProperties": false,
  "properties": {
    "backend": {
      "$ref": "#/$defs/Backend",
      "default": "auto"
    },
    "fixed_strings": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000003"
    },
    "ignore_case": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "max_lines": {
      "default": 100,
      "format": "uint",
      "maximum": 500,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000003"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "pattern": {
      "maxLength": 4096,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "start_line": {
      "default": 1,
      "format": "uint",
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### glob

`agena.fs.glob` · **Summary**: Find paths with glob patterns.

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use `glob` for focused path discovery before reading or editing files. Use kind=file/directory/all and exclude globs to narrow results. Results are paginated (default 200, maximum 1000); scans are cancellable, bounded to 100,000 entries / 10 seconds between I/O, and 256 KiB of paths. Pagination is deterministic for an unchanged directory tree. Ripgrep-compatible hidden/ignore rules are applied unless `include_ignored` is true or the base path explicitly names an ignored directory.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `exclude` | `array<string>` | — | — | Exclude matching base-path-relative globs. Exclusions win. |
| `include_ignored` | `boolean` | — | `false` | Include dependency, VCS, and build-output directories that are skipped<br>by default (`.git`, `node_modules`, `target`, `dist`, and caches). |
| `kind` | `GlobKind` | — | `all` | Filter by path kind; defaults to both files and directories. |
| `limit` | `integer / null` | — | — | Maximum paths to return. Defaults to 200 and cannot exceed 1000. |
| `offset` | `integer / null` | — | — | Number of matching paths to skip before returning results. |
| `path` | `string / null` | — | — | Optional base path. Defaults to the workspace root. |
| `pattern` | `string` | ✓ | — | Glob pattern to match. |

**Input schema**:
```json
{
  "$defs": {
    "GlobKind": {
      "description": "Filter by path kind; defaults to both files and directories.",
      "enum": [
        "all",
        "file",
        "directory"
      ],
      "type": "string",
      "x-agena-order": "000005"
    }
  },
  "additionalProperties": false,
  "description": "Input of the glob tool.",
  "properties": {
    "exclude": {
      "description": "Exclude matching base-path-relative globs. Exclusions win.",
      "items": {
        "maxLength": 1024,
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 32,
      "type": "array",
      "x-agena-order": "000006"
    },
    "include_ignored": {
      "default": false,
      "description": "Include dependency, VCS, and build-output directories that are skipped\nby default (`.git`, `node_modules`, `target`, `dist`, and caches).",
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "kind": {
      "$ref": "#/$defs/GlobKind",
      "default": "all",
      "description": "Filter by path kind; defaults to both files and directories."
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
      "x-agena-order": "000001"
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

`agena.fs.grep` · **Summary**: Search text with ripgrep, returning lines, paths, or counts.

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use regex or fixed_strings with case=sensitive/insensitive/smart. Pattern whitespace is significant. mode=content returns structured lines with optional before_context/after_context (0–20); files returns each matching path once; count returns matching-line counts per file, omitting zeroes. max_results is global (1–500): lines for content, files otherwise, never a per-file count cap. include and includes are ORed relative-path globs; exclude wins. Hidden/ignored paths follow ripgrep rules unless explicitly targeted or include_ignored=true. Search is bounded to 32 MiB/file, 256 MiB total, 25,000 files, 100,000 entries, 20 seconds between I/O/callbacks, and 256 KiB of records. Lines over 4 KiB are visibly shortened; lines beyond the 2 MiB search buffer may be skipped. scan_complete distinguishes an incomplete scan from clipped display text; partial counts are lower bounds. Files mode stops at its first match. Binary data detected while scanning is excluded. Narrow path or filters if truncated; use fs.read for nearby lines. Blocking filesystem I/O itself has no hard deadline.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `after_context` | `integer` | — | `0` | Context lines after each match (0–20); content mode only. |
| `before_context` | `integer` | — | `0` | Context lines before each match (0–20); content mode only. |
| `case` | `GrepCase` | — | `sensitive` | Sensitive by default; smart ignores case for patterns without uppercase. |
| `exclude` | `array<string>` | — | — | Exclude relative-path globs; exclusions override all includes. |
| `fixed_strings` | `boolean` | — | `false` | Treat pattern literally instead of as a regex. |
| `include` | `string / null` | — | — | Optional glob filter applied before matching lines. |
| `include_ignored` | `boolean` | — | `false` | Include hidden and ignored files that are skipped by default according<br>to ripgrep-compatible ignore rules. |
| `includes` | `array<string>` | — | — | Additional include globs, ORed with `include`. An empty set includes all. |
| `max_results` | `integer / null` | — | — | Global matching-line limit in content mode, matching-file limit otherwise.<br>Defaults to 500; 1–500. Context lines do not consume this limit. |
| `mode` | `GrepMode` | — | `content` | Content, matching file paths, or matching-line counts per file. |
| `path` | `string / null` | — | — | Optional target: a directory to search recursively, or a single file.<br>Defaults to the workspace root. |
| `pattern` | `string` | ✓ | — | Regex or fixed string to search for. Whitespace is significant. |

**Input schema**:
```json
{
  "$defs": {
    "GrepCase": {
      "description": "Sensitive by default; smart ignores case for patterns without uppercase.",
      "oneOf": [
        {
          "enum": [
            "sensitive",
            "insensitive"
          ],
          "type": "string"
        },
        {
          "const": "smart",
          "description": "Ignore case unless the pattern contains an uppercase character.",
          "type": "string"
        }
      ],
      "x-agena-order": "000005"
    },
    "GrepMode": {
      "description": "Content, matching file paths, or matching-line counts per file.",
      "oneOf": [
        {
          "enum": [
            "content"
          ],
          "type": "string"
        },
        {
          "const": "files",
          "description": "Return each matching file once, stopping at its first matching line.",
          "type": "string"
        },
        {
          "const": "count",
          "description": "Count matching lines per file, omitting files without matches.",
          "type": "string"
        }
      ],
      "x-agena-order": "000006"
    }
  },
  "additionalProperties": false,
  "description": "Input of the grep tool.",
  "properties": {
    "after_context": {
      "default": 0,
      "description": "Context lines after each match (0–20); content mode only.",
      "format": "uint32",
      "maximum": 20,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000010"
    },
    "before_context": {
      "default": 0,
      "description": "Context lines before each match (0–20); content mode only.",
      "format": "uint32",
      "maximum": 20,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000009"
    },
    "case": {
      "$ref": "#/$defs/GrepCase",
      "default": "sensitive",
      "description": "Sensitive by default; smart ignores case for patterns without uppercase."
    },
    "exclude": {
      "description": "Exclude relative-path globs; exclusions override all includes.",
      "items": {
        "maxLength": 1024,
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 32,
      "type": "array",
      "x-agena-order": "000008"
    },
    "fixed_strings": {
      "default": false,
      "description": "Treat pattern literally instead of as a regex.",
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "include": {
      "description": "Optional glob filter applied before matching lines.",
      "maxLength": 1024,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "include_ignored": {
      "default": false,
      "description": "Include hidden and ignored files that are skipped by default according\nto ripgrep-compatible ignore rules.",
      "type": "boolean",
      "x-agena-order": "000003"
    },
    "includes": {
      "description": "Additional include globs, ORed with `include`. An empty set includes all.",
      "items": {
        "maxLength": 1024,
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 32,
      "type": "array",
      "x-agena-order": "000007"
    },
    "max_results": {
      "description": "Global matching-line limit in content mode, matching-file limit otherwise.\nDefaults to 500; 1–500. Context lines do not consume this limit.",
      "format": "uint32",
      "maximum": 500,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000011"
    },
    "mode": {
      "$ref": "#/$defs/GrepMode",
      "default": "content",
      "description": "Content, matching file paths, or matching-line counts per file."
    },
    "path": {
      "description": "Optional target: a directory to search recursively, or a single file.\nDefaults to the workspace root.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "pattern": {
      "description": "Regex or fixed string to search for. Whitespace is significant.",
      "maxLength": 16384,
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

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use `read` for text previews and directory listings. Binary files return local references, not model-visible bytes. Use a provider cloud_image_understanding/cloud_document_understanding tool or explicitly attach media to the composer to send its contents.

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
  "description": "Input of the file read tool.",
  "properties": {
    "file_path": {
      "description": "File or directory path to read. Relative paths are resolved from the\nworkspace root.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
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

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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
      "maxLength": 16777216,
      "type": "string",
      "x-agena-order": "000002"
    },
    "old": {
      "maxLength": 16777216,
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

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

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

### write

`agena.fs.write` · **Summary**: Create a UTF-8 text file or replace one at an expected revision.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

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
      "maxLength": 16777216,
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

**Version** `0.1.0` · **Tools** 12

Google cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.

### cloud_code_execution

`agena.gemini.cloud_code_execution` · **Summary**: Execute code in Google cloud infrastructure, not on this computer.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses the official Interactions code_execution declaration. Computation executes on Google infrastructure; no returned function call is executed by Agena.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
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
      "description": "Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.",
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

### cloud_document_understanding

`agena.gemini.cloud_document_understanding` · **Summary**: Send explicit PDF/text documents to Google cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_file_delete

`agena.gemini.cloud_file_delete` · **Summary**: Request deletion of an owned file from Google cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_search

`agena.gemini.cloud_file_search` · **Summary**: Search configured Google cloud file stores, not files on this computer.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. tool_options supports file_search_store_names, metadata_filter, and top_k.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
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
      "description": "Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.",
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

### cloud_file_status

`agena.gemini.cloud_file_status` · **Summary**: Query the remote status of an owned Google cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `handle` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "handle": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "handle"
  ],
  "type": "object"
}
```

### cloud_file_upload

`agena.gemini.cloud_file_upload` · **Summary**: Upload one permitted local file to Google cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_sha256` | `string / null` | — | `null` |  |
| `expires_in_seconds` | `integer / null` | — | `null` | Optional provider expiry. OpenAI/Anthropic default to one day.<br>Google uses its own lifecycle and rejects custom expiry. |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "expires_in_seconds": {
      "default": null,
      "description": "Optional provider expiry. OpenAI/Anthropic default to one day.\nGoogle uses its own lifecycle and rejects custom expiry.",
      "format": "uint32",
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
      "x-agena-order": "000000"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

### cloud_google_maps

`agena.gemini.cloud_google_maps` · **Summary**: Query Google Maps data in Google cloud and return grounding sources.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports enable_widget, latitude, and longitude.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
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
      "description": "Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.",
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

### cloud_google_search

`agena.gemini.cloud_google_search` · **Summary**: Search Google and ground answers in Google cloud, not the local browser.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options.search_types accepts web_search, image_search, and enterprise_web_search.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
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
      "description": "Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.",
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

### cloud_image_edit

`agena.gemini.cloud_image_edit` · **Summary**: Upload permitted images for editing in Google cloud; save the returned image separately.

**Tags**: `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to Google; returned images are saved as separate local artifacts. Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.

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

### cloud_image_generation

`agena.gemini.cloud_image_generation` · **Summary**: Generate images in Google cloud; save returned images as local attachments.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses generateContent with responseModalities TEXT and IMAGE. Configure GEMINI_IMAGE_MODEL or input.model. Inline image data is persisted as managed attachments.

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

### cloud_image_understanding

`agena.gemini.cloud_image_understanding` · **Summary**: Send explicit images to Google cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` |  |
| `prompt` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ImageDetail": {
      "enum": [
        "auto",
        "low",
        "high"
      ],
      "type": "string",
      "x-agena-order": "000003"
    },
    "MediaSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "expected_sha256": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "path": {
              "type": "string"
            },
            "source": {
              "const": "local",
              "type": "string"
            }
          },
          "required": [
            "source",
            "path"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "handle": {
              "type": "string"
            },
            "source": {
              "const": "cloud",
              "type": "string"
            }
          },
          "required": [
            "source",
            "handle"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "detail": {
      "$ref": "#/$defs/ImageDetail",
      "default": "auto"
    },
    "inputs": {
      "items": {
        "$ref": "#/$defs/MediaSource"
      },
      "type": "array",
      "x-agena-order": "000000"
    },
    "max_output_tokens": {
      "default": 4096,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "prompt": {
      "maxLength": 64000,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "inputs",
    "prompt"
  ],
  "type": "object"
}
```

### cloud_url_context

`agena.gemini.cloud_url_context` · **Summary**: Retrieve and ground URL content in Google cloud; no local-file access.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses the official url_context tool. Put URLs in the prompt or official request fields.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
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
      "description": "Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.",
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

**Runtime**: streaming `buffered`

**Help**:
> Use only when you are blocked on a decision that belongs to the user: a preference, a direction choice, or a choice with no reasonable default. If a sensible default exists or you can verify the answer yourself, proceed instead of asking. Ask all necessary clarifying questions at once. Never use this tool to ask whether you should proceed or to seek plan approval.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `auto_resolution_ms` | `integer / null` | — | — | Automatically continue without an answer after this many milliseconds.<br>Values are limited to 60 seconds through 10 minutes. |
| `body_markdown` | `string` | — | — | Optional Markdown body shown in the review dialog. Only the plan<br>approval review (`kind == "review"`) sets it to the full plan document;<br>other ask_user requests leave it empty. |
| `kind` | `string` | — | — |  |
| `questions` | `array<UserInputQuestion>` | — | — |  |
| `title` | `string` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "UserInputOption": {
      "description": "Option offered in a user input question.",
      "properties": {
        "description": {
          "type": "string"
        },
        "label": {
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
      "description": "Question asked to the user with selectable options.",
      "properties": {
        "allow_custom": {
          "type": "boolean"
        },
        "header": {
          "maxLength": 12,
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
        "question"
      ],
      "type": "object"
    }
  },
  "description": "Input of the ask-user tool.",
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
      "x-agena-order": "000003"
    },
    "body_markdown": {
      "description": "Optional Markdown body shown in the review dialog. Only the plan\napproval review (`kind == \"review\"`) sets it to the full plan document;\nother ask_user requests leave it empty.",
      "maxLength": 16000,
      "type": "string",
      "x-agena-order": "000002"
    },
    "kind": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "questions": {
      "items": {
        "$ref": "#/$defs/UserInputQuestion"
      },
      "maxItems": 3,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000004"
    },
    "title": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "type": "object",
  "x-agena-relations": [
    "required_unless_present `questions[].allow_custom` unless `questions[].options` present",
    "distinct_trimmed_within `questions[].options[].label` within `questions[]`"
  ]
}
```

### notify

`agena.interaction.notify` · **Summary**: Show a non-blocking Markdown notification to the user.

**Tags**: `interactive`

**Runtime**: streaming `buffered`

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
  "description": "Input of the interaction notify tool.",
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

**Tags**: `query` `lsp` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — | Zero-based UTF-16 code-unit offset within the line (LSP default encoding). |
| `file_path` | `string` | ✓ | — |  |
| `line` | `integer` | ✓ | — | Zero-based source line. |

**Input schema**:
```json
{
  "description": "Input of the LSP definition tool.",
  "properties": {
    "character": {
      "description": "Zero-based UTF-16 code-unit offset within the line (LSP default encoding).",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000"
    },
    "line": {
      "description": "Zero-based source line.",
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

**Tags**: `query` `lsp` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `file_path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "description": "Input of the LSP diagnostics tool.",
  "properties": {
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
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

**Tags**: `query` `lsp` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — | Zero-based UTF-16 code-unit offset within the line (LSP default encoding). |
| `file_path` | `string` | ✓ | — |  |
| `line` | `integer` | ✓ | — | Zero-based source line. |

**Input schema**:
```json
{
  "description": "Input of the LSP hover tool.",
  "properties": {
    "character": {
      "description": "Zero-based UTF-16 code-unit offset within the line (LSP default encoding).",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000"
    },
    "line": {
      "description": "Zero-based source line.",
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

**Tags**: `query` `lsp` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `character` | `integer` | ✓ | — | Zero-based UTF-16 code-unit offset within the line (LSP default encoding). |
| `file_path` | `string` | ✓ | — |  |
| `include_declaration` | `boolean` | — | `true` |  |
| `line` | `integer` | ✓ | — | Zero-based source line. |

**Input schema**:
```json
{
  "description": "Input of the LSP references tool.",
  "properties": {
    "character": {
      "description": "Zero-based UTF-16 code-unit offset within the line (LSP default encoding).",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000000.000002"
    },
    "file_path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000.000000"
    },
    "include_declaration": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "line": {
      "description": "Zero-based source line.",
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

`agena.lsp.servers` · **Summary**: List configured language servers and executable availability.

**Tags**: `query` `lsp` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Checks each command against its configured PATH from the workspace root without executing it; command_available=null means lookup was inconclusive. Presence does not guarantee successful initialization. Relative commands can resolve differently in individual project roots. running_roots lists initialized instances. Extension-specific servers take priority over catch-all servers; ties use lexical server name.

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

**Tags**: `query` `mcp` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `execute` `mcp` `mutate`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `mcp` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `filesystem` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `content` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `expected_sha256` | `string / null` | — | `null` | Required when updating an existing record; returned by memory.get/write. |
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
      "maxLength": 8388608,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000003"
    },
    "description": {
      "default": "",
      "maxLength": 64000,
      "type": "string",
      "x-agena-order": "000001"
    },
    "expected_sha256": {
      "default": null,
      "description": "Required when updating an existing record; returned by memory.get/write.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
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

## agena.monitor

**Version** `0.1.0` · **Tools** 2

Continuous-stream background monitoring tools.

### start

`agena.monitor.start` · **Summary**: Start a continuous background monitor.

**Tags**: `execute` `shell` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Start a continuous background monitor. Pass exactly one of `command` (a long-running shell command, e.g. `tail -f`) or `ws` (a WebSocket endpoint; text frames become events). The monitor starts immediately and returns a `monitor_id`. You will be notified with a `system_notification` on each event — keep working, do not poll or sleep. Terminate it with `monitor.stop`; it can also end on source exit/disconnection, timeout, cancellation or session end.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `command` | `string / null` | — | — |  |
| `description` | `string` | — | `` |  |
| `persistent` | `boolean` | — | `false` |  |
| `timeout_ms` | `integer / null` | — | — |  |
| `ws` | `MonitorWsInput / null` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "MonitorWsInput": {
      "description": "WebSocket endpoint monitored by the monitor tool.",
      "properties": {
        "protocols": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "url": {
          "type": "string"
        }
      },
      "required": [
        "url"
      ],
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "command": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "description": {
      "default": "",
      "type": "string",
      "x-agena-order": "000004"
    },
    "persistent": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000003"
    },
    "timeout_ms": {
      "format": "uint64",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "ws": {
      "anyOf": [
        {
          "$ref": "#/$defs/MonitorWsInput"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

### stop

`agena.monitor.stop` · **Summary**: Stop one background monitor.

**Tags**: `mutate` `execute` `shell`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `monitor_id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "monitor_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "monitor_id"
  ],
  "type": "object"
}
```

## agena.notebook

**Version** `0.1.0` · **Tools** 1

Revision-safe Jupyter notebook cell editing.

### edit_cell

`agena.notebook.edit_cell` · **Summary**: Replace, insert, or delete one Jupyter notebook cell with a revision check.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `NotebookEditAction` | ✓ | — |  |
| `cell_index` | `integer` | ✓ | — |  |
| `cell_type` | `NotebookCellType / null` | — | — |  |
| `expected_sha256` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |
| `preserve_outputs` | `boolean` | — | `false` | Explicit opt-in to retaining code outputs; retained output may be stale. |
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
      "default": false,
      "description": "Explicit opt-in to retaining code outputs; retained output may be stale.",
      "type": "boolean",
      "x-agena-order": "000005"
    },
    "source": {
      "default": "",
      "maxLength": 16777216,
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

**Version** `0.1.0` · **Tools** 6

Plan orchestration and plan-autorun tools.

### clear

`agena.plan.clear` · **Summary**: Remove the current plan.

**Tags**: `mutate` `planning`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### edit

`agena.plan.edit` · **Summary**: Edit the current plan's steps and checks.

**Tags**: `mutate` `planning`

**Runtime**: streaming `buffered`

**Help**:
> Address steps and checks by 1-based index: `step` + `status` (with an optional `note`) updates a step, `step` + `check` + `status` updates a check. This tool NEVER requests user approval and NEVER changes the plan phase — the plan stays in whatever phase it is in. Use `plan.phase` for plan-level phase transitions and `plan.review` to request approval.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `check` | `integer / null` | — | — | 1-based index of the check within the step to update (1 = first check). Requires `step`. |
| `expected_revision` | `string / null` | — | — | Optional revision from plan.get; mismatches never overwrite newer state. |
| `note` | `string / null` | — | — |  |
| `status` | `WorkflowPlanStepStatus / null` | — | — |  |
| `step` | `integer / null` | — | — | 1-based index of the step to update (1 = first step). |

**Input schema**:
```json
{
  "$defs": {
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
  "description": "Edit the current plan's steps and checks. Never requests user approval and never changes the plan phase. Address steps and checks by their 1-based index (step 1 is the first step; check 1 is the first check within the step): use `step` + `status` (with an optional `note`) to update a step, or `step` + `check` + `status` to update a check.",
  "properties": {
    "check": {
      "description": "1-based index of the check within the step to update (1 = first check). Requires `step`.",
      "format": "uint",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "expected_revision": {
      "description": "Optional revision from plan.get; mismatches never overwrite newer state.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "note": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
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
      "x-agena-order": "000003"
    },
    "step": {
      "description": "1-based index of the step to update (1 = first step).",
      "format": "uint",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

### get

`agena.plan.get` · **Summary**: Inspect the current plan state.

**Tags**: `query` `planning` `read_only`

**Runtime**: streaming `buffered`

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

### phase

`agena.plan.phase` · **Summary**: Transition the current plan's phase.

**Tags**: `mutate` `interactive` `planning`

**Runtime**: streaming `buffered`

**Help**:
> Plan-level phase transitions between `planning`, `active`, `blocked`, `completed`, and `cancelled`, with optional `autorun` and (for `completed`) `summary`. Transitions into `active`, `blocked`, or `completed` request approval by default only when the current phase is not already approved (`active`, `blocked`, or `completed`). Leave `request_approval` omitted or true for normal transitions; an already approved plan does not request another review for progress or completion. Passing `request_approval: false` requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; never change settings to bypass approval. To complete a plan with steps, mark the required steps/checks `completed` via `plan.edit` first, then call this tool separately with `phase: completed`.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `autorun` | `boolean / null` | — | — | Whether an approved active plan should keep running automatically. |
| `expected_revision` | `string / null` | — | — | Optional revision from plan.get; mismatches never overwrite newer state. |
| `phase` | `WorkflowPlanPhase / null` | — | — | Canonical plan phase. Use `planning`, `active`, `blocked`, `completed`, or `cancelled`. |
| `request_approval` | `boolean / null` | — | — | Whether to request user approval for this plan-level phase change. Defaults to true: transitions into active/blocked/completed request approval only when the current phase is not already approved (active/blocked/completed). Passing false requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`. |
| `summary` | `string / null` | — | — | Optional completion summary. This is only applied when `phase` is `completed`. |

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
    }
  },
  "additionalProperties": false,
  "description": "Transition the current plan's phase. `phase` moves the plan between `planning`, `active`, `blocked`, `completed`, and `cancelled`; `autorun` and `summary` are optional modifiers. A transition into `active`, `blocked`, or `completed` requests review only when the current phase is not already approved (`active`, `blocked`, or `completed`). Leave `request_approval` omitted or true for normal transitions. Passing false requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; never change settings to bypass approval. To complete a plan with steps, first mark the relevant steps or checks `completed` via `plan.edit`, then make a separate call with `phase: completed`.",
  "properties": {
    "autorun": {
      "description": "Whether an approved active plan should keep running automatically.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "expected_revision": {
      "description": "Optional revision from plan.get; mismatches never overwrite newer state.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
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
      "x-agena-order": "000001"
    },
    "request_approval": {
      "description": "Whether to request user approval for this plan-level phase change. Defaults to true: transitions into active/blocked/completed request approval only when the current phase is not already approved (active/blocked/completed). Passing false requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "summary": {
      "description": "Optional completion summary. This is only applied when `phase` is `completed`.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    }
  },
  "type": "object"
}
```

### review

`agena.plan.review` · **Summary**: Request user approval of the current plan before it becomes active.

**Tags**: `mutate` `interactive` `planning`

**Runtime**: streaming `buffered`

**Help**:
> Request user approval of the current saved plan; this may pause for the user. The plan.phase tool also requests review for transitions that need approval. It reviews the current saved plan and, when the user approves, moves it from `planning` to `active`. Call it after creating or refining the plan with `plan.set` / `plan.edit`. If the user leaves feedback or rejects, the plan stays in `planning` so you can revise it and propose again.

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Request user approval of the current saved plan to make it active. This can pause for the user; plan.phase can also request review for transitions that need approval.",
  "properties": {},
  "type": "object"
}
```

### set

`agena.plan.set` · **Summary**: Create or replace the current plan without requesting approval.

**Tags**: `mutate` `planning`

**Runtime**: streaming `buffered`

**Help**:
> Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. This tool never blocks on the user: it saves the plan and returns. With `request_approval: true` (the default) the plan stays in the `planning` phase and you must call `plan.review` to request user approval before it becomes active. Pass `request_approval: false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation` — the plan then becomes active immediately. Never change settings to bypass approval. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `tasks.run` exploration when the scope spans multiple areas), clarify with `interaction.ask` when available, replace the plan content with `plan.set`, and update progress/notes with `plan.edit`. When the plan is complete, call `plan.review` to present it for approval; never ask whether the plan is acceptable via `interaction.ask`.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `autorun` | `boolean / null` | — | — |  |
| `document_markdown` | `string / null` | — | — |  |
| `expected_revision` | `string / null` | — | — | Optional revision from plan.get; mismatches never overwrite newer state. |
| `objective` | `string` | ✓ | — |  |
| `request_approval` | `boolean / null` | — | — | Whether to request user approval before the plan becomes active. Defaults to true when omitted: the plan stays in `planning` and you must call `plan.review` to request approval. Pass `false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; the plan then becomes active immediately. Never change settings to bypass approval. |
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
  "description": "Create or overwrite the current active-session plan. If a plan already exists, this replaces it and resets the phase to planning. Use `steps[].title` for steps, `steps[].checks[].text` for checks, and `autorun` to control whether approved active plans should keep running automatically. This tool never blocks on the user: with `request_approval` true (the default) the plan is saved in the `planning` phase and you must call `plan.review` to request user approval before it becomes active; with `request_approval: false` it is applied directly and becomes active immediately, which requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`.",
  "properties": {
    "autorun": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "document_markdown": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "expected_revision": {
      "description": "Optional revision from plan.get; mismatches never overwrite newer state.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "objective": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "request_approval": {
      "description": "Whether to request user approval before the plan becomes active. Defaults to true when omitted: the plan stays in `planning` and you must call `plan.review` to request approval. Pass `false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; the plan then becomes active immediately. Never change settings to bypass approval.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "steps": {
      "description": "Ordered plan steps. Each step item uses `title`; nested checks use `text`.",
      "items": {
        "$ref": "#/$defs/WorkflowPlanStepInput"
      },
      "type": "array",
      "x-agena-order": "000004"
    },
    "title": {
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    }
  },
  "required": [
    "objective"
  ],
  "type": "object"
}
```

## agena.report

**Version** `0.1.0` · **Tools** 1

Structured review and verification findings.

### findings

`agena.report.findings` · **Summary**: Publish structured file-and-line findings for UI and integrations.

**Tags**: `mutate` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

## agena.session

**Version** `0.1.0` · **Tools** 6

Inspect and manage the current runtime session and its environment, model, and token state.

### environment

`agena.session.environment` · **Summary**: Inspect the runtime workspace, git state, shell, platform, and available host CLIs.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### executables

`agena.session.executables` · **Summary**: Inspect installed modern CLIs, their task-specific usage, and optional versions.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Resolves tools from the Agena server PATH and workspace, including fd/fdfind and bat/batcat aliases. Does not install tools or read interactive shell startup files. Omit names for installed tools and a compact missing list; pass names to inspect specific tools. probe_versions runs bounded version commands only when 1–8 names are supplied. refresh bypasses the 15-second availability cache. Presence does not establish plugin/model dependencies or authorize execution.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `names` | `array<string>` | — | `[]` | Curated tool names or aliases. Empty lists installed tools and missing names. |
| `probe_versions` | `boolean` | — | `false` | Explicitly run bounded version probes. Requires 1–8 named tools. |
| `refresh` | `boolean` | — | `false` | Bypass the 15-second availability cache after installing/changing tools. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "names": {
      "default": [],
      "description": "Curated tool names or aliases. Empty lists installed tools and missing names.",
      "items": {
        "minLength": 1,
        "type": "string"
      },
      "maxItems": 32,
      "type": "array",
      "x-agena-order": "000000"
    },
    "probe_versions": {
      "default": false,
      "description": "Explicitly run bounded version probes. Requires 1–8 named tools.",
      "type": "boolean",
      "x-agena-order": "000001"
    },
    "refresh": {
      "default": false,
      "description": "Bypass the 15-second availability cache after installing/changing tools.",
      "type": "boolean",
      "x-agena-order": "000002"
    }
  },
  "type": "object"
}
```

### get

`agena.session.get` · **Summary**: Inspect the current session metadata.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### model

`agena.session.model` · **Summary**: Inspect the current session model identity, runtime modes, and model token limits.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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

### tokens

`agena.session.tokens` · **Summary**: Inspect current and projected token use, effective limits, and remaining session budget.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

## agena.settings

**Version** `0.1.0` · **Tools** 7

Inspect and edit Agena's global and workspace agena.json settings.

### delete

`agena.settings.delete` · **Summary**: Delete one settings value.

**Tags**: `mutate` `filesystem` `settings` `settings_write`

**Runtime**: streaming `buffered`

**Help**:
> Deletes from the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `dry_run` | `boolean` | — | `false` |  |
| `expected_revision` | `string / null` | — | `null` |  |
| `layer` | `SettingsLayer / null` | — | — |  |
| `path` | `string` | ✓ | — |  |
| `reload` | `boolean / null` | — | — |  |
| `validate` | `boolean / null` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "SettingsLayer": {
      "description": "Config file layer targeted by a settings edit.",
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
    "expected_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
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
      "x-agena-order": "000001"
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
    "path"
  ],
  "type": "object"
}
```

### get

`agena.settings.get` · **Summary**: Read one settings path.

**Tags**: `query` `discovery` `filesystem` `settings` `settings_read`

**Runtime**: streaming `buffered`

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
      "description": "Source layer of a settings read or write.",
      "enum": [
        "effective",
        "file"
      ],
      "type": "string"
    },
    "SettingsLayer": {
      "description": "Config file layer targeted by a settings edit.",
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

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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
      "description": "Source layer of a settings read or write.",
      "enum": [
        "effective",
        "file"
      ],
      "type": "string"
    },
    "SettingsLayer": {
      "description": "Config file layer targeted by a settings edit.",
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

**Runtime**: streaming `buffered`

**Help**:
> Deep-merges a JSON object into the global or workspace config selected by `layer`, then validates the combined layered configuration; null object entries delete keys. Use `dry_run=true` to preview without writing.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `changes` | `any` | ✓ | — |  |
| `dry_run` | `boolean` | — | `false` |  |
| `expected_revision` | `string / null` | — | `null` |  |
| `layer` | `SettingsLayer / null` | — | — |  |
| `path` | `string / null` | — | `null` |  |
| `reload` | `boolean / null` | — | — |  |
| `validate` | `boolean / null` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "SettingsLayer": {
      "description": "Config file layer targeted by a settings edit.",
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
      "x-agena-order": "000002"
    },
    "dry_run": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000004"
    },
    "expected_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
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
      "x-agena-order": "000003"
    },
    "path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "reload": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000005"
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

**Runtime**: streaming `buffered`

**Help**:
> Writes the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing; dry runs request read permission for both config files instead of write permission.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `dry_run` | `boolean` | — | `false` |  |
| `expected_revision` | `string / null` | — | `null` |  |
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
      "description": "Config file layer targeted by a settings edit.",
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
      "x-agena-order": "000004"
    },
    "expected_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
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
      "x-agena-order": "000003"
    },
    "path": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "reload": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "validate": {
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "value": {
      "x-agena-order": "000002"
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

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `layer` | `SettingsLayer / null` | — | `null` |  |

**Input schema**:
```json
{
  "$defs": {
    "SettingsLayer": {
      "description": "Config file layer targeted by a settings edit.",
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

**Version** `0.1.0` · **Tools** 7

Shell command execution and background process tools.

### list

`agena.shell.list` · **Summary**: List active background processes.

**Tags**: `query` `discovery` `shell` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `query` `shell` `read_only`

**Runtime**: streaming `buffered`

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

### resize

`agena.shell.resize` · **Summary**: Resize an interactive terminal.

**Tags**: `mutate` `shell`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cols` | `integer` | ✓ | — |  |
| `process_id` | `string` | ✓ | — |  |
| `rows` | `integer` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "cols": {
      "format": "uint16",
      "maximum": 400,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "rows": {
      "format": "uint16",
      "maximum": 200,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "process_id",
    "rows",
    "cols"
  ],
  "type": "object"
}
```

### run

`agena.shell.run` · **Summary**: Run one shell process.

**Tags**: `execute` `shell` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Run a shell command. Declare `reads`, `writes` and outbound `network` targets; use empty arrays when none. Set `tty=true` for an interactive CLI, REPL or full-screen terminal. This retains a PTY across tool calls and returns a process_id, incremental output, last_seq, and a current terminal screen. `yield_time_ms` (default 1000, maximum 30000) only controls this call's initial output wait: it never terminates the process. `timeout_ms`, when supplied, is the terminal's overall lifetime limit. Continue with shell.write; read without input with shell.write(chars="") or shell.logs; use shell.resize for dimensions, shell.signal for interrupt/terminate/kill, and shell.stop for cleanup. Never assume a quiet prompt means completion. tty is incompatible with monitor. Without tty, normal foreground behavior is unchanged. `run_in_background=true` or `monitor` starts a non-interactive managed command; completion is notified by system_notification, so do not poll merely to wait for those jobs.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cols` | `integer` | — | `80` |  |
| `command` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `monitor` | `ShellMonitorInput / null` | — | — |  |
| `network` | `array<string>` | — | `[]` | Outbound network targets the command may connect to: host names,<br>`host:port`, or URLs. Pass an empty array `[]` when the command has no<br>network effect. |
| `reads` | `array<string>` | — | `[]` | Files and directories the command may read. Declare only the actual<br>files/directories affected - never the executables, interpreters, or<br>tools being invoked (e.g. `node`, `python`, `uv`, `git`, `cargo`) or<br>their installation directories. Pass an empty array `[]` when the<br>command reads nothing beyond its executables. |
| `rows` | `integer` | — | `24` | Initial terminal dimensions, in character cells. |
| `run_in_background` | `boolean` | — | `false` |  |
| `shell` | `ProcessShell` | — | `bash` |  |
| `timeout_ms` | `integer / null` | — | — |  |
| `tty` | `boolean` | — | `false` | Allocate a persistent pseudo-terminal. Use shell.write for subsequent<br>input; yielding output does not stop the process. Incompatible with monitor. |
| `workdir` | `string / null` | — | — |  |
| `writes` | `array<string>` | — | `[]` | Files and directories the command may create, modify, or delete.<br>Declare only the actual files/directories affected - never the<br>executables, interpreters, or tools being invoked (e.g. `node`,<br>`python`, `uv`, `git`, `cargo`) or their installation directories.<br>Pass an empty array `[]` when the command writes nothing. |
| `yield_time_ms` | `integer` | — | `1000` | Maximum initial wait for terminal output, not a process timeout (0–30000 ms). |

**Input schema**:
```json
{
  "$defs": {
    "ProcessShell": {
      "description": "Shell used to run a process.",
      "enum": [
        "bash",
        "powershell"
      ],
      "type": "string",
      "x-agena-order": "000000"
    },
    "ShellMonitorInput": {
      "additionalProperties": false,
      "description": "Optional completion and capture policy for a managed shell process. Adding\nthis object makes `shell.run` a monitored background invocation and returns\nthe same `process_id` consumed by `shell.list`, `shell.logs` and `shell.stop`.\nPatterns that determine shell command success or failure.",
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
      "description": "How a shell monitor pattern is matched.",
      "enum": [
        "literal",
        "regex"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "description": "Input of a shell command execution.",
  "properties": {
    "cols": {
      "default": 80,
      "format": "uint16",
      "maximum": 400,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001.000004"
    },
    "command": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001.000000"
    },
    "description": {
      "default": "",
      "type": "string",
      "x-agena-order": "000001.000005"
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
      "x-agena-order": "000001.000010"
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
      "x-agena-order": "000001.000008"
    },
    "rows": {
      "default": 24,
      "description": "Initial terminal dimensions, in character cells.",
      "format": "uint16",
      "maximum": 200,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000001.000003"
    },
    "run_in_background": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000002"
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
      "x-agena-order": "000001.000006"
    },
    "tty": {
      "default": false,
      "description": "Allocate a persistent pseudo-terminal. Use shell.write for subsequent\ninput; yielding output does not stop the process. Incompatible with monitor.",
      "type": "boolean",
      "x-agena-order": "000001.000001"
    },
    "workdir": {
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001.000000"
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
      "x-agena-order": "000001.000009"
    },
    "yield_time_ms": {
      "default": 1000,
      "description": "Maximum initial wait for terminal output, not a process timeout (0–30000 ms).",
      "format": "uint64",
      "maximum": 30000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000001.000002"
    }
  },
  "required": [
    "command"
  ],
  "type": "object"
}
```

### signal

`agena.shell.signal` · **Summary**: Interrupt or terminate an interactive terminal.

**Tags**: `mutate` `execute` `shell`

**Runtime**: streaming `buffered`

**Help**:
> interrupt targets the current Unix foreground process group without closing the shell (ConPTY uses terminal Ctrl-C). terminate requests graceful session cleanup and then kills remaining jobs; kill skips the grace period. This is distinct from typing a control byte into a raw-mode program. The same owning session/workspace is required. shell.stop is equivalent to terminate.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `process_id` | `string` | ✓ | — |  |
| `signal` | `ShellSignal` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ShellSignal": {
      "description": "Out-of-band process control. On Unix, interrupt signals the foreground\nprocess group even in raw mode. Windows uses ConPTY Ctrl-C semantics.",
      "enum": [
        "interrupt",
        "terminate",
        "kill"
      ],
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "additionalProperties": false,
  "properties": {
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "signal": {
      "$ref": "#/$defs/ShellSignal"
    }
  },
  "required": [
    "process_id",
    "signal"
  ],
  "type": "object"
}
```

### stop

`agena.shell.stop` · **Summary**: Stop one background process.

**Tags**: `mutate` `execute` `shell`

**Runtime**: streaming `buffered`

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

### write

`agena.shell.write` · **Summary**: Write to an interactive terminal and read its response.

**Tags**: `mutate` `execute` `shell`

**Runtime**: streaming `buffered`

**Help**:
> Continue a process started with shell.run(tty=true). chars is exact terminal input: never trim or automatically append a newline. Send \r for Enter, \u0003 for Ctrl-C, \u0004 for Ctrl-D, \t for Tab, or terminal escape sequences for arrow/function keys. Use chars="" to read without sending input. Omit since_seq to read previously unread output; use an explicit last_seq to replay/page output. wait_ms defaults to 250 and is capped at 30000; a wait timeout does not kill the CLI. Input is an execution operation: declare every affected reads/writes path (relative to the Agena workspace) and network target, including effects of commands entered inside a shell/REPL. Requires the same owning session and workspace as the launch. A partial-write error requests terminal termination; do not resend the full input blindly. Process exit, not absence of output, indicates completion.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `chars` | `string` | — | `` | Exact UTF-8 text/control characters. Empty reads without writing. Use<br>\r for Enter, \u0003 for Ctrl-C, \u0004 for Ctrl-D; at most 65536 bytes. |
| `network` | `array<string>` | — | `[]` | Outbound targets the entered operation may contact. |
| `process_id` | `string` | ✓ | — |  |
| `reads` | `array<string>` | — | `[]` | Paths the entered operation may read, relative to the Agena workspace. |
| `since_seq` | `integer / null` | — | — | Output cursor from a previous result. Omit to consume the session's<br>unread output; explicit cursors permit replay without changing it. |
| `wait_ms` | `integer` | — | `250` |  |
| `writes` | `array<string>` | — | `[]` | Paths the entered operation may modify, relative to the Agena workspace. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Input is terminal data, not a new shell command. Never trim it or append a newline.",
  "properties": {
    "chars": {
      "default": "",
      "description": "Exact UTF-8 text/control characters. Empty reads without writing. Use\n\\r for Enter, \\u0003 for Ctrl-C, \\u0004 for Ctrl-D; at most 65536 bytes.",
      "type": "string",
      "x-agena-order": "000001"
    },
    "network": {
      "default": [],
      "description": "Outbound targets the entered operation may contact.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000006"
    },
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "reads": {
      "default": [],
      "description": "Paths the entered operation may read, relative to the Agena workspace.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000004"
    },
    "since_seq": {
      "description": "Output cursor from a previous result. Omit to consume the session's\nunread output; explicit cursors permit replay without changing it.",
      "format": "uint64",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "wait_ms": {
      "default": 250,
      "format": "uint64",
      "maximum": 30000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000003"
    },
    "writes": {
      "default": [],
      "description": "Paths the entered operation may modify, relative to the Agena workspace.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000005"
    }
  },
  "required": [
    "process_id"
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

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

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

**Tags**: `query` `snapshot` `read_only`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

## agena.tasks

**Version** `0.1.0` · **Tools** 7

Delegated subtask orchestration tools.

### cancel

`agena.tasks.cancel` · **Summary**: Cancel a running delegated task and its child execution.

**Tags**: `subtask` `mutate` `task`

**Runtime**: streaming `buffered`

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

### followup

`agena.tasks.followup` · **Summary**: Resume a terminal delegated task with a follow-up prompt.

**Tags**: `subtask` `mutate` `task`

**Runtime**: streaming `buffered`

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

**Tags**: `subtask` `query` `read_only` `task`

**Runtime**: streaming `buffered`

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

**Tags**: `subtask` `query` `discovery` `read_only` `task`

**Runtime**: streaming `buffered`

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

**Tags**: `subtask` `mutate` `task`

**Runtime**: streaming `buffered`

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

**Tags**: `subtask` `query` `read_only` `task`

**Runtime**: streaming `buffered`

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

`agena.tasks.run` · **Summary**: Delegate a bounded task to a subagent session. Set `run_in_background` to run it in the background and be notified when it settles. Attach command names in `commands` so the child session applies their instructions as task guidance.

**Tags**: `subtask` `execute` `task`

**Runtime**: streaming `buffered`

**Help**:
> Reach for this tool when the work matches an available command or subagent type, when you have independent work to run in parallel, or when answering would mean reading across several files — delegate it and you keep the conclusion, not the file dumps. For a single-fact lookup where you already know the file, symbol, or value, search directly; once you have delegated a search, do not also run it yourself — wait for the result. Do small tasks yourself instead of delegating; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; do not redo work you already delegated. Never delegate understanding: brief the subagent with concrete file paths, line numbers, and what to change, then check its result. Set `commands` to command names, slash spellings or aliases (for example a read-only review command for a review task, or an explore command for an exploration task); the child session receives the instructions those commands name and should follow them. Unknown names are rejected before the subtask starts. Use the `agena.commands` plugin's `list` tool to discover what this workspace offers. By default the subtask runs inline and this call returns its final result before returning. With `run_in_background: true` the subtask runs in the background: the tool returns immediately with a task id and the result is delivered as a `system_notification` when it settles — do not poll tasks.get/tasks.output waiting for it.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `commands` | `array<string>` | — | — | Optional command names, slash spellings or aliases to attach to the<br>delegated subtask's first user message as lazy references. The child<br>model receives catalog metadata and reads the instructions through the<br>plugin that declared the command. Use commands appropriate to the task:<br>for example a read-only review task can attach a review command, an<br>exploration task can attach an explore command. Unknown names are<br>rejected before the subtask starts. |
| `description` | `string` | ✓ | — | Short label for the subtask session. |
| `max_cost_microusd` | `integer / null` | — | — | Cumulative child-completion cost ceiling in USD micro-units (one<br>millionth of a USD). Integer micro-units avoid a floating-point value<br>becoming a durable budget boundary; for example, 250000 means $0.25. |
| `max_tokens` | `integer / null` | — | — | Cumulative child-completion token budget. This includes prompt,<br>output, reasoning and cache token accounting reported by the route. |
| `prompt` | `string` | ✓ | — | Full instruction payload for the delegated subtask. |
| `run_in_background` | `boolean` | — | `false` | Run the subtask in the background (default false). When false (default)<br>the subtask runs inline and this call returns its final result before<br>the tool call returns. When true, the tool returns immediately with a<br>task id and the result is delivered as a `system_notification` when the<br>subtask settles — do not poll tasks.get/tasks.output waiting for it. |
| `selection` | `TaskModelSelection / null` | — | — | Optional model and mode overrides. Explicit values take precedence over<br>the parent session. |
| `task_id` | `string / null` | — | — | Resume an existing subtask session instead of creating a new one. |
| `timeout_ms` | `integer / null` | — | — | Overall task timeout. A timeout cancels the child execution and returns<br>a structured `timed_out` task result. |

**Input schema**:
```json
{
  "$defs": {
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
  "description": "Input of the task tool.",
  "properties": {
    "commands": {
      "description": "Optional command names, slash spellings or aliases to attach to the\ndelegated subtask's first user message as lazy references. The child\nmodel receives catalog metadata and reads the instructions through the\nplugin that declared the command. Use commands appropriate to the task:\nfor example a read-only review task can attach a review command, an\nexploration task can attach an explore command. Unknown names are\nrejected before the subtask starts.",
      "items": {
        "type": "string"
      },
      "type": [
        "array",
        "null"
      ],
      "x-agena-order": "000003"
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
    "run_in_background": {
      "default": false,
      "description": "Run the subtask in the background (default false). When false (default)\nthe subtask runs inline and this call returns its final result before\nthe tool call returns. When true, the tool returns immediately with a\ntask id and the result is delivered as a `system_notification` when the\nsubtask settles — do not poll tasks.get/tasks.output waiting for it.",
      "type": "boolean",
      "x-agena-order": "000002"
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

## agena.tools

**Version** `0.1.0` · **Tools** 7

Tool API discovery functions. The runtime resolves tools_call directly to its execution target.

### help

`agena.tools.help` · **Tool API gateway handler** · **Summary**: Get reusable schemas, examples, and usage notes for one Agena execution tool or a batch of tools.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `tool` | `ToolApiStringBatch` | ✓ | — | One exact execution-tool name, or a non-empty array of exact names, to<br>inspect. Use exact names known from the prompt/context or returned by<br>`tools_list` or `tools_search`; known names need no preliminary search.<br>This help checks current availability and supplies the live contract. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ],
      "description": "One exact execution-tool name, or a non-empty array of exact names, to\ninspect. Use exact names known from the prompt/context or returned by\n`tools_list` or `tools_search`; known names need no preliminary search.\nThis help checks current availability and supplies the live contract.",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "tool": {
      "$ref": "#/$defs/ToolApiStringBatch",
      "description": "One exact execution-tool name, or a non-empty array of exact names, to\ninspect. Use exact names known from the prompt/context or returned by\n`tools_list` or `tools_search`; known names need no preliminary search.\nThis help checks current availability and supplies the live contract."
    }
  },
  "required": [
    "tool"
  ],
  "type": "object"
}
```

### list

`agena.tools.list` · **Tool API gateway handler** · **Summary**: Enumerate current tools across one plugin or a batch of plugin targets.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tools to return. |
| `offset` | `integer / null` | — | — | Number of tools to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids,<br>with OR semantics. It scopes tools by owner for `tools_list` and selects<br>plugin records directly for `plugins_list`. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ]
    }
  },
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
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids,\nwith OR semantics. It scopes tools by owner for `tools_list` and selects\nplugin records directly for `plugins_list`.",
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

`agena.tools.plugins_list` · **Tool API gateway handler** · **Summary**: Enumerate one or many selected plugins with version, summary, tags, and tool count.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tools to return. |
| `offset` | `integer / null` | — | — | Number of tools to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids,<br>with OR semantics. It scopes tools by owner for `tools_list` and selects<br>plugin records directly for `plugins_list`. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ]
    }
  },
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
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids,\nwith OR semantics. It scopes tools by owner for `tools_list` and selects\nplugin records directly for `plugins_list`.",
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

`agena.tools.plugins_search` · **Tool API gateway handler** · **Summary**: Search loaded plugins with one or many queries and optional multi-plugin scope.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of search results to return. |
| `offset` | `integer / null` | — | — | Number of matching tools to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids,<br>with OR semantics. It scopes tools by owner for `tools_search` and<br>plugin records directly for `plugins_search`. |
| `query` | `ToolApiStringBatch` | ✓ | — | One search query, or a non-empty array of queries, used to rank matching<br>tool names and summaries. Batched queries are evaluated independently. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ],
      "description": "One search query, or a non-empty array of queries, used to rank matching\ntool names and summaries. Batched queries are evaluated independently.",
      "x-agena-order": "000000"
    }
  },
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
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids,\nwith OR semantics. It scopes tools by owner for `tools_search` and\nplugin records directly for `plugins_search`.",
      "x-agena-order": "000003"
    },
    "query": {
      "$ref": "#/$defs/ToolApiStringBatch",
      "description": "One search query, or a non-empty array of queries, used to rank matching\ntool names and summaries. Batched queries are evaluated independently."
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
  "required": [
    "query"
  ],
  "type": "object"
}
```

### plugins_tags

`agena.tools.plugins_tags` · **Tool API gateway handler** · **Summary**: List plugin tags across one plugin or a batch of plugin targets.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tags to return. |
| `offset` | `integer / null` | — | — | Number of tags to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids.<br>Only tags belonging to tools or plugins from any selected plugin are<br>counted. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ]
    }
  },
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
    },
    "plugin": {
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids.\nOnly tags belonging to tools or plugins from any selected plugin are\ncounted.",
      "x-agena-order": "000002"
    }
  },
  "type": "object"
}
```

### search

`agena.tools.search` · **Tool API gateway handler** · **Summary**: Search execution tools with one or many queries across one or many plugin targets.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of search results to return. |
| `offset` | `integer / null` | — | — | Number of matching tools to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids,<br>with OR semantics. It scopes tools by owner for `tools_search` and<br>plugin records directly for `plugins_search`. |
| `query` | `ToolApiStringBatch` | ✓ | — | One search query, or a non-empty array of queries, used to rank matching<br>tool names and summaries. Batched queries are evaluated independently. |
| `tag` | `string / null` | — | — | Optional single tag filter such as `query` or `network`. |
| `tags` | `array<string>` | — | — | Optional tag filters. When present, all normalized tags must match. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ],
      "description": "One search query, or a non-empty array of queries, used to rank matching\ntool names and summaries. Batched queries are evaluated independently.",
      "x-agena-order": "000000"
    }
  },
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
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids,\nwith OR semantics. It scopes tools by owner for `tools_search` and\nplugin records directly for `plugins_search`.",
      "x-agena-order": "000003"
    },
    "query": {
      "$ref": "#/$defs/ToolApiStringBatch",
      "description": "One search query, or a non-empty array of queries, used to rank matching\ntool names and summaries. Batched queries are evaluated independently."
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
  "required": [
    "query"
  ],
  "type": "object"
}
```

### tags

`agena.tools.tags` · **Tool API gateway handler** · **Summary**: List tool tags across one plugin or a batch of plugin targets.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `limit` | `integer / null` | — | — | Maximum number of tags to return. |
| `offset` | `integer / null` | — | — | Number of tags to skip before returning results. |
| `plugin` | `ToolApiStringBatch / null` | — | — | Optional plugin selector: one plugin id or a non-empty array of ids.<br>Only tags belonging to tools or plugins from any selected plugin are<br>counted. |

**Input schema**:
```json
{
  "$defs": {
    "ToolApiStringBatch": {
      "anyOf": [
        {
          "minLength": 1,
          "type": "string"
        },
        {
          "items": {
            "type": "string"
          },
          "minItems": 1,
          "type": "array"
        }
      ]
    }
  },
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
    },
    "plugin": {
      "anyOf": [
        {
          "$ref": "#/$defs/ToolApiStringBatch"
        },
        {
          "type": "null"
        }
      ],
      "description": "Optional plugin selector: one plugin id or a non-empty array of ids.\nOnly tags belonging to tools or plugins from any selected plugin are\ncounted.",
      "x-agena-order": "000002"
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

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `frame_selector` | `string / null` | — | — | Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs). |
| `ref` | `integer / null` | — | — | Snapshot-local index returned by `browser_snapshot.elements[].ref`.<br>It is valid only while the page DOM has not materially changed. |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |
| `snapshot_id` | `string / null` | — | — | ID of the snapshot that supplied ref. Required with ref; stale refs are rejected. |
| `timeout_ms` | `integer` | — | `30000` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "frame_selector": {
      "description": "Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).",
      "maxLength": 4096,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "ref": {
      "description": "Snapshot-local index returned by `browser_snapshot.elements[].ref`.\nIt is valid only while the page DOM has not materially changed.",
      "format": "uint16",
      "maximum": 199,
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "selector": {
      "maxLength": 4096,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "snapshot_id": {
      "description": "ID of the snapshot that supplied ref. Required with ref; stale refs are rejected.",
      "maxLength": 128,
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
      "x-agena-order": "000005"
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

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

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

**Tags**: `network` `interactive` `mutate` `filesystem`

**Runtime**: streaming `buffered`

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

**Tags**: `network` `interactive` `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `network` `interactive` `mutate` `read_only`

**Runtime**: streaming `buffered`

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

**Tags**: `network` `interactive` `mutate` `filesystem`

**Runtime**: streaming `buffered`

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

`agena.web.browser_shutdown` · **Summary**: Close browser pages owned by the current Agena session without affecting other callers.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Caller-scoped shutdown closes owned pages only. The shared Chrome process and other sessions are not stopped. Global browser shutdown is reserved for trusted host lifecycle control.

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

**Tags**: `network` `interactive` `query` `read_only`

**Runtime**: streaming `buffered`

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

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `frame_selector` | `string / null` | — | — | Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs). |
| `press_enter` | `boolean` | — | `false` |  |
| `ref` | `integer / null` | — | — |  |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |
| `snapshot_id` | `string / null` | — | — | ID of the snapshot that supplied ref. Required with ref; stale refs are rejected. |
| `text` | `string` | ✓ | — |  |
| `timeout_ms` | `integer` | — | `30000` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "frame_selector": {
      "description": "Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).",
      "maxLength": 4096,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "press_enter": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000006"
    },
    "ref": {
      "format": "uint16",
      "maximum": 199,
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "selector": {
      "maxLength": 4096,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "snapshot_id": {
      "description": "ID of the snapshot that supplied ref. Required with ref; stale refs are rejected.",
      "maxLength": 128,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "text": {
      "maxLength": 65536,
      "type": "string",
      "x-agena-order": "000003"
    },
    "timeout_ms": {
      "default": 30000,
      "format": "uint64",
      "maximum": 120000,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000007"
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

**Tags**: `network` `interactive` `query` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `frame_selector` | `string / null` | — | — | Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs). |
| `selector` | `string / null` | — | — |  |
| `session_id` | `string` | ✓ | — |  |
| `text` | `string / null` | — | — |  |
| `timeout_ms` | `integer` | — | `30000` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "frame_selector": {
      "description": "Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).",
      "maxLength": 4096,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "selector": {
      "maxLength": 4096,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "session_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "text": {
      "maxLength": 4096,
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
      "x-agena-order": "000004"
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

**Tags**: `network` `discovery` `mutate`

**Runtime**: streaming `buffered`

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

**Tags**: `network` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use this tool after search when you need evidence from the actual page rather than search snippets. If you already know what facts you need, set `prompt` so Agena prioritizes the most relevant excerpts from the page in the returned text output.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `extractor` | `ExtractionBackend / null` | — | — |  |
| `prompt` | `string / null` | — | — |  |
| `render_js` | `boolean / null` | — | — |  |
| `url` | `string` | ✓ | — |  |
| `use_cache` | `boolean` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "ExtractionBackend": {
      "enum": [
        "readability",
        "trafilatura"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "extractor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ExtractionBackend"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000004"
    },
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

**Tags**: `network` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Discover candidate pages; fetch 1-3 relevant URLs for factual answers. Omit engine or use auto for the configured search provider (HTML, Brave, Tavily, Exa or SearXNG). Explicit bing/duckduckgo/baidu selects that HTML engine. API providers return at most 20 results and report failures without switching providers. Domain filters accept bare hostnames; exclusions win. Snippets are previews, not fetched-page evidence.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `allowed_domains` | `array<string>` | — | — |  |
| `blocked_domains` | `array<string>` | — | — |  |
| `engine` | `WebSearchEngineSelection / null` | — | — |  |
| `max_results` | `integer / null` | — | — |  |
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
      "maxItems": 64,
      "type": "array",
      "x-agena-order": "000003"
    },
    "blocked_domains": {
      "items": {
        "type": "string"
      },
      "maxItems": 64,
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
      "format": "uint32",
      "maximum": 50,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "query": {
      "maxLength": 8192,
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

