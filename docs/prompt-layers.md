# Layered prompt fragments

Agena assembles its system prompt as an ordered list of fragments. The prompt
library shows three required built-ins for identity and shared rules, tool
protocol, and communication and delivery. Every session receives these
fragments. The identity section covers evidence, authorization, and context.
Live capability guidance applies when the run exposes tools.
The required groups can be moved within the order and have read-only bodies.
Legacy built-in IDs still resolve to their containing group. When a layer
lists multiple IDs for one group, the last occurrence sets its position.

The built-in library also supplies reusable role instructions under
`agena.profile.*`. These read-only fragments take effect when a layer
references them. They can also be used as parts of a composition.

| ID | Title |
| --- | --- |
| `agena.profile.investigation` | Investigation and evidence |
| `agena.profile.implementation` | Implementation and verification |
| `agena.profile.read-only` | Read-only work |
| `agena.profile.planning` | Planning and acceptance criteria |
| `agena.profile.verification` | Verification and evidence |

The built-in agent profiles reference these fragments in their own prompt order.

Saved prompts and agent profile instructions use the same ordered composition.
Use a layer to select reusable prompts, define inline fragments, or control
which optional instructions are inherited.

## Saved prompt entries

Reusable entries live in the `prompts.library` array of the global or workspace
configuration. An entry has an ID, instructions, and optional title, category,
model, and tool conditions. Editing an entry updates future runs that reference
it. When both scopes contain the same ID, the workspace entry takes precedence.
Use `order` to attach a saved entry to a layer.

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

File-backed `prompts/<id>.md` fragments and inline fragments also participate in
resolution. The saved prompt picker lists JSON-backed entries.

Use `prompts` for session, profile, and provider-model instructions. To move
text from `additional_system_prompt`, save it as a prompt fragment and add its
id to the applicable prompt layer.

## Data model

### `PromptLayer`

| Field | Meaning |
| --- | --- |
| `mode` | `append` (default) keeps fragments from broader scopes. `replace` clears inherited optional fragments before applying this layer. |
| `order` | Fragment IDs in render order. Listing a built-in ID sets its position alongside the selected prompts. |
| `disabled` | Optional inherited fragment IDs hidden by this layer. |
| `inline` | Fragments defined directly in this layer. |

Global and workspace `PromptScopeConfig` also include an independent
`library` array of saved `PromptFragment` entries. The entries are available
for referencing from narrower layers, but do not execute until selected in
`order`. Session layers contain references in `order`, not a copied body.

A layer rejects duplicate IDs in `order` or `inline`, and an ID that appears in
both `disabled` and `order` or `inline`. The `agena.system.*` namespace is
reserved for the server catalog. Those IDs can only appear in `order`.

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

The resolver starts with the applicable built-ins in catalog order. It then
processes layers from broadest to narrowest.

1. Global layer, using the global fragment library.
2. Workspace layer, using the workspace and inherited global libraries.
3. Matching global `prompts.models` overlays, in configuration order, using the
   global fragment library.
4. Matching workspace `prompts.models` overlays, in configuration order, using
   the workspace and inherited global libraries.
5. Selected [agent profile](agent-profiles.md) layers, from parent to child.
   These use the global/workspace libraries and can reference inline fragments
   declared by ancestor profiles. Selecting a profile also supplies its model,
   permission defaults, and turn limit.
6. Session layer. Its ordered entries may refer to saved global or workspace
   fragments, or to legacy session-local inline fragments.
7. AI Bot layer. A Bot's order can position built-ins, selected global prompts,
   and its own instructions where the Bot persona applies.
8. Request layer. Explicit per-run system instructions replace accumulated
   optional selections for that request. The Bot's own instructions still apply
   in its main conversation. Omission adds no layer.

Each layer applies the following steps.

1. A `replace` layer clears accumulated optional fragments. The applicable
   built-ins keep the order established so far.
2. Each ID in `disabled` is recorded and removed from the resolved fragments.
3. Each ID in `order` is processed in sequence. A built-in is moved to the end
   if it applies to the run. A custom fragment is resolved from inline definitions
   or the scoped library. Suppressed fragments and fragments whose conditions
   fail are skipped.
4. After variable substitution, each selected custom fragment replaces any earlier
   occurrence with the same ID and goes at the end.

