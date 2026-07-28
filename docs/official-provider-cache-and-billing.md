# Official provider cache, usage, and billing accounting

Snapshot date: **2026-07-28**.

This implementation treats every `chatgpt.*`, `gemini.*`, and `claude.*` invocation as a distinct provider request. Its usage is attributed to the actual provider/model/operation and attached to the outer Agena assistant message as nested accounting. Session totals, provider/model breakdowns, subtask budgets, SQLite reports, TUI, and Web UI recursively expand those observations instead of charging the outer model.

## Source baselines

| Provider | Official SDK repository | Commit used |
| --- | --- | --- |
| OpenAI | `openai/openai-python` | `6ba31bcbb2df31fa1890f51877104133c0a0be60` (2.49.0) |
| Google | `googleapis/python-genai` | `e8714cafa739045481aedf639783f151e6a0d1e9` |
| Anthropic | `anthropics/anthropic-sdk-python` | `60c64fba5c2bf340567f627328e57cf0196b868f` (0.120.0) |

The machine-readable file `official-provider-cache-accounting-source-snapshot.json` records the exact SDK files and fields used.

## Normalized token categories

The categories counted by `CompletionUsage::own_total_tokens()` are mutually exclusive:

- `input_tokens`: uncached input, excluding cache reads/writes;
- `output_tokens`: visible output, excluding reasoning/thinking;
- `reasoning_tokens`: reasoning/thinking output;
- `cache_write_tokens`: input written to cache;
- `cache_read_tokens`: input read from cache;
- `other_tokens`: provider-authoritative total not explained by known categories.

`cache_write_5m_tokens`, `cache_write_1h_tokens`, and `tool_use_tokens` are informational breakdowns and are not added to the total a second time.

## Cost provenance

Costs are never silently treated as authoritative when only estimated:

1. provider-recorded cost wins when present;
2. otherwise configured Model Catalog pricing is used;
3. otherwise the dated built-in pricing snapshot is used;
4. non-token units are added only when an official list price is known;
5. account tier, free quota, negotiated pricing, runtime duration, storage, image parameters, or other unknown dimensions set `cost_estimate_incomplete` and remain visible as unpriced units.

The API reports `recorded_cost_usd`, `estimated_cost_usd`, `total_cost_usd`, and `unpriced_runs` separately. A list-price estimate is not represented as an invoice.

## OpenAI / ChatGPT

OpenAI usage normalization reads:

- `input_tokens` / `prompt_tokens`;
- `input_tokens_details.cached_tokens`;
- `input_tokens_details.cache_write_tokens`;
- `output_tokens` / `completion_tokens`;
- `output_tokens_details.reasoning_tokens`;
- `cost_in_usd_ticks` when a compatible endpoint provides it.

Caching behavior:

- cache disabled: no `prompt_cache_key` or cache options are sent;
- automatic: a stable workspace/provider/model/tool key is sent;
- GPT-5.6+ automatic mode uses implicit 30-minute cache options;
- GPT-5.6+ explicit mode requires a stable developer prefix and places an official explicit breakpoint on it;
- continuation requests use `previous_response_id` and may contain callback items without repeating the user prompt.

Cache writes use the official model-specific write rate in the pricing snapshot; GPT-5.6 cache writes use the 1.25× input multiplier and cache reads use the published cached-input rate.

## Gemini

Interactions usage normalization reads:

- `total_input_tokens`;
- `total_output_tokens`;
- `total_thought_tokens`;
- `total_cached_tokens`;
- `total_tool_use_tokens`;
- `total_tokens`;
- `grounding_tool_count`.

GenerateContent normalization reads the corresponding `usageMetadata` fields, including cached content and tool-use prompt tokens. Interactions continue with `previous_interaction_id`; GenerateContent image requests support official `cachedContent` references. Grounding counts are preserved as non-token billable units. List-price estimates vary by model generation, while free quota and account tier remain marked as incomplete.

## Claude / Anthropic

Anthropic normalization reads:

- `input_tokens` and `output_tokens`;
- `output_tokens_details.thinking_tokens`;
- `cache_read_input_tokens`;
- `cache_creation_input_tokens`;
- `cache_creation.ephemeral_5m_input_tokens`;
- `cache_creation.ephemeral_1h_input_tokens`;
- `server_tool_use.web_search_requests`;
- `server_tool_use.web_fetch_requests`;
- `server_tool_use.code_execution_requests` when returned.

The wrapper supports official top-level ephemeral cache control with 5-minute or 1-hour TTL. Estimates apply Anthropic's 1.25× and 2× cache-write multipliers and 0.1× cache-read multiplier. Code execution request count is recorded, but runtime cost remains unpriced because the response does not contain sufficient container-duration/free-allowance information.

## Provider response receipts

Full provider JSON is persisted under `.agena/tool-results/provider-tools` with binary values redacted and a SHA-256 receipt. Model-visible tool output contains only a compact result, sources, continuation IDs, pending calls, usage, and receipt metadata. This prevents the outer model from paying again to read large duplicated provider payloads.

## Compatibility

All new fields use serde defaults. Existing persisted usage JSON remains readable, and SQLite continues to store one JSON usage object per assistant message without a schema migration. The legacy field name `runs` remains in public aggregate DTOs for compatibility, but its meaning is now **provider requests**, including nested provider-backed tool requests.
