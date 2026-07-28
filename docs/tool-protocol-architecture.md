# Agena Tool Protocol Architecture

Agena exposes exactly five provider-facing Tool API functions:

- `tools_list`
- `tools_search`
- `tools_help`
- `tools_tags`
- `tools_call`

Every other tool is an ordinary execution tool discovered and invoked through that gateway. Tools backed by official provider services, such as `chatgpt.web_search`, `chatgpt.image_generation`, `chatgpt.image_edit`, and the corresponding `gemini.*` / `claude.*` tools, have the same catalog, help, permission, Skill allowlist, and invocation semantics as all other Agena tools. Their provider-specific HTTP transport is an implementation detail.

There is no direct/deferred/hidden exposure tier and no conversation-level provider-native tool configuration. Saved model configuration containing `agena_tools.direct`, `agena_tools.provider_native`, `provider_tools`, `provider_native_tools`, or `native_tools` is rejected instead of being silently interpreted as a second tool surface.

## Official provider tool configuration

The bundled `agena.chatgpt`, `agena.gemini`, and `agena.claude` plugins read their credentials from environment variables and can be overridden through normal plugin configuration. For example, configure the ChatGPT plugin with:

```json
{
  "plugins": {
    "list": {
      "agena.chatgpt": {
        "enabled": true,
        "config": {
          "base_url": "https://api.openai.com/v1",
          "api_key_env": "OPENAI_API_KEY",
          "responses_model": "gpt-4.1",
          "image_model": "gpt-image-1"
        }
      }
    }
  }
}
```

The model never receives `chatgpt.*`, `gemini.*`, or `claude.*` tools as provider function declarations. It discovers them with `tools_list`/`tools_search`, reads their schemas with `tools_help`, and invokes them through `tools_call` like any other execution tool.
