# Provider API tools

The bundled `chatgpt`, `claude`, and `gemini` plugins use the **active session provider**. There are no plugin API-key, base-URL, timeout, or default-model settings. Credentials are resolved for each invocation through a native bundled-plugin host callback; they are not exposed over external plugin RPC or returned in tool output.

These plugins currently expose 74 tools: 28 OpenAI, 16 Claude, and 30 Gemini. `session.models` is an additional discovery tool. The generated [tools reference](../crates/agena-bundled-plugins/generated/tools-reference.md) contains every action and input schema.

## Connection and model selection

1. Use `session.models` to discover the current provider's model catalog, current model, metadata, and capabilities. Discovery checks network permission first, and falls back to configured models when remote discovery fails. An unknown or inferred capability is not a guarantee that a gateway implements an API.
2. A model-based request defaults to the **actual current session model**. Pass `model` to select a model from the invoked vendor that supports the operation. Selecting `gemini.speech`, for example, does not implicitly switch a Claude session to a Google model.
3. The vendor tool chooses the protocol and configured protocol path, while retaining the current provider connection. A compatible configured adapter supplies authentication/header options. Claude honors a configured `messages_url`; Gemini Live honors a configured `realtime_ws_url`.
4. Model discovery and resource management operations such as list/get/delete/download do not need an inference model. Claude batch entries inherit the top-level/current model when their `params.model` is absent. OpenAI batch JSONL files retain their explicit per-request models. Existing managed agents retain their configuration; inline agent creation uses the current/explicit model if no nested model was provided.

No request silently falls back to an official endpoint or to local execution. The selected gateway and account must implement and authorize the actual vendor API.

## Capability coverage

| Capability | OpenAI (`chatgpt`) | Anthropic (`claude`) | Google (`gemini`) |
| --- | --- | --- | --- |
| Provider models | `models`, `session.models` | `models`, `session.models` | `models`, `session.models` |
| Image/document understanding | `image_understanding`, `document_understanding` | Same | Same |
| Audio/video understanding | `audio_understanding` (local WAV/MP3) | No separate native audio/video endpoint | `audio_understanding`, `video_understanding` |
| Remote files and outputs | Signed `file_upload/status/delete`; `files` list/upload/get/wait/delete/download | Same; downloads require vendor-generated files | Signed uploads; `files` list/get/wait/delete/download |
| Knowledge stores | `knowledge`: vector-store CRUD, file indexing, file batches, direct search | No separate native vector-store API | `knowledge`: File Search Store CRUD, import, direct resumable upload, document management; `operations` |
| Hosted code and search | Existing web/file search, code interpreter, shell | Existing web search/fetch, code execution, advisor | Existing code execution, URL context, Google Search/Maps, file search |
| Native tool combinations and callbacks | `inference`: Responses, bounded SSE | `inference`: Messages, Skills/programmatic calling, bounded SSE | `inference`: Interactions, agents/Deep Research, bounded SSE |
| Async lifecycle | `responses` create/get/wait/cancel/delete/input items/compact | `batches` lifecycle/results | `interactions`, `operations`, `batches` |
| Batch inference | `batches`; result file through `files.download` | `batches`, including JSONL results | `batches`, generation/embedding create and update; result file through `files.download` |
| Images | Existing Responses `image_generation`, `image_edit`; direct Images `image_generate` | No standalone image generation API | Existing `image_generation`, `image_edit` |
| Audio | `transcription` (also translations), `speech`, `realtime` | No standalone audio generation API | `speech`, `realtime`, `music_generation` |
| Video | No retired Sora API added | No standalone video generation API | `video_generation`, `video_edit` through current Interactions models |
| Embeddings/moderation | `embeddings`, `moderation` | No corresponding standalone endpoints | `embeddings` single/synchronous batch; asynchronous batches through `batches` |
| Token counting | `count_tokens` | `count_tokens` | `count_tokens` |
| Skills/context cache | Native tool combinations and provider caching options | `skills` CRUD/version uploads; execute via `inference` | `caches` CRUD for models supporting explicit caching |
| Managed agents | `agents`: reusable agents, sessions/events/items, artifacts/downloads, turns, subagents, traces, environments/files | `agents`: agents/versions, sessions/events/resources/threads, environments, deployments/runs | `agents`: create/list/get/delete; run through `inference` |
| Other platform workflows | `platform`: conversations/items, fine-tuning jobs/checkpoints/permissions/graders, evals/runs/outputs, multipart uploads, voice consents and voice creation/deletion | Official Messages fields through `inference` | Official Interactions fields through `inference` |

