# Agena Bundled Tools Reference

> Generated file — do not edit by hand. Regenerate with:
>
> ```bash
> agena inspect --tools-reference > crates/agena-bundled-plugins/generated/tools-reference.md
> ```

This document is deterministically generated from the real `agena-bundled-plugins` plugin manifests, covering **23 plugins and 208 tool definitions**.

- Each tool entry includes: name, summary, detailed help (`before_help` / `help` / `after_help`), tags, the streaming runtime flag, an input parameter table, and the full input / output JSON Schema.
- This catalog contains plugin execution tools only. The runtime-owned Tool API protocol (`plugins_list`, `tools_list`, `tools_help`, `tools_call`) has its own registry and is excluded from plugin and tool counts.
- Execution-tool names (`plugin.tool`, full key `agena.<plugin>.<tool>`) appear only in `tools_help.tool` / `tools_call.tool`; they never become Provider function names.

## Table of Contents

- [`agena.agent`](#agenaagent) — Agent communication and delegation. (5 tools)
- [`agena.chatgpt`](#agenachatgpt) — Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. OpenAI cloud inference, files, knowledge stores, images, audio, realtime voice, batches and agent APIs. Inputs leave this computer; no local execution fallback. (27 tools)
- [`agena.claude`](#agenaclaude) — Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Anthropic cloud inference, search, code execution, files, Skills, batches and managed-agent APIs. Inputs leave this computer; no local execution fallback. (15 tools)
- [`agena.code`](#agenacode) — Structured code search and syntax inspection tools. (3 tools)
- [`agena.commands`](#agenacommands) — Declare the built-in commands every Agena client renders locally, and project skills from other agent ecosystems into the same surface. (6 tools)
- [`agena.cron`](#agenacron) — Cron-style and one-shot wakeup scheduling tools. (7 tools)
- [`agena.fs`](#agenafs) — Filesystem command tools for read/search and explicit edits. (10 tools)
- [`agena.gemini`](#agenagemini) — Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Google cloud inference, search, files, knowledge stores, images, audio, video, music, Live voice and agent APIs. Inputs leave this computer; no local execution fallback. (29 tools)
- [`agena.inline_ui`](#agenainline_ui) — Inline Generative UI: composable, stateful interfaces and live work updates inside answers. Start with guide; discover layout/state/actions/live progressively, then prepare/read/update. (5 tools)
- [`agena.interaction`](#agenainteraction) — User interaction tools. (2 tools)
- [`agena.lsp`](#agenalsp) — LSP read-only observability and navigation tools. (5 tools)
- [`agena.mcp`](#agenamcp) — MCP discovery and bridge tools. (26 tools)
- [`agena.memory`](#agenamemory) — Persistent memory with searchable retrieval and write tools. (5 tools)
- [`agena.monitor`](#agenamonitor) — WebSocket event subscriptions with background notifications. Use shell.watch for local command-output monitoring. (2 tools)
- [`agena.notebook`](#agenanotebook) — Revision-safe Jupyter notebook cell editing. (1 tools)
- [`agena.plan`](#agenaplan) — Plan orchestration and plan-autorun tools. (6 tools)
- [`agena.provider`](#agenaprovider) — Discover model routes across all enabled adapters of the current provider. (1 tools)
- [`agena.report`](#agenareport) — Structured review and verification findings. (1 tools)
- [`agena.session`](#agenasession) — Inspect and manage the current runtime session and its environment, model, and token state. (6 tools)
- [`agena.settings`](#agenasettings) — Inspect and edit Agena's global and workspace agena.json settings. (7 tools)
- [`agena.shell`](#agenashell) — Shell commands: exec waits, spawn runs in background, watch monitors output events, open provides an interactive terminal/PTY; shared logs and lifecycle controls manage them. (11 tools)
- [`agena.skills`](#agenaskills) — Discover, install, author, and manage complete Agent Skills packages. (12 tools)
- [`agena.web`](#agenaweb) — Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering. (16 tools)

## agena.agent

**Version** `0.1.0` · **Tools** 5

Agent communication and delegation.

### cancel

`agena.agent.cancel` · **Summary**: Stop a direct child execution. Preserves the agent session for agent.followup.

**Tags**: `agent`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `target` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "target": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "target"
  ],
  "type": "object"
}
```

### followup

`agena.agent.followup` · **Summary**: Give a direct child more work. Queues guidance while running; starts another execution in the same session when idle.

**Tags**: `agent`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `message` | `string` | ✓ | — |  |
| `target` | `string` | ✓ | — | An agent_id, a visible agent name, or `parent`. |

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
    "target": {
      "description": "An agent_id, a visible agent name, or `parent`.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "target",
    "message"
  ],
  "type": "object"
}
```

### message

`agena.agent.message` · **Summary**: Queue a message to a visible agent. Does not start idle recipients or pause the sender. Use agent.wait when you need a reply.

**Tags**: `agent`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `message` | `string` | ✓ | — |  |
| `target` | `string` | ✓ | — | An agent_id, a visible agent name, or `parent`. |

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
    "target": {
      "description": "An agent_id, a visible agent name, or `parent`.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "target",
    "message"
  ],
  "type": "object"
}
```

### spawn

`agena.agent.spawn` · **Summary**: Create an independent child and return its handle immediately. Its final reply is delivered automatically. Parent history is not copied; include scope and constraints in prompt.

**Tags**: `agent`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `commands` | `array<string>` | — | `[]` | Command instructions attached to the initial message. |
| `max_cost_microusd` | `integer / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — | Per-execution token budget, retained for followup executions. |
| `name` | `string` | ✓ | — | Unique nonnumeric name: 1–64 lowercase letters, digits or underscores; `parent` and `self` are reserved. |
| `prompt` | `string` | ✓ | — | Scope, constraints, required context and output contracts, and expected final result.<br>Parent history is not copied; include any chart or UI contract the task needs. |
| `selection` | `AgentModelSelection / null` | — | — |  |
| `timeout_ms` | `integer / null` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "AgentModelSelection": {
      "additionalProperties": false,
      "properties": {
        "adapter": {
          "default": null,
          "type": [
            "string",
            "null"
          ]
        },
        "model": {
          "default": null,
          "type": [
            "string",
            "null"
          ]
        },
        "parallel_tool_calls": {
          "default": null,
          "type": [
            "boolean",
            "null"
          ]
        },
        "provider": {
          "default": null,
          "type": [
            "string",
            "null"
          ]
        },
        "speed_mode": {
          "default": null,
          "type": [
            "string",
            "null"
          ]
        },
        "thinking_mode": {
          "default": null,
          "type": [
            "string",
            "null"
          ]
        },
        "verbosity": {
          "default": null,
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
    "commands": {
      "default": [],
      "description": "Command instructions attached to the initial message.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000002"
    },
    "max_cost_microusd": {
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "max_tokens": {
      "description": "Per-execution token budget, retained for followup executions.",
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "name": {
      "description": "Unique nonnumeric name: 1–64 lowercase letters, digits or underscores; `parent` and `self` are reserved.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "prompt": {
      "description": "Scope, constraints, required context and output contracts, and expected final result.\nParent history is not copied; include any chart or UI contract the task needs.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "selection": {
      "anyOf": [
        {
          "$ref": "#/$defs/AgentModelSelection"
        },
        {
          "type": "null"
        }
      ],
      "x-agena-order": "000003"
    },
    "timeout_ms": {
      "format": "uint64",
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    }
  },
  "required": [
    "name",
    "prompt"
  ],
  "type": "object"
}
```

### wait

`agena.agent.wait` · **Summary**: Wait on your own mailbox for messages, new user input or timeout. Returns only a wake reason; messages enter context automatically. Do not poll.

**Tags**: `agent`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `timeout_ms` | `integer / null` | — | — | Defaults to 60 seconds. Wait for messages instead of polling statuses. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "timeout_ms": {
      "description": "Defaults to 60 seconds. Wait for messages instead of polling statuses.",
      "format": "uint64",
      "maximum": 300000,
      "minimum": 1,
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

## agena.chatgpt

**Version** `0.1.0` · **Tools** 27

Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. OpenAI cloud inference, files, knowledge stores, images, audio, realtime voice, batches and agent APIs. Inputs leave this computer; no local execution fallback.

### agents

`agena.chatgpt.agents` · **Summary**: Manage vendor-hosted agents, sessions and environments.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model for agent creation. Resource management needs no model. resource=agents: create/list/get/update/delete; sessions (default): create/list/get/wait/update/delete/events/create_event/items; environments: create/list/get/add_file/list_files. Additional resources: sessions/artifacts list/get/download/delete; sessions/turns list/get/items; sessions/subagents list/get/items; sessions/subagents/turns list/get/items; sessions/traces list. Nested resources use id=session, item_id=artifact/turn/subagent; subagent turns also use sub_item_id=turn. download saves actual artifact bytes. Event streams are bounded by wait_seconds and accept last_event_id for resumption. parameters/query use official schemas. Hosted agents do not automatically receive local files or tools.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### audio_understanding

`agena.chatgpt.audio_understanding` · **Summary**: Analyze audio with a selected OpenAI model.

**Tags**: `network`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting audio input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### batches

`agena.chatgpt.batches` · **Summary**: Create, inspect, cancel and retrieve asynchronous batch jobs.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, cancel; Claude/Gemini also delete, Claude also results. parameters contains official batch input. OpenAI result output_file_id is downloaded with files.download; Gemini output file names use files.download. Claude results are saved as a JSONL attachment. A wait is bounded and never resubmits a job.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### code_interpreter

`agena.chatgpt.code_interpreter` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Run Python in an OpenAI cloud container, not the Agena local workspace.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `streaming`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### containers

`agena.chatgpt.containers` · **Summary**: Manage OpenAI cloud containers and their input/output files.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, delete, upload/add_file, list_files, get_file, delete_file, download_file. id is container ID; item_id is container-file ID. Add an existing file with parameters.file_id, or upload path. download_file saves actual bytes locally. Container execution is remote.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### count_tokens

`agena.chatgpt.count_tokens` · **Summary**: Count tokens using the vendor token-counting endpoint.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### document_understanding

`agena.chatgpt.document_understanding` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Send explicit PDF/text documents to OpenAI cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### embeddings

`agena.chatgpt.embeddings` · **Summary**: Create OpenAI embeddings for supplied inputs.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. parameters.input contains text or token inputs; parameters supports dimensions and encoding_format. Requires an embedding model. Large vectors are saved in the response receipt.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### file_delete

`agena.chatgpt.file_delete` · **Summary**: Request deletion of an owned file from OpenAI cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

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

### file_search

`agena.chatgpt.file_search` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Search configured OpenAI cloud file stores, not files on this computer.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### file_status

`agena.chatgpt.file_status` · **Summary**: Query the remote status of an owned OpenAI cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

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

### file_upload

`agena.chatgpt.file_upload` · **Summary**: Upload one permitted local file to OpenAI cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Uploads accept arbitrary explicit data files up to 512 MiB and optional SHA256 revision checks; analysis has separate format/size limits. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

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

### files

`agena.chatgpt.files` · **Summary**: Manage remote files and download cloud-generated artifacts.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### image_edit

`agena.chatgpt.image_edit` · **Summary**: Edit local images or compose from references directly with the OpenAI Images API; model selects a GPT Image drawing model.

**Tags**: `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Prefer this for editing local images or composing from reference images. Sends POST images/edits directly to a GPT Image drawing model. Supply prompt as natural-language edit instructions and images as one to sixteen permitted local image paths; this tool reads and uploads them directly. To select an OpenAI GPT Image model supported by the current provider, explicitly pass model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare. These are examples, not an exhaustive list. Here model is the drawing model. It defaults to the current session model when omitted, so explicitly select a drawing model when the session uses a chat model. Example: {"model":"gpt-image-2","prompt":"Make the scarf red; preserve the cat's face and composition","images":["./cat.png"]}. options accepts model-supported size, quality, n, output_format, and other Images edit fields. For another edit, pass the previous output's local image path back in images with the next prompt. Use chatgpt.image_generate for text-only generation; use chatgpt.responses_image_generation when you need Responses server-side response context. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support Images edits and the selected model. Only supplied images and prompt are uploaded; local paths are permission checked. Returned images are saved as separate managed local attachments, preserving the originals.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `images` | `array<string>` | ✓ | — | One to sixteen permitted local image paths to upload as originals or references. URLs and remote file IDs are not accepted here. |
| `model` | `string / null` | — | — | GPT Image model ID on the current provider, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Defaults to the session model; explicitly select an image model when the session uses a chat model. |
| `options` | `object` | — | — | Additional Images edit fields supported by the chosen model, e.g. size, quality, n, and output_format. model, prompt, and image are supplied separately. |
| `prompt` | `string` | ✓ | — | Natural-language edit instructions, including details to preserve from the supplied images. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "images": {
      "description": "One to sixteen permitted local image paths to upload as originals or references. URLs and remote file IDs are not accepted here.",
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
      "description": "GPT Image model ID on the current provider, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Defaults to the session model; explicitly select an image model when the session uses a chat model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "options": {
      "additionalProperties": true,
      "description": "Additional Images edit fields supported by the chosen model, e.g. size, quality, n, and output_format. model, prompt, and image are supplied separately.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    },
    "prompt": {
      "description": "Natural-language edit instructions, including details to preserve from the supplied images.",
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

### image_generate

`agena.chatgpt.image_generate` · **Summary**: Generate images directly with the OpenAI Images API; set model to a GPT Image drawing model such as gpt-image-2.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Prefer this for ordinary text-to-image tasks. Sends POST images/generations directly to a GPT Image drawing model using the prompt supplied by the current Agena AI. A plain-text prompt is sufficient. To select an OpenAI GPT Image model supported by the current provider, explicitly pass model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare. These are examples, not an exhaustive list. Here model is the drawing model. It defaults to the current session model when omitted, so explicitly select a drawing model when the session uses a chat model. Use action=create. Example: {"action":"create","model":"gpt-image-2","prompt":"A watercolor gray cat wearing a yellow scarf"}. parameters accepts model-supported size, quality, n, output_format, background, and other official image fields. Use chatgpt.image_edit for local-image edits or references. Use chatgpt.responses_image_generation when you need Responses server-side conversation context; that tool makes an additional chat-model request. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support the Images API and selected model. Prompts are sent to that endpoint; returned base64 images are persisted as managed local attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### image_understanding

`agena.chatgpt.image_understanding` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Send explicit images to OpenAI cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### inference

`agena.chatgpt.inference` · **Summary**: Run a vendor inference request with official tool combinations and continuation.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### knowledge

`agena.chatgpt.knowledge` · **Summary**: Create, populate, search and manage OpenAI vector stores.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, update, delete, search, add_file, list_files, get_file, update_file, delete_file, download_file, create_batch, get_batch, cancel_batch, list_batch_files. id is the vector store ID; item_id identifies a nested file or file batch. Upload with files.upload purpose=assistants, then add_file with parameters.file_id. Poll get_file until indexing completes; file_upload alone does not index. File batch creation uses parameters.file_ids. search returns chunks directly without a generation model.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### moderation

`agena.chatgpt.moderation` · **Summary**: Classify supplied content with the OpenAI Moderations API.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. parameters.input contains supported text/image inputs. Requires a moderation model, not an ordinary chat model. Returns provider categories and scores.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### platform

`agena.chatgpt.platform` · **Summary**: Manage OpenAI conversations, fine-tuning jobs, evaluations and uploads.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model for fine-tuning. Resource operations need no model. resource=conversations: create/get/update/delete/items/add_items/get_item/delete_item; fine_tuning/jobs: create/list/get/wait/cancel/events/pause/unpause; fine_tuning/jobs/checkpoints: list (id=job); fine_tuning/checkpoints/permissions: create/list/delete (id=checkpoint, item_id=permission); fine_tuning/alpha/graders: run/validate; evals: create/list/get/update/delete/create_run/list_runs/get_run/cancel_run/delete_run; evals/runs/output_items: list/get (id=eval, item_id=run, sub_item_id=output for get); uploads: create/add_part/complete/cancel (add_part path, complete parameters.part_ids); audio/voice_consents: create/list/get/update/delete (create path is the consent recording, parameters.name/language); audio/voices: create/delete (audio-sample creation uses path and parameters.name/consent, prompt creation uses parameters.type=prompt/name/prompt without a file). Voice recordings have a 10 MiB limit and supported audio MIME types. Nested grader model fields remain in parameters. No API key, billing or membership administration.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### realtime

`agena.chatgpt.realtime` · **Summary**: Run one bounded OpenAI realtime voice turn and save audio.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Defaults to the current session model. Use an OpenAI realtime voice model; otherwise explicitly pass model. Reuses the current provider endpoint and credentials. Supply text or an explicit raw 16-bit mono PCM audio_path. Returns transcript, actual WAV audio, completion state and provider events. Does not open a microphone or execute tool calls. duration_seconds is 1..50; partial output is retained on a receive deadline. Requires the gateway to expose the vendor WebSocket API.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `audio_path` | `string / null` | — | `null` | Explicit raw signed 16-bit little-endian mono PCM file, not WAV/MP3. |
| `duration_seconds` | `integer` | — | `20` | Complete turn deadline, 1..50 seconds. Partial audio is retained on timeout. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `input_sample_rate` | `integer` | — | `24000` | Input sample rate. OpenAI requires 24000; Gemini permits 16000 or 24000. |
| `model` | `string / null` | — | `null` | Defaults to current session model; explicitly pass a compatible OpenAI<br>Realtime or Google Gemini Live model when the current model is unsuitable. |
| `output_path` | `string / null` | — | `null` |  |
| `session` | `object` | — | `{}` | Official session/setup fields. model and output audio format are managed. |
| `text` | `string / null` | — | `null` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "audio_path": {
      "default": null,
      "description": "Explicit raw signed 16-bit little-endian mono PCM file, not WAV/MP3.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "duration_seconds": {
      "default": 20,
      "description": "Complete turn deadline, 1..50 seconds. Partial audio is retained on timeout.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000007"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "input_sample_rate": {
      "default": 24000,
      "description": "Input sample rate. OpenAI requires 24000; Gemini permits 16000 or 24000.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "description": "Defaults to current session model; explicitly pass a compatible OpenAI\nRealtime or Google Gemini Live model when the current model is unsuitable.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "output_path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "session": {
      "additionalProperties": true,
      "default": {},
      "description": "Official session/setup fields. model and output audio format are managed.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    },
    "text": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

### responses

`agena.chatgpt.responses` · **Summary**: Retrieve, wait for, cancel or delete OpenAI background responses.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model. Actions: create/get/wait/cancel/delete/items/compact. Create accepts prompt or parameters.input, optional background/tools, and top-level stream=true for bounded SSE. compact calls responses/compact with parameters.input and optional instructions. Other actions use id. wait_seconds is 1..50; get/wait never recreate a task. SSE returns events, sse_events with id/event/retry, and last_event_id; pass last_event_id to resume supported event resources. Continuation via previous_response_id creates a new response. Never retry create automatically.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### responses_image_generation

`agena.chatgpt.responses_image_generation` · **Summary**: Ask an additional OpenAI Responses chat model to call GPT Image; model selects chat, tool_options.model selects the drawing model.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Use when you need Responses server-side conversation context for drawing or editing. Submits an additional POST responses request: current Agena AI -> Responses chat model -> hosted image_generation tool / GPT Image drawing model -> image. The Responses chat model interprets the request, may rewrite the drawing prompt, and calls the drawing model to produce the image. Top-level model selects this additional Responses CHAT model; it defaults to the current session model, which must support the hosted image tool. If needed, explicitly pass model to select a compatible chat model. tool_options.model separately selects the DRAWING model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare when supported by the current provider. Put GPT Image IDs in tool_options.model. prompt can be plain text. Supply reference images through input_items using supported image URLs, base64 data URLs, or file IDs; upload or encode local images first. previous_response_id references a prior OpenAI response whose context the provider retains; it is not an Agena session ID. tool_options accepts action (auto/generate/edit), model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size according to model support. Prefer chatgpt.image_generate for ordinary text-to-image and chatgpt.image_edit for local-image edits or references; repeated direct edits can pass the prior output path back to image_edit. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support Responses and its hosted image tool. Only explicitly supplied prompts, input_items and referenced response context are included; Agena history and local project files are not automatically attached. Returned base64 images are persisted as managed local attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Explicit Responses history and image references (supported URLs, base64 data URLs, or file IDs). Upload or encode local images first. Agena conversation history is not automatically included; client tool callbacks are rejected. |
| `model` | `string / null` | — | — | Responses CHAT model that can call the hosted image_generation tool; defaults to the current session model. Select the GPT Image drawing model in tool_options.model. |
| `previous_response_id` | `string / null` | — | — | OpenAI response ID from an earlier call to this tool, used to continue its server-side context. This is not an Agena session ID. |
| `prompt` | `string / null` | — | — | Drawing or editing request for an additional Responses chat model, which calls the hosted GPT Image tool. May be omitted when input_items supplies the request. |
| `request_options` | `object` | — | — | Additional outer Responses request fields. model, input, tools, stream, and previous_response_id are supplied separately. |
| `stable_instructions` | `string / null` | — | — | Developer instructions for this additional Responses request. |
| `tool_options` | `object` | — | — | Hosted image tool settings. model selects the DRAWING model, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Other model-supported fields include action (auto/generate/edit), size, quality, and output_format. type is protected. |

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
      "description": "Explicit Responses history and image references (supported URLs, base64 data URLs, or file IDs). Upload or encode local images first. Agena conversation history is not automatically included; client tool callbacks are rejected.",
      "items": true,
      "type": "array",
      "x-agena-order": "000006"
    },
    "model": {
      "description": "Responses CHAT model that can call the hosted image_generation tool; defaults to the current session model. Select the GPT Image drawing model in tool_options.model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "previous_response_id": {
      "description": "OpenAI response ID from an earlier call to this tool, used to continue its server-side context. This is not an Agena session ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "prompt": {
      "description": "Drawing or editing request for an additional Responses chat model, which calls the hosted GPT Image tool. May be omitted when input_items supplies the request.",
      "maxLength": 64000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "request_options": {
      "additionalProperties": true,
      "description": "Additional outer Responses request fields. model, input, tools, stream, and previous_response_id are supplied separately.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000004"
    },
    "stable_instructions": {
      "description": "Developer instructions for this additional Responses request.",
      "maxLength": 256000,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "tool_options": {
      "additionalProperties": true,
      "description": "Hosted image tool settings. model selects the DRAWING model, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Other model-supported fields include action (auto/generate/edit), size, quality, and output_format. type is protected.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000003"
    }
  },
  "type": "object"
}
```

### shell

`agena.chatgpt.shell` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Run shell commands in an OpenAI cloud container, never in the local terminal.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `streaming`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Defaults to container_auto. Only container_auto or container_reference with container_id is accepted. Local/custom environments and client callbacks are rejected. Uploaded provider files are separate from Agena local files; there is no local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### speech

`agena.chatgpt.speech` · **Summary**: Generate speech and save the resulting audio.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. prompt is the exact text to speak; parameters.voice and other official speech fields control voice, format and style. Select an OpenAI TTS model. Saves actual audio bytes as an attachment; output_path optionally selects a new workspace file.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### transcription

`agena.chatgpt.transcription` · **Summary**: Transcribe or translate an audio file using OpenAI Audio APIs.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. path selects one authorized audio file; model defaults to the current session model, so explicitly pass an appropriate transcription model when necessary. resource=translations selects audio translation; otherwise transcription. parameters supports language, prompt, response_format and timestamp/diarization fields according to model compatibility. Text, JSON and subtitle outputs are retained as artifacts.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### web_search

`agena.chatgpt.web_search` · **Summary**: Requires an OpenAI model; defaults to the current model or accepts model override. Search the web in OpenAI cloud and return sources; not a local browser operation.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Hosted results and response_id are returned for follow-up; this plugin never executes client tool callbacks.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include` | `array<string>` | — | — | Optional Responses include selectors. |
| `input_items` | `array<any>` | — | — | Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

**Version** `0.1.0` · **Tools** 15

Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Anthropic cloud inference, search, code execution, files, Skills, batches and managed-agent APIs. Inputs leave this computer; no local execution fallback.

### advisor

`agena.claude.advisor` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Consult an advisor model in Anthropic cloud using the supplied context.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `streaming`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Only supplied context is available; the local repository and session transcript are not automatically uploaded. Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### agents

`agena.claude.agents` · **Summary**: Manage vendor-hosted agents, sessions and environments.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Anthropic Claude model or explicitly pass model for agent creation. Resource management needs no model. resource=agents (default): create/list/get/update/archive/list_versions/get_version; sessions: create/list/get/wait/update/delete/archive/events/create_event/stream_events; environments: create/list/get/update/delete/archive; deployments: create/list/get/update/archive/run/pause/unpause; deployment_runs: list/get. sessions/resources supports create/list/get/update/delete with id=session and item_id=resource. sessions/threads supports list/get/archive/events/stream_events with id=session and item_id=thread. Event streams retain id/event/retry and accept last_event_id; wait_seconds bounds receipt time. parameters/query/beta_headers follow official schemas. No local tools execute implicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### batches

`agena.claude.batches` · **Summary**: Create, inspect, cancel and retrieve asynchronous batch jobs.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, cancel; Claude/Gemini also delete, Claude also results. parameters contains official batch input. OpenAI result output_file_id is downloaded with files.download; Gemini output file names use files.download. Claude results are saved as a JSONL attachment. A wait is bounded and never resubmits a job.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### code_execution

`agena.claude.code_execution` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Execute code in Anthropic cloud infrastructure, not on this computer.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `streaming`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### count_tokens

`agena.claude.count_tokens` · **Summary**: Count tokens using the vendor token-counting endpoint.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### document_understanding

`agena.claude.document_understanding` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Send explicit PDF/text documents to Anthropic cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### file_delete

`agena.claude.file_delete` · **Summary**: Request deletion of an owned file from Anthropic cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

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

### file_status

`agena.claude.file_status` · **Summary**: Query the remote status of an owned Anthropic cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

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

### file_upload

`agena.claude.file_upload` · **Summary**: Upload one permitted local file to Anthropic cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

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

### files

`agena.claude.files` · **Summary**: Manage remote files and download cloud-generated artifacts.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### image_understanding

`agena.claude.image_understanding` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Send explicit images to Anthropic cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### inference

`agena.claude.inference` · **Summary**: Run a vendor inference request with official tool combinations and continuation.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### skills

`agena.claude.skills` · **Summary**: Create, inspect and manage Claude cloud Skills and versions.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, delete, create_version, list_versions, get_version, delete_version. create/create_version use path or paths for multipart files; include SKILL.md and supporting files. id is skill ID; item_id is version. Run Skills using inference with parameters.container.skills and the official code_execution tool plus required beta_headers; then download generated file IDs with files.download. These are cloud Skills, separate from local Agena skills.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### web_fetch

`agena.claude.web_fetch` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Fetch and process web content in Anthropic cloud, not through the local browser.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### web_search

`agena.claude.web_search` · **Summary**: Requires an Anthropic Claude model; defaults to the current model or accepts model override. Search the web in Anthropic cloud and return sources; not a local browser operation.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `beta_headers` | `array<string>` | — | — | Additional official Anthropic beta feature headers. |
| `cache_ttl` | `ClaudeCacheTtl / null` | — | — |  |
| `max_tokens` | `integer / null` | — | — |  |
| `messages` | `array<any>` | — | — | Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

`agena.commands.get` · **Summary**: Read one discovered skill.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Read one skill's instructions in full and apply them to the current task.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | `string` | ✓ | — | Canonical name, or an alias, of the skill to read. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "name": {
      "description": "Canonical name, or an alias, of the skill to read.",
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

`agena.commands.install` · **Summary**: Install a workspace-managed skill document.

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

`agena.commands.list` · **Summary**: List discovered skills.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> List discovered skills, with paging. Each entry names the skill, its source, and whether it can be edited in this workspace.

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

`agena.commands.remove` · **Summary**: Remove a workspace-managed skill document.

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

**Version** `0.1.0` · **Tools** 10

Filesystem command tools for read/search and explicit edits.

### apply_patch

`agena.fs.apply_patch` · **Summary**: Apply a text patch to workspace files.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Use `apply_patch` for explicit text patch operations against workspace files. The `patch` argument is a plain-text patch that MUST start with the exact marker line `*** Begin Patch` and end with the exact marker line `*** End Patch`. Inside, use only these directives: `*** Update File: <path>` followed by `@@`-separated hunks (context lines start with a space, removed lines with `-`, added lines with `+`), `*** Add File: <path>` with every content line prefixed by `+`, or `*** Delete File: <path>`. A patch that does not start with `*** Begin Patch` is rejected. Relative paths resolve against the current Agena workspace, not a previous shell command's working directory. When intentionally editing another worktree, use explicit absolute paths permitted by the runtime; a shell cd does not change subsequent file-tool path resolution.

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

**Runtime**: streaming `streaming`

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
> Use read for text previews and directory listings. offset/limit page 1-based lines or entries. For very long single lines, use byte_offset (zero-based) and byte_limit (4–16384, default 8192) to seek directly to a small UTF-8 text range; continue with read_info.next_byte_offset. Byte ranges cannot combine with line offsets/limits, directories or attachment mode. Binary files return local references, not model-visible bytes. Call fs.read_media to see a local image, PDF, audio or video with the current conversation model; use fs.document when you only need extracted PDF/Office text.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `byte_limit` | `integer / null` | — | — | Byte-range page size (4–16384; default 8192). May start at byte 0 when<br>byte_offset is omitted. UTF-8 boundaries are preserved. |
| `byte_offset` | `integer / null` | — | — | Zero-based byte position for a text byte-range read. Cannot be combined<br>with line/entry offset or limit. Useful for very long single lines. |
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
      "x-agena-order": "000005"
    }
  },
  "description": "Input of the file read tool.",
  "properties": {
    "byte_limit": {
      "description": "Byte-range page size (4–16384; default 8192). May start at byte 0 when\nbyte_offset is omitted. UTF-8 boundaries are preserved.",
      "format": "uint32",
      "maximum": 16384,
      "minimum": 4,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "byte_offset": {
      "description": "Zero-based byte position for a text byte-range read. Cannot be combined\nwith line/entry offset or limit. Useful for very long single lines.",
      "format": "uint64",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
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

### read_media

`agena.fs.read_media` · **Summary**: Read a local image, PDF, audio or video into the current model's context.

**Tags**: `query` `filesystem` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Call fs.read_media whenever you need to see an image/screenshot, inspect PDF page visuals, listen to audio, or inspect a video stored locally. This returns actual immutable file bytes to the current conversation model, like a user attachment; no separate cloud-analysis provider or credential configuration is needed. Use tools_help for the live schema, then tools_call with tool=fs.read_media and input={path:...}; paths resolve from the active workspace. Image and PDF support depends on the current model. Audio requires a supported OpenAI Chat audio model or Gemini; video requires Gemini. Unsupported protocols, models, MIME types or sizes return an explicit error; a filename/reference is not media understanding. Supported preparation includes PNG/JPEG/GIF/WebP, PDF, common audio/video and UTF-8 text; maximum 20 MiB/file, with lower encoded-request limits on some protocols. expected_sha256 optionally guards the revision. Use fs.read for ordinary text, fs.document for extracted PDF/Office text; do not repeatedly read binary data with fs.read expecting to see it.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_sha256` | `string / null` | — | — | Optional revision guard from fs.stat or a previous read_media result. |
| `path` | `string` | ✓ | — | Local image, PDF, audio, video or UTF-8 file. Relative paths resolve<br>against the active workspace, like the other filesystem tools. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Read immutable media bytes into the current conversation model's context.",
  "properties": {
    "expected_sha256": {
      "description": "Optional revision guard from fs.stat or a previous read_media result.",
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "path": {
      "description": "Local image, PDF, audio, video or UTF-8 file. Relative paths resolve\nagainst the active workspace, like the other filesystem tools.",
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

**Version** `0.1.0` · **Tools** 29

Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Google cloud inference, search, files, knowledge stores, images, audio, video, music, Live voice and agent APIs. Inputs leave this computer; no local execution fallback.

### agents

`agena.gemini.agents` · **Summary**: Manage vendor-hosted agents, sessions and environments.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions depend on the vendor resource: create, list, get, update, delete, wait, archive, events, create_event, items, list_versions, get_version, run, pause, unpause. Unsupported actions are rejected per collection. resource selects the documented collection. OpenAI: agents, sessions (default), environments. Claude: agents (default), sessions, environments, deployments. Gemini: agents. parameters/query carry official fields, including nested agent.model when required. Runs on vendor infrastructure; local files and tools are not automatically available. Inference on a Gemini agent uses inference with parameters.agent and no model override. Provider support and access are required.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### audio_understanding

`agena.gemini.audio_understanding` · **Summary**: Analyze audio with a selected Google Gemini model.

**Tags**: `network`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting audio input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### batches

`agena.gemini.batches` · **Summary**: Create, inspect, cancel and retrieve asynchronous batch jobs.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Google Gemini model supporting generation or embedding batches, or explicitly pass model. Actions: create/list/get/wait/cancel/delete/update. resource=generate_content (default) creates batchGenerateContent; resource=embeddings creates asyncBatchEmbedContent. parameters uses the official batch body. update calls updateGenerateContentBatch or updateEmbedContentBatch, with id=batches/ID, top-level model and the official update fields; query.updateMask selects fields. Get/wait/cancel/delete need only id. Results may be inline or downloaded using files.download. wait_seconds is 1..50; waits never resubmit jobs.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### caches

`agena.gemini.caches` · **Summary**: Manage explicit Gemini context caches for compatible models.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, update, delete. create uses the current session model unless model is supplied; parameters contains contents, ttl/expireTime and other official fields. update changes supported lifecycle fields using query.updateMask. id accepts cachedContents/ID. Explicit caching is model-dependent; models with implicit caching only reject this API.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### code_execution

`agena.gemini.code_execution` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Execute code in Google cloud infrastructure, not on this computer.

**Tags**: `network` `interactive` `read_only`

**Runtime**: streaming `streaming`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses the official Interactions code_execution declaration. Computation executes on Google infrastructure; no returned function call is executed by Agena.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### count_tokens

`agena.gemini.count_tokens` · **Summary**: Count tokens using the vendor token-counting endpoint.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### document_understanding

`agena.gemini.document_understanding` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Send explicit PDF/text documents to Google cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### embeddings

`agena.gemini.embeddings` · **Summary**: Create Gemini text or multimodal embeddings.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Google Gemini embedding model or explicitly pass model. create uses models/{model}:embedContent with prompt or parameters.content and optional taskType/title/outputDimensionality. create_batch uses models/{model}:batchEmbedContents with parameters.requests array; each request inherits the selected model if absent, and an explicit top-level model overrides entry models. Async embedding batches use batches with resource=embeddings. Large vectors are retained in the receipt.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### file_delete

`agena.gemini.file_delete` · **Summary**: Request deletion of an owned file from Google cloud; preserve the local original.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.

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

### file_search

`agena.gemini.file_search` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Search configured Google cloud file stores, not files on this computer.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. tool_options supports file_search_store_names, metadata_filter, and top_k.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### file_status

`agena.gemini.file_status` · **Summary**: Query the remote status of an owned Google cloud file, not a local path.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.

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

### file_upload

`agena.gemini.file_upload` · **Summary**: Upload one permitted local file to Google cloud and return a session-owned handle.

**Tags**: `mutate` `network`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Uploads accept arbitrary explicit data files up to 512 MiB and optional SHA256 revision checks; analysis has separate format/size limits. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.

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

### files

`agena.gemini.files` · **Summary**: Manage remote files and download cloud-generated artifacts.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### google_maps

`agena.gemini.google_maps` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Query Google Maps data in Google cloud and return grounding sources.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports enable_widget, latitude, and longitude.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

`agena.gemini.google_search` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Search Google and ground answers in Google cloud, not the local browser.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options.search_types accepts web_search, image_search, and enterprise_web_search.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

`agena.gemini.image_edit` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Upload permitted images for editing in Google cloud; save the returned image separately.

**Tags**: `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to Google; returned images are saved as separate local artifacts. Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cached_content` | `string / null` | — | — |  |
| `generation_config` | `object` | — | — |  |
| `images` | `array<string>` | ✓ | — |  |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

`agena.gemini.image_generation` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Generate images in Google cloud; save returned images as local attachments.

**Tags**: `network` `interactive` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses generateContent with responseModalities TEXT and IMAGE. Use the current session model or specify input.model. Inline image data is persisted as managed attachments.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cached_content` | `string / null` | — | — | Existing Gemini cachedContents resource name. |
| `generation_config` | `object` | — | — |  |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### image_understanding

`agena.gemini.image_understanding` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Send explicit images to Google cloud for understanding; not local file viewing.

**Tags**: `query` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

### inference

`agena.gemini.inference` · **Summary**: Run a vendor inference request with official tool combinations and continuation.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### interactions

`agena.gemini.interactions` · **Summary**: Retrieve, wait for, cancel or delete Gemini background interactions.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, get, wait, cancel, delete. create accepts prompt or parameters.input and background=true. For Deep Research/managed agents use parameters.agent instead of a model; supplying both is rejected. id is an existing interaction ID for get/wait/cancel/delete. Polling does not create a new interaction.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### knowledge

`agena.gemini.knowledge` · **Summary**: Create, populate and manage Gemini File Search Stores.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Uses the current provider connection. Model-based tools default to the current session model; use a compatible Google Gemini model or explicitly pass model. Resource management needs no model. Actions: create/list/get/delete/import_file/upload/list_files/get_file/delete_file. id is store ID or fileSearchStores/ID; item_id is document ID. upload directly sends one authorized path into the store using resumable upload; parameters supports displayName/mimeType/customMetadata/chunkingConfig. Alternatively file_upload then import_file with parameters.fileName=files/ID. Poll returned operations with operations.get/wait, including fileSearchStores/ID/upload/operations/ID. Upload readiness is different from indexing completion. Use file_search with the resulting store name. Delete query.force=true explicitly deletes a nonempty store.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### music_generation

`agena.gemini.music_generation` · **Summary**: Generate music using Gemini Lyria and save audio attachments.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Select an available Lyria music model. prompt or parameters.input describes the music and optional image inputs; parameters.response_format contains supported audio settings. Returned lyrics and metadata remain in the receipt; inline audio becomes a local attachment. This is not a speech or ordinary chat model operation.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### operations

`agena.gemini.operations` · **Summary**: Inspect or wait for Gemini file-import operations.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: get, wait. id is operations/ID or fileSearchStores/ID/operations/ID. wait_seconds is 1..50. Returns done/error/response without creating another import.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### realtime

`agena.gemini.realtime` · **Summary**: Run one bounded Google Gemini realtime voice turn and save audio.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Defaults to the current session model. Use a Google Gemini realtime voice model; otherwise explicitly pass model. Reuses the current provider endpoint and credentials. Supply text or an explicit raw 16-bit mono PCM audio_path. Returns transcript, actual WAV audio, completion state and provider events. Does not open a microphone or execute tool calls. duration_seconds is 1..50; partial output is retained on a receive deadline. Requires the gateway to expose the vendor WebSocket API.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `audio_path` | `string / null` | — | `null` | Explicit raw signed 16-bit little-endian mono PCM file, not WAV/MP3. |
| `duration_seconds` | `integer` | — | `20` | Complete turn deadline, 1..50 seconds. Partial audio is retained on timeout. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `input_sample_rate` | `integer` | — | `24000` | Input sample rate. OpenAI requires 24000; Gemini permits 16000 or 24000. |
| `model` | `string / null` | — | `null` | Defaults to current session model; explicitly pass a compatible OpenAI<br>Realtime or Google Gemini Live model when the current model is unsuitable. |
| `output_path` | `string / null` | — | `null` |  |
| `session` | `object` | — | `{}` | Official session/setup fields. model and output audio format are managed. |
| `text` | `string / null` | — | `null` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "audio_path": {
      "default": null,
      "description": "Explicit raw signed 16-bit little-endian mono PCM file, not WAV/MP3.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "duration_seconds": {
      "default": 20,
      "description": "Complete turn deadline, 1..50 seconds. Partial audio is retained on timeout.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000007"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "input_sample_rate": {
      "default": 24000,
      "description": "Input sample rate. OpenAI requires 24000; Gemini permits 16000 or 24000.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "model": {
      "default": null,
      "description": "Defaults to current session model; explicitly pass a compatible OpenAI\nRealtime or Google Gemini Live model when the current model is unsuitable.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000000"
    },
    "output_path": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "session": {
      "additionalProperties": true,
      "default": {},
      "description": "Official session/setup fields. model and output audio format are managed.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000005"
    },
    "text": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    }
  },
  "type": "object"
}
```

### speech

`agena.gemini.speech` · **Summary**: Generate Gemini text-to-speech audio as local attachments.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. prompt is the text to speak. Use a Gemini TTS model and parameters.generation_config.speech_config for single/multiple speakers and voices. Advanced input with speech_metadata belongs in parameters.input. Agena sets response_format.type=audio. Audio-only output is saved; output_path optionally selects a new local file.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### url_context

`agena.gemini.url_context` · **Summary**: Requires a Google Gemini model; defaults to the current model or accepts model override. Retrieve and ground URL content in Google cloud; no local-file access.

**Tags**: `network` `interactive` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses the official url_context tool. Put URLs in the prompt or official request fields.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `input_steps` | `array<any>` | — | — | Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted. |
| `model` | `string / null` | — | — | Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable. |
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
      "description": "Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.",
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

### video_edit

`agena.gemini.video_edit` · **Summary**: Edit or extend a video using Gemini multimodal interactions.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Select a supported video editing model. parameters.input contains the explicit video/media inputs and instructions, or use prompt with parameters.previous_interaction_id for conversational editing. parameters.response_format contains official video settings. Use interactions.get/wait for background work. Inline video becomes an attachment.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### video_generation

`agena.gemini.video_generation` · **Summary**: Generate a video using the Gemini Interactions API.

**Tags**: `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Use a supported Gemini video generation model, such as the current Omni family, not a chat-only model. prompt or parameters.input supplies text/images/audio/video. parameters.response_format supplies official video settings such as aspect_ratio and resolution. Supports background execution; inspect the returned interaction ID with interactions.get/wait. Inline video becomes an attachment; remote file references can be retrieved using files.download. Do not use retired Veo preview aliases.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `Action` | — | `list` | Operation. Each tool documents its supported actions. No automatic retries. |
| `beta_headers` | `array<string>` | — | `[]` | Official beta feature names only; cannot override authentication headers. |
| `expected_sha256` | `string / null` | — | `null` |  |
| `id` | `string / null` | — | `null` | Vendor resource ID, or its documented resource name (not an HTTP URL). |
| `item_id` | `string / null` | — | `null` | Nested resource ID, such as a vector-store file, skill version or event. |
| `last_event_id` | `string / null` | — | `null` | Resume an event stream from the provider's last SSE event ID. |
| `model` | `string / null` | — | `null` | Defaults to the current session model for model-based requests. Use a<br>model from this tool's vendor with the required capability; otherwise<br>explicitly pass model. Resource list/get/delete operations need no model. |
| `output_path` | `string / null` | — | `null` | Optional workspace output path. Existing files are never overwritten.<br>Omit to save downloads as managed attachments. |
| `parameters` | `object` | — | `{}` | Official API request fields. model/stream are controlled by Agena.<br>This is request JSON, never local commands or connection configuration. |
| `path` | `string / null` | — | `null` | Explicit local input file for multipart uploads/transcription. |
| `paths` | `array<string>` | — | `[]` | Additional local paths, e.g. the files of a custom Claude Skill. |
| `prompt` | `string / null` | — | `null` |  |
| `query` | `object` | — | `{}` | Pagination, filters and other official query parameters. |
| `resource` | `string / null` | — | `null` | Optional collection for platform/managed-agent operations; see tool help. |
| `stream` | `boolean` | — | `false` | Opt in to a bounded inference event stream. Returns complete received<br>events and a cursor; does not keep a background connection open. |
| `sub_item_id` | `string / null` | — | `null` | Third-level resource ID, e.g. an eval output item or a subagent turn. |
| `wait_seconds` | `integer` | — | `20` | Bound a wait call to 1..50 seconds. An unfinished task is returned as<br>pending with its ID; call wait again rather than creating another task. |

**Input schema**:
```json
{
  "$defs": {
    "Action": {
      "description": "Operation. Each tool documents its supported actions. No automatic retries.",
      "enum": [
        "list",
        "create",
        "get",
        "update",
        "delete",
        "download",
        "upload",
        "cancel",
        "wait",
        "search",
        "add_file",
        "list_files",
        "get_file",
        "update_file",
        "delete_file",
        "download_file",
        "import_file",
        "create_batch",
        "get_batch",
        "cancel_batch",
        "list_batch_files",
        "results",
        "create_version",
        "list_versions",
        "get_version",
        "delete_version",
        "events",
        "items",
        "archive",
        "add_part",
        "complete",
        "add_items",
        "get_item",
        "delete_item",
        "create_run",
        "list_runs",
        "get_run",
        "cancel_run",
        "create_event",
        "pause",
        "unpause",
        "run",
        "compact",
        "stream_events",
        "delete_run",
        "validate"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/Action",
      "default": "list",
      "description": "Operation. Each tool documents its supported actions. No automatic retries."
    },
    "beta_headers": {
      "default": [],
      "description": "Official beta feature names only; cannot override authentication headers.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000015"
    },
    "expected_sha256": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000012"
    },
    "id": {
      "default": null,
      "description": "Vendor resource ID, or its documented resource name (not an HTTP URL).",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "item_id": {
      "default": null,
      "description": "Nested resource ID, such as a vector-store file, skill version or event.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "last_event_id": {
      "default": null,
      "description": "Resume an event stream from the provider's last SSE event ID.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000016"
    },
    "model": {
      "default": null,
      "description": "Defaults to the current session model for model-based requests. Use a\nmodel from this tool's vendor with the required capability; otherwise\nexplicitly pass model. Resource list/get/delete operations need no model.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "output_path": {
      "default": null,
      "description": "Optional workspace output path. Existing files are never overwritten.\nOmit to save downloads as managed attachments.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000013"
    },
    "parameters": {
      "additionalProperties": true,
      "default": {},
      "description": "Official API request fields. model/stream are controlled by Agena.\nThis is request JSON, never local commands or connection configuration.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000008"
    },
    "path": {
      "default": null,
      "description": "Explicit local input file for multipart uploads/transcription.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000010"
    },
    "paths": {
      "default": [],
      "description": "Additional local paths, e.g. the files of a custom Claude Skill.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000011"
    },
    "prompt": {
      "default": null,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000006"
    },
    "query": {
      "additionalProperties": {
        "type": "string"
      },
      "default": {},
      "description": "Pagination, filters and other official query parameters.",
      "properties": {},
      "type": "object",
      "x-agena-order": "000009"
    },
    "resource": {
      "default": null,
      "description": "Optional collection for platform/managed-agent operations; see tool help.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "stream": {
      "default": false,
      "description": "Opt in to a bounded inference event stream. Returns complete received\nevents and a cursor; does not keep a background connection open.",
      "type": "boolean",
      "x-agena-order": "000007"
    },
    "sub_item_id": {
      "default": null,
      "description": "Third-level resource ID, e.g. an eval output item or a subagent turn.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "wait_seconds": {
      "default": 20,
      "description": "Bound a wait call to 1..50 seconds. An unfinished task is returned as\npending with its ID; call wait again rather than creating another task.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000014"
    }
  },
  "type": "object"
}
```

### video_understanding

`agena.gemini.video_understanding` · **Summary**: Analyze video with a selected Google Gemini model.

**Tags**: `network`

**Runtime**: streaming `buffered`

**Help**:
> Model-based tools default to the current session model. Use a Google Gemini model supporting video input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `detail` | `ImageDetail` | — | `auto` |  |
| `inputs` | `array<MediaSource>` | ✓ | — |  |
| `max_output_tokens` | `integer` | — | `4096` |  |
| `model` | `string / null` | — | `null` | Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable. |
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
      "description": "Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.",
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

## agena.inline_ui

**Version** `0.1.0` · **Tools** 5

Inline Generative UI: composable, stateful interfaces and live work updates inside answers. Start with guide; discover layout/state/actions/live progressively, then prepare/read/update.

### guide

`agena.inline_ui.guide` · **Summary**: Learn Inline Generative UI progressively: overview or one kind's schema and example.

**Tags**: `read_only` `discovery`

**Runtime**: streaming `buffered`

**Help**:
> Read overview once, then only the needed topic. Results are instructions, not a published UI. Prepare content, then put the returned embed in your answer; validate is an optional dry run.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `topic` | `GuideTopic` | — | `overview` | Start with overview; request only the content kind needed for this answer. |

**Input schema**:
```json
{
  "$defs": {
    "GuideTopic": {
      "description": "Start with overview; request only the content kind needed for this answer.",
      "enum": [
        "overview",
        "paragraph",
        "callout",
        "steps",
        "comparison",
        "metrics",
        "table",
        "details",
        "chart",
        "panel",
        "layout",
        "state",
        "actions",
        "live"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "topic": {
      "$ref": "#/$defs/GuideTopic",
      "default": "overview",
      "description": "Start with overview; request only the content kind needed for this answer."
    }
  },
  "type": "object"
}
```

### prepare

`agena.inline_ui.prepare` · **Summary**: Prepare immutable rich content and return an embed reference. Insert it at the desired position in your normal answer; this tool does not display the content.

**Tags**: `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Read guide for the needed kinds. Prepares one versioned passage without displaying it. Paste the entire returned inline-ui fenced block at its intended position between normal Markdown paragraphs. Keep only the reference ID inside; do not add an outer code fence. All text is plain text; no HTML/JS, services, callbacks, or side effects. Use supplied facts, label examples, and keep sources/uncertainty. Do include the returned embed in the final answer; do not print the document JSON. Invalid input creates no reference.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `any` | ✓ | — | Bounded semantic document: version=1, optional title, ordered blocks.<br>Read guide(topic) for exact schemas. No HTML, commands or executable code. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "description": "Bounded semantic document: version=1, optional title, ordered blocks.\nRead guide(topic) for exact schemas. No HTML, commands or executable code.",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### read

`agena.inline_ui.read` · **Summary**: Read a prepared UI's latest revision and document before editing. User-local selections are not agent data.

**Tags**: `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `reference` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "reference": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "reference"
  ],
  "type": "object"
}
```

### update

`agena.inline_ui.update` · **Summary**: Atomically update the same inline UI while working: data, nodes, complete document or phase. Requires expected_revision; conflicts apply nothing. Existing user input is preserved.

**Tags**: `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Read guide(topic=live). Display the prepared reference early in the answer, then call update after actual work changes. Prefer a panel patch with block_id and changed data or whole-node upserts. Never invent progress. Use phase=complete when finished. This changes only agent-authored UI content, not the user's local selections or external services.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `update` | `InlineUpdate` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "InlineBlock": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "program": {
              "$ref": "#/$defs/UiProgram"
            },
            "type": {
              "const": "panel",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "program"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "text": {
              "type": "string"
            },
            "type": {
              "const": "paragraph",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "text"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "text": {
              "type": "string"
            },
            "title": {
              "type": "string"
            },
            "tone": {
              "$ref": "#/$defs/InlineTone",
              "default": "note"
            },
            "type": {
              "const": "callout",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "text"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "items": {
              "items": {
                "$ref": "#/$defs/InlineStep"
              },
              "type": "array"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "steps",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "items"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "columns": {
              "items": {
                "type": "string"
              },
              "type": "array"
            },
            "id": {
              "type": "string"
            },
            "rows": {
              "items": {
                "$ref": "#/$defs/InlineComparisonRow"
              },
              "type": "array"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "comparison",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "columns",
            "rows"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "items": {
              "items": {
                "$ref": "#/$defs/InlineMetric"
              },
              "type": "array"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "metrics",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "items"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "columns": {
              "items": {
                "type": "string"
              },
              "type": "array"
            },
            "id": {
              "type": "string"
            },
            "rows": {
              "items": {
                "items": {
                  "type": "string"
                },
                "type": "array"
              },
              "type": "array"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "table",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "columns",
            "rows"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "id": {
              "type": "string"
            },
            "text": {
              "type": "string"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "details",
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "title",
            "text"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "chart_type": {
              "$ref": "#/$defs/InlineChartType"
            },
            "id": {
              "type": "string"
            },
            "labels": {
              "items": {
                "type": "string"
              },
              "type": "array"
            },
            "series": {
              "items": {
                "$ref": "#/$defs/InlineSeries"
              },
              "type": "array"
            },
            "title": {
              "type": "string"
            },
            "type": {
              "const": "chart",
              "type": "string"
            },
            "x_label": {
              "type": "string"
            },
            "y_label": {
              "type": "string"
            }
          },
          "required": [
            "type",
            "id",
            "title",
            "chart_type",
            "labels",
            "series"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    },
    "InlineChartType": {
      "enum": [
        "line",
        "bar",
        "area",
        "pie"
      ],
      "type": "string"
    },
    "InlineComparisonRow": {
      "additionalProperties": false,
      "properties": {
        "label": {
          "type": "string"
        },
        "values": {
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "label",
        "values"
      ],
      "type": "object"
    },
    "InlineDocument": {
      "additionalProperties": false,
      "properties": {
        "blocks": {
          "description": "Ordered semantic content. Each id is unique within this document.",
          "items": {
            "$ref": "#/$defs/InlineBlock"
          },
          "type": "array"
        },
        "title": {
          "description": "Optional heading for the whole passage, not a separate app/window title.",
          "type": "string"
        },
        "version": {
          "description": "Protocol version. Currently exactly 1; unknown versions are rejected.",
          "format": "uint8",
          "maximum": 255,
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "version",
        "blocks"
      ],
      "type": "object"
    },
    "InlineIcon": {
      "enum": [
        "number",
        "shield",
        "layers",
        "sparkles",
        "link",
        "check",
        "search",
        "code"
      ],
      "type": "string"
    },
    "InlineMetric": {
      "additionalProperties": false,
      "properties": {
        "detail": {
          "type": "string"
        },
        "label": {
          "type": "string"
        },
        "value": {
          "description": "A supplied fact including its unit. Never an inferred or invented value.",
          "type": "string"
        }
      },
      "required": [
        "label",
        "value"
      ],
      "type": "object"
    },
    "InlinePanelPatch": {
      "additionalProperties": false,
      "properties": {
        "block_id": {
          "type": "string"
        },
        "data": {
          "additionalProperties": true,
          "default": {},
          "properties": {},
          "type": "object"
        },
        "nodes": {
          "default": [],
          "description": "Whole-node upserts preserve identity and allow incremental layout growth.",
          "items": {
            "$ref": "#/$defs/UiNode"
          },
          "type": "array"
        },
        "remove": {
          "default": [],
          "description": "Explicitly remove a subtree. Dangling/duplicate/cyclic references reject\nthe entire transaction, leaving the previous valid revision visible.",
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "block_id"
      ],
      "type": "object"
    },
    "InlinePhase": {
      "enum": [
        "working",
        "complete",
        "error"
      ],
      "type": "string"
    },
    "InlineSeries": {
      "additionalProperties": false,
      "properties": {
        "name": {
          "type": "string"
        },
        "values": {
          "items": {
            "type": [
              "number",
              "null"
            ]
          },
          "type": "array"
        }
      },
      "required": [
        "name",
        "values"
      ],
      "type": "object"
    },
    "InlineStep": {
      "additionalProperties": false,
      "properties": {
        "icon": {
          "$ref": "#/$defs/InlineIcon",
          "default": "number"
        },
        "label": {
          "description": "Optional short stage label, such as \"P0\". Does not imply execution status.",
          "type": "string"
        },
        "text": {
          "type": "string"
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "title",
        "text"
      ],
      "type": "object"
    },
    "InlineTone": {
      "enum": [
        "note",
        "tip",
        "warning"
      ],
      "type": "string"
    },
    "InlineUpdate": {
      "additionalProperties": false,
      "properties": {
        "document": {
          "anyOf": [
            {
              "$ref": "#/$defs/InlineDocument"
            },
            {
              "type": "null"
            }
          ]
        },
        "expected_revision": {
          "format": "uint64",
          "minimum": 0,
          "type": "integer"
        },
        "patch": {
          "anyOf": [
            {
              "$ref": "#/$defs/InlinePanelPatch"
            },
            {
              "type": "null"
            }
          ]
        },
        "phase": {
          "anyOf": [
            {
              "$ref": "#/$defs/InlinePhase"
            },
            {
              "type": "null"
            }
          ]
        },
        "reference": {
          "type": "string"
        }
      },
      "required": [
        "reference",
        "expected_revision"
      ],
      "type": "object",
      "x-agena-order": "000000"
    },
    "UiAction": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "key": {
              "type": "string"
            },
            "type": {
              "const": "set",
              "type": "string"
            },
            "value": true
          },
          "required": [
            "type",
            "key",
            "value"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "key": {
              "type": "string"
            },
            "type": {
              "const": "toggle",
              "type": "string"
            }
          },
          "required": [
            "type",
            "key"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "type": {
              "const": "reset",
              "type": "string"
            }
          },
          "required": [
            "type"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "text": true,
            "type": {
              "const": "copy",
              "type": "string"
            }
          },
          "required": [
            "type",
            "text"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    },
    "UiKind": {
      "enum": [
        "stack",
        "row",
        "grid",
        "card",
        "text",
        "heading",
        "metric",
        "badge",
        "divider",
        "tabs",
        "select",
        "checkbox",
        "switch",
        "slider",
        "number",
        "input",
        "button",
        "progress",
        "chart",
        "table",
        "tasks",
        "timeline",
        "callout",
        "details"
      ],
      "type": "string"
    },
    "UiNode": {
      "additionalProperties": false,
      "properties": {
        "action": {
          "description": "Explicitly declared local action; not an arbitrary tool or URL.",
          "type": [
            "string",
            "null"
          ]
        },
        "bind": {
          "description": "A typed local state field. Controls write only this field.",
          "type": [
            "string",
            "null"
          ]
        },
        "children": {
          "default": [],
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "id": {
          "type": "string"
        },
        "kind": {
          "$ref": "#/$defs/UiKind"
        },
        "props": {
          "additionalProperties": true,
          "default": {},
          "properties": {},
          "type": "object"
        },
        "visible": {
          "description": "Boolean expression; hidden subtrees do not render or accept events."
        }
      },
      "required": [
        "id",
        "kind"
      ],
      "type": "object"
    },
    "UiOption": {
      "additionalProperties": false,
      "properties": {
        "label": {
          "type": "string"
        },
        "value": {
          "type": "string"
        }
      },
      "required": [
        "value",
        "label"
      ],
      "type": "object"
    },
    "UiProgram": {
      "additionalProperties": false,
      "properties": {
        "actions": {
          "additionalProperties": {
            "items": {
              "$ref": "#/$defs/UiAction"
            },
            "type": "array"
          },
          "default": {},
          "properties": {},
          "type": "object"
        },
        "computed": {
          "additionalProperties": true,
          "default": {},
          "properties": {},
          "type": "object"
        },
        "data": {
          "additionalProperties": true,
          "default": {},
          "properties": {},
          "type": "object"
        },
        "nodes": {
          "items": {
            "$ref": "#/$defs/UiNode"
          },
          "type": "array"
        },
        "root": {
          "type": "string"
        },
        "state": {
          "additionalProperties": {
            "$ref": "#/$defs/UiStateField"
          },
          "default": {},
          "properties": {},
          "type": "object"
        }
      },
      "required": [
        "root",
        "nodes"
      ],
      "type": "object"
    },
    "UiStateField": {
      "additionalProperties": false,
      "properties": {
        "initial": true,
        "kind": {
          "$ref": "#/$defs/UiStateKind"
        },
        "max": {
          "type": [
            "number",
            "null"
          ]
        },
        "max_length": {
          "default": 1024,
          "format": "uint",
          "minimum": 0,
          "type": "integer"
        },
        "min": {
          "type": [
            "number",
            "null"
          ]
        },
        "options": {
          "default": [],
          "items": {
            "$ref": "#/$defs/UiOption"
          },
          "type": "array"
        },
        "step": {
          "type": [
            "number",
            "null"
          ]
        }
      },
      "required": [
        "kind",
        "initial"
      ],
      "type": "object"
    },
    "UiStateKind": {
      "enum": [
        "boolean",
        "number",
        "text",
        "choice",
        "selection"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "update": {
      "$ref": "#/$defs/InlineUpdate"
    }
  },
  "required": [
    "update"
  ],
  "type": "object"
}
```

### validate

`agena.inline_ui.validate` · **Summary**: Check an inline reply document without publishing it; reports precise validation errors.

**Tags**: `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `any` | ✓ | — | Bounded semantic document: version=1, optional title, ordered blocks.<br>Read guide(topic) for exact schemas. No HTML, commands or executable code. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "description": "Bounded semantic document: version=1, optional title, ordered blocks.\nRead guide(topic) for exact schemas. No HTML, commands or executable code.",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "document"
  ],
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
> Use for a decision that belongs to the user: a preference, an ambiguous requirement, authorization for a specific dangerous action, or whether to squash a concrete commit range before an authorized push. Show the target, consequences, and distinct options including a safe decline/defer choice. For a written answer, include an option with custom: true; that option may be the only choice. Reuse authorization already given for the same action and scope. If a sensible default exists within that authorization or you can verify the answer yourself, proceed. Ask all necessary questions together through this tool, never by ending the turn with a plain-text question. A timeout, cancellation, or empty answer is not approval; continue only independent, already-authorized work. Do not ask a generic 'should I proceed?' or seek plan approval here; use plan.review for plan approval.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `auto_resolution_ms` | `integer / null` | — | — | Automatically continue without an answer after this many milliseconds.<br>Values are limited to 60 seconds through 10 minutes.<br>A timeout is not approval; only independent, already-authorized work may continue. |
| `body_markdown` | `string` | — | — | Optional Markdown body shown in the review dialog. Only the plan<br>approval review (`kind == "review"`) sets it to the full plan document;<br>other ask_user requests leave it empty. |
| `kind` | `string` | — | — |  |
| `questions` | `array<UserInputQuestion>` | — | — | One to three questions. Each option is a normal choice unless `custom: true`,<br>which lets the user select it and write an answer. A custom option may be<br>the only option; do not invent a fixed choice for free-text questions. |
| `title` | `string` | — | — |  |

**Input schema**:
```json
{
  "$defs": {
    "UserInputOption": {
      "description": "Option offered in a user input question.",
      "properties": {
        "custom": {
          "description": "This option accepts a user-written answer instead of its label.\nA question may contain only this option. At most one custom option is supported.",
          "type": "boolean"
        },
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
          "description": "Compatibility for older requests. New questions use an option with `custom: true`.",
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
      "description": "Automatically continue without an answer after this many milliseconds.\nValues are limited to 60 seconds through 10 minutes.\nA timeout is not approval; only independent, already-authorized work may continue.",
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
      "description": "One to three questions. Each option is a normal choice unless `custom: true`,\nwhich lets the user select it and write an answer. A custom option may be\nthe only option; do not invent a fixed choice for free-text questions.",
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

**Version** `0.1.0` · **Tools** 26 · **Condition** `runtime:mcp-manager`

MCP discovery and bridge tools.

### connections.auth.cancel

`agena.mcp.connections.auth.cancel` · **Summary**: Cancel the pending OAuth flow for this connection.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `flow_id` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "flow_id": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "id",
    "flow_id"
  ],
  "type": "object"
}
```

### connections.auth.complete

`agena.mcp.connections.auth.complete` · **Summary**: Finish a pending OAuth sign-in using the exact returned callback URL.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `callback_url` | `string` | ✓ | — |  |
| `flow_id` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "callback_url": {
      "type": "string",
      "x-agena-order": "000002"
    },
    "flow_id": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "id",
    "flow_id",
    "callback_url"
  ],
  "type": "object"
}
```

### connections.auth.logout

`agena.mcp.connections.auth.logout` · **Summary**: Remove stored OAuth credentials while preserving any bearer credential.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — | Opaque ID returned by connections.list or connections.get. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Opaque ID returned by connections.list or connections.get.",
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

### connections.auth.start

`agena.mcp.connections.auth.start` · **Summary**: Start OAuth sign-in and return the authorization URL and pending flow ID.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |
| `redirect_uri` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    },
    "redirect_uri": {
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "id",
    "redirect_uri"
  ],
  "type": "object"
}
```

### connections.bridge.set_enabled

`agena.mcp.connections.bridge.set_enabled` · **Summary**: Enable or disable the built-in external MCP bridge in a configuration scope.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | `boolean` | ✓ | — |  |
| `scope` | `McpConnectionScope` | — | `workspace` |  |

**Input schema**:
```json
{
  "$defs": {
    "McpConnectionScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "properties": {
    "enabled": {
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/McpConnectionScope",
      "default": "workspace"
    }
  },
  "required": [
    "enabled"
  ],
  "type": "object"
}
```

### connections.create

`agena.mcp.connections.create` · **Summary**: Create a persistent external MCP connection and queue activation.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `config` | `any` | ✓ | — |  |
| `name` | `string` | ✓ | — |  |
| `scope` | `McpConnectionScope` | — | `workspace` |  |

**Input schema**:
```json
{
  "$defs": {
    "McpConnectionScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "properties": {
    "config": true,
    "name": {
      "type": "string"
    },
    "scope": {
      "$ref": "#/$defs/McpConnectionScope",
      "default": "workspace"
    }
  },
  "required": [
    "name",
    "config"
  ],
  "type": "object"
}
```

### connections.credential.delete

`agena.mcp.connections.credential.delete` · **Summary**: Delete the stored bearer credential for this connection endpoint.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — | Opaque ID returned by connections.list or connections.get. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Opaque ID returned by connections.list or connections.get.",
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

### connections.credential.set

`agena.mcp.connections.credential.set` · **Summary**: Store an encrypted bearer credential for this connection endpoint.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |
| `token` | `string` | ✓ | — | Bearer credential to store. Never returned by this tool. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    },
    "token": {
      "description": "Bearer credential to store. Never returned by this tool.",
      "type": "string",
      "x-agena-order": "000001"
    }
  },
  "required": [
    "id",
    "token"
  ],
  "type": "object"
}
```

### connections.delete

`agena.mcp.connections.delete` · **Summary**: Delete a connection from its scope using its current revision.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_revision": {
      "type": "string",
      "x-agena-order": "000001"
    },
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "id",
    "expected_revision"
  ],
  "type": "object"
}
```

### connections.get

`agena.mcp.connections.get` · **Summary**: Read a connection, its revision and redacted configuration.

**Tags**: `query` `mcp` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — | Opaque ID returned by connections.list or connections.get. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Opaque ID returned by connections.list or connections.get.",
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

### connections.import

`agena.mcp.connections.import` · **Summary**: Atomically import selected connections using the preview revision and original document.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `any` | ✓ | — |  |
| `expected_revision` | `string` | ✓ | — |  |
| `replace` | `boolean` | — | `false` |  |
| `scope` | `McpConnectionScope` | — | `workspace` |  |
| `selected` | `array<string>` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "McpConnectionScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "properties": {
    "document": true,
    "expected_revision": {
      "type": "string"
    },
    "replace": {
      "default": false,
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/McpConnectionScope",
      "default": "workspace"
    },
    "selected": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "document",
    "selected",
    "expected_revision"
  ],
  "type": "object"
}
```

### connections.import_preview

`agena.mcp.connections.import_preview` · **Summary**: Validate native servers or mcpServers JSON and preview redacted candidates without saving.

**Tags**: `query` `mcp` `read_only`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `any` | ✓ | — |  |
| `scope` | `McpConnectionScope` | — | `workspace` |  |

**Input schema**:
```json
{
  "$defs": {
    "McpConnectionScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "properties": {
    "document": true,
    "scope": {
      "$ref": "#/$defs/McpConnectionScope",
      "default": "workspace"
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### connections.list

`agena.mcp.connections.list` · **Summary**: List saved global and server-workspace MCP connections with live status.

**Tags**: `query` `mcp` `read_only`

**Runtime**: streaming `buffered`

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### connections.reconnect

`agena.mcp.connections.reconnect` · **Summary**: Reconnect the effective enabled connection and refresh capabilities.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — | Opaque ID returned by connections.list or connections.get. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Opaque ID returned by connections.list or connections.get.",
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

### connections.set_enabled

`agena.mcp.connections.set_enabled` · **Summary**: Enable or disable a saved MCP connection and queue activation.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | `boolean` | ✓ | — |  |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "properties": {
    "enabled": {
      "type": "boolean"
    },
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "id",
    "enabled",
    "expected_revision"
  ],
  "type": "object"
}
```

### connections.test

`agena.mcp.connections.test` · **Summary**: Test an isolated connection and close it; disabled and shadowed copies can be tested.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — | Opaque ID returned by connections.list or connections.get. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "Opaque ID returned by connections.list or connections.get.",
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

### connections.update

`agena.mcp.connections.update` · **Summary**: Replace connection configuration using its revision; explicitly preserve returned secret paths.

**Tags**: `mcp` `mutate`

**Runtime**: streaming `buffered`

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `config` | `any` | ✓ | — |  |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |
| `preserve_secrets` | `array<string>` | — | `[]` |  |

**Input schema**:
```json
{
  "properties": {
    "config": true,
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string",
      "x-agena-order": "000000"
    },
    "preserve_secrets": {
      "default": [],
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "id",
    "config",
    "expected_revision"
  ],
  "type": "object"
}
```

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

WebSocket event subscriptions with background notifications. Use shell.watch for local command-output monitoring.

### start

`agena.monitor.start` · **Summary**: Subscribe to WebSocket text events and notify the AI in bounded batches.

**Tags**: `execute` `network` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Start a background WebSocket subscription using ws.url and optional protocols. This tool does not execute local commands; use shell.watch for command-output events. The endpoint is checked as a network effect. Return monitor_id and continue working; text events arrive as bounded system_notification batches at most once per second, followed by a final notification when the subscription ends. Do not poll or sleep to wait. timeout_ms defaults to 300000 and is enforced across connection establishment and the feed lifetime (maximum 3600000). The subscription also ends on disconnect, explicit stop, cancellation or session end. Use monitor.stop for cleanup.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `description` | `string` | — | `` |  |
| `timeout_ms` | `integer / null` | — | — |  |
| `ws` | `object` | ✓ | — |  |

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
      "type": "object",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "description": {
      "default": "",
      "type": "string",
      "x-agena-order": "000002"
    },
    "timeout_ms": {
      "format": "uint64",
      "maximum": 3600000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "ws": {
      "$ref": "#/$defs/MonitorWsInput",
      "properties": {
        "protocols": {
          "items": {
            "type": "string"
          },
          "type": "array",
          "x-agena-order": "000000.000001"
        },
        "url": {
          "minLength": 1,
          "type": "string",
          "x-agena-order": "000000.000000"
        }
      },
      "required": [
        "url"
      ],
      "type": "object"
    }
  },
  "required": [
    "ws"
  ],
  "type": "object"
}
```

### stop

`agena.monitor.stop` · **Summary**: Stop one WebSocket subscription.

**Tags**: `mutate` `network`

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
> Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. This tool never blocks on the user: it saves the plan and returns. With `request_approval: true` (the default) the plan stays in the `planning` phase and you must call `plan.review` to request user approval before it becomes active. Pass `request_approval: false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation` — the plan then becomes active immediately. Never change settings to bypass approval. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `agent.spawn` exploration when the scope spans multiple areas), clarify with `interaction.ask` when available, replace the plan content with `plan.set`, and update progress/notes with `plan.edit`. When the plan is complete, call `plan.review` to present it for approval; never ask whether the plan is acceptable via `interaction.ask.

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

## agena.provider

**Version** `0.1.0` · **Tools** 1

Discover model routes across all enabled adapters of the current provider.

### models

`agena.provider.models` · **Summary**: List or inspect model routes across all enabled adapters of the current provider, including discovery sources and failures.

**Tags**: `query` `discovery` `network`

**Runtime**: streaming `buffered`

**Help**:
> Use this to choose a model before calling chatgpt, claude, gemini or other provider tools. The current session identifies the provider; its selected chat model does not restrict the results. action=list returns compact routes across all enabled adapters, with optional query text search, adapter_id, limit (1–200, default 100), and cursor pagination. Continue with next_cursor and the same filters; restart if the list changes. action=get requires an exact id and returns its full metadata; specify adapter_id if the ID occurs on multiple adapters. Provider API pagination is followed automatically. Models include source=provider_discovery or configured; adapters reports discovery status and safe warnings. complete=false means some remote discoveries failed; has_more refers only to this tool's output pages. Configured-only adapters retain that policy. Catalog data enriches returned routes without adding catalog-only models. No model type is inferred for filtering. Reuses the current provider's connection and checks network permission; no model, API key or endpoint input is required. A listed model does not guarantee support for a specific API or account operation.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `action` | `ModelsAction` | — | `list` | list returns compact model routes; get returns full metadata for id. |
| `adapter_id` | `string / null` | — | `null` | Optional adapter within the current provider. Omit to query all enabled adapters. |
| `cursor` | `string / null` | — | `null` | Opaque next_cursor from list; keep the same query and adapter_id.<br>If the provider's model list changes, restart without a cursor. |
| `id` | `string / null` | — | `null` | Exact model ID for get. Use adapter_id when the ID has multiple routes. |
| `limit` | `integer` | — | `100` | Maximum list entries per page, 1–200. Defaults to 100. |
| `query` | `string / null` | — | `null` | Optional case-insensitive text search over IDs and existing metadata.<br>This is a text filter, not a model-type classifier. list only. |

**Input schema**:
```json
{
  "$defs": {
    "ModelsAction": {
      "description": "list returns compact model routes; get returns full metadata for id.",
      "enum": [
        "list",
        "get"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "properties": {
    "action": {
      "$ref": "#/$defs/ModelsAction",
      "default": "list",
      "description": "list returns compact model routes; get returns full metadata for id."
    },
    "adapter_id": {
      "default": null,
      "description": "Optional adapter within the current provider. Omit to query all enabled adapters.",
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "cursor": {
      "default": null,
      "description": "Opaque next_cursor from list; keep the same query and adapter_id.\nIf the provider's model list changes, restart without a cursor.",
      "maxLength": 256,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "id": {
      "default": null,
      "description": "Exact model ID for get. Use adapter_id when the ID has multiple routes.",
      "maxLength": 512,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "limit": {
      "default": 100,
      "description": "Maximum list entries per page, 1–200. Defaults to 100.",
      "format": "uint",
      "maximum": 200,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000004"
    },
    "query": {
      "default": null,
      "description": "Optional case-insensitive text search over IDs and existing metadata.\nThis is a text filter, not a model-type classifier. list only.",
      "maxLength": 256,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000001"
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

**Version** `0.1.0` · **Tools** 11

Shell commands: exec waits, spawn runs in background, watch monitors output events, open provides an interactive terminal/PTY; shared logs and lifecycle controls manage them.

### exec

`agena.shell.exec` · **Summary**: Execute a non-interactive shell command and wait for its final output and exit result.

**Tags**: `execute` `shell` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Run a command to completion. Declare actual reads/writes paths and network targets; use empty arrays when none. timeout_ms limits lifetime (default 120000); timeout/cancellation cleans up the owned process tree. max_output_bytes sets the output budget (1024–16384, default 16384), accounting for escaping and reserving lifecycle metadata; previews keep beginning/end and at most 200 lines. Capture is independent of preview: output_resource refers to the same cursor-based content resource used by all Parts. Capture status and retained ranges are independent of process exit and preview limits. Use shell.spawn for background work, shell.watch for readiness/selected notifications, and shell.open for interactive input.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `command` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `max_output_bytes` | `integer / null` | — | — | Model output budget, including JSON escaping. Default/max 16384 bytes;<br>minimum 1024. Capture and process lifetime are independent of this budget. |
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
      "description": "Shell used to run a process.",
      "enum": [
        "bash",
        "powershell"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "description": "A non-interactive command. The selected tool determines whether it waits\nfor completion (`shell.exec`) or returns a background receipt (`shell.spawn`).",
  "properties": {
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
    "max_output_bytes": {
      "description": "Model output budget, including JSON escaping. Default/max 16384 bytes;\nminimum 1024. Capture and process lifetime are independent of this budget.",
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000003"
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
      "x-agena-order": "000001.000007"
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
      "x-agena-order": "000001.000005"
    },
    "shell": {
      "$ref": "#/$defs/ProcessShell",
      "default": "bash"
    },
    "timeout_ms": {
      "format": "uint64",
      "maximum": 86400000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000002"
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
      "x-agena-order": "000001.000006"
    }
  },
  "required": [
    "command"
  ],
  "type": "object"
}
```

### list

`agena.shell.list` · **Summary**: List this session's background shell jobs, watched commands and interactive terminals, including their type and state.

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

`agena.shell.logs` · **Summary**: Read bounded output with an explicit replay cursor; shell.read is the unified, automatically consuming entry point.

**Tags**: `query` `shell` `background` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Explicit replay of shell process output. since_seq defaults to 0; continue with last_seq AND next_event_offset as event_offset whenever nonzero. last_seq refers to fully consumed events; partially returned event text is resumable without loss. max_output_bytes (1024–16384, default 16384) includes escaping and event structure, with room reserved for lifecycle/cursor metadata. limit caps event count; wait_ms is a diagnostic wait (max 30000). has_more describes buffered output. output_resource identifies the canonical retained content; capture completeness is separate from the preview. Use shell.read without since_seq for new unread output. Background jobs notify when they finish; do not poll to wait.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `event_offset` | `integer` | — | `0` |  |
| `limit` | `integer / null` | — | — |  |
| `max_output_bytes` | `integer / null` | — | — |  |
| `process_id` | `string` | ✓ | — |  |
| `since_seq` | `integer` | — | `0` | Return events after this cursor; continue with the returned last_seq. |
| `wait_ms` | `integer` | — | `0` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "event_offset": {
      "default": 0,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "limit": {
      "format": "uint32",
      "maximum": 2000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "max_output_bytes": {
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "since_seq": {
      "default": 0,
      "description": "Return events after this cursor; continue with the returned last_seq.",
      "format": "uint64",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "wait_ms": {
      "default": 0,
      "format": "uint64",
      "maximum": 30000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000005"
    }
  },
  "required": [
    "process_id"
  ],
  "type": "object"
}
```

### open

`agena.shell.open` · **Summary**: Open a persistent interactive shell terminal/PTY for a CLI, REPL or full-screen program; continue with shell.read and shell.write.

**Tags**: `execute` `shell` `terminal` `interactive` `tty` `pty` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Return process_id, output, last_seq, next_event_offset and state after confirmed startup. yield_time_ms (default 1000, max 30000) limits only the initial wait, never lifetime; timeout_ms is an optional lifetime deadline. Interactive exit does not send background completion notifications or wake the AI. max_output_bytes (1024–16384, default 16384) controls preview, not capture. With include_screen the available content budget is split between raw output and a bounded screen. Silence/prompt/yield is not exit. Continue with shell.read or exact-input shell.write; shell.signal interrupts/terminates/kills, shell.resize changes dimensions and shell.stop cleans up. Declare initial effects and subsequent write effects.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `cols` | `integer` | — | `80` |  |
| `command` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `include_screen` | `boolean` | — | `false` | Return the current rendered screen in addition to incremental output.<br>Enable for full-screen interfaces; ordinary prompts need only output. |
| `max_output_bytes` | `integer / null` | — | — | Model output budget, including JSON escaping. Default/max 16384 bytes;<br>minimum 1024. Capture and process lifetime are independent of this budget. |
| `network` | `array<string>` | — | `[]` | Outbound network targets the command may connect to: host names,<br>`host:port`, or URLs. Pass an empty array `[]` when the command has no<br>network effect. |
| `reads` | `array<string>` | — | `[]` | Files and directories the command may read. Declare only the actual<br>files/directories affected - never the executables, interpreters, or<br>tools being invoked (e.g. `node`, `python`, `uv`, `git`, `cargo`) or<br>their installation directories. Pass an empty array `[]` when the<br>command reads nothing beyond its executables. |
| `rows` | `integer` | — | `24` |  |
| `shell` | `ProcessShell` | — | `bash` |  |
| `timeout_ms` | `integer / null` | — | — |  |
| `workdir` | `string / null` | — | — |  |
| `writes` | `array<string>` | — | `[]` | Files and directories the command may create, modify, or delete.<br>Declare only the actual files/directories affected - never the<br>executables, interpreters, or tools being invoked (e.g. `node`,<br>`python`, `uv`, `git`, `cargo`) or their installation directories.<br>Pass an empty array `[]` when the command writes nothing. |
| `yield_time_ms` | `integer` | — | `1000` | Maximum initial output wait, in milliseconds. This is not a timeout. |

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
    }
  },
  "additionalProperties": false,
  "description": "Open a persistent interactive shell/PTY. Waiting for initial output never\nends the terminal's lifetime and never creates a completion-notified job.",
  "properties": {
    "cols": {
      "default": 80,
      "format": "uint16",
      "maximum": 400,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000004"
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
    "include_screen": {
      "default": false,
      "description": "Return the current rendered screen in addition to incremental output.\nEnable for full-screen interfaces; ordinary prompts need only output.",
      "type": "boolean",
      "x-agena-order": "000005"
    },
    "max_output_bytes": {
      "description": "Model output budget, including JSON escaping. Default/max 16384 bytes;\nminimum 1024. Capture and process lifetime are independent of this budget.",
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000003"
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
      "x-agena-order": "000001.000007"
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
      "x-agena-order": "000001.000005"
    },
    "rows": {
      "default": 24,
      "format": "uint16",
      "maximum": 200,
      "minimum": 1,
      "type": "integer",
      "x-agena-order": "000003"
    },
    "shell": {
      "$ref": "#/$defs/ProcessShell",
      "default": "bash"
    },
    "timeout_ms": {
      "format": "uint64",
      "maximum": 86400000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000002"
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
      "x-agena-order": "000001.000006"
    },
    "yield_time_ms": {
      "default": 1000,
      "description": "Maximum initial output wait, in milliseconds. This is not a timeout.",
      "format": "uint64",
      "maximum": 30000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    }
  },
  "required": [
    "command"
  ],
  "type": "object"
}
```

### read

`agena.shell.read` · **Summary**: Read bounded incremental output and state from any owned background shell job or interactive terminal, without input.

**Tags**: `query` `shell` `terminal` `interactive` `tty` `pty` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Works with process_id from shell.spawn, shell.watch or shell.open. Omit since_seq and event_offset to consume unread output; explicit since_seq replays without changing the automatic cursor. A partial event returns last_seq for fully consumed events and next_event_offset for the next event; pass both as since_seq/event_offset to resume. A nonzero event_offset requires since_seq and must preserve UTF-8 boundaries. Reads/writes on one process serialize automatic consumption; cancellation leaves it alive. wait_ms defaults to 250 (max 30000) and is a bounded output wait, not completion. max_output_bytes is 1024–16384 (default 16384), counting escaping/event structure and reserving metadata. include_screen is for PTYs only; enabling it splits the content budget with a bounded screen. ready is independent of terminal status. has_more describes the rolling buffer. output_resource identifies retained content outside the rolling preview. Background completion is notified once, so do not poll merely to wait.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `event_offset` | `integer` | — | `0` | Resume within the next event using the previous next_event_offset.<br>A nonzero value requires since_seq; omit both for automatic consumption. |
| `include_screen` | `boolean` | — | `false` |  |
| `limit` | `integer / null` | — | — |  |
| `max_output_bytes` | `integer / null` | — | — |  |
| `process_id` | `string` | ✓ | — |  |
| `since_seq` | `integer / null` | — | — |  |
| `wait_ms` | `integer` | — | `250` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "description": "Read a background job or terminal without input. Explicit cursors replay output;\nomitting the cursor consumes only previously unread output.",
  "properties": {
    "event_offset": {
      "default": 0,
      "description": "Resume within the next event using the previous next_event_offset.\nA nonzero value requires since_seq; omit both for automatic consumption.",
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000002"
    },
    "include_screen": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000006"
    },
    "limit": {
      "format": "uint32",
      "maximum": 2000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000005"
    },
    "max_output_bytes": {
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "process_id": {
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    },
    "since_seq": {
      "format": "uint64",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "wait_ms": {
      "default": 250,
      "format": "uint64",
      "maximum": 30000,
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000004"
    }
  },
  "required": [
    "process_id"
  ],
  "type": "object"
}
```

### resize

`agena.shell.resize` · **Summary**: Resize an interactive shell.open terminal in character rows and columns.

**Tags**: `mutate` `shell` `terminal` `tty` `pty`

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

### signal

`agena.shell.signal` · **Summary**: Interrupt, gracefully terminate or kill an owned interactive shell terminal.

**Tags**: `mutate` `execute` `shell` `terminal` `interactive` `tty` `pty`

**Runtime**: streaming `buffered`

**Help**:
> interrupt targets the Unix foreground process group without closing the shell; ConPTY uses terminal Ctrl-C. terminate requests graceful cleanup then kills remaining jobs; kill skips the grace period. Out-of-band interruption also works for raw-mode programs. Requires the owning session/workspace. shell.stop is equivalent to terminate.

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

### spawn

`agena.shell.spawn` · **Summary**: Spawn a non-interactive shell command in the background; return immediately so the AI can continue, then notify on completion.

**Tags**: `execute` `shell` `background` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Launch with closed stdin; return process_id only after actual startup and registration. Continue useful work immediately. Completion, failure, timeout or stop is delivered once as system_notification; do not poll merely to wait. Use shell.read for bounded diagnostic output, shell.list for state and shell.stop for cleanup. Omitted timeout_ms permits running until exit/stop; a supplied timeout is enforced. Declare reads/writes/network effects. Attach or replace a watch on this same process with shell.watch(process_id, policy); omit/null policy to remove it. A process-targeted watch never spawns a second command or a second background operation. Use shell.open when later interactive input is required.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `command` | `string` | ✓ | — |  |
| `description` | `string` | — | `` |  |
| `max_output_bytes` | `integer / null` | — | — | Model output budget, including JSON escaping. Default/max 16384 bytes;<br>minimum 1024. Capture and process lifetime are independent of this budget. |
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
      "description": "Shell used to run a process.",
      "enum": [
        "bash",
        "powershell"
      ],
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "additionalProperties": false,
  "description": "A non-interactive command. The selected tool determines whether it waits\nfor completion (`shell.exec`) or returns a background receipt (`shell.spawn`).",
  "properties": {
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
    "max_output_bytes": {
      "description": "Model output budget, including JSON escaping. Default/max 16384 bytes;\nminimum 1024. Capture and process lifetime are independent of this budget.",
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000003"
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
      "x-agena-order": "000001.000007"
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
      "x-agena-order": "000001.000005"
    },
    "shell": {
      "$ref": "#/$defs/ProcessShell",
      "default": "bash"
    },
    "timeout_ms": {
      "format": "uint64",
      "maximum": 86400000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001.000002"
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
      "x-agena-order": "000001.000006"
    }
  },
  "required": [
    "command"
  ],
  "type": "object"
}
```

### stop

`agena.shell.stop` · **Summary**: Stop an owned background shell job, watched command or interactive terminal and clean up its process tree.

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

### watch

`agena.shell.watch` · **Summary**: Start a command with readiness/selected output notifications, or attach, replace or remove the watch on an existing background shell process.

**Tags**: `execute` `shell` `background` `monitor` `watch` `mutate`

**Runtime**: streaming `buffered`

**Help**:
> Provide either command with shell/effects (new launch) or process_id (existing noninteractive process); never both. Launch binds the watch before the process can output. A process target replaces its single policy and keeps the original launch/completion operation. Omit or pass null policy on a process target to remove the watch without stopping the process; an empty policy disables ordinary notifications. Attach observes future output by default; since_seq optionally scans retained raw logs once. Identical policy updates are idempotent. ready_pattern notifies once and leaves the service running. include_pattern is opt-in; omission sends no ordinary output notifications. notifications defaults to once; on_change deduplicates unchanged matches and coalesces changed matches to the latest, using notification_interval_ms (default 30000, min 1000). Readiness/completion bypass that throttle. success_pattern/failure_pattern stop the process tree; failure wins. pattern_kind applies to all patterns (regex default, literal available); preserve whitespace. quiet_period_ms stops successfully after no raw stdout/stderr activity. timeout_ms belongs only to the launch and remains enforced after watch changes/removal. Notification filters do not filter raw content capture. Read with shell.read and stop with shell.stop; do not poll merely to wait. PTYs and WebSocket subscriptions cannot receive this watch.

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
    "ShellMonitorPatternKind": {
      "description": "Applies consistently to all four patterns; regex is the default.",
      "enum": [
        "literal",
        "regex"
      ],
      "type": "string",
      "x-agena-order": "000001.000006"
    },
    "ShellWatchNotifications": {
      "description": "Default once; on_change explicitly enables recurring changed matches.",
      "oneOf": [
        {
          "const": "once",
          "description": "Notify on the first include_pattern match per configuration.",
          "type": "string"
        },
        {
          "const": "on_change",
          "description": "Notify on changed matches, deduplicated and rate limited.",
          "type": "string"
        }
      ],
      "x-agena-order": "000001.000004"
    },
    "ShellWatchPolicy": {
      "additionalProperties": false,
      "description": "Select output notifications and optional completion conditions. The\ncommand's timeout_ms is the only lifetime deadline; omitting it permits\nrunning until exit, stop or a completion condition.",
      "properties": {
        "failure_pattern": {
          "description": "Stop the process tree with failure when an output line matches. Failure\ntakes precedence if both completion patterns match the same line.",
          "type": [
            "string",
            "null"
          ]
        },
        "include_pattern": {
          "description": "Select which lines notify the AI. Omission disables ordinary output\nnotifications. All output remains available through shell.read/archive.",
          "type": [
            "string",
            "null"
          ]
        },
        "notification_interval_ms": {
          "description": "Recurring notifications have at least this interval (default 30000 ms).\nReadiness and completion are never delayed by this throttle.",
          "format": "uint64",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        },
        "notifications": {
          "$ref": "#/$defs/ShellWatchNotifications",
          "default": "once",
          "description": "Default once; on_change explicitly enables recurring changed matches."
        },
        "pattern_kind": {
          "$ref": "#/$defs/ShellMonitorPatternKind",
          "default": "regex",
          "description": "Applies consistently to all four patterns; regex is the default."
        },
        "quiet_period_ms": {
          "description": "Stop successfully after this period with no stdout/stderr activity.\nExcluded output also resets this timer.",
          "format": "uint64",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ]
        },
        "ready_pattern": {
          "description": "Notify once when the service is ready, without stopping it. This is\nindependent of include notifications and final process completion.",
          "type": [
            "string",
            "null"
          ]
        },
        "success_pattern": {
          "description": "Stop the process tree successfully when an output line matches.",
          "type": [
            "string",
            "null"
          ]
        }
      },
      "type": "object",
      "x-agena-order": "000002"
    }
  },
  "anyOf": [
    {
      "additionalProperties": false,
      "description": "Input of a shell command execution.",
      "properties": {
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
        "max_output_bytes": {
          "description": "Model output budget, including JSON escaping. Default/max 16384 bytes;\nminimum 1024. Capture and process lifetime are independent of this budget.",
          "format": "uint32",
          "maximum": 16384,
          "minimum": 1024,
          "type": [
            "integer",
            "null"
          ],
          "x-agena-order": "000001.000003"
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
          "x-agena-order": "000001.000007"
        },
        "policy": {
          "$ref": "#/$defs/ShellWatchPolicy",
          "additionalProperties": false,
          "default": {
            "notifications": "once",
            "pattern_kind": "regex"
          },
          "properties": {
            "failure_pattern": {
              "description": "Stop the process tree with failure when an output line matches. Failure\ntakes precedence if both completion patterns match the same line.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000002.000003"
            },
            "include_pattern": {
              "description": "Select which lines notify the AI. Omission disables ordinary output\nnotifications. All output remains available through shell.read/archive.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000002.000001"
            },
            "notification_interval_ms": {
              "description": "Recurring notifications have at least this interval (default 30000 ms).\nReadiness and completion are never delayed by this throttle.",
              "format": "uint64",
              "maximum": 3600000,
              "minimum": 1000,
              "type": [
                "integer",
                "null"
              ],
              "x-agena-order": "000002.000005"
            },
            "notifications": {
              "$ref": "#/$defs/ShellWatchNotifications",
              "default": "once",
              "description": "Default once; on_change explicitly enables recurring changed matches."
            },
            "pattern_kind": {
              "$ref": "#/$defs/ShellMonitorPatternKind",
              "default": "regex",
              "description": "Applies consistently to all four patterns; regex is the default."
            },
            "quiet_period_ms": {
              "description": "Stop successfully after this period with no stdout/stderr activity.\nExcluded output also resets this timer.",
              "format": "uint64",
              "maximum": 3600000,
              "minimum": 1,
              "type": [
                "integer",
                "null"
              ],
              "x-agena-order": "000002.000007"
            },
            "ready_pattern": {
              "description": "Notify once when the service is ready, without stopping it. This is\nindependent of include notifications and final process completion.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000002.000000"
            },
            "success_pattern": {
              "description": "Stop the process tree successfully when an output line matches.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000002.000002"
            }
          },
          "type": "object"
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
          "x-agena-order": "000001.000005"
        },
        "shell": {
          "$ref": "#/$defs/ProcessShell",
          "default": "bash"
        },
        "timeout_ms": {
          "format": "uint64",
          "maximum": 86400000,
          "minimum": 1,
          "type": [
            "integer",
            "null"
          ],
          "x-agena-order": "000001.000002"
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
          "x-agena-order": "000001.000006"
        }
      },
      "required": [
        "command"
      ],
      "type": "object"
    },
    {
      "additionalProperties": false,
      "properties": {
        "policy": {
          "additionalProperties": false,
          "anyOf": [
            {
              "$ref": "#/$defs/ShellWatchPolicy"
            },
            {
              "type": "null"
            }
          ],
          "default": null,
          "description": "Replace the entire watch policy. Omit or pass null to remove it;\nremoving a watch leaves the process and completion notification intact.",
          "properties": {
            "failure_pattern": {
              "description": "Stop the process tree with failure when an output line matches. Failure\ntakes precedence if both completion patterns match the same line.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000001.000003"
            },
            "include_pattern": {
              "description": "Select which lines notify the AI. Omission disables ordinary output\nnotifications. All output remains available through shell.read/archive.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000001.000001"
            },
            "notification_interval_ms": {
              "description": "Recurring notifications have at least this interval (default 30000 ms).\nReadiness and completion are never delayed by this throttle.",
              "format": "uint64",
              "maximum": 3600000,
              "minimum": 1000,
              "type": [
                "integer",
                "null"
              ],
              "x-agena-order": "000001.000005"
            },
            "notifications": {
              "$ref": "#/$defs/ShellWatchNotifications",
              "default": "once",
              "description": "Default once; on_change explicitly enables recurring changed matches."
            },
            "pattern_kind": {
              "$ref": "#/$defs/ShellMonitorPatternKind",
              "default": "regex",
              "description": "Applies consistently to all four patterns; regex is the default."
            },
            "quiet_period_ms": {
              "description": "Stop successfully after this period with no stdout/stderr activity.\nExcluded output also resets this timer.",
              "format": "uint64",
              "maximum": 3600000,
              "minimum": 1,
              "type": [
                "integer",
                "null"
              ],
              "x-agena-order": "000001.000007"
            },
            "ready_pattern": {
              "description": "Notify once when the service is ready, without stopping it. This is\nindependent of include notifications and final process completion.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000001.000000"
            },
            "success_pattern": {
              "description": "Stop the process tree successfully when an output line matches.",
              "maxLength": 16384,
              "minLength": 1,
              "type": [
                "string",
                "null"
              ],
              "x-agena-order": "000001.000002"
            }
          },
          "type": "object",
          "x-agena-order": "000001"
        },
        "process_id": {
          "minLength": 1,
          "type": "string",
          "x-agena-order": "000000"
        },
        "since_seq": {
          "description": "Optionally inspect retained output after this sequence on attachment.\nOmit for future output only; unavailable older output is not replayed.",
          "format": "uint64",
          "minimum": 0,
          "type": [
            "integer",
            "null"
          ],
          "x-agena-order": "000002"
        }
      },
      "required": [
        "process_id"
      ],
      "type": "object"
    }
  ],
  "description": "Either start a command with an atomic watch, or replace/remove the watch on\nan existing noninteractive process. A process target never starts a command.",
  "properties": {},
  "type": "object"
}
```

### write

`agena.shell.write` · **Summary**: Send exact nonempty input to an interactive shell.open terminal and collect a bounded, resumable response.

**Tags**: `mutate` `execute` `shell` `terminal` `interactive` `tty` `pty`

**Runtime**: streaming `buffered`

**Help**:
> chars is exact terminal input: never trim or append a newline. Send \r for Enter, \u0003 for Ctrl-C, \u0004 for Ctrl-D, \t for Tab, or terminal escape sequences. Use shell.read without input. Omit since_seq/event_offset for unread output; explicit cursors replay without changing automatic consumption. Continue a partial result with last_seq and next_event_offset as since_seq/event_offset. wait_ms defaults to 250 (max 30000), independent of lifetime. max_output_bytes is 1024–16384 (default 16384); include_screen splits that content budget. Cursor/effect validation occurs before sending input. Declare reads/writes/network effects of the entered operation. Requires the owning session/workspace. A partial-write error requests termination; never blindly resend the full input. Use shell.signal for out-of-band interruption and shell.stop for cleanup.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `chars` | `string` | ✓ | — | Nonempty exact UTF-8 text/control characters. Use shell.read to read. Use<br>\r for Enter, \u0003 for Ctrl-C, \u0004 for Ctrl-D; at most 65536 bytes. |
| `event_offset` | `integer` | — | `0` |  |
| `include_screen` | `boolean` | — | `false` |  |
| `max_output_bytes` | `integer / null` | — | — |  |
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
      "description": "Nonempty exact UTF-8 text/control characters. Use shell.read to read. Use\n\\r for Enter, \\u0003 for Ctrl-C, \\u0004 for Ctrl-D; at most 65536 bytes.",
      "maxLength": 65536,
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000001"
    },
    "event_offset": {
      "default": 0,
      "format": "uint32",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000003"
    },
    "include_screen": {
      "default": false,
      "type": "boolean",
      "x-agena-order": "000006"
    },
    "max_output_bytes": {
      "format": "uint32",
      "maximum": 16384,
      "minimum": 1024,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "network": {
      "default": [],
      "description": "Outbound targets the entered operation may contact.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000009"
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
      "x-agena-order": "000007"
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
      "x-agena-order": "000005"
    },
    "writes": {
      "default": [],
      "description": "Paths the entered operation may modify, relative to the Agena workspace.",
      "items": {
        "type": "string"
      },
      "type": "array",
      "x-agena-order": "000008"
    }
  },
  "required": [
    "process_id",
    "chars"
  ],
  "type": "object"
}
```

## agena.skills

**Version** `0.1.0` · **Tools** 12

Discover, install, author, and manage complete Agent Skills packages.

### create

`agena.skills.create` · **Summary**: Create a skill in the workspace or global library.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Provide a complete SKILL.md with YAML name and description plus Markdown instructions. Validates the Agent Skills specification and refuses to replace an existing package. Add scripts, references, or assets with write_resource or install a complete package.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — |  |
| `scope` | `SkillScope` | — | `workspace` |  |

**Input schema**:
```json
{
  "$defs": {
    "SkillScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "document": {
      "type": "string"
    },
    "scope": {
      "$ref": "#/$defs/SkillScope",
      "default": "workspace"
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### get

`agena.skills.get` · **Summary**: Read a skill's full instructions and resource inventory.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use an ID returned by list. Relative references resolve against the skill directory shown in path. Supporting resources are loaded separately with read_resource. Read-only plugin and compatibility packages can be installed into a managed scope for editing. allowed-tools is metadata and does not change existing tool permissions.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

### install

`agena.skills.install` · **Summary**: Install complete skill packages into the workspace or global library.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Preview first, then send selected candidate paths and expected_source_revision. Explicit replace=true is required for existing names. An empty selection installs all valid candidates. All resources and executable bits are preserved; installation never executes code. Global packages are available across projects.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_source_revision` | `string / null` | — | `null` |  |
| `replace` | `boolean` | — | `false` |  |
| `scope` | `SkillScope` | — | `workspace` |  |
| `selected` | `array<string>` | — | `[]` |  |
| `source` | `SkillInstallSource` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "SkillInstallSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "kind": {
              "const": "github",
              "type": "string"
            },
            "reference": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "subpath": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
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
        },
        {
          "additionalProperties": false,
          "properties": {
            "kind": {
              "const": "folder",
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
            "data_base64": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "filename": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "kind": {
              "const": "archive",
              "type": "string"
            },
            "path": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "url": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            }
          },
          "required": [
            "kind"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "files": {
              "items": {
                "$ref": "#/$defs/SkillUploadFile"
              },
              "type": "array"
            },
            "kind": {
              "const": "files",
              "type": "string"
            }
          },
          "required": [
            "kind",
            "files"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    },
    "SkillScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    },
    "SkillUploadFile": {
      "additionalProperties": false,
      "properties": {
        "content_base64": {
          "type": "string"
        },
        "executable": {
          "default": false,
          "type": "boolean"
        },
        "path": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "content_base64"
      ],
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "expected_source_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ]
    },
    "replace": {
      "default": false,
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/SkillScope",
      "default": "workspace"
    },
    "selected": {
      "default": [],
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "source": {
      "$ref": "#/$defs/SkillInstallSource"
    }
  },
  "required": [
    "source"
  ],
  "type": "object"
}
```

### list

`agena.skills.list` · **Summary**: Find available skills or inspect the skill library.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Search by name or description. By default return enabled, valid, effective entries. Set include_inactive to inspect disabled, invalid, and shadowed packages. IDs identify exact package copies; scope is global or workspace. Use get to load instructions only when relevant.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `include_inactive` | `boolean` | — | `false` | Include disabled, invalid, and shadowed entries for management. |
| `limit` | `integer / null` | — | `null` |  |
| `offset` | `integer` | — | `0` |  |
| `query` | `string / null` | — | `null` |  |
| `scope` | `SkillScope / null` | — | `null` |  |

**Input schema**:
```json
{
  "$defs": {
    "SkillScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    }
  },
  "additionalProperties": false,
  "properties": {
    "include_inactive": {
      "default": false,
      "description": "Include disabled, invalid, and shadowed entries for management.",
      "type": "boolean"
    },
    "limit": {
      "default": null,
      "format": "uint",
      "minimum": 0,
      "type": [
        "integer",
        "null"
      ]
    },
    "offset": {
      "default": 0,
      "format": "uint",
      "minimum": 0,
      "type": "integer"
    },
    "query": {
      "default": null,
      "type": [
        "string",
        "null"
      ]
    },
    "scope": {
      "anyOf": [
        {
          "$ref": "#/$defs/SkillScope"
        },
        {
          "type": "null"
        }
      ],
      "default": null
    }
  },
  "type": "object"
}
```

### preview_install

`agena.skills.preview_install` · **Summary**: Preview skills in a GitHub repository, archive, or folder.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Sources: github URL (optional reference/subpath), folder path, archive path/URL/uploaded base64, or uploaded files. Supports ZIP, tar, and tar.gz. Preview lists candidates and name conflicts and returns a source revision. It does not write the library or run bundled code.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_source_revision` | `string / null` | — | `null` |  |
| `replace` | `boolean` | — | `false` |  |
| `scope` | `SkillScope` | — | `workspace` |  |
| `selected` | `array<string>` | — | `[]` |  |
| `source` | `SkillInstallSource` | ✓ | — |  |

**Input schema**:
```json
{
  "$defs": {
    "SkillInstallSource": {
      "oneOf": [
        {
          "additionalProperties": false,
          "properties": {
            "kind": {
              "const": "github",
              "type": "string"
            },
            "reference": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "subpath": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
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
        },
        {
          "additionalProperties": false,
          "properties": {
            "kind": {
              "const": "folder",
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
            "data_base64": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "filename": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "kind": {
              "const": "archive",
              "type": "string"
            },
            "path": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "url": {
              "default": null,
              "type": [
                "string",
                "null"
              ]
            }
          },
          "required": [
            "kind"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "files": {
              "items": {
                "$ref": "#/$defs/SkillUploadFile"
              },
              "type": "array"
            },
            "kind": {
              "const": "files",
              "type": "string"
            }
          },
          "required": [
            "kind",
            "files"
          ],
          "type": "object"
        }
      ],
      "properties": {},
      "type": "object"
    },
    "SkillScope": {
      "enum": [
        "global",
        "workspace"
      ],
      "type": "string"
    },
    "SkillUploadFile": {
      "additionalProperties": false,
      "properties": {
        "content_base64": {
          "type": "string"
        },
        "executable": {
          "default": false,
          "type": "boolean"
        },
        "path": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "content_base64"
      ],
      "type": "object"
    }
  },
  "additionalProperties": false,
  "properties": {
    "expected_source_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ]
    },
    "replace": {
      "default": false,
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/SkillScope",
      "default": "workspace"
    },
    "selected": {
      "default": [],
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "source": {
      "$ref": "#/$defs/SkillInstallSource"
    }
  },
  "required": [
    "source"
  ],
  "type": "object"
}
```

### read_resource

`agena.skills.read_resource` · **Summary**: Read a skill's UTF-8 reference, script, or other text resource.

**Tags**: `query` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> The path is relative to the skill directory. Use get for SKILL.md. Binary assets are retained by package installation; this tool returns text resources only.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "path": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "path"
  ],
  "type": "object"
}
```

### remove

`agena.skills.remove` · **Summary**: Remove a managed skill and all of its bundled files.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Use the exact ID and current revision from get. Removes only that workspace or global package; other copies and read-only sources remain. Disable with set_enabled when you want to keep the package installed.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "expected_revision"
  ],
  "type": "object"
}
```

### remove_resource

`agena.skills.remove_resource` · **Summary**: Delete one resource from a managed skill.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Supply the current package revision from get, or a text resource revision from read_resource. Package revisions also support binary and large resources without reading their contents. SKILL.md and installation metadata cannot be removed through this operation; use remove for the complete skill.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string"
    },
    "path": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "path",
    "expected_revision"
  ],
  "type": "object"
}
```

### set_enabled

`agena.skills.set_enabled` · **Summary**: Enable or disable an installed skill.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Use its current revision. Disabled packages remain available for management but are excluded from automatic skill discovery. Project packages override global copies; the library reports shadowed copies explicitly.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `enabled` | `boolean` | ✓ | — |  |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "enabled": {
      "type": "boolean"
    },
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "enabled",
    "expected_revision"
  ],
  "type": "object"
}
```

### update

`agena.skills.update` · **Summary**: Edit an installed SKILL.md without discarding bundled resources.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Read get first and supply its current revision. Preserve the directory-matching name and any metadata you want to keep. A stale revision fails without changing the package. Resource files remain in place.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — |  |
| `expected_revision` | `string` | ✓ | — |  |
| `id` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "type": "string"
    },
    "expected_revision": {
      "type": "string"
    },
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "document",
    "expected_revision"
  ],
  "type": "object"
}
```

### validate

`agena.skills.validate` · **Summary**: Validate a SKILL.md document before saving or installing.

**Tags**: `query` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Checks complete YAML frontmatter, required and optional standard fields, and name rules. Supply name to check the containing directory name. Returns actionable errors and warnings; no files are changed.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `document` | `string` | ✓ | — |  |
| `name` | `string / null` | — | `null` |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "type": "string"
    },
    "name": {
      "default": null,
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "document"
  ],
  "type": "object"
}
```

### write_resource

`agena.skills.write_resource` · **Summary**: Create or update a skill's UTF-8 resource.

**Tags**: `mutate` `filesystem`

**Runtime**: streaming `buffered`

**Help**:
> Use a relative path such as references/guide.md or scripts/check.py. Omit expected_revision only for a new file; use read_resource's revision to replace an existing file. Edit SKILL.md with update. This operation never executes scripts.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `content` | `string` | ✓ | — |  |
| `expected_revision` | `string / null` | — | `null` |  |
| `id` | `string` | ✓ | — |  |
| `path` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "content": {
      "type": "string"
    },
    "expected_revision": {
      "default": null,
      "type": [
        "string",
        "null"
      ]
    },
    "id": {
      "type": "string"
    },
    "path": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "path",
    "content"
  ],
  "type": "object"
}
```

## agena.web

**Version** `0.1.0` · **Tools** 16

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

**Runtime**: streaming `streaming`

**Help**:
> Traverse links breadth first with bounded concurrency (default 4, maximum 8; also capped by config). max_pages counts attempts and cache hits, including failures. same_host_only defaults true. Per-host pacing, robots, depth and URL-discovery budgets still apply. Omit render_js for HTTP first with conditional rendering when enabled; true forces browser and false forces HTTP. Report includes per-URL page_errors and effective concurrency. Only complete, readable 2xx documents enter storage. Use web.query to locate evidence in the resulting local index and web.read to read it.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `concurrency` | `integer / null` | — | — | Concurrent page reads; default 4, hard maximum 8, capped by crawl.limits.concurrency. |
| `max_depth` | `integer / null` | — | — |  |
| `max_pages` | `integer / null` | — | — |  |
| `render_js` | `boolean / null` | — | — | Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser. |
| `same_host_only` | `boolean / null` | — | — |  |
| `start_url` | `string` | ✓ | — |  |
| `use_cache` | `boolean` | — | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "concurrency": {
      "description": "Concurrent page reads; default 4, hard maximum 8, capped by crawl.limits.concurrency.",
      "format": "uint32",
      "maximum": 8,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000006"
    },
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
      "description": "Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser.",
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
> Read actual page evidence after search. Markdown preserves main content, sibling articles, code, tables and resolved links. Returns content_status, extraction_strategy, final_url, warnings, page_id and next_offset. max_chars defaults to 8000 (maximum 24000); continue long pages with web.read without refetching. prompt adds relevant excerpts from the full snapshot. Use fetch_many for independent known URLs. Omit render_js for HTTP first and one JavaScript-shell browser retry when browser.enabled; false forces HTTP, true forces an isolated browser. One shared fetch deadline includes admission, HTTP, rendering and extraction. Blocked/empty/partial pages are not reusable cache entries. Read statuses before using the text as evidence.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `extractor` | `ExtractionBackend / null` | — | — |  |
| `max_chars` | `integer / null` | — | — | Initial Unicode-character preview. Default 8000, maximum 24000; use web.read for continuation. |
| `prompt` | `string / null` | — | — |  |
| `render_js` | `boolean / null` | — | — | Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser. |
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
    "max_chars": {
      "description": "Initial Unicode-character preview. Default 8000, maximum 24000; use web.read for continuation.",
      "format": "uint32",
      "maximum": 24000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000005"
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
      "description": "Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser.",
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

### fetch_many

`agena.web.fetch_many` · **Summary**: Read several independent web pages concurrently.

**Tags**: `network` `read_only`

**Runtime**: streaming `streaming`

**Help**:
> Fetch 1–8 known URLs concurrently (default concurrency 4, maximum 8), deduplicating normalized URLs in input order. Use after search to gather independent sources in one call. Each result includes its own status/error, content_status, bounded Markdown, page_id and next_offset for web.read. Failures do not discard other pages; partial also flags unreadable or truncated sources. Per-host pacing and global HTTP/browser limits still apply. Omit render_js for HTTP first and one conditional JavaScript-shell browser retry if enabled; true forces browser, false forces HTTP. No CAPTCHA bypass. max_chars is per page (default 4000, maximum 8000).

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `concurrency` | `integer / null` | — | `null` | In-flight page reads, default 4, maximum 8. Per-host pacing still applies. |
| `extractor` | `ExtractionBackend / null` | — | `null` |  |
| `max_chars` | `integer / null` | — | `null` | Per-page Unicode-character preview, default 4000, maximum 8000. Continue each page with web.read. |
| `render_js` | `boolean / null` | — | `null` | Omit for HTTP first with conditional rendering when browser.enabled; false forces HTTP; true forces browser. |
| `urls` | `array<string>` | ✓ | — | One to eight known HTTP(S) URLs; normalized duplicates are fetched once, in first-occurrence order. |
| `use_cache` | `boolean` | — | `true` |  |

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
    "concurrency": {
      "default": null,
      "description": "In-flight page reads, default 4, maximum 8. Per-host pacing still applies.",
      "format": "uint32",
      "maximum": 8,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "extractor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ExtractionBackend"
        },
        {
          "type": "null"
        }
      ],
      "default": null,
      "x-agena-order": "000005"
    },
    "max_chars": {
      "default": null,
      "description": "Per-page Unicode-character preview, default 4000, maximum 8000. Continue each page with web.read.",
      "format": "uint32",
      "maximum": 8000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "render_js": {
      "default": null,
      "description": "Omit for HTTP first with conditional rendering when browser.enabled; false forces HTTP; true forces browser.",
      "type": [
        "boolean",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "urls": {
      "description": "One to eight known HTTP(S) URLs; normalized duplicates are fetched once, in first-occurrence order.",
      "items": {
        "type": "string"
      },
      "maxItems": 8,
      "minItems": 1,
      "type": "array",
      "x-agena-order": "000000"
    },
    "use_cache": {
      "default": true,
      "type": "boolean",
      "x-agena-order": "000003"
    }
  },
  "required": [
    "urls"
  ],
  "type": "object"
}
```

### query

`agena.web.query` · **Summary**: Find evidence in locally crawled pages and obtain readable snapshots.

**Tags**: `network` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Search the index populated by web.crawl, without new network requests. max_results defaults to 5, maximum 20. Hits include source URL, stored fetch time, chunk preview, page_id, and read_offset for web.read. Stored content may be stale; refetch its URL when freshness matters. Permissions are revalidated before exposing each hit; denied hits are omitted and counted. This searches stored documents only; use web.search to discover public pages.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_results` | `integer / null` | — | `null` |  |
| `query` | `string` | ✓ | — | Search the local crawl index; does not search the public web or refresh stored pages. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "max_results": {
      "default": null,
      "format": "uint32",
      "maximum": 20,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "query": {
      "description": "Search the local crawl index; does not search the public web or refresh stored pages.",
      "maxLength": 4096,
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

### read

`agena.web.read` · **Summary**: Continue reading an immutable fetched-page snapshot.

**Tags**: `network` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Use page_id and next_offset returned by fetch, fetch_many or query. Returns a contiguous slice of the same extracted Markdown without refetching; offset counts Unicode characters, not bytes. max_chars defaults to 8000, maximum 24000. next_offset=null means the end of available extracted text, not necessarily a complete source: inspect truncated and content_status. Snapshots expire after 15 minutes and share a 32 MiB memory budget; if evicted, fetch the URL or query the crawl index again. Revalidates requested and final URL permissions.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `max_chars` | `integer / null` | — | `null` | Characters to return, default 8000, maximum 24000. |
| `offset` | `integer` | — | `0` | Unicode character offset from next_offset; default 0. No network refetch. |
| `page_id` | `string` | ✓ | — | Immutable snapshot handle returned by fetch, fetch_many, or query. Expires after 15 minutes or memory eviction. |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "max_chars": {
      "default": null,
      "description": "Characters to return, default 8000, maximum 24000.",
      "format": "uint32",
      "maximum": 24000,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
    },
    "offset": {
      "default": 0,
      "description": "Unicode character offset from next_offset; default 0. No network refetch.",
      "format": "uint",
      "minimum": 0,
      "type": "integer",
      "x-agena-order": "000001"
    },
    "page_id": {
      "description": "Immutable snapshot handle returned by fetch, fetch_many, or query. Expires after 15 minutes or memory eviction.",
      "minLength": 1,
      "type": "string",
      "x-agena-order": "000000"
    }
  },
  "required": [
    "page_id"
  ],
  "type": "object"
}
```

### search

`agena.web.search` · **Summary**: Find candidate public-web pages to fetch.

**Tags**: `network` `discovery` `read_only`

**Runtime**: streaming `buffered`

**Help**:
> Discover candidate pages; fetch 1-3 relevant URLs for factual answers. Omit engine or use auto for the configured provider (HTML, Brave, Tavily, Exa or SearXNG). HTML auto searches eight free public websites (DuckDuckGo, Bing, Baidu, Yandex, Google, Yahoo, Brave and Naver) concurrently, deduplicates URLs and combines rankings. Set engine to one name or a comma-separated list such as baidu,google to search only those HTML websites, overriding API settings. Case and surrounding spaces are ignored; duplicates run once in first-occurrence order. auto must stand alone; empty and unknown names are errors. max_results caps the final total; max_results_per_engine and max_pages_per_engine independently cap each selected HTML source (defaults: 10 candidates, 1 page including the first). Increase both source budgets for deeper retrieval; filtering or a larger final limit never triggers extra pages. No global offset or continuation cursor. Effective limits appear in the HTML response. Each result's engines lists contributing engines; source is the publisher. Multi-engine HTML searches preserve successful sources with partial and engine_errors; all selected engines failing is an error. A single engine's failure is returned directly. Inspect engine_reports for per-source result counts, page counts, cache hits and structured issues. Searches to the same engine are serialized and paced; successful identical queries are reused for two minutes. Rate limits honor Retry-After and verification failures enter cooldown: respect retry_after_secs, use other working sources and avoid bursts of overlapping queries. Verification and unknown pages are failures, not empty results. A confirmed JavaScript shell gets one browser attempt per page when browser.enabled. Later-page failures preserve earlier results with partial diagnostics. Browser rendering does not solve human verification or consent. API providers return at most 20 results without switching providers and reject HTML-only budget arguments. Domain filters accept bare hostnames; exclusions win. Snippets are previews, not fetched-page evidence.

**Input parameters**:
| Parameter | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `allowed_domains` | `array<string>` | — | — |  |
| `blocked_domains` | `array<string>` | — | — |  |
| `engine` | `string / null` | — | — | Omit or use auto for the configured provider. HTML auto searches all eight engines concurrently.<br>Use one name or a comma-separated list, e.g. "baidu,google", to query only those free HTML<br>websites concurrently, overriding the configured API provider. Names: duckduckgo, bing, baidu,<br>yandex, google, yahoo, brave, naver. Case and surrounding spaces are ignored; duplicates run<br>once, in first-occurrence order. auto must stand alone; empty or unknown names are errors. |
| `max_pages_per_engine` | `integer / null` | — | — | HTML only: maximum pages per engine, including the first page; not a starting page or offset.<br>Default 1 (configurable), capped by search.max_pages_per_engine. Stops early when the candidate<br>budget is reached or a page has no new URLs. Increase both HTML budgets for deeper retrieval. |
| `max_results` | `integer / null` | — | — | Maximum total results after domain filtering, URL deduplication and ranking.<br>Default 5, capped by search.max_limit (default 20). Does not increase HTML retrieval budgets. |
| `max_results_per_engine` | `integer / null` | — | — | HTML only: distinct candidates to collect per engine before domain filtering.<br>Default 10 (configurable); independent of max_results and capped by search.max_results_per_engine. |
| `query` | `string` | ✓ | — |  |

**Input schema**:
```json
{
  "additionalProperties": false,
  "properties": {
    "allowed_domains": {
      "items": {
        "type": "string"
      },
      "maxItems": 64,
      "type": "array",
      "x-agena-order": "000005"
    },
    "blocked_domains": {
      "items": {
        "type": "string"
      },
      "maxItems": 64,
      "type": "array",
      "x-agena-order": "000006"
    },
    "engine": {
      "description": "Omit or use auto for the configured provider. HTML auto searches all eight engines concurrently.\nUse one name or a comma-separated list, e.g. \"baidu,google\", to query only those free HTML\nwebsites concurrently, overriding the configured API provider. Names: duckduckgo, bing, baidu,\nyandex, google, yahoo, brave, naver. Case and surrounding spaces are ignored; duplicates run\nonce, in first-occurrence order. auto must stand alone; empty or unknown names are errors.",
      "maxLength": 256,
      "minLength": 1,
      "type": [
        "string",
        "null"
      ],
      "x-agena-order": "000004"
    },
    "max_pages_per_engine": {
      "description": "HTML only: maximum pages per engine, including the first page; not a starting page or offset.\nDefault 1 (configurable), capped by search.max_pages_per_engine. Stops early when the candidate\nbudget is reached or a page has no new URLs. Increase both HTML budgets for deeper retrieval.",
      "format": "uint32",
      "maximum": 5,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000003"
    },
    "max_results": {
      "description": "Maximum total results after domain filtering, URL deduplication and ranking.\nDefault 5, capped by search.max_limit (default 20). Does not increase HTML retrieval budgets.",
      "format": "uint32",
      "maximum": 50,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000001"
    },
    "max_results_per_engine": {
      "description": "HTML only: distinct candidates to collect per engine before domain filtering.\nDefault 10 (configurable); independent of max_results and capped by search.max_results_per_engine.",
      "format": "uint32",
      "maximum": 50,
      "minimum": 1,
      "type": [
        "integer",
        "null"
      ],
      "x-agena-order": "000002"
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

