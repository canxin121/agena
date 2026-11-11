# Layered prompt fragments

Agena keeps its built-in identity, tool, presentation, workflow, conversation,
and project guidance separate from user-configured instructions. Prompt layers
add ordered fragments after that built-in text. A layer can append fragments,
discard broader configured fragments, hide inherited fragment ids, and define
inline fragments, but it does not replace the built-in prompt itself.

## Saved prompt library (Web settings)

**Settings → Runtime & Session → Saved prompt library** is the reusable prompt
catalog. Global and workspace catalogs have independent CRUD editors; entries
have a title, free-form category, body, optional model-route and tool conditions,
and a generated internal id. A saved prompt is **not** automatically included in
any system prompt. Editing its body updates all future resolutions that
reference it rather than copying instructions into each session.

**Settings → Runtime & Session → Prompt layers** is the attachment editor. Select
a saved prompt by title to append its id to the chosen global, workspace, or
session layer. It supports ordering, hiding inherited entries, append/replace,
and resolved previews. The TUI `o`, `h`, and `u` shortcuts also present
saved prompts by name. The global layer can select global saved prompts;
workspace and session layers can select global and workspace prompts. A
workspace entry with the same id takes precedence where both scopes are
available. Existing inline fragments remain editable for compatibility.

The session layer is edited on the same page as the other layers: choose the
**current session** scope. It always edits the conversation this client used
last, which is recorded whenever any pane or bot binding opens one, so the
layer selector names that conversation instead of asking for one. There is no
separate session settings surface and no `?sessionId=`: the last used
conversation is the target, so permissions and instructions for a conversation
are reached from one place.

Saved entries live in the `library` array of the existing `prompts`
configuration, separate from `order` and `inline`:

```json
{
  "prompts": {
    "library": [
      {
        "id": "saved-global-1",
        "title": "Research",
        "category": "research",
        "body": "Verify important claims before answering."
      }
    ],
    "order": ["saved-global-1"]
  }
}
```

Legacy `prompts/<id>.md` fragments and the older inline fragments are still
resolved and editable through their existing mechanisms. The Web library page
manages JSON-backed entries; file-backed Markdown fragments remain managed as
files. The picker does not currently enumerate those Markdown files.

The previous `additional_system_prompt` setting has been retired. Configuration
files and the session API have no compatibility alias for it, and session
updates reject it as an unknown field. On startup, SQLite initialization does
migrate the retired field inside each stored session's `configuration_json`:
a string becomes one inline fragment named `legacy-session-prompt`, an empty or
whitespace-only string becomes an empty `replace` layer, `null` removes the key,
and any other JSON type aborts initialization with a diagnostic. If the session
already has a non-null `prompts` layer, that layer is preserved. Provider-model
`additional_system_prompt` entries are configuration, not session storage, and
must be removed manually.

## Data model

### `PromptLayer`

| Field | Meaning |
| --- | --- |
| `mode` | `append` (default) keeps fragments from broader scopes. `replace` first discards every fragment resolved by earlier scopes, then adds this layer. |
| `order` | Fragment ids in render order. |
| `disabled` | Inherited fragment ids hidden by this layer. |
| `inline` | Fragments defined directly in this layer. |

Global and workspace `PromptScopeConfig` also include an independent
`library` array of saved `PromptFragment` entries. The entries are available
for referencing from narrower layers, but do not execute until selected in
`order`. Session layers contain references in `order`, not a copied body.

A layer rejects duplicate ids in `order`, duplicate ids in `inline`, and an id
that appears in both `disabled` and `order` or `inline`.

### `PromptFragment`

| Field | Meaning |
| --- | --- |
| `id` | 1–64 ASCII characters from `a-z`, `0-9`, `.`, `_`, and `-`; the first character cannot be `.`. |
| `title` | Human-facing preview/editor metadata. It is not inserted into the prompt text. |
| `category` | Optional organizational label in the saved prompt library. Not inserted into prompt text. |
| `body` | Instruction text. Supports variable substitution and has a 16 KiB limit. |
| `when` | Optional model-route and tool-availability condition. |

### `PromptCondition`

| Field | Meaning |
| --- | --- |
| `models` | Route patterns. Any matching pattern includes the fragment; an empty list means no route restriction. |
| `requires_tools` | `true` includes the fragment only when tools are available; `false` includes it only when tools are unavailable. Omit it for no tool condition. |