“Native tool combinations” means the vendor request accepts its documented tool definitions and request fields. Provider-returned local computer/browser/shell/editor/function calls are returned as **data**. They do not execute automatically on the host. The calling AI can explicitly use Agena tools under normal permissions, then submit the vendor-formatted result in another inference request. Cloud MCP and hosted execution run under the vendor's own configuration.

## Common resource input

Resource tools accept `action`, `id`, `item_id`, `sub_item_id`, `resource`, `parameters`, `query`, and, when appropriate, `model`, `prompt`, `path`/`paths`, and `output_path`. Nested managed-agent collections are selected by documented names such as `sessions/artifacts` or `sessions/resources`; these are allowlisted collections, not caller-supplied endpoint paths.

- `parameters` contains the official request JSON, not connection settings. `parameters.model` and `parameters.stream` are rejected; use the top-level fields. Nested model fields in official batch/evaluation/agent schemas are retained where applicable. Multipart arrays become repeated bracketed fields and nested objects become bracketed keys, so transcription timestamps and diarization options can use their natural JSON shape.
- `query` carries pagination and filters. Credential query parameters and URL/path injection are rejected.
- `path`/`paths` are explicit authorized local inputs. Uploads accept binary datasets and archives as well as media, up to 512 MiB total. Voice samples and consent recordings accept one file of up to 10 MiB; multipart audio MIME types are inferred from the filename. Optional SHA-256 verification occurs before sending bytes.
- `output_path` creates a new file and never overwrites an existing file. Without it, downloads and generated media become managed attachments. Multiple outputs are saved separately.
- `beta_headers` contains official beta names only. It cannot replace authentication headers. Some Claude managed-agent/Skills APIs require account access and beta headers.
- `wait_seconds` is 1–50 seconds (default 20). `wait` only polls an existing ID, retains pending state on a deadline or later polling error, and never recreates a task. Event streams are also bounded and retain complete received events. Top-level `stream=true` opts inference/response creation into bounded SSE. `events` contains the provider data; `sse_events` retains `id`, `event`, `retry` and `event_index` into that array. `last_event_id` is the last retained cursor and can be supplied on a subsequent event request when the vendor supports resumption. A streamed create must never be automatically repeated to resume it.
- Complete response JSON is persisted as a receipt. Model-visible output has a bounded preview; image/audio/video base64 becomes files and is redacted from receipts. Remote URLs remain references, without an authenticated fetch to an arbitrary origin.
- Native inference usage is attributed through Agena's provider-tool usage metadata. Pending creates defer attribution until a completed observation reports usage. Atomic workspace markers prevent repeated or concurrent queries of the same completed task from emitting that usage again. OpenAI batch aggregate usage, Claude JSONL batch results (grouped by model), and Gemini inline batch results are also supported. File-backed Gemini batch outputs remain explicit downloads; their JSONL usage is not parsed automatically. Markers are written at the tool artifact boundary, not as a transaction with the runtime usage ledger; a process crash between those operations can still require reconciliation. Missing media/runtime/storage prices and batch discounts are marked incomplete. Polling without provider-reported usage does not invent token counts.

## Examples

These are execution-tool names for `tools_help` / `tools_call`, not additional top-level model functions.

Discover a suitable model:

```json
{"tool":"session.models","input":{"query":"embedding"}}
```

Create an OpenAI vector store, then index an already uploaded file:

