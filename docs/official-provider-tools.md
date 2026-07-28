# Official provider-backed ordinary tools

Snapshot date: **2026-07-28**.

Agena still exposes only `tools_list`, `tools_search`, `tools_help`, `tools_tags`, and `tools_call` through the outer AI provider function protocol. Every entry below is an ordinary execution tool discovered and invoked through that gateway. The provider's official tool declaration is created only inside the selected ordinary tool implementation.

## Official source baselines

- OpenAI: `openai/openai-python`, current Responses `ToolParam` union and related request/output item types.
- Google: `googleapis/python-genai`, current Interactions `ToolParam` union and `function_result` continuation step.
- Anthropic: `anthropics/anthropic-sdk-python`, current Beta `BetaToolUnionParam` and latest versioned tool declarations.

The plugin schemas intentionally expose `tool_options` and `request_options` extension maps so newly added official fields can be used without reintroducing a second tool registry. Provider-controlled fields such as `type`, `model`, `input/messages`, and `tools` are protected from override.

## ChatGPT/OpenAI

`chatgpt.web_search`, `chatgpt.web_search_preview`, `chatgpt.file_search`, `chatgpt.computer`, `chatgpt.computer_use_preview`, `chatgpt.mcp`, `chatgpt.code_interpreter`, `chatgpt.programmatic_tool_calling`, `chatgpt.image_generation`, `chatgpt.local_shell`, `chatgpt.shell`, `chatgpt.tool_search`, `chatgpt.apply_patch`, `chatgpt.function`, `chatgpt.custom`, `chatgpt.namespace`, and the direct image endpoint convenience tool `chatgpt.image_edit`.

## Gemini

`gemini.code_execution`, `gemini.url_context`, `gemini.google_search`, `gemini.file_search`, `gemini.google_maps`, `gemini.computer_use`, `gemini.mcp_server`, `gemini.retrieval`, `gemini.function`, `gemini.image_generation`, and `gemini.image_edit`.

## Claude/Anthropic

`claude.bash`, `claude.code_execution`, `claude.computer`, `claude.memory`, `claude.text_editor`, `claude.web_search`, `claude.web_fetch`, `claude.advisor`, `claude.tool_search_bm25`, `claude.tool_search_regex`, and `claude.mcp_toolset`.

## Client-action continuation

Computer, shell, patch, editor, memory, generic function, tool-search, approval, and MCP calls can require a client action. The first invocation returns `response_id` plus normalized `pending_calls`. The caller executes those actions through the existing Agena permission-controlled tools and calls the same provider wrapper again using the official continuation field:

- OpenAI: `previous_response_id` plus official callback objects in `input_items`.
- Gemini: `previous_interaction_id` plus official `function_result` steps in `input_steps`.
- Claude: append the prior assistant content and user `tool_result` blocks to `messages`.

This preserves the provider's official protocol without making any of these tools outer provider functions.

## Downloaded source fingerprints

- OpenAI Responses ToolParam: `download-unavailable` (0 bytes)
- Google Gemini Interactions Tool: `download-unavailable` (0 bytes)
- Anthropic BetaToolUnionParam: `download-unavailable` (0 bytes)

The vendor files were used only as local reference inputs and are not included in the repository archive.