Both conditions must hold when both are present. Conditions are evaluated after
the fragment is found and before its body is rendered.

## Resolution order

The resolver starts with the broadest layer and appends toward the narrowest:

1. Global layer, using the global fragment library.
2. Workspace layer, using the workspace and inherited global libraries.
3. Matching global `prompts.models` overlays, in configuration order, using the
   global fragment library.
4. Matching workspace `prompts.models` overlays, in configuration order, using
   the workspace and inherited global libraries.
5. Session layer. Its ordered entries may refer to saved global or workspace
   fragments, or to legacy session-local inline fragments.
6. Request layer. This is an internal resolver scope; the current runtime always
   resolves it as empty.

Each layer is processed in this order:

1. A `replace` mode clears every configured fragment accumulated from earlier
   scopes. It does not remove the built-in prompt, conversation instructions, or
   project guidance.
2. Every id in `disabled` is added to a suppression set and removed from the
   resolved fragments.
3. Every id in `order` is resolved from that layer's inline fragments first and
   its scoped `.md` library second. Suppressed ids are skipped. A fragment whose
   condition does not match is skipped with a warning.
4. Variables are substituted, then the fragment replaces any earlier fragment
   with the same id and is appended at its new position.

Each resolved fragment body is trimmed before assembly. Fragments whose trimmed
body is empty are omitted from the composed text, while
`PromptResolution.fragments` still retains their provenance. The remaining
bodies are joined with one blank line between fragments.

### Overrides and hiding

- A narrower layer that orders the same id replaces the broader fragment body
  and moves it to the narrower layer's position.
- Within one layer, an inline fragment wins over a file with the same id.
- A `disabled` entry suppresses that id for the whole resolution, including all
  narrower layers and later `replace` layers. A narrower layer cannot re-enable
  it; remove the `disabled` entry from the layer that added it.
- A `replace` layer only discards fragments resolved so far. It does not clear
  the suppression set, and later layers can still append their own fragments.
- A layer resolves ids from its own inline fragments first, then its available
  saved prompts and file library. Workspace and session layers can reference
  broader saved prompts, with workspace ids taking precedence where shared.
- A model overlay resolves ids from its own inline fragments and the libraries
  available to its configuration scope.

Conditions that do not match and unknown variables produce warnings. Warnings
are visible in previews and logged by the server, but do not prevent assembly.
Structural errors such as an invalid id, a missing ordered fragment, or an
oversized body are reported instead of silently dropping instructions. During
execution the assembled prompt contains an explicit
`[Additional instructions could not be assembled: ...]` marker; preview requests
return the error.

## Route patterns and variables

`when.models` and a model overlay's `match` use the route form
`provider/adapter/model`, or the short `provider/model` form when the adapter is
omitted. Only `*` is special; it matches any run of characters. Patterns are
trimmed, and an empty pattern never matches.

Examples:

- `anthropic/*` matches every model on the `anthropic` provider.
- `local/openai_responses/deepseek/*` matches that full route.
- `openai/gpt-*` matches the short provider/model form.

Fragment bodies substitute these case-sensitive variables:

| Variable | Value |
| --- | --- |
| `{{model}}` | Model id. |
| `{{provider}}` | Provider id. |
| `{{adapter}}` | Adapter id, or an empty string when the route has no adapter. |
| `{{workspace}}` | Effective workspace root. |
| `{{platform}}` | Operating-system family, such as `macos`, `linux`, or `windows`. |
| `{{session_id}}` | Numeric session id. |

Whitespace inside the braces is ignored, so `{{ model }}` works too. Unknown
variables are preserved verbatim and reported in the preview warnings.

## Configuration and fragment files

Global and workspace configuration files use the same top-level `prompts` key.
The loader keeps the two layers separate and composes them rather than letting
the workspace file replace the global layer.

| Scope | Configuration | Fragment directory |
| --- | --- | --- |
| Global | `prompts` in the global `agena.json` | The `prompts/` directory beside that file. |
| Workspace | `prompts` in `<workspace>/.agena/agena.json` | `<workspace>/.agena/prompts/` |

The default data root is `~/agena`, so the default global configuration is
`~/agena/agena.json` and its fragments live in `~/agena/prompts/`. `--data-dir`
and `AGENA_DATA_DIR` move the data root and therefore the global configuration
and fragment directory together.