A layer can interleave selected custom fragments with all applicable built-ins by
listing their IDs in the desired order. Unlisted built-ins keep their relative
order. The runtime adds a **Session relationship** fragment before optional
instructions. Delegated subagent sessions receive the parent-agent workflow.
Root sessions, user-created children, forks, and rewinds receive the
user-facing workflow. The runtime also adds active profile context,
conversation and project guidance, and an agent roster where applicable.
Session previews include these sections in the same order used to prepare a
model request. Per-run options and plugin transforms are applied at execution
time.

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
`[Additional instructions could not be assembled: ...]` marker, visible in the
session preview alongside the required built-ins.

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
    "order": ["agena.system.identity", "house-style", "tool-policy", "agena.system.tools"],
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
source session configuration, including its layer and selected agent profile.
Ordinary child sessions inherit the parent's saved profile. Delegated agents use
the profile chosen by `agent.spawn`, or the configured subagent default, and do
not copy the parent's role instructions. Their session-local layer starts empty
unless the creation request supplies `prompts`. Unselected sessions resolve the
default role for their relation. Profile selection and its create/update semantics are documented in
[Agent profiles](agent-profiles.md).

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

In the TUI, open **Runtime & Session → System prompt** while a session is
current or selected. `Tab` switches between global, workspace, and session
scopes.

| Key | Action |
| --- | --- |
| `a` | Add an inline fragment and edit its body. |
| `Enter` | Read the selected prompt. |
| `e` | Edit the selected inline body. |
| `t` | Edit the selected inline title. |
| `d` | Remove the selected fragment from this layer. |
| `[` / `]`, or `Ctrl+Up` / `Ctrl+Down` | Reorder the selected fragment. |
| `l` | Browse built-in and saved prompts in the library. |
| `o` | Choose a saved prompt by title and add it to `order`. |
| `h` / `u` | Choose an inherited prompt to hide or restore. |
| `i` | Rename an inline fragment and update layer references. |
| `m` | Toggle `append` / `replace`. |
| `p` | Load the saved-state preview. |
| `x` | Clear the layer draft. |
| `Ctrl+S` | Save the scope. |
| `Esc` | Close. |

In the Web client, open **Settings → AI & sessions → System prompt** and choose
**Global layer**, **Workspace layer**, or **Current session**. Use **Add prompt**
to select from the library. The arrows move items up or down. A lock marks the
required entries. The editor shows the built-ins and prompts selected at that
scope. **Preview saved prompt** shows the session's full resolved order, including
inherited prompts, model rules, and runtime context.

In the Web client, open **Settings → AI & sessions → Prompt library** to create,
edit, or categorize reusable prompts. Choose **Global layer** or **Workspace
layer** in the page toolbar. Select **Built-in prompts** to browse the server
catalog. The category buttons filter the list. The toolbar creates fragments and compositions in the
selected category, with **Uncategorized** as the destination from **All
categories**. The selected category's add button creates a fragment in that
category. A composition lets you order existing prompts and preview the
combined text before saving.

Open **Settings → AI & sessions → Agent profiles** to edit a role’s instructions
and attach saved prompts under **Instructions**. The global and workspace
`prompts` settings are also available in **Advanced settings**.

## AI Bot prompts

Open an AI Bot's settings in the Web client, or select **System prompt** in the
TUI Bot form. The ordered list can mix required built-ins, reusable global
library entries, and the Bot's own additional instructions. Existing Bot
instruction text appears as one editable fragment. Select a row to read it or
edit the Bot's own text, and use the movement controls to change its position.
The resolved **System prompt** preview for the Bot's conversation shows how the
order combines with its profile and current model.

The Bot API stores the explicit order in `persona.prompts.ids`. The ID
`agena.bot.instructions` positions the literal `persona.instructions` body.
An existing body without a position follows the selected references. The
channel option applies the selected library references and explicit order to
connected channels. Bot-owned instructions follow the Bot's existing main
conversation rules.

## Migrating from `additional_system_prompt`

To move existing instructions into prompt layers:

1. Put each reusable instruction in the appropriate `prompts/<id>.md` file.
2. Add its id to that scope's `order`.
3. Use an inline session fragment only for instructions that belong to one
   session.
4. Remove the retired settings from the configuration files.

The old model replaced lower scopes. The new default is composition; use
`disabled` to hide one inherited fragment or `replace` to deliberately discard
all broader configured fragments.