```json
{"tool":"chatgpt.knowledge","input":{"action":"create","parameters":{"name":"Project docs"}}}
{"tool":"chatgpt.knowledge","input":{"action":"add_file","id":"vs_example","parameters":{"file_id":"file_example"}}}
{"tool":"chatgpt.knowledge","input":{"action":"get_file","id":"vs_example","item_id":"file_example"}}
{"tool":"chatgpt.knowledge","input":{"action":"search","id":"vs_example","parameters":{"query":"How does the provider connection work?"}}}
```

Import a Gemini uploaded file into a store and poll the returned operation:

```json
{"tool":"gemini.knowledge","input":{"action":"import_file","id":"fileSearchStores/example","parameters":{"fileName":"files/example"}}}
{"tool":"gemini.operations","input":{"action":"wait","id":"fileSearchStores/example/operations/import_example","wait_seconds":20}}
```

Run a background Responses request using an explicitly selected available model:

```json
{"tool":"chatgpt.responses","input":{"action":"create","model":"<available OpenAI model>","prompt":"Research this question","parameters":{"background":true,"tools":[{"type":"web_search"}]}}}
{"tool":"chatgpt.responses","input":{"action":"wait","id":"resp_example","wait_seconds":20}}
```

Generate Gemini speech without adding connection settings:

```json
{"tool":"gemini.speech","input":{"model":"<available Gemini TTS model>","prompt":"Hello!","parameters":{"generation_config":{"speech_config":[{"voice":"Kore"}]}},"output_path":"speech.wav"}}
```

## Realtime voice

`chatgpt.realtime` and `gemini.realtime` perform one bounded WebSocket voice turn using text and/or an explicit raw signed 16-bit little-endian mono PCM file. OpenAI input is 24 kHz; Gemini permits 16 or 24 kHz. They wait for the provider's setup acknowledgement before sending input, save returned PCM as WAV, and retain partial received audio/transcripts when the receive deadline expires. `duration_seconds` is 1–50 seconds.

These tools do not access a microphone, run background audio capture, or execute returned local tool calls. WAV/MP3 input must be explicitly converted to raw PCM before using the realtime tool. A timed-out request may already have incurred charges and must not be automatically repeated.

## CLIProxyAPI gateway

The companion CLIProxyAPI implementation adds authenticated resource forwarding for these workflows, Gemini model embedding/batch actions, resumable Google Files/File Search uploads, Gemini Live, and opt-in native OpenAI Realtime WebSocket forwarding. Resource requests reuse the gateway's existing native API credentials, proxy settings, and final payload policy; gateway credentials are not forwarded upstream.

`X-Agena-Provider-API` disambiguates vendor protocols sharing `/v1` paths. Native OpenAI speech and Realtime forwarding are opt-in with `X-Agena-Provider-API: chatgpt`, so existing xAI speech and Codex OAuth realtime clients retain their dispatch. Native Realtime honors the configured OpenAI API-key endpoint and proxy, and applies final payload policy to session/response configuration frames. SSE resource forwarding preserves `Last-Event-ID`. Existing completion/Images/Interactions routes retain their normal model dispatch.

A chat subscription/OAuth account does not automatically provide Files, vector-store, Skills, platform, or native Live access. If no eligible native credential exists, the new resource routes return an explicit unsupported error. This implementation does **not** add multi-account resource ownership, sticky routing, failover, or credential affinity. Use a single eligible upstream credential for workflows requiring a resource to stay on the account that created it.


## Additional resource workflows

OpenAI voices and consent recordings do not need a model. Create a consent recording before creating a voice from an audio sample:

```json
{"tool":"chatgpt.platform","input":{"resource":"audio/voice_consents","action":"create","path":"consent.wav","parameters":{"name":"Speaker consent","language":"zh"}}}
{"tool":"chatgpt.platform","input":{"resource":"audio/voices","action":"create","path":"sample.wav","parameters":{"name":"Speaker","consent":"cons_example"}}}
```