Example global or workspace layer:

```json
{
  "prompts": {
    "mode": "append",
    "order": ["house-style", "tool-policy"],
    "disabled": ["legacy-verbose"],
    "inline": [],
    "models": [
      {
        "match": "anthropic/*",
        "mode": "append",
        "order": ["anthropic-notes"],
        "disabled": [],
        "inline": []
      }
    ]
  }
}
```

`prompts.models` entries are route-scoped layers inside the global or workspace
configuration. They are not provider/model catalog settings. The CLI, TUI, and
Web layer editors preserve existing `models` entries when saving the normal
layer; editing those overlays currently requires editing the configuration file
directly. `agena prompts clear` is the exception: it deletes the whole
`prompts` key, including model overlays.

Each fragment is a Markdown file whose filename stem is the fragment id. Only
`*.md` files are loaded, and a missing fragment directory is treated as empty.
Optional YAML front matter supplies metadata; the remaining text is the body.

```markdown
---
title: House style
when:
  models:
    - "anthropic/*"
  requires_tools: true
---
Prefer concrete examples. Call out tradeoffs before recommending a default.
```

A referenced `order` id that has neither a matching file nor an inline fragment
fails configuration loading with the expected path or scope in the error.
Unreadable fragment files and invalid front matter also fail loading.

The session layer is stored in the session database rather than in a fragment
directory. It can reference saved prompts and Markdown fragments in the global
or workspace library without copying their bodies, as well as legacy inline
session fragments.

## Size budgets

- Each fragment body is limited to 16 KiB (16,384 UTF-8 bytes), measured before
  variable substitution. Validation fails with the fragment id and actual byte
  count.
- The resolved configured text has a 128 KiB display/overflow budget. The
  resolver reports `over_budget: true` and the preview UI shows the size and an
  over-budget warning, but this value alone does not reject a save or abort
  assembly.
- The execution path estimates the assembled system prompt (built-in text plus
  the resolved fragments) against the selected model's prompt-token budget. If
  that system prompt alone exceeds the budget, the run fails and the error names
  up to three largest contributing fragments as `id (scope), N bytes`. Shorten
  or disable those fragments.

## Preview

`GET /api/v1/sessions/{session_id}/prompts/preview` resolves the configured
layers for a saved session and returns:

```json
{
  "text": "Resolved instruction text",
  "fragments": [
    {
      "id": "house-style",
      "title": "House style",
      "scope": "global",
      "position": 0,
      "bytes": 42,
      "sha256": "...",
      "text": "Prefer concrete examples."
    }
  ],
  "warnings": [],
  "total_bytes": 42,
  "over_budget": false
}
```

The preview uses the session's saved layer and its resolved default model route.
It sets `tools_enabled` from that route's tool mode, defaulting to enabled. It
does not include the built-in prompt, the internal request layer, or unsaved
editor drafts. A real run can therefore differ when it selects another route,
disables tools, or is native compaction (which resolves with
`tools_enabled: false`).

## Session API

Session creation accepts a top-level `prompts` layer in
`POST /api/v1/sessions`. `PUT /api/v1/sessions/{session_id}` uses these update
states:

| Request body | Effect |
| --- | --- |
| `prompts` omitted or `null` | Keep the stored session layer. |
| `{"prompts": {}}` | Replace it with an empty layer, resuming inheritance from broader scopes. |
| `{"prompts": {"mode": "append", "order": [], "disabled": [], "inline": []}}` | Same as an empty layer. |
| A populated object | Replace the stored session layer. |

`GET /api/v1/sessions/{session_id}/state` exposes the stored layer under
`execution.prompts` when it is non-empty. Forking and rewinding preserve the
source session configuration, including its layer. Newly created child and
delegated-agent sessions start with an empty layer unless their creation request
supplies `prompts`.

## CLI

Prompt commands are clients of the running long-lived server and print JSON by
default (`--format json` is currently the only format).

