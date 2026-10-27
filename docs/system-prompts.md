# Additional system prompts

Agena can append your own instructions to its built-in session prompt. Configure
one default for every project, a project default, a model override, or a session
override. The chosen value is added after the built-in prompt; conversation
boundaries and repository guidance are assembled separately.

## Inheritance and explicit disabling

The persistent scopes have this precedence, from highest to lowest:

1. Session.
2. Model, identified by provider, adapter, and model.
3. Project/workspace.
4. Global.

Each scope replaces lower-priority additional instructions. The scopes are not
concatenated. A request's existing `system` option is a temporary override above
these four scopes; it does not change the stored session setting.

| Value | Meaning |
| --- | --- |
| Missing or `null` | Inherit the next available value. |
| `""` | Explicitly disable inherited additional instructions. |
| A nonempty string | Use this text, preserving its formatting. |

For example, a global prompt can be enabled while a particular model has `""`.
That model receives no additional instructions unless its session or individual
request supplies a higher-priority override. Removing the model override restores
inheritance. Empty strings are resolved before whitespace is trimmed for prompt
assembly, so they cannot accidentally reactivate a lower-priority value.

Disabling additional instructions retains Agena's built-in role, tool, and
presentation guidance. The same resolution applies to ordinary main and subagent
requests, including requests with tools disabled, and to native compaction.
Independent title, permission-review, and summary tasks keep their own prompts.

## Web settings

Open **Settings → Runtime & Session → Additional system prompt**. Choose Global,
Workspace, or Current session, then select one of:

- **Inherit:** remove the override at this scope.
- **Custom text:** enter the additional instructions.
- **Disabled:** save an explicit empty override.

Use **Effective settings** to inspect the resolved additional instructions for
the selected session. The page displays the session ID and distinguishes an
explicitly disabled value from an unconfigured value. Built-in instructions are
not included in this preview.

For a model override, open the model in **Provider Studio** and use its
**Additional system prompt** editor. The setting belongs to that model route;
models with the same name under different providers or adapters remain separate.
An implicit adapter selection uses the same route resolution as model execution.

## Configuration files

Global and project files use the same field:

```json
{
  "session": {
    "additional_system_prompt": "Explain important tradeoffs and keep examples concise."
  }
}
```

Set the project field to `""` to override an inherited global default with no
additional text. Delete the field, or set it to `null`, to inherit again.

Model settings use
`providers.<provider>.adapters.<adapter>.models.<model>.additional_system_prompt`.
For example, the relevant portion of an existing provider configuration can be:

```json
{
  "providers": {
    "my-provider": {
      "adapters": {
        "openai_responses": {
          "models": {
            "my-model": {
              "additional_system_prompt": ""
            }
          }
        }
      }
    }
  }
}
```

Keep the provider's existing connection and authentication fields. A project
model entry that omits this field inherits the corresponding global model value,
even when that entry changes other model settings. Saving or refreshing a model's
catalog template preserves the configured additional instructions.

## Session API

The existing creation endpoint accepts `additional_system_prompt`. The existing
update endpoint, `PUT /api/v1/sessions/{id}`, distinguishes three update states:

| Request body | Effect |
| --- | --- |
| Field omitted | Keep the session's existing override. |
| `{"additional_system_prompt":null}` | Restore inheritance. |
| `{"additional_system_prompt":""}` | Disable inherited additional instructions. |
| `{"additional_system_prompt":"Use concise examples."}` | Set a session override. |

Prompt and metadata changes in one update are committed atomically. The existing
session-configuration size limit still applies. Older sessions without the field
inherit normally; no database migration is required.

`GET /api/v1/sessions/{id}/state` returns:

```json
{
  "execution": {
    "additional_system_prompt": null,
    "effective_additional_system_prompt": "Use concise examples."
  }
}
```

The first field is the stored session override. The second is the resolved
session/model/project/global value and excludes per-request `system` overrides.
Both preserve the distinction between `null` and `""`. Session lists do not carry
the full instruction text.

Fork and rewind preserve the source session's raw override, including inheritance
and explicit disabling. Newly created child sessions and delegated subagents do
not copy the parent's session override; they resolve their own model and defaults.

## CLI and TUI

```sh
# Inspect the stored and effective values.
agena sessions system-prompt 123

# Set text or load an exact UTF-8 file.
agena sessions system-prompt 123 --text "Use concise examples."
agena sessions system-prompt 123 --file prompt.txt

# Explicitly disable, or restore inheritance.
agena sessions system-prompt 123 --text ""
agena sessions system-prompt 123 --inherit
```

In the TUI's Runtime/Session settings, select **Session additional system prompt**
to open the multiline editor. Ctrl+S saves the exact text, including an empty
value; Enter inserts a newline. **Restore session prompt inheritance** removes the
session override.

Settings are resolved during request assembly rather than copied into the stored
session override. After configuration reload, newly submitted executions use the
updated defaults. Running or queued executions retain their existing runtime
configuration.