`audio/voice_consents` supports create/list/get/update/delete; `audio/voices` supports create/delete. Prompt-based voices use `parameters.type=prompt`, `parameters.name` and `parameters.prompt`, without a file. Vendor account access and the voice's supported output APIs still apply.

Download an OpenAI agent artifact, inspect a Claude session resource, or list its threads:

```json
{"tool":"chatgpt.agents","input":{"resource":"sessions/artifacts","action":"download","id":"session_example","item_id":"artifact_example","output_path":"result.csv"}}
{"tool":"claude.agents","input":{"resource":"sessions/resources","action":"get","id":"session_example","item_id":"resource_example"}}
{"tool":"claude.agents","input":{"resource":"sessions/threads","action":"list","id":"session_example"}}
```

OpenAI also supports `sessions/turns` and `sessions/subagents` list/get/items, `sessions/subagents/turns` list/get/items, and `sessions/traces` list. Subagent turns use `id=session`, `item_id=subagent`, `sub_item_id=turn`. Claude session resources support create/list/get/update/delete; threads support list/get/archive/events/stream_events. `deployment_runs` supports list/get. Required Claude beta names belong in `beta_headers`.

OpenAI `platform` collections also include `fine_tuning/jobs/checkpoints` (list), `fine_tuning/checkpoints/permissions` (create/list/delete), `fine_tuning/alpha/graders` (run/validate), and `evals/runs/output_items` (list/get). Eval outputs use `id=eval`, `item_id=run`, and `sub_item_id=output` for get. `evals` also supports `delete_run`. Checkpoint model IDs containing colons are safely encoded as a single segment.

Use Gemini synchronous batch embeddings or asynchronous embedding batches:

```json
{"tool":"gemini.embeddings","input":{"action":"create_batch","model":"<available Google embedding model>","parameters":{"requests":[{"content":{"parts":[{"text":"First document"}]}},{"content":{"parts":[{"text":"Second document"}]}}]}}}
{"tool":"gemini.batches","input":{"action":"create","resource":"embeddings","model":"<available Google embedding model>","parameters":{"batch":{"displayName":"Embedding dataset","inputConfig":{"fileName":"files/example"}}}}}
```

Synchronous entries inherit the selected model if absent; an explicit top-level model overrides their models. Gemini `batches.update` selects `updateGenerateContentBatch` by default or `updateEmbedContentBatch` with `resource=embeddings`. Supply the official update body in `parameters`, the selected top-level model, and optional `query.updateMask`.

Upload directly to a File Search store and poll the returned operation:

```json
{"tool":"gemini.knowledge","input":{"action":"upload","id":"fileSearchStores/example","path":"document.txt","parameters":{"displayName":"Project documentation","customMetadata":[{"key":"source","stringValue":"project"}]}}}
{"tool":"gemini.operations","input":{"action":"wait","id":"fileSearchStores/example/upload/operations/upload_example"}}
```

The upload uses the current provider origin for both initialization and finalization, and checks network permission for each target. `parameters` can also include `mimeType` and `chunkingConfig`. The separate Files-then-import workflow remains available.

## Remaining boundaries

The realtime tools still perform a bounded voice turn, rather than holding a persistent session for video input, reconnect/resume, or tool-result continuation. Responses WebSocket inference is not exposed by these plugin tools. Managed-agent automations/templates/vaults and Claude memory stores/dreams are not included in the resource allowlist. Administrative API keys, organization membership and billing APIs are outside these tools.

## Official references

- [OpenAI API reference](https://developers.openai.com/api/reference/overview), [Agents API](https://developers.openai.com/api/docs/guides/agents-api/overview)
- [Claude API reference](https://platform.claude.com/docs/en/api/overview), [Skills](https://platform.claude.com/docs/en/build-with-claude/skills-guide)
- [Gemini Files](https://ai.google.dev/api/files), [File Search](https://ai.google.dev/gemini-api/docs/file-search), [Live](https://ai.google.dev/api/live), [Interactions](https://ai.google.dev/gemini-api/docs/interactions-overview)