```sh
agena prompts list
agena prompts list --scope workspace
agena prompts list --scope session --session-id 7
agena prompts preview 7

agena prompts add --id house-style --title "House style" --body "Prefer examples."
agena prompts add --id route-notes --file notes.md --model "anthropic/*" \
  --requires-tools true

agena prompts edit --id house-style --body "Prefer short examples."
agena prompts edit --id house-style --new-id team-style --title "Team style"
agena prompts edit --id route-notes --clear-models --requires-tools inherit

agena prompts remove --id route-notes
agena prompts move --id house-style --up
agena prompts move --id house-style --position 1
agena prompts disable --id legacy-verbose
agena prompts enable --id legacy-verbose
agena prompts mode replace
agena prompts set --file layer.json
agena prompts clear
```

`--scope` is `global` (the default), `workspace`, or `session`. Session commands
require `--session-id`; the other scopes reject it. `preview` takes the session
id as a positional argument and has no `--scope`.

| Subcommand | Behavior |
| --- | --- |
| `list` | Print the stored layer for one scope. |
| `preview <session-id>` | Print the resolved prompt, provenance, warnings, and size. |
| `add --id <id>` | Add an inline fragment and append it to `order`. Accepts `--title`, `--body`, `--file`, repeatable `--model`, and `--requires-tools`. |
| `edit --id <id>` | Edit an inline fragment. Accepts `--new-id`, `--title`, `--body`, `--file`, repeatable `--model`, `--clear-models`, and `--requires-tools`. |
| `remove --id <id>` | Remove the fragment's references from this layer and safe model-overlay references. |
| `move --id <id>` | Move in `order` with `--up`, `--down`, or a one-based `--position`. |
| `disable --id <id>` / `enable --id <id>` | Add or remove an inherited fragment id from `disabled`. |
| `mode append\|replace` | Set this layer's combination mode. |
| `set --file <path>` | Replace the layer with a JSON `PromptLayer` object. Use `-` to read stdin. Global and workspace `models` overlays are preserved. |
| `clear` | Clear the active layer and model overlays. A global/workspace setting with no saved entries is deleted; saved library entries are retained. For sessions, use an empty layer. |

`add`, `edit`, and `set` accept `--file -` to read UTF-8 text from standard
input. `--file` passes raw body text for `add` and `edit`; it does not parse
fragment front matter. `set` expects only `mode`, `order`, `disabled`, and
`inline`; a full scope file containing `models` must be written through the
configuration editor or edited directly.

## TUI and Web editors

In the TUI, open **Runtime & Session → Prompt layers** while a session is
current or selected. `Tab` switches between global, workspace, and session
scopes.

| Key | Action |
| --- | --- |
| `a` | Add an inline fragment and edit its body. |
| `e` / `Enter` | Edit the selected inline body. |
| `t` | Edit the selected inline title. |
| `d` | Remove the selected fragment from this layer. |
| `Ctrl+Up` / `Ctrl+Down` | Reorder the selected fragment. |
| `o` | Choose a saved prompt by title and add it to `order`. |
| `h` / `u` | Choose an inherited prompt to hide or restore. |
| `i` | Rename an inline fragment and update layer references. |
| `m` | Toggle `append` / `replace`. |
| `p` | Load the saved-state preview. |
| `x` | Clear the layer draft. |
| `Ctrl+S` | Save the scope. |
| `Esc` | Close. |

The TUI preserves model overlays but does not edit fragment conditions or model
overlays. Its preview is the saved server state and excludes unsaved edits.

In the Web client, open **Settings → Runtime & Session → Prompt layers**. The
scope selector provides **Effective settings**, **Global layer**,
**Workspace layer**, and **Current session**. The editor supports layer mode,
inline id/title/body, model patterns, tool-availability conditions, ordering,
hiding inherited ids, save, and an effective preview with size, warnings, and
over-budget state. Provider Studio no longer contains a prompt editor. Web
previews and TUI previews exclude the built-in prompt and unsaved drafts.

## Migrating from `additional_system_prompt`

There is no compatibility alias. Stored session values are migrated once by the
SQLite initializer as described above; provider-model configuration values are
not migrated and must be removed manually. To move any remaining text:

1. Put each reusable instruction in the appropriate `prompts/<id>.md` file.
2. Add its id to that scope's `order`.
3. Use an inline session fragment only for instructions that belong to one
   session.
4. Remove any provider-model `additional_system_prompt` entries. Session
   database values do not need a manual step: the initializer performs the
   migration described at the top of this page.

The old model replaced lower scopes. The new default is composition; use
`disabled` to hide one inherited fragment or `replace` to deliberately discard
all broader configured fragments.
