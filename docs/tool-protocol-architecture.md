# Agena Tool Protocol Architecture

Agena exposes exactly five provider-facing Tool API functions:

- `tools_list`
- `tools_search`
- `tools_help`
- `tools_tags`
- `tools_call`

Every other tool is an ordinary execution tool discovered and invoked through that gateway. Tools backed by official provider services, such as `openai.web_search` and `openai.image_generation`, have the same catalog, help, permission, and invocation semantics as all other Agena tools. Their provider-specific transport is an implementation detail.

There is no direct/deferred/hidden exposure tier and no conversation-level provider-native tool configuration.
